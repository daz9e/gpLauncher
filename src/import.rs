//! Importing modpacks as new instances:
//! - Modrinth `.mrpack` (mods are downloaded from the URLs in `modrinth.index.json`);
//! - MultiMC / Prism Launcher instance exports (`.zip` with `instance.cfg` and `mmc-pack.json`).

use std::fs;
use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use zip::ZipArchive;

use crate::Reporter;
use crate::download::{self, Job};
use crate::instance::{self, Instance, Loader};

/// Files the launcher knows how to import.
pub fn is_importable(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    matches!(ext.as_deref(), Some("mrpack" | "zip"))
}

/// Creates a new instance from a modpack file. On failure nothing is left behind.
pub fn import(data_dir: &Path, file: &Path, reporter: &Reporter) -> Result<Instance> {
    let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
    reporter.status(format!("Importing {name}"));
    let mut zip =
        ZipArchive::new(fs::File::open(file).with_context(|| format!("opening {}", file.display()))?)
            .with_context(|| format!("{name} is not a zip archive"))?;

    let stem = file.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let mut created: Option<Instance> = None;
    let result = if zip.by_name("modrinth.index.json").is_ok() {
        import_mrpack(data_dir, &mut zip, reporter, &mut created)
    } else if let Some(prefix) = find_mmc_root(&mut zip) {
        import_mmc(data_dir, &mut zip, &prefix, &stem, reporter, &mut created)
    } else if zip.by_name("manifest.json").is_ok() {
        bail!("{name}: CurseForge modpacks are not supported yet; use .mrpack or a Prism/MultiMC export")
    } else {
        bail!("{name}: no modrinth.index.json or instance.cfg found, not a modpack")
    };
    if result.is_err()
        && let Some(inst) = &created
    {
        let _ = instance::delete(inst);
    }
    result
}

// ---- Modrinth ------------------------------------------------------------

fn import_mrpack<R: Read + Seek>(
    data_dir: &Path,
    zip: &mut ZipArchive<R>,
    reporter: &Reporter,
    created: &mut Option<Instance>,
) -> Result<Instance> {
    let index: Value = {
        let mut text = String::new();
        zip.by_name("modrinth.index.json")?.read_to_string(&mut text)?;
        serde_json::from_str(&text).context("parsing modrinth.index.json")?
    };
    if index["game"].as_str().is_some_and(|g| g != "minecraft") {
        bail!("the modpack is not for Minecraft");
    }
    let deps = &index["dependencies"];
    let minecraft = deps["minecraft"].as_str().context("the modpack does not specify a Minecraft version")?;
    let (loader, loader_version) = [
        ("fabric-loader", Loader::Fabric),
        ("quilt-loader", Loader::Quilt),
        ("forge", Loader::Forge),
        ("neoforge", Loader::NeoForge),
    ]
    .into_iter()
    .find_map(|(key, loader)| deps[key].as_str().map(|v| (loader, v)))
    .unwrap_or((Loader::Vanilla, ""));

    let name = index["name"].as_str().unwrap_or("Modrinth pack");
    let inst = created.insert(instance::create(data_dir, name, minecraft, loader, loader_version)?);
    let game_dir = inst.game_dir.clone();

    let mut jobs = Vec::new();
    for file in index["files"].as_array().into_iter().flatten() {
        if file["env"]["client"].as_str() == Some("unsupported") {
            continue;
        }
        let rel = file["path"].as_str().context("modpack file entry has no path")?;
        let path = game_dir.join(safe_rel(rel)?);
        let url = file["downloads"]
            .as_array()
            .and_then(|d| d.iter().find_map(Value::as_str))
            .with_context(|| format!("no download URL for {rel}"))?;
        jobs.push(Job {
            url: url.to_string(),
            path,
            sha1: file["hashes"]["sha1"].as_str().map(String::from),
            size: file["fileSize"].as_u64(),
        });
    }

    reporter.status("Extracting modpack files");
    extract_dir(zip, "overrides/", &game_dir)?;
    extract_dir(zip, "client-overrides/", &game_dir)?;
    download::run(jobs, "Downloading mods", reporter)?;
    Ok(inst.clone())
}

// ---- MultiMC / Prism -----------------------------------------------------

/// Folder inside the archive that holds `instance.cfg` (`""` or `"Name/"`).
fn find_mmc_root<R: Read + Seek>(zip: &mut ZipArchive<R>) -> Option<String> {
    zip.file_names()
        .filter_map(|n| n.strip_suffix("instance.cfg"))
        .filter(|prefix| prefix.is_empty() || prefix.ends_with('/'))
        .min_by_key(|prefix| prefix.len())
        .map(String::from)
}

fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Option<String> {
    let mut text = String::new();
    zip.by_name(name).ok()?.read_to_string(&mut text).ok()?;
    Some(text)
}

fn import_mmc<R: Read + Seek>(
    data_dir: &Path,
    zip: &mut ZipArchive<R>,
    prefix: &str,
    fallback_name: &str,
    reporter: &Reporter,
    created: &mut Option<Instance>,
) -> Result<Instance> {
    let cfg = read_entry(zip, &format!("{prefix}instance.cfg")).unwrap_or_default();
    let cfg_value = |key: &str| {
        cfg.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };

    let mut minecraft = cfg_value("IntendedVersion").unwrap_or_default();
    let mut loader = (Loader::Vanilla, String::new());
    if let Some(pack) = read_entry(zip, &format!("{prefix}mmc-pack.json")) {
        let pack: Value = serde_json::from_str(&pack).context("parsing mmc-pack.json")?;
        for c in pack["components"].as_array().into_iter().flatten() {
            let version = c["version"].as_str().unwrap_or_default().to_string();
            match c["uid"].as_str().unwrap_or_default() {
                "net.minecraft" => minecraft = version,
                "net.fabricmc.fabric-loader" => loader = (Loader::Fabric, version),
                "org.quiltmc.quilt-loader" => loader = (Loader::Quilt, version),
                "net.minecraftforge" => loader = (Loader::Forge, version),
                "net.neoforged" => loader = (Loader::NeoForge, version),
                _ => {}
            }
        }
    }
    if minecraft.is_empty() {
        bail!("no Minecraft version in instance.cfg / mmc-pack.json");
    }

    let name = cfg_value("name").unwrap_or_else(|| fallback_name.to_string());
    let inst = created.insert(instance::create(data_dir, &name, &minecraft, loader.0, &loader.1)?);
    if cfg_value("OverrideMemory").as_deref() == Some("true") {
        inst.memory_mb = cfg_value("MaxMemAlloc").and_then(|v| v.parse().ok());
    }
    if cfg_value("OverrideJavaArgs").as_deref() == Some("true") {
        inst.jvm_args = cfg_value("JvmArgs").unwrap_or_default();
    }
    inst.save()?;

    reporter.status("Extracting instance files");
    let game_dir = inst.game_dir.clone();
    for sub in ["minecraft/", ".minecraft/"] {
        extract_dir(zip, &format!("{prefix}{sub}"), &game_dir)?;
    }
    Ok(inst.clone())
}

// ---- helpers ---------------------------------------------------------------

/// Extracts every entry under `prefix` into `dest`, keeping the relative layout.
fn extract_dir<R: Read + Seek>(zip: &mut ZipArchive<R>, prefix: &str, dest: &Path) -> Result<()> {
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.name().strip_prefix(prefix).map(String::from) else { continue };
        if rel.is_empty() || entry.is_dir() {
            continue;
        }
        let out = dest.join(safe_rel(&rel)?);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(&out).with_context(|| format!("creating {}", out.display()))?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

/// Rejects absolute paths and `..` so an archive cannot write outside the instance.
fn safe_rel(rel: &str) -> Result<PathBuf> {
    let path = Path::new(rel);
    if rel.is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
        bail!("unsafe path in archive: {rel}");
    }
    Ok(path.to_path_buf())
}
