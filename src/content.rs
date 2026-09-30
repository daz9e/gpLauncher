//! Files an instance adds to the game: mods, resource packs and shader packs.
//!
//! Disabled files keep their place with a `.disabled` suffix, like other launchers do, so the game
//! ignores them and the launcher can turn them back on.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use serde_json::Value;
use zip::ZipArchive;

use crate::instance::Instance;

const DISABLED: &str = ".disabled";
const ICON_SIZE: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Mods,
    ResourcePacks,
    ShaderPacks,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Mods, Kind::ResourcePacks, Kind::ShaderPacks];

    /// Folder inside the game directory.
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Mods => "mods",
            Kind::ResourcePacks => "resourcepacks",
            Kind::ShaderPacks => "shaderpacks",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Mods => "Mods",
            Kind::ResourcePacks => "Resource packs",
            Kind::ShaderPacks => "Shader packs",
        }
    }

    /// One item, lower case: "mod", "resource pack".
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Mods => "mod",
            Kind::ResourcePacks => "resource pack",
            Kind::ShaderPacks => "shader pack",
        }
    }

    /// Modrinth `project_type`.
    pub fn modrinth_type(self) -> &'static str {
        match self {
            Kind::Mods => "mod",
            Kind::ResourcePacks => "resourcepack",
            Kind::ShaderPacks => "shader",
        }
    }

    pub fn dir(self, inst: &Instance) -> PathBuf {
        inst.game_dir.join(self.folder())
    }

    /// Whether `path` looks like something of this kind (by extension; folders count for packs).
    pub fn accepts(self, path: &Path) -> bool {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        let name = name.strip_suffix(DISABLED).unwrap_or(&name);
        match self {
            Kind::Mods => name.ends_with(".jar") || name.ends_with(".litemod"),
            Kind::ResourcePacks | Kind::ShaderPacks => name.ends_with(".zip") || path.is_dir(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub path: PathBuf,
    /// File name without the `.disabled` suffix.
    pub file_name: String,
    pub enabled: bool,
    /// Display name from the metadata, or the file name.
    pub name: String,
    /// Mod id from the metadata.
    pub id: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    /// Cached PNG of the icon inside the file.
    pub icon: Option<PathBuf>,
    pub size: u64,
    pub modified: u64,
}

/// Everything in the kind's folder, sorted by name. Icons are cached under `cache_dir`.
pub fn list(inst: &Instance, kind: Kind, cache_dir: &Path) -> Vec<Item> {
    let dir = kind.dir(inst);
    let mut items: Vec<Item> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| kind.accepts(p))
        .map(|p| read_item(&p, kind, cache_dir))
        .collect();
    items.sort_by_key(|i| i.name.to_lowercase());
    items
}

fn read_item(path: &Path, kind: Kind, cache_dir: &Path) -> Item {
    let raw_name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let enabled = !raw_name.ends_with(DISABLED);
    let file_name = raw_name.strip_suffix(DISABLED).unwrap_or(&raw_name).to_string();
    let meta = fs::metadata(path).ok();
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let mut item = Item {
        path: path.to_path_buf(),
        name: display_stem(&file_name),
        file_name,
        enabled,
        size: meta.as_ref().map_or(0, |m| if m.is_dir() { 0 } else { m.len() }),
        modified,
        ..Default::default()
    };
    let icon = match kind {
        Kind::Mods => read_mod(path, &mut item).ok().flatten(),
        Kind::ResourcePacks => read_pack(path, &mut item),
        Kind::ShaderPacks => None,
    };
    if let Some((bytes, _)) = icon.filter(|(b, _)| !b.is_empty()) {
        item.icon = cache_icon(cache_dir, path, item.size, modified, &bytes);
    }
    item
}

/// `sodium-fabric-0.5.8+mc1.20.1` → `sodium-fabric-0.5.8+mc1.20.1` without the extension.
fn display_stem(file_name: &str) -> String {
    Path::new(file_name).file_stem().unwrap_or_default().to_string_lossy().into_owned()
}

/// Icon bytes and their name inside the archive.
type Icon = (Vec<u8>, String);

/// Fills `item` from the mod's metadata; returns its icon.
fn read_mod(path: &Path, item: &mut Item) -> Result<Option<Icon>> {
    let mut zip = ZipArchive::new(fs::File::open(path)?)?;
    let mut icon_path = None;

    if let Some(text) = entry_text(&mut zip, "fabric.mod.json") {
        // Some mods put raw newlines inside strings, which strict JSON forbids.
        let v: Value = serde_json::from_str(&text.replace(['\n', '\r'], " "))?;
        item.id = str_of(&v["id"]);
        set_if(&mut item.name, str_of(&v["name"]));
        item.version = str_of(&v["version"]);
        item.description = str_of(&v["description"]);
        item.authors = people(&v["authors"]);
        icon_path = match &v["icon"] {
            Value::String(s) => Some(s.clone()),
            // Sizes → paths; take the biggest.
            Value::Object(map) => map
                .iter()
                .max_by_key(|(k, _)| k.parse::<u32>().unwrap_or(0))
                .and_then(|(_, p)| p.as_str().map(String::from)),
            _ => None,
        };
    } else if let Some(text) = entry_text(&mut zip, "quilt.mod.json") {
        let v: Value = serde_json::from_str(&text)?;
        let loader = &v["quilt_loader"];
        let meta = &loader["metadata"];
        item.id = str_of(&loader["id"]);
        set_if(&mut item.name, str_of(&meta["name"]));
        item.version = str_of(&loader["version"]);
        item.description = str_of(&meta["description"]);
        item.authors =
            meta["contributors"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
        icon_path = meta["icon"].as_str().map(String::from);
    } else if let Some(text) = entry_text(&mut zip, "META-INF/neoforge.mods.toml")
        .or_else(|| entry_text(&mut zip, "META-INF/mods.toml"))
    {
        let v: toml::Value = toml::from_str(&text).context("parsing mods.toml")?;
        let first = v.get("mods").and_then(|m| m.as_array()).and_then(|m| m.first());
        let get = |key: &str| {
            first
                .and_then(|m| m.get(key))
                .or_else(|| v.get(key))
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        item.id = get("modId");
        set_if(&mut item.name, get("displayName"));
        item.version = get("version");
        if item.version.contains("${") {
            item.version = manifest_version(&mut zip).unwrap_or_default();
        }
        item.description = get("description");
        let authors = get("authors");
        item.authors = authors.split(',').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
        icon_path = Some(get("logoFile")).filter(|s| !s.is_empty());
    } else if let Some(text) = entry_text(&mut zip, "mcmod.info") {
        let v: Value = serde_json::from_str(&text.replace(['\n', '\r'], " "))?;
        let m = match &v {
            Value::Array(list) => list.first().cloned().unwrap_or_default(),
            v => v["modList"][0].clone(),
        };
        item.id = str_of(&m["modid"]);
        set_if(&mut item.name, str_of(&m["name"]));
        item.version = str_of(&m["version"]);
        item.description = str_of(&m["description"]);
        item.authors = people(&m["authorList"]);
        icon_path = Some(str_of(&m["logoFile"])).filter(|s| !s.is_empty());
    }
    item.description = item.description.split_whitespace().collect::<Vec<_>>().join(" ");

    Ok(icon_path.and_then(|p| {
        let p = p.trim_start_matches('/').to_string();
        entry_bytes(&mut zip, &p).map(|b| (b, p))
    }))
}

fn read_pack(path: &Path, item: &mut Item) -> Option<Icon> {
    let (mcmeta, icon) = if path.is_dir() {
        (fs::read_to_string(path.join("pack.mcmeta")).ok(), fs::read(path.join("pack.png")).ok())
    } else {
        let mut zip = ZipArchive::new(fs::File::open(path).ok()?).ok()?;
        (entry_text(&mut zip, "pack.mcmeta"), entry_bytes(&mut zip, "pack.png"))
    };
    if let Some(v) = mcmeta.and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok())
    {
        item.description = text_component(&v["pack"]["description"]);
    }
    icon.map(|b| (b, "pack.png".into()))
}

/// Plain text of a JSON text component (a string, a list, or an object with `text` and `extra`).
/// Translation keys are dropped: without the game's language files they read as noise.
fn text_component(v: &Value) -> String {
    let text = match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().map(text_component).collect(),
        Value::Object(o) => {
            let mut s = o.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
            if let Some(extra) = o.get("extra") {
                s += &text_component(extra);
            }
            s
        }
        _ => String::new(),
    };
    strip_formatting(&text)
}

/// Removes `§x` color codes.
fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn manifest_version(zip: &mut ZipArchive<fs::File>) -> Option<String> {
    let text = entry_text(zip, "META-INF/MANIFEST.MF")?;
    text.lines().find_map(|l| l.strip_prefix("Implementation-Version:")).map(|v| v.trim().to_string())
}

fn set_if(slot: &mut String, value: String) {
    if !value.trim().is_empty() {
        *slot = value.trim().to_string();
    }
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().trim().to_string()
}

/// Author lists are strings or `{ "name": ... }` objects.
fn people(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().or(p["name"].as_str()).map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn entry_bytes(zip: &mut ZipArchive<fs::File>, name: &str) -> Option<Vec<u8>> {
    let mut entry = zip.by_name(name).ok()?;
    // Guard against absurd entries.
    if entry.size() > 32 * 1024 * 1024 {
        return None;
    }
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn entry_text(zip: &mut ZipArchive<fs::File>, name: &str) -> Option<String> {
    entry_bytes(zip, name).map(|b| String::from_utf8_lossy(&b).trim_start_matches('\u{feff}').to_string())
}

/// Writes a small PNG of `bytes`, keyed by the file's identity, and returns its path.
fn cache_icon(cache_dir: &Path, file: &Path, size: u64, modified: u64, bytes: &[u8]) -> Option<PathBuf> {
    let key = format!("{}|{size}|{modified}", file.display());
    let name: String = Md5::digest(key.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let path = cache_dir.join(format!("{name}.png"));
    if path.is_file() {
        return Some(path);
    }
    let image = image::load_from_memory(bytes).ok()?;
    let image = if image.width() > ICON_SIZE || image.height() > ICON_SIZE {
        // Pixel art stays crisp with nearest-neighbour scaling.
        image.resize(ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Nearest)
    } else {
        image
    };
    fs::create_dir_all(cache_dir).ok()?;
    let tmp = path.with_extension("png.part");
    image.save_with_format(&tmp, image::ImageFormat::Png).ok()?;
    fs::rename(&tmp, &path).ok()?;
    Some(path)
}

/// Turns an item on or off by renaming it; returns the new path.
pub fn set_enabled(item: &Item, enabled: bool) -> Result<PathBuf> {
    if item.enabled == enabled {
        return Ok(item.path.clone());
    }
    let dir = item.path.parent().context("file has no folder")?;
    let target = match enabled {
        true => dir.join(&item.file_name),
        false => dir.join(format!("{}{DISABLED}", item.file_name)),
    };
    if target.exists() {
        bail!("{} already exists", target.display());
    }
    fs::rename(&item.path, &target).with_context(|| format!("renaming {}", item.path.display()))?;
    Ok(target)
}

pub fn delete(item: &Item) -> Result<()> {
    if item.path.is_dir() { fs::remove_dir_all(&item.path) } else { fs::remove_file(&item.path) }
        .with_context(|| format!("deleting {}", item.path.display()))
}

/// Copies files into the kind's folder. Returns how many were added; unsuitable files are skipped.
pub fn add_files(inst: &Instance, kind: Kind, paths: &[PathBuf]) -> Result<usize> {
    let dir = kind.dir(inst);
    fs::create_dir_all(&dir)?;
    let mut added = 0;
    for path in paths.iter().filter(|p| p.is_file() && kind.accepts(p)) {
        let name = path.file_name().context("no file name")?;
        let dest = dir.join(name);
        if dest != *path {
            fs::copy(path, &dest).with_context(|| format!("copying {}", path.display()))?;
            added += 1;
        }
    }
    Ok(added)
}

/// `12.3 MB`, `640 KB`.
pub fn human_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.0} KB", bytes as f64 / 1024.),
        1_048_576..1_073_741_824 => format!("{:.1} MB", bytes as f64 / 1_048_576.),
        _ => format!("{:.2} GB", bytes as f64 / 1_073_741_824.),
    }
}

/// Worlds in the instance's `saves` folder.
#[derive(Clone, Debug)]
pub struct World {
    pub path: PathBuf,
    pub folder: String,
    pub name: String,
    pub icon: Option<PathBuf>,
    pub last_played: u64,
    pub game_mode: Option<&'static str>,
}

pub fn worlds(inst: &Instance) -> Vec<World> {
    let mut out: Vec<World> = fs::read_dir(inst.game_dir.join("saves"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("level.dat").is_file())
        .map(|path| {
            let folder = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let level = crate::nbt::read_level(&path.join("level.dat")).unwrap_or_default();
            let modified = fs::metadata(path.join("level.dat"))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            World {
                icon: Some(path.join("icon.png")).filter(|p| p.is_file()),
                name: level.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| folder.clone()),
                last_played: level.last_played.map_or(modified, |ms| ms / 1000),
                game_mode: level.game_type.map(|g| match g {
                    1 => "Creative",
                    2 => "Adventure",
                    3 => "Spectator",
                    _ => "Survival",
                }),
                folder,
                path,
            }
        })
        .collect();
    out.sort_by_key(|w| std::cmp::Reverse(w.last_played));
    out
}

/// Size of a folder with everything inside it.
pub fn dir_size(path: &Path) -> u64 {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}

/// Screenshots, newest first.
pub fn screenshots(inst: &Instance) -> Vec<PathBuf> {
    let mut out: Vec<(u64, PathBuf)> = fs::read_dir(inst.game_dir.join("screenshots"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .map(|p| {
            let t = fs::metadata(&p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            (t, p)
        })
        .collect();
    out.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    out.into_iter().map(|(_, p)| p).collect()
}

/// Log files the game wrote: `logs/*.log(.gz)` and crash reports, newest first.
pub fn log_files(inst: &Instance) -> Vec<PathBuf> {
    let mut out: Vec<(u64, PathBuf)> = ["logs", "crash-reports"]
        .iter()
        .flat_map(|d| fs::read_dir(inst.game_dir.join(d)).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            name.ends_with(".log") || name.ends_with(".log.gz") || name.ends_with(".txt")
        })
        .map(|p| {
            let t = fs::metadata(&p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            (t, p)
        })
        .collect();
    out.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    out.into_iter().map(|(_, p)| p).collect()
}

/// Text of a log file, un-gzipping `.gz` files.
pub fn read_log(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if path.extension().is_some_and(|e| e == "gz") {
        let mut text = Vec::new();
        flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut text)?;
        return Ok(String::from_utf8_lossy(&text).into_owned());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
