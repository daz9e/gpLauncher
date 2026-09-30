//! Modrinth API (api.modrinth.com), no key needed.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::content::Kind;
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

// ---- mods, resource packs and shaders -------------------------------------------

/// A project that can be added to an instance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub author: String,
    pub summary: String,
    pub downloads: u64,
    pub icon_url: Option<String>,
}

impl Project {
    pub fn url(&self, kind: Kind) -> String {
        format!("https://modrinth.com/{}/{}", kind.modrinth_type(), self.slug)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    /// `required`, `optional`, `incompatible` or `embedded`.
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub number: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub url: String,
    pub file_name: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub dependencies: Vec<Dependency>,
}

#[derive(Serialize)]
pub struct ProjectPage {
    pub projects: Vec<Project>,
    pub total: u64,
}

/// Searches projects of `kind`. `loaders` are OR-ed; empty = any.
pub fn search_projects(
    kind: Kind,
    query: &str,
    game_version: Option<&str>,
    loaders: &[&str],
    sort: Sort,
    offset: u64,
    limit: u64,
) -> Result<ProjectPage> {
    let index = match sort {
        Sort::Relevance if query.trim().is_empty() => "downloads",
        Sort::Relevance => "relevance",
        Sort::Downloads => "downloads",
        Sort::Updated => "updated",
        Sort::Newest => "newest",
    };
    let mut facets = vec![vec![format!("project_type:{}", kind.modrinth_type())]];
    if let Some(version) = game_version {
        facets.push(vec![format!("versions:{version}")]);
    }
    if !loaders.is_empty() {
        facets.push(loaders.iter().map(|l| format!("categories:{l}")).collect());
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
    let projects = v["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| Project {
            id: str_field(h, "project_id"),
            slug: h["slug"].as_str().or(h["project_id"].as_str()).unwrap_or_default().to_string(),
            title: str_field(h, "title"),
            author: str_field(h, "author"),
            summary: str_field(h, "description"),
            downloads: h["downloads"].as_u64().unwrap_or(0),
            icon_url: h["icon_url"].as_str().filter(|s| !s.is_empty()).map(String::from),
        })
        .collect();
    Ok(ProjectPage { projects, total: v["total_hits"].as_u64().unwrap_or(0) })
}

/// Versions of a project that fit the loaders and game version (empty = any), newest first.
pub fn project_versions(project: &str, loaders: &[&str], game_version: Option<&str>) -> Result<Vec<Version>> {
    let mut query = Vec::new();
    if !loaders.is_empty() {
        query.push(("loaders", serde_json::to_string(loaders)?));
    }
    if let Some(g) = game_version {
        query.push(("game_versions", serde_json::to_string(&[g])?));
    }
    let v = get(&format!("/project/{project}/version"), &query)?;
    Ok(v.as_array().into_iter().flatten().filter_map(parse_version).collect())
}

pub fn version(id: &str) -> Result<Version> {
    parse_version(&get(&format!("/version/{id}"), &[])?).context("version has no files")
}

/// Versions the files with these SHA-1 hashes belong to, keyed by hash.
pub fn versions_by_hash(hashes: &[String]) -> Result<HashMap<String, Version>> {
    if hashes.is_empty() {
        return Ok(HashMap::new());
    }
    let v = post("/version_files", json!({ "hashes": hashes, "algorithm": "sha1" }))?;
    Ok(by_hash(&v))
}

/// Newest versions fitting the loaders and game version for files with these hashes.
pub fn latest_by_hash(
    hashes: &[String],
    loaders: &[&str],
    game_version: Option<&str>,
) -> Result<HashMap<String, Version>> {
    if hashes.is_empty() {
        return Ok(HashMap::new());
    }
    let mut body = json!({ "hashes": hashes, "algorithm": "sha1" });
    if !loaders.is_empty() {
        body["loaders"] = json!(loaders);
    }
    if let Some(g) = game_version {
        body["game_versions"] = json!([g]);
    }
    Ok(by_hash(&post("/version_files/update", body)?))
}

fn by_hash(v: &Value) -> HashMap<String, Version> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(hash, ver)| Some((hash.clone(), parse_version(ver)?)))
        .collect()
}

/// Projects by id, in no particular order.
pub fn projects(ids: &[String]) -> Result<Vec<Project>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let v = get("/projects", &[("ids", serde_json::to_string(ids)?)])?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|p| Project {
            id: str_field(p, "id"),
            slug: str_field(p, "slug"),
            title: str_field(p, "title"),
            author: String::new(),
            summary: str_field(p, "description"),
            downloads: p["downloads"].as_u64().unwrap_or(0),
            icon_url: p["icon_url"].as_str().filter(|s| !s.is_empty()).map(String::from),
        })
        .collect())
}

fn parse_version(v: &Value) -> Option<Version> {
    let files = v["files"].as_array()?;
    let file = files.iter().find(|f| f["primary"].as_bool() == Some(true)).or(files.first())?;
    let strings = |v: &Value| -> Vec<String> {
        v.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
    };
    Some(Version {
        id: str_field(v, "id"),
        project_id: str_field(v, "project_id"),
        name: str_field(v, "name"),
        number: str_field(v, "version_number"),
        game_versions: strings(&v["game_versions"]),
        loaders: strings(&v["loaders"]),
        url: file["url"].as_str()?.to_string(),
        file_name: file["filename"].as_str()?.to_string(),
        sha1: file["hashes"]["sha1"].as_str().map(String::from),
        size: file["size"].as_u64(),
        dependencies: v["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| Dependency {
                project_id: d["project_id"].as_str().map(String::from),
                version_id: d["version_id"].as_str().map(String::from),
                kind: str_field(d, "dependency_type"),
            })
            .collect(),
    })
}

fn str_field(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

fn post(path: &str, body: Value) -> Result<Value> {
    let url = format!("{API}{path}");
    let resp = http::agent().post(&url).send_json(body).with_context(|| format!("POST {url}"))?;
    http::read_json(resp, &url)
}
