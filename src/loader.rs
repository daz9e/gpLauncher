//! Mod loader profiles: turns an instance's (version, loader) pair into a launchable version id.
//!
//! Fabric and Quilt publish ready-made profiles with `inheritsFrom`, so they are simply saved
//! into `versions/`. Forge and NeoForge are installed by [`crate::forge`].

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::forge::{self, LoaderVersion};
use crate::instance::{Instance, Loader};
use crate::version::VersionEntry;
use crate::{Reporter, http};

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
const QUILT_META: &str = "https://meta.quiltmc.org/v3";

/// Makes sure the instance's loader profile exists and returns the version id to launch.
/// Resolves (and saves) the latest loader version when the instance has none yet.
/// `java` is used to run the Forge/NeoForge installer instead of the version's own runtime.
pub fn prepare(
    root: &Path,
    inst: &mut Instance,
    manifest: &[VersionEntry],
    java: Option<&Path>,
    reporter: &Reporter,
) -> Result<String> {
    let meta = match inst.loader {
        Loader::Vanilla => return Ok(inst.minecraft.clone()),
        Loader::Fabric => FABRIC_META,
        Loader::Quilt => QUILT_META,
        Loader::Forge | Loader::NeoForge => return prepare_forge(root, inst, manifest, java, reporter),
    };
    let mc = inst.minecraft.clone();

    if inst.loader_version.is_empty() {
        reporter.status(format!("Looking up the latest {} version", inst.loader.label()));
        let list = http::get_json(&format!("{meta}/versions/loader/{mc}"))?;
        let entries = list.as_array().context("loader list is not an array")?;
        let pick = |e: &&Value| {
            e["loader"]["stable"].as_bool().unwrap_or_else(|| {
                // Quilt has no `stable` flag; skip betas.
                !e["loader"]["version"].as_str().unwrap_or("-").contains('-')
            })
        };
        let version = entries
            .iter()
            .find(pick)
            .or(entries.first())
            .and_then(|e| e["loader"]["version"].as_str())
            .with_context(|| format!("{} does not support Minecraft {mc}", inst.loader.label()))?;
        inst.loader_version = version.to_string();
        let _ = inst.save();
    }

    let prefix = if inst.loader == Loader::Fabric { "fabric-loader" } else { "quilt-loader" };
    let id = format!("{prefix}-{}-{mc}", inst.loader_version);
    let path = root.join("versions").join(&id).join(format!("{id}.json"));
    if !path.is_file() {
        reporter.status(format!("Downloading profile {id}"));
        let url = format!("{meta}/versions/loader/{mc}/{}/profile/json", inst.loader_version);
        let mut profile = http::get_json(&url)?;
        // Store under the id we look for, whatever the meta server calls it.
        profile["id"] = Value::String(id.clone());
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, serde_json::to_vec_pretty(&profile)?)?;
    }
    Ok(id)
}

/// Minecraft versions the loader has builds for, or `None` when it cannot be told
/// (vanilla, or loaders without a meta server).
pub fn supported_versions(loader: Loader) -> Result<Option<HashSet<String>>> {
    let meta = match loader {
        Loader::Fabric => FABRIC_META,
        Loader::Quilt => QUILT_META,
        _ => return Ok(None),
    };
    let list = http::get_json(&format!("{meta}/versions/game"))?;
    let entries = list.as_array().context("game version list is not an array")?;
    Ok(Some(entries.iter().filter_map(|e| e["version"].as_str().map(String::from)).collect()))
}

fn prepare_forge(
    root: &Path,
    inst: &mut Instance,
    manifest: &[VersionEntry],
    java: Option<&Path>,
    reporter: &Reporter,
) -> Result<String> {
    if inst.loader_version.is_empty() {
        reporter.status(format!("Looking up the latest {} version", inst.loader.label()));
        let list = forge::versions(inst.loader, &inst.minecraft)?;
        let pick = list.iter().find(|v| v.stable).or(list.first()).with_context(|| {
            format!("{} does not support Minecraft {}", inst.loader.label(), inst.minecraft)
        })?;
        inst.loader_version = pick.version.clone();
        let _ = inst.save();
    }
    // Profiles installed by hand with the official installer still work.
    if let Some(id) = find_local_forge(root, inst) {
        return Ok(id);
    }
    forge::install(
        root,
        &inst.game_dir,
        inst.loader,
        &inst.minecraft,
        &inst.loader_version,
        manifest,
        java,
        reporter,
    )
}

/// Loader versions available for Minecraft `mc`, newest first.
pub fn versions(loader: Loader, mc: &str) -> Result<Vec<LoaderVersion>> {
    let meta = match loader {
        Loader::Vanilla => return Ok(Vec::new()),
        Loader::Forge | Loader::NeoForge => return forge::versions(loader, mc),
        Loader::Fabric => FABRIC_META,
        Loader::Quilt => QUILT_META,
    };
    let list = http::get_json(&format!("{meta}/versions/loader/{mc}"))?;
    Ok(list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let version = e["loader"]["version"].as_str()?.to_string();
            let stable = e["loader"]["stable"].as_bool().unwrap_or(!version.contains('-'));
            Some(LoaderVersion { version, stable })
        })
        .collect())
}

fn find_local_forge(root: &Path, inst: &Instance) -> Option<String> {
    let (mc, v) = (&inst.minecraft, &inst.loader_version);
    let candidates = match inst.loader {
        Loader::Forge => {
            vec![format!("{mc}-forge-{v}"), format!("{mc}-forge{mc}-{v}"), format!("forge-{mc}-{v}")]
        }
        _ => vec![format!("neoforge-{v}"), format!("{mc}-neoforge-{v}")],
    };
    let id = forge::installed_id(root, inst.loader, mc, v);
    id.into_iter()
        .chain(candidates)
        .find(|id| root.join("versions").join(id).join(format!("{id}.json")).is_file())
}
