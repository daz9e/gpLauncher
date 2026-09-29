//! Modrinth API (api.modrinth.com), no key needed.

use anyhow::{Context, Result};
use serde_json::Value;

use crate::http;
use crate::modpack::{Filters, Pack, PackVersion, Page, Platform, Sort};

const API: &str = "https://api.modrinth.com/v2";

fn get(path: &str, query: &[(&str, String)]) -> Result<Value> {
    let url = format!("{API}{path}");
    let resp = http::agent()
        .get(&url)
        .query_pairs(query.iter().map(|(k, v)| (*k, v.as_str())))
        .call()
        .with_context(|| format!("GET {url}"))?;
    http::read_json(resp, &url)
}

pub fn search(query: &str, filters: &Filters, offset: u64, limit: u64) -> Result<Page> {
    let index = match filters.sort {
        Sort::Relevance if query.trim().is_empty() => "downloads",
        Sort::Relevance => "relevance",
        Sort::Downloads => "downloads",
        Sort::Updated => "updated",
        Sort::Newest => "newest",
    };
    // Facets are AND-ed lists of OR-ed conditions.
    let mut facets = vec![vec!["project_type:modpack".to_string()]];
    if let Some(version) = &filters.game_version {
        facets.push(vec![format!("versions:{version}")]);
    }
    if let Some(loader) = filters.loader {
        facets.push(vec![format!("categories:{}", loader.label().to_lowercase())]);
    }
    let v = get(
        "/search",
        &[
            ("query", query.trim().into()),
            ("facets", serde_json::to_string(&facets)?),
            ("index", index.into()),
            ("offset", offset.to_string()),
            ("limit", limit.to_string()),
        ],
    )?;
    let packs = v["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| Pack {
            platform: Platform::Modrinth,
            id: h["project_id"].as_str().unwrap_or_default().to_string(),
            title: h["title"].as_str().unwrap_or_default().to_string(),
            author: h["author"].as_str().unwrap_or_default().to_string(),
            summary: h["description"].as_str().unwrap_or_default().to_string(),
            downloads: h["downloads"].as_u64().unwrap_or(0),
            icon_url: h["icon_url"].as_str().filter(|s| !s.is_empty()).map(String::from),
            website: h["slug"].as_str().map(|slug| format!("https://modrinth.com/modpack/{slug}")),
        })
        .collect();
    Ok(Page { packs, total: v["total_hits"].as_u64().unwrap_or(0) })
}

/// Versions of a modpack, newest first (the API already sorts them).
pub fn versions(pack_id: &str) -> Result<Vec<PackVersion>> {
    let v = get(&format!("/project/{pack_id}/version"), &[])?;
    let strings = |v: &Value| -> Vec<String> {
        v.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
    };
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|ver| {
            let files = ver["files"].as_array()?;
            let file = files.iter().find(|f| f["primary"].as_bool() == Some(true)).or(files.first())?;
            Some(PackVersion {
                id: ver["id"].as_str().unwrap_or_default().to_string(),
                name: ver["version_number"].as_str().or(ver["name"].as_str()).unwrap_or_default().to_string(),
                game_versions: strings(&ver["game_versions"]),
                loaders: strings(&ver["loaders"]).into_iter().map(|l| capitalize(&l)).collect(),
                url: file["url"].as_str().map(String::from),
                file_name: file["filename"].as_str().unwrap_or("modpack.mrpack").to_string(),
                sha1: file["hashes"]["sha1"].as_str().map(String::from),
                size: file["size"].as_u64(),
            })
        })
        .collect())
}

fn capitalize(s: &str) -> String {
    match s {
        "neoforge" => "NeoForge".into(),
        _ => s.chars().take(1).flat_map(char::to_uppercase).chain(s.chars().skip(1)).collect(),
    }
}
