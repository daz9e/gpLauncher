//! Exporting an instance as a MultiMC / Prism Launcher `.zip`, which [`crate::import`] reads back.

use std::fs;
use std::io::{Seek, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::Reporter;
use crate::instance::{Instance, Loader};

/// Icon key written to `instance.cfg`; the icon itself goes next to it as `<key>.png`.
pub const ICON_KEY: &str = "gplauncher_icon";

pub fn export(inst: &Instance, dest: &Path, reporter: &Reporter) -> Result<()> {
    reporter.status(format!("Exporting {}", inst.name));
    let mut files = Vec::new();
    collect(&inst.game_dir, "minecraft", &mut files)?;
    let total: u64 = files.iter().map(|(_, _, len)| len).sum();

    let tmp = dest.with_extension("zip.part");
    let result = (|| {
        let mut zip =
            ZipWriter::new(fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?);
        let options = SimpleFileOptions::default();
        zip.start_file("instance.cfg", options)?;
        zip.write_all(instance_cfg(inst).as_bytes())?;
        zip.start_file("mmc-pack.json", options)?;
        zip.write_all(serde_json::to_string_pretty(&mmc_pack(inst))?.as_bytes())?;
        if let Some(icon) = &inst.icon {
            add_file(&mut zip, &format!("{ICON_KEY}.png"), icon, options)?;
        }
        let mut done = 0;
        for (name, path, len) in &files {
            add_file(&mut zip, name, path, options.large_file(*len >= u32::MAX as u64))?;
            done += len;
            reporter.progress(done, total);
        }
        zip.finish()?;
        fs::rename(&tmp, dest).with_context(|| format!("writing {}", dest.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn add_file<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    name: &str,
    path: &Path,
    options: SimpleFileOptions,
) -> Result<()> {
    zip.start_file(name, options)?;
    let mut file = fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    std::io::copy(&mut file, zip)?;
    Ok(())
}

/// Every file under `dir` as (archive name, path, size).
fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, std::path::PathBuf, u64)>) -> Result<()> {
    let Ok(entries) = fs::read_dir(dir) else { return Ok(()) };
    for entry in entries {
        let entry = entry?;
        let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&entry.path(), &name, out)?;
        } else if kind.is_file() {
            out.push((name, entry.path(), entry.metadata()?.len()));
        }
    }
    Ok(())
}

fn instance_cfg(inst: &Instance) -> String {
    let mut cfg = vec![
        "InstanceType=OneSix".to_string(),
        format!("name={}", inst.name),
        format!("IntendedVersion={}", inst.minecraft),
    ];
    if inst.icon.is_some() {
        cfg.push(format!("iconKey={ICON_KEY}"));
    }
    if let Some(mb) = inst.memory_mb {
        cfg.extend(["OverrideMemory=true".into(), format!("MaxMemAlloc={mb}")]);
    }
    if !inst.jvm_args.is_empty() {
        cfg.extend(["OverrideJavaArgs=true".into(), format!("JvmArgs={}", inst.jvm_args)]);
    }
    if let Some((width, height)) = inst.resolution() {
        cfg.extend([
            "OverrideWindow=true".into(),
            format!("MinecraftWinWidth={width}"),
            format!("MinecraftWinHeight={height}"),
        ]);
    }
    cfg.join("\n") + "\n"
}

fn mmc_pack(inst: &Instance) -> serde_json::Value {
    let mut components =
        vec![json!({ "uid": "net.minecraft", "version": inst.minecraft, "important": true })];
    let uid = match inst.loader {
        Loader::Vanilla => None,
        Loader::Fabric => Some("net.fabricmc.fabric-loader"),
        Loader::Quilt => Some("org.quiltmc.quilt-loader"),
        Loader::Forge => Some("net.minecraftforge"),
        Loader::NeoForge => Some("net.neoforged"),
    };
    if let Some(uid) = uid {
        // Fabric and Quilt need intermediary mappings, which Prism does not add by itself.
        if matches!(inst.loader, Loader::Fabric | Loader::Quilt) {
            components.push(json!({ "uid": "net.fabricmc.intermediary", "version": inst.minecraft }));
        }
        let mut loader = json!({ "uid": uid });
        if !inst.loader_version.is_empty() {
            loader["version"] = json!(inst.loader_version);
        }
        components.push(loader);
    }
    json!({ "formatVersion": 1, "components": components })
}

/// Suggested file name for an export.
pub fn file_name(inst: &Instance) -> String {
    let name: String = inst
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_.()[]".contains(c) { c } else { '_' })
        .collect();
    format!("{}.zip", name.trim())
}
