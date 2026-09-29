//! Mod loader profiles: turns an instance's (version, loader) pair into a launchable version id.
//!
//! Fabric and Quilt publish ready-made profiles with `inheritsFrom`, so they are simply saved
//! into `versions/`. Forge and NeoForge need their installer to run; a profile installed that
//! way into the launcher's `versions/` folder is picked up if present.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::instance::{Instance, Loader};
use crate::{Reporter, http};

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
const QUILT_META: &str = "https://meta.quiltmc.org/v3";

/// Makes sure the instance's loader profile exists and returns the version id to launch.
/// Resolves (and saves) the latest loader version when the instance has none yet.
pub fn prepare(root: &Path, inst: &mut Instance, reporter: &Reporter) -> Result<String> {
    let meta = match inst.loader {
        Loader::Vanilla => return Ok(inst.minecraft.clone()),
        Loader::Fabric => FABRIC_META,
        Loader::Quilt => QUILT_META,
        Loader::Forge | Loader::NeoForge => return find_local_forge(root, inst),
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

fn find_local_forge(root: &Path, inst: &Instance) -> Result<String> {
    let (mc, v) = (&inst.minecraft, &inst.loader_version);
    let candidates = match inst.loader {
        Loader::Forge => {
            vec![format!("{mc}-forge-{v}"), format!("{mc}-forge{mc}-{v}"), format!("forge-{mc}-{v}")]
        }
        _ => vec![format!("neoforge-{v}"), format!("{mc}-neoforge-{v}")],
    };
    for id in candidates {
        if root.join("versions").join(&id).join(format!("{id}.json")).is_file() {
            return Ok(id);
        }
    }
    bail!(
        "{} {v} cannot be installed automatically yet. Run its official installer \
         (Install client) into {} and try again.",
        inst.loader.label(),
        root.display()
    )
}
