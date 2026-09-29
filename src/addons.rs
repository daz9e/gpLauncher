//! Installing and updating mods, resource packs and shaders from Modrinth.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::content::{self, Item, Kind};
use crate::instance::{Instance, Loader};
use crate::modrinth::{self, Version};
use crate::{Reporter, http};

/// Modrinth loader names whose files fit the instance, best first. Empty = any.
pub fn loaders(inst: &Instance, kind: Kind) -> Vec<&'static str> {
    match kind {
        Kind::Mods => match inst.loader {
            Loader::Vanilla => Vec::new(),
            Loader::Fabric => vec!["fabric"],
            // Quilt runs most Fabric mods.
            Loader::Quilt => vec!["quilt", "fabric"],
            Loader::Forge => vec!["forge"],
            Loader::NeoForge => vec!["neoforge"],
        },
        Kind::ResourcePacks | Kind::ShaderPacks => Vec::new(),
    }
}

/// The Minecraft version files must support; shaders work across versions.
pub fn game_version(inst: &Instance, kind: Kind) -> Option<String> {
    match kind {
        Kind::ShaderPacks => None,
        _ => Some(inst.minecraft.clone()),
    }
}

/// Modrinth versions of the installed files, keyed by their path.
pub fn identify(items: &[Item]) -> Result<HashMap<PathBuf, Version>> {
    let hashes: Vec<(PathBuf, String)> = items
        .iter()
        .filter(|i| i.path.is_file())
        .filter_map(|i| Some((i.path.clone(), http::sha1_file(&i.path).ok()?)))
        .collect();
    let list: Vec<String> = hashes.iter().map(|(_, h)| h.clone()).collect();
    let mut found = modrinth::versions_by_hash(&list)?;
    Ok(hashes.into_iter().filter_map(|(path, hash)| Some((path, found.remove(&hash)?))).collect())
}

/// Installs `version` and, for mods, its required dependencies that are not installed yet.
/// `installed` holds the Modrinth project ids already in the instance.
/// Returns the names of the files that were added.
pub fn install(
    inst: &Instance,
    kind: Kind,
    version: &Version,
    installed: &HashSet<String>,
    reporter: &Reporter,
) -> Result<Vec<String>> {
    let loaders = loaders(inst, kind);
    let game_version = game_version(inst, kind);
    let mut queue = VecDeque::from([version.clone()]);
    let mut seen: HashSet<String> = installed.clone();
    seen.insert(version.project_id.clone());
    let mut added = Vec::new();

    while let Some(v) = queue.pop_front() {
        if kind == Kind::Mods {
            for dep in v.dependencies.iter().filter(|d| d.kind == "required") {
                let project = dep.project_id.clone();
                if project.as_ref().is_some_and(|p| seen.contains(p)) {
                    continue;
                }
                let resolved = match (&dep.version_id, &project) {
                    (Some(id), _) => modrinth::version(id).ok(),
                    (None, Some(p)) => modrinth::project_versions(p, &loaders, game_version.as_deref())
                        .ok()
                        .and_then(|list| list.into_iter().next()),
                    (None, None) => None,
                };
                match resolved {
                    Some(dep) if seen.insert(dep.project_id.clone()) => queue.push_back(dep),
                    Some(_) => {}
                    None => reporter.log(format!(
                        "[!] {}: could not find a fitting version of dependency {}",
                        v.file_name,
                        project.as_deref().unwrap_or("?")
                    )),
                }
                if added.len() + queue.len() > 64 {
                    bail!("too many dependencies");
                }
            }
        }
        reporter.status(format!("Downloading {}", v.file_name));
        if download(inst, kind, &v, reporter)? {
            added.push(v.file_name.clone());
        }
    }
    Ok(added)
}

/// Downloads a version's file into the instance; `false` if it is already there.
fn download(inst: &Instance, kind: Kind, v: &Version, reporter: &Reporter) -> Result<bool> {
    let name = safe_name(&v.file_name)?;
    let dir = kind.dir(inst);
    let path = dir.join(&name);
    if path.exists() || dir.join(format!("{name}.disabled")).exists() {
        return Ok(false);
    }
    let total = v.size.unwrap_or(0);
    let done = std::sync::atomic::AtomicU64::new(0);
    http::download(&v.url, &path, v.sha1.as_deref(), &|n| {
        let d = done.fetch_add(n, std::sync::atomic::Ordering::Relaxed) + n;
        reporter.progress(d, total);
    })?;
    reporter.progress(0, 0);
    Ok(true)
}

fn safe_name(name: &str) -> Result<String> {
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        bail!("unsafe file name {name:?}");
    }
    Ok(name.to_string())
}

#[derive(Clone, Debug)]
pub struct Update {
    pub item: Item,
    /// Version number of the installed file, when Modrinth knows it.
    pub current: Option<String>,
    pub latest: Version,
}

/// Items with a newer fitting version on Modrinth.
pub fn check_updates(inst: &Instance, kind: Kind, items: &[Item]) -> Result<Vec<Update>> {
    let hashes: Vec<(usize, String)> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.path.is_file())
        .filter_map(|(ix, i)| Some((ix, http::sha1_file(&i.path).ok()?)))
        .collect();
    let list: Vec<String> = hashes.iter().map(|(_, h)| h.clone()).collect();
    let loaders = loaders(inst, kind);
    let game_version = game_version(inst, kind);
    let latest = modrinth::latest_by_hash(&list, &loaders, game_version.as_deref())?;
    let current = modrinth::versions_by_hash(&list).unwrap_or_default();
    Ok(hashes
        .into_iter()
        .filter_map(|(ix, hash)| {
            let latest = latest.get(&hash)?;
            let now = current.get(&hash);
            if now.is_some_and(|c| c.id == latest.id) || latest.sha1.as_deref() == Some(hash.as_str()) {
                return None;
            }
            Some(Update {
                item: items[ix].clone(),
                current: now.map(|c| c.number.clone()),
                latest: latest.clone(),
            })
        })
        .collect())
}

/// Replaces the item's file with the update, keeping it disabled if it was.
pub fn apply_update(inst: &Instance, kind: Kind, update: &Update, reporter: &Reporter) -> Result<()> {
    let name = safe_name(&update.latest.file_name)?;
    let dir = update.item.path.parent().map(PathBuf::from).unwrap_or_else(|| kind.dir(inst));
    let target = match update.item.enabled {
        true => dir.join(&name),
        false => dir.join(format!("{name}.disabled")),
    };
    let tmp = dir.join(format!(".{name}.update"));
    reporter.status(format!("Updating {}", update.item.name));
    http::download(&update.latest.url, &tmp, update.latest.sha1.as_deref(), &|_| {})?;
    if target != update.item.path {
        content::delete(&update.item)?;
    }
    fs::rename(&tmp, &target).with_context(|| format!("replacing {}", target.display()))?;
    Ok(())
}
