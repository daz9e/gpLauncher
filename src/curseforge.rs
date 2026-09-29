//! CurseForge API (api.curseforge.com). Every request needs an API key from console.curseforge.com.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::http;
use crate::instance::Loader;
use crate::modpack::{Filters, Pack, PackVersion, Page, Platform, Sort};

const API: &str = "https://api.curseforge.com/v1";
const MINECRAFT: &str = "432";
const MODPACKS_CLASS: &str = "4471";
/// The API refuses `index + pageSize` above this.
const MAX_RESULTS: u64 = 10_000;

fn get(key: &str, path: &str, query: &[(&str, String)]) -> Result<Value> {
    let url = format!("{API}{path}");
    let resp = http::agent()
        .get(&url)
        .header("x-api-key", key)
        .query_pairs(query.iter().map(|(k, v)| (*k, v.as_str())))
        .call()
        .with_context(|| format!("GET {url}"))?;
    read(resp, &url)
}

fn post(key: &str, path: &str, body: Value) -> Result<Value> {
    let url = format!("{API}{path}");
    let resp = http::agent()
        .post(&url)
        .header("x-api-key", key)
        .send_json(body)
        .with_context(|| format!("POST {url}"))?;
    read(resp, &url)
}

fn read(resp: ureq::http::Response<ureq::Body>, url: &str) -> Result<Value> {
    if resp.status() == 403 {
        anyhow::bail!("CurseForge rejected the API key");
    }
    http::read_json(resp, url)
}

pub fn search(key: &str, query: &str, filters: &Filters, offset: u64, limit: u64) -> Result<Page> {
    let limit = limit.min(MAX_RESULTS.saturating_sub(offset));
    // The API has no relevance order; popularity is what CurseForge's own site uses.
    let sort_field = match filters.sort {
        Sort::Relevance => "2",
        Sort::Downloads => "6",
        Sort::Updated => "3",
        Sort::Newest => "11",
    };
    let mut params = vec![
        ("gameId", MINECRAFT.to_string()),
        ("classId", MODPACKS_CLASS.into()),
        ("searchFilter", query.trim().into()),
        ("sortField", sort_field.into()),
        ("sortOrder", "desc".into()),
        ("index", offset.to_string()),
        ("pageSize", limit.to_string()),
    ];
    if let Some(version) = &filters.game_version {
        params.push(("gameVersion", version.clone()));
    }
    if let Some(loader) = filters.loader.and_then(loader_type) {
        params.push(("modLoaderType", loader.to_string()));
    }
    let v = get(key, "/mods/search", &params)?;
    let packs = v["data"].as_array().into_iter().flatten().map(pack).collect();
    let total = v["pagination"]["totalCount"].as_u64().unwrap_or(0).min(MAX_RESULTS);
    Ok(Page { packs, total })
}

/// CurseForge's `ModLoaderType` ids.
fn loader_type(loader: Loader) -> Option<u8> {
    match loader {
        Loader::Vanilla => None,
        Loader::Forge => Some(1),
        Loader::Fabric => Some(4),
        Loader::Quilt => Some(5),
        Loader::NeoForge => Some(6),
    }
}

fn pack(m: &Value) -> Pack {
    Pack {
        platform: Platform::CurseForge,
        id: m["id"].as_u64().unwrap_or(0).to_string(),
        title: m["name"].as_str().unwrap_or_default().to_string(),
        author: m["authors"][0]["name"].as_str().unwrap_or_default().to_string(),
        summary: m["summary"].as_str().unwrap_or_default().to_string(),
        downloads: m["downloadCount"].as_f64().unwrap_or(0.) as u64,
        icon_url: m["logo"]["thumbnailUrl"].as_str().filter(|s| !s.is_empty()).map(String::from),
        website: m["links"]["websiteUrl"].as_str().map(String::from),
    }
}

/// Client files of a modpack, newest first.
pub fn versions(key: &str, pack_id: &str) -> Result<Vec<PackVersion>> {
    let v = get(key, &format!("/mods/{pack_id}/files"), &[("pageSize", "50".into())])?;
    let mut out: Vec<(String, PackVersion)> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| !f["isServerPack"].as_bool().unwrap_or(false))
        .map(|f| {
            let tags: Vec<&str> =
                f["gameVersions"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let (game_versions, loaders) = tags
                .iter()
                .filter(|t| !matches!(**t, "Client" | "Server"))
                .partition::<Vec<&str>, _>(|t| t.starts_with(|c: char| c.is_ascii_digit()));
            let date = f["fileDate"].as_str().unwrap_or_default().to_string();
            let version = PackVersion {
                id: f["id"].as_u64().unwrap_or(0).to_string(),
                name: f["displayName"].as_str().unwrap_or_default().to_string(),
                game_versions: game_versions.into_iter().map(String::from).collect(),
                loaders: loaders.into_iter().map(String::from).collect(),
                url: f["downloadUrl"].as_str().map(String::from),
                file_name: f["fileName"].as_str().unwrap_or("modpack.zip").to_string(),
                sha1: sha1(f),
                size: f["fileLength"].as_u64(),
            };
            (date, version)
        })
        .collect();
    // ISO 8601 dates sort as strings.
    out.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(out.into_iter().map(|(_, v)| v).collect())
}

fn sha1(file: &Value) -> Option<String> {
    file["hashes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|h| h["algo"].as_u64() == Some(1))
        .and_then(|h| h["value"].as_str())
        .map(String::from)
}

/// A file listed in a modpack manifest, resolved to where it can be downloaded.
pub struct ModFile {
    pub file_name: String,
    /// `None` when the author does not allow launchers to download it.
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    /// Folder inside the game directory (`mods`, `resourcepacks`, ...).
    pub folder: &'static str,
    /// Page where the file can be downloaded by hand.
    pub page: String,
}

/// Resolves `(projectID, fileID)` pairs from a manifest.
pub fn resolve_files(key: &str, files: &[(u64, u64)]) -> Result<Vec<ModFile>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let file_ids: Vec<u64> = files.iter().map(|f| f.1).collect();
    let mod_ids: Vec<u64> = files.iter().map(|f| f.0).collect();
    let resolved = post(key, "/mods/files", json!({ "fileIds": file_ids }))?;
    let mods = post(key, "/mods", json!({ "modIds": mod_ids, "filterPcOnly": false }))?;

    let mods: HashMap<u64, &Value> =
        mods["data"].as_array().into_iter().flatten().filter_map(|m| Some((m["id"].as_u64()?, m))).collect();
    resolved["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let file_id = f["id"].as_u64().context("file without id")?;
            let project = mods.get(&f["modId"].as_u64().unwrap_or(0));
            let website = project
                .and_then(|m| m["links"]["websiteUrl"].as_str())
                .unwrap_or("https://www.curseforge.com");
            Ok(ModFile {
                file_name: f["fileName"].as_str().context("file without a name")?.to_string(),
                url: f["downloadUrl"].as_str().filter(|u| !u.is_empty()).map(String::from),
                sha1: sha1(f),
                size: f["fileLength"].as_u64(),
                folder: folder_for_class(project.and_then(|m| m["classId"].as_u64())),
                page: format!("{website}/files/{file_id}"),
            })
        })
        .collect()
}

fn folder_for_class(class: Option<u64>) -> &'static str {
    match class {
        Some(12) => "resourcepacks",
        Some(6552) => "shaderpacks",
        Some(17) => "saves",
        _ => "mods",
    }
}
