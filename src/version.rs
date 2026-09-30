use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{http, platform};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Clone, Debug, Serialize)]
pub struct VersionEntry {
    pub id: String,
    /// `release`, `snapshot`, `old_beta`, `old_alpha` or `local` (e.g. Fabric/Forge profiles).
    pub kind: String,
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub installed: bool,
}

/// Mojang versions (newest first) followed by locally installed versions
/// that are not in the manifest (modloader profiles etc.).
/// Falls back to the cached manifest when offline.
pub fn list(dir: &Path) -> Result<Vec<VersionEntry>> {
    let cache = dir.join("versions/version_manifest_v2.json");
    let manifest = match http::get_json(MANIFEST_URL) {
        Ok(v) => {
            let _ = fs::create_dir_all(cache.parent().unwrap());
            let _ = fs::write(&cache, serde_json::to_vec(&v)?);
            v
        }
        Err(e) => match fs::read(&cache) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(_) => return Err(e.context("failed to fetch the version list")),
        },
    };

    let local = local_versions(dir);
    let mut out = Vec::new();
    let mut known = HashSet::new();
    for v in manifest["versions"].as_array().into_iter().flatten() {
        let id = v["id"].as_str().unwrap_or_default().to_string();
        known.insert(id.clone());
        out.push(VersionEntry {
            installed: local.contains(&id),
            id,
            kind: v["type"].as_str().unwrap_or("release").to_string(),
            url: v["url"].as_str().map(String::from),
            sha1: v["sha1"].as_str().map(String::from),
        });
    }
    let mut extra: Vec<_> = local.into_iter().filter(|id| !known.contains(id)).collect();
    extra.sort();
    out.extend(extra.into_iter().map(|id| VersionEntry {
        id,
        kind: "local".into(),
        url: None,
        sha1: None,
        installed: true,
    }));
    Ok(out)
}

fn local_versions(dir: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(entries) = fs::read_dir(dir.join("versions")) {
        for e in entries.flatten() {
            let id = e.file_name().to_string_lossy().to_string();
            if e.path().join(format!("{id}.json")).is_file() {
                set.insert(id);
            }
        }
    }
    set
}

/// Loads (downloading if needed) the version JSON and resolves `inheritsFrom`.
pub fn load(dir: &Path, id: &str, manifest: &[VersionEntry]) -> Result<Value> {
    load_inner(dir, id, manifest, 0)
}

fn load_inner(dir: &Path, id: &str, manifest: &[VersionEntry], depth: u32) -> Result<Value> {
    if depth > 8 {
        bail!("inheritsFrom chain of {id} is too deep");
    }
    let path = dir.join("versions").join(id).join(format!("{id}.json"));
    let entry = manifest.iter().find(|e| e.id == id && e.url.is_some());
    if let Some(entry) = entry
        && !http::file_ok(&path, entry.sha1.as_deref(), None)
    {
        http::download(entry.url.as_ref().unwrap(), &path, entry.sha1.as_deref(), &|_| {})?;
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("version {id} not found ({})", path.display()))?;
    let json: Value = serde_json::from_str(&text).with_context(|| format!("parsing {id}.json"))?;

    match json["inheritsFrom"].as_str() {
        Some(parent_id) => {
            let parent = load_inner(dir, parent_id, manifest, depth + 1)?;
            Ok(merge(parent, json))
        }
        None => Ok(json),
    }
}

/// Merges a child profile (e.g. Fabric) on top of its parent version.
fn merge(parent: Value, child: Value) -> Value {
    let Value::Object(mut out) = parent.clone() else { return child };
    let Value::Object(child) = child else { return Value::Object(out) };

    for (key, value) in child {
        match key.as_str() {
            "libraries" => {
                // Child libraries win over parent ones with the same group:artifact[:classifier].
                let mut libs: Vec<Value> = value.as_array().cloned().unwrap_or_default();
                let taken: HashSet<String> = libs.iter().filter_map(lib_key).collect();
                for lib in parent["libraries"].as_array().into_iter().flatten() {
                    if lib_key(lib).is_none_or(|k| !taken.contains(&k)) {
                        libs.push(lib.clone());
                    }
                }
                out.insert(key, Value::Array(libs));
            }
            "arguments" => {
                let mut args = Map::new();
                for side in ["game", "jvm"] {
                    let mut list: Vec<Value> =
                        parent["arguments"][side].as_array().cloned().unwrap_or_default();
                    list.extend(value[side].as_array().cloned().unwrap_or_default());
                    args.insert(side.into(), Value::Array(list));
                }
                out.insert(key, Value::Object(args));
            }
            "inheritsFrom" => {}
            _ => {
                out.insert(key, value);
            }
        }
    }
    // The game jar belongs to the parent.
    if !out.contains_key("jar") || out["jar"].is_null() {
        let jar = parent.get("jar").cloned().unwrap_or_else(|| parent["id"].clone());
        out.insert("jar".into(), jar);
    }
    Value::Object(out)
}

fn lib_key(lib: &Value) -> Option<String> {
    let name = lib["name"].as_str()?;
    let parts: Vec<&str> = name.split(':').collect();
    match parts.as_slice() {
        [g, a, _v] => Some(format!("{g}:{a}")),
        [g, a, _v, c, ..] => Some(format!("{g}:{a}:{c}")),
        _ => None,
    }
}

/// Evaluates a Mojang `rules` array. No rules means "allowed".
pub fn rules_allow(rules: &Value, features: &HashMap<&str, bool>) -> bool {
    let Some(rules) = rules.as_array() else { return true };
    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, features) {
            allowed = rule["action"].as_str() == Some("allow");
        }
    }
    allowed
}

fn rule_matches(rule: &Value, features: &HashMap<&str, bool>) -> bool {
    if let Some(os) = rule["os"].as_object() {
        if let Some(name) = os.get("name").and_then(Value::as_str)
            && name != platform::os_name()
        {
            return false;
        }
        if let Some(arch) = os.get("arch").and_then(Value::as_str)
            && arch != platform::arch()
        {
            return false;
        }
        // `os.version` is a regex used only for ancient macOS quirks; we ignore it.
    }
    if let Some(feats) = rule["features"].as_object() {
        for (name, want) in feats {
            let have = features.get(name.as_str()).copied().unwrap_or(false);
            if Some(have) != want.as_bool() {
                return false;
            }
        }
    }
    true
}

/// `group:artifact:version[:classifier][@ext]` -> relative maven path.
pub fn maven_path(name: &str) -> Option<String> {
    let (coords, ext) = match name.split_once('@') {
        Some((c, e)) => (c, e),
        None => (name, "jar"),
    };
    let parts: Vec<&str> = coords.split(':').collect();
    let (group, artifact, version, classifier) = match parts.as_slice() {
        [g, a, v] => (*g, *a, *v, None),
        [g, a, v, c] => (*g, *a, *v, Some(*c)),
        _ => return None,
    };
    let file = match classifier {
        Some(c) => format!("{artifact}-{version}-{c}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!("{}/{artifact}/{version}/{file}", group.replace('.', "/")))
}
