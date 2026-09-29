//! Instances: separate game folders, each with its own Minecraft version and mod loader.
//!
//! Layout: `<data_dir>/instances/<id>/instance.json` plus `<id>/minecraft/` (saves, mods, configs).
//! Versions, libraries, assets and Java runtimes stay shared in the launcher folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    #[default]
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl Loader {
    pub fn label(self) -> &'static str {
        match self {
            Loader::Vanilla => "Vanilla",
            Loader::Fabric => "Fabric",
            Loader::Quilt => "Quilt",
            Loader::Forge => "Forge",
            Loader::NeoForge => "NeoForge",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Instance {
    /// Folder name under `instances/`.
    #[serde(skip)]
    pub id: String,
    #[serde(skip)]
    pub dir: PathBuf,
    /// Working directory of the game.
    #[serde(skip)]
    pub game_dir: PathBuf,
    /// The icon file in the instance folder (see [`ICON_FILE`]), when there is one.
    #[serde(skip)]
    pub icon: Option<PathBuf>,
    pub name: String,
    /// Minecraft version id (e.g. `1.20.1`) or any local version profile.
    pub minecraft: String,
    pub loader: Loader,
    /// Empty = latest stable, resolved on first launch.
    pub loader_version: String,
    /// Overrides the global memory setting.
    pub memory_mb: Option<u32>,
    /// Appended to the global JVM arguments.
    pub jvm_args: String,
    /// Overrides the global Java binary; empty = the launcher setting.
    pub java_path: String,
    /// Game window size; `None` = the launcher setting.
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    /// `None` = the launcher setting.
    pub fullscreen: Option<bool>,
    /// Unix time of the last launch, for sorting.
    pub last_played: u64,
    /// Seconds played in total.
    pub play_time: u64,
    /// Group the instance is shown under; empty = ungrouped.
    pub group: String,
}

impl Instance {
    /// A throwaway instance that runs straight from `game_dir` (used by the CLI).
    pub fn ephemeral(minecraft: &str, game_dir: PathBuf) -> Instance {
        Instance {
            name: minecraft.to_string(),
            minecraft: minecraft.to_string(),
            dir: game_dir.clone(),
            game_dir,
            ..Default::default()
        }
    }

    pub fn description(&self) -> String {
        match self.loader {
            Loader::Vanilla => self.minecraft.clone(),
            loader if self.loader_version.is_empty() => format!("{} · {}", self.minecraft, loader.label()),
            loader => format!("{} · {} {}", self.minecraft, loader.label(), self.loader_version),
        }
    }

    /// Width and height when both are set.
    pub fn resolution(&self) -> Option<(u32, u32)> {
        self.window_width.zip(self.window_height)
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(&self.game_dir)?;
        fs::write(self.dir.join("instance.json"), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    /// Uses the image at `source` as the icon, stored as a small PNG.
    pub fn set_icon(&mut self, source: &Path) -> Result<()> {
        let image = image::open(source).with_context(|| format!("{} is not an image", source.display()))?;
        let image =
            if image.width() > 256 || image.height() > 256 { image.thumbnail(256, 256) } else { image };
        self.clear_icon()?;
        // A new name each time: image caches key on the path.
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let path = self.dir.join(format!("icon-{stamp}.png"));
        image.save_with_format(&path, image::ImageFormat::Png)?;
        self.icon = Some(path);
        Ok(())
    }

    pub fn clear_icon(&mut self) -> Result<()> {
        for path in icon_files(&self.dir) {
            fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;
        }
        self.icon = None;
        Ok(())
    }

    pub fn touch(&mut self) {
        self.last_played = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = self.save();
    }
}

/// Instance icon, next to `instance.json`. Icons chosen in the launcher are `icon-<time>.png`.
pub const ICON_FILE: &str = "icon.png";

fn icon_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            name == ICON_FILE || (name.starts_with("icon-") && name.ends_with(".png"))
        })
        .collect();
    out.sort();
    out
}

fn instances_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("instances")
}

/// All instances, most recently played first.
pub fn list(data_dir: &Path) -> Vec<Instance> {
    let mut out: Vec<Instance> = fs::read_dir(instances_dir(data_dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| load(&e.path()).ok())
        .collect();
    out.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.name.cmp(&b.name)));
    out
}

pub fn load(dir: &Path) -> Result<Instance> {
    let bytes = fs::read(dir.join("instance.json"))?;
    let mut inst: Instance = serde_json::from_slice(&bytes).context("parsing instance.json")?;
    inst.id = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
    inst.dir = dir.to_path_buf();
    inst.game_dir = dir.join("minecraft");
    inst.icon = icon_files(dir).pop();
    Ok(inst)
}

/// Creates a new instance folder with a unique id derived from `name`.
pub fn create(
    data_dir: &Path,
    name: &str,
    minecraft: &str,
    loader: Loader,
    loader_version: &str,
) -> Result<Instance> {
    let name = name.trim();
    let name = if name.is_empty() { minecraft } else { name };
    if minecraft.is_empty() {
        bail!("Minecraft version is empty");
    }
    let root = instances_dir(data_dir);
    fs::create_dir_all(&root)?;

    let base = sanitize(name);
    let mut id = base.clone();
    let mut n = 2;
    while root.join(&id).exists() {
        id = format!("{base} ({n})");
        n += 1;
    }
    let dir = root.join(&id);
    let inst = Instance {
        game_dir: dir.join("minecraft"),
        dir,
        id,
        name: name.to_string(),
        minecraft: minecraft.to_string(),
        loader,
        loader_version: loader_version.to_string(),
        ..Default::default()
    };
    inst.save()?;
    Ok(inst)
}

/// Copies `inst` with all its files into a new instance called `name`.
pub fn duplicate(data_dir: &Path, inst: &Instance, name: &str) -> Result<Instance> {
    let mut copy = create(data_dir, name, &inst.minecraft, inst.loader, &inst.loader_version)?;
    let result = copy_dir(&inst.dir, &copy.dir);
    if let Err(e) = result {
        let _ = delete(&copy);
        return Err(e);
    }
    copy.memory_mb = inst.memory_mb;
    copy.jvm_args = inst.jvm_args.clone();
    copy.java_path = inst.java_path.clone();
    (copy.window_width, copy.window_height) = (inst.window_width, inst.window_height);
    copy.fullscreen = inst.fullscreen;
    copy.group = inst.group.clone();
    copy.icon = inst.icon.as_ref().and_then(|p| p.file_name()).map(|name| copy.dir.join(name));
    copy.save()?;
    Ok(copy)
}

/// Copies the contents of `from` into `to` (except `instance.json`, which the caller writes).
fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        let (src, dest) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&src, &dest)?;
        } else if kind.is_file() && src != from.join("instance.json") {
            fs::copy(&src, &dest).with_context(|| format!("copying {}", src.display()))?;
        }
    }
    Ok(())
}

pub fn delete(inst: &Instance) -> Result<()> {
    if !inst.dir.join("instance.json").is_file() {
        bail!("{} does not look like an instance folder", inst.dir.display());
    }
    fs::remove_dir_all(&inst.dir).with_context(|| format!("deleting {}", inst.dir.display()))
}

/// Folder name safe on every OS.
fn sanitize(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let s: String = s.trim().trim_matches('.').chars().take(64).collect();
    let s = s.trim().to_string();
    // Windows refuses these names (with any extension) for files and folders.
    let stem = s.split('.').next().unwrap_or_default().trim().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    match s.is_empty() {
        true => "instance".into(),
        false if reserved => format!("_{s}"),
        false => s,
    }
}
