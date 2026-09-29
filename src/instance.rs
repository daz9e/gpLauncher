//! Instances: separate game folders, each with its own Minecraft version and mod loader.
//!
//! Layout: `<data_dir>/instances/<id>/instance.json` plus `<id>/minecraft/` (saves, mods, configs).
//! Versions, libraries, assets and Java runtimes stay shared in the launcher folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
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
    /// Unix time of the last launch, for sorting.
    pub last_played: u64,
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

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(&self.game_dir)?;
        fs::write(self.dir.join("instance.json"), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn touch(&mut self) {
        self.last_played = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = self.save();
    }
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
