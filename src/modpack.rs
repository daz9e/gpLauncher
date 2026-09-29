//! Browsing and installing modpacks from online platforms, on top of [`crate::import`].

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};

use crate::import::{self, Imported};
use crate::instance::Loader;
use crate::settings::Settings;
use crate::{Reporter, curseforge, http, modrinth};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    Modrinth,
    CurseForge,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Platform::Modrinth => "Modrinth",
            Platform::CurseForge => "CurseForge",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pack {
    pub platform: Platform,
    pub id: String,
    pub title: String,
    pub author: String,
    pub summary: String,
    pub downloads: u64,
    pub icon_url: Option<String>,
    pub website: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PackVersion {
    pub id: String,
    pub name: String,
    pub game_versions: Vec<String>,
    /// Display names (`Fabric`, `Forge`, ...).
    pub loaders: Vec<String>,
    /// `None` when the platform does not allow launchers to download it.
    pub url: Option<String>,
    pub file_name: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// Best match for the query; most popular without one.
    #[default]
    Relevance,
    Downloads,
    Updated,
    Newest,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Relevance, Sort::Downloads, Sort::Updated, Sort::Newest];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Relevance => "Relevance",
            Sort::Downloads => "Downloads",
            Sort::Updated => "Recently updated",
            Sort::Newest => "Newest",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    pub game_version: Option<String>,
    /// Never [`Loader::Vanilla`]: modpacks always have a loader.
    pub loader: Option<Loader>,
    pub sort: Sort,
}

impl Filters {
    /// Whether a version of a pack fits the Minecraft version and loader filters.
    pub fn matches(&self, v: &PackVersion) -> bool {
        self.game_version.as_ref().is_none_or(|g| v.game_versions.contains(g))
            && self.loader.is_none_or(|l| v.loaders.iter().any(|name| name.eq_ignore_ascii_case(l.label())))
    }
}

pub struct Page {
    pub packs: Vec<Pack>,
    /// Number of results the query has in total.
    pub total: u64,
}

/// A platform plus whatever it needs to be queried.
#[derive(Clone, Debug)]
pub enum Source {
    Modrinth,
    CurseForge { api_key: String },
}

impl Source {
    pub fn search(&self, query: &str, filters: &Filters, offset: u64, limit: u64) -> Result<Page> {
        match self {
            Source::Modrinth => modrinth::search(query, filters, offset, limit),
            Source::CurseForge { api_key } => curseforge::search(api_key, query, filters, offset, limit),
        }
    }

    pub fn versions(&self, pack_id: &str) -> Result<Vec<PackVersion>> {
        match self {
            Source::Modrinth => modrinth::versions(pack_id),
            Source::CurseForge { api_key } => curseforge::versions(api_key, pack_id),
        }
    }
}

/// Downloads the modpack file and imports it as a new instance named after the pack.
pub fn install(
    settings: &Settings,
    pack: &Pack,
    version: &PackVersion,
    reporter: &Reporter,
) -> Result<Imported> {
    let Some(url) = &version.url else {
        let page = pack.website.as_deref().unwrap_or("the modpack page");
        bail!(
            "{} does not allow launchers to download this file; get it from {page} and import it",
            pack.title
        );
    };
    let dir = settings.data_dir.join("cache/modpacks");
    let file = dir.join(safe_file_name(&version.file_name));
    reporter.status(format!("Downloading {}", pack.title));
    let total = version.size.unwrap_or(0);
    let done = std::sync::atomic::AtomicU64::new(0);
    http::download(url, &file, version.sha1.as_deref(), &|n| {
        let d = done.fetch_add(n, std::sync::atomic::Ordering::Relaxed) + n;
        reporter.progress(d, total);
    })?;
    reporter.progress(0, 0);

    let result = import::import(settings, &file, reporter);
    let _ = fs::remove_file(&file);
    let mut imported = result?;
    // The file name inside the pack is often a slug; the platform title reads better.
    if !pack.title.trim().is_empty() {
        imported.instance.name = pack.title.trim().to_string();
        imported.instance.save()?;
    }
    Ok(imported)
}

/// Icons are stored as small static PNGs: platforms serve anything up to 1024px animated GIFs,
/// which are slow to decode and keep the UI redrawing while they animate.
const ICON_SIZE: u32 = 96;

/// Icon for `url` as a small PNG, cached under the launcher folder.
pub fn icon(data_dir: &Path, url: &str) -> Result<PathBuf> {
    let name: String = Md5::digest(url.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let path = data_dir.join("cache/icons").join(format!("{name}.png"));
    if path.is_file() {
        return Ok(path);
    }
    let mut resp = http::agent().get(url).call().with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("GET {url}: HTTP {}", resp.status());
    }
    let bytes = resp.body_mut().with_config().limit(16 * 1024 * 1024).read_to_vec()?;
    // Decodes only the first frame of animated images.
    let image = image::load_from_memory(&bytes).context("decoding icon")?;
    let image = if image.width() > ICON_SIZE || image.height() > ICON_SIZE {
        image.thumbnail(ICON_SIZE, ICON_SIZE)
    } else {
        image
    };
    fs::create_dir_all(path.parent().unwrap())?;
    // Write to a temp file first so a half-written icon is never picked up.
    let tmp = path.with_extension("png.part");
    image.save_with_format(&tmp, image::ImageFormat::Png)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

fn safe_file_name(name: &str) -> String {
    let name: String =
        name.chars().map(|c| if c.is_alphanumeric() || "-_.+ ".contains(c) { c } else { '_' }).collect();
    match name.trim_matches('.') {
        "" => "modpack".into(),
        n => n.to_string(),
    }
}
