use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::download::{self, Job};
use crate::version::{self, VersionEntry};
use crate::{Reporter, http, platform};

const LIBRARIES_URL: &str = "https://libraries.minecraft.net/";
const RESOURCES_URL: &str = "https://resources.download.minecraft.net/";

/// Everything the launch step needs once files are on disk.
pub struct Prepared {
    pub id: String,
    pub version: Value,
    pub classpath: Vec<PathBuf>,
    pub natives_dir: PathBuf,
    pub assets_root: PathBuf,
    pub assets_index: String,
    /// Directory for `${game_assets}` (virtual/legacy assets).
    pub game_assets: PathBuf,
    pub logging_arg: Option<String>,
    pub java_component: String,
    pub java_major: u32,
}

/// Downloads and verifies everything needed to run version `id` from `root`.
pub fn install(
    root: &Path,
    game_dir: &Path,
    id: &str,
    manifest: &[VersionEntry],
    reporter: &Reporter,
) -> Result<Prepared> {
    reporter.status(format!("Fetching version {id}"));
    let version = version::load(root, id, manifest)?;
    let features: HashMap<&str, bool> = HashMap::new();
    let mut jobs = Vec::new();

    // Client jar
    let jar_id = version["jar"].as_str().unwrap_or(id).to_string();
    let client_jar = root.join("versions").join(&jar_id).join(format!("{jar_id}.jar"));
    let client = &version["downloads"]["client"];
    if let Some(url) = client["url"].as_str() {
        jobs.push(Job {
            url: url.into(),
            path: client_jar.clone(),
            sha1: client["sha1"].as_str().map(String::from),
            size: client["size"].as_u64(),
        });
    } else if !client_jar.is_file() {
        bail!("version {id} has no client.jar download");
    }

    // Libraries
    let libraries_dir = root.join("libraries");
    let mut classpath = Vec::new();
    let mut native_jars: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for lib in version["libraries"].as_array().into_iter().flatten() {
        if !version::rules_allow(&lib["rules"], &features) {
            continue;
        }
        let patched = library_override(lib);
        let lib = patched.as_ref().unwrap_or(lib);
        let name = lib["name"].as_str().unwrap_or_default();
        let downloads = &lib["downloads"];

        // Legacy natives: a classifier per OS, extracted into the natives dir.
        if let Some(classifier) = lib["natives"][platform::os_name()].as_str() {
            let classifier = classifier.replace("${arch}", if platform::is_64bit() { "64" } else { "32" });
            let art = &downloads["classifiers"][&classifier];
            if let (Some(path), Some(url)) = (art["path"].as_str(), art["url"].as_str()) {
                let path = libraries_dir.join(path);
                jobs.push(Job {
                    url: url.into(),
                    path: path.clone(),
                    sha1: art["sha1"].as_str().map(String::from),
                    size: art["size"].as_u64(),
                });
                let exclude = lib["extract"]["exclude"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                native_jars.push((path, exclude));
            }
            // Some natives entries have no main artifact.
            if downloads["artifact"].is_null() {
                continue;
            }
        }

        let art = &downloads["artifact"];
        let (rel_path, url) = if !art.is_null() {
            (art["path"].as_str().map(String::from), art["url"].as_str().map(String::from))
        } else {
            // Maven-style library (Fabric, Quilt, ...).
            let rel = version::maven_path(name);
            let base = lib["url"].as_str().unwrap_or(LIBRARIES_URL);
            let base = if base.ends_with('/') { base.to_string() } else { format!("{base}/") };
            let url = rel.as_ref().map(|r| format!("{base}{r}"));
            (rel, url)
        };
        let Some(rel_path) = rel_path.or_else(|| version::maven_path(name)) else { continue };
        let path = libraries_dir.join(&rel_path);
        match url.filter(|u| !u.is_empty()) {
            Some(url) => jobs.push(Job {
                url,
                path: path.clone(),
                sha1: art["sha1"].as_str().or(lib["sha1"].as_str()).map(String::from),
                size: art["size"].as_u64().or(lib["size"].as_u64()),
            }),
            None if !path.is_file() => {
                reporter.log(format!("[!] no download for library {name}, skipping"));
                continue;
            }
            None => {}
        }
        if !classpath.contains(&path) {
            classpath.push(path);
        }
    }
    // A profile that inherits the game (Fabric, Forge, ...) runs it from a jar named after itself,
    // like the official launcher does: Forge's `ignoreList` finds the game jar by that name and
    // fails with split packages when it is missing.
    let launch_jar = match jar_id == id {
        true => client_jar.clone(),
        false => root.join("versions").join(id).join(format!("{id}.jar")),
    };
    classpath.push(launch_jar.clone());

    // Asset index
    let assets_root = root.join("assets");
    let index_info = &version["assetIndex"];
    let assets_index =
        index_info["id"].as_str().or(version["assets"].as_str()).unwrap_or("legacy").to_string();
    let index_path = assets_root.join("indexes").join(format!("{assets_index}.json"));
    if let Some(url) = index_info["url"].as_str() {
        let sha1 = index_info["sha1"].as_str();
        if !http::file_ok(&index_path, sha1, None) {
            reporter.status("Downloading asset index");
            http::download(url, &index_path, sha1, &|_| {})?;
        }
    }
    let index: Value = serde_json::from_slice(
        &fs::read(&index_path).with_context(|| format!("asset index {assets_index}"))?,
    )?;
    let objects = index["objects"].as_object().cloned().unwrap_or_default();
    for obj in objects.values() {
        let Some(hash) = obj["hash"].as_str() else { continue };
        let sub = format!("{}/{hash}", &hash[..2]);
        jobs.push(Job {
            url: format!("{RESOURCES_URL}{sub}"),
            path: assets_root.join("objects").join(&sub),
            sha1: Some(hash.into()),
            size: obj["size"].as_u64(),
        });
    }

    // log4j config (also carries the Log4Shell mitigation for old versions)
    let mut logging_arg = None;
    let log_cfg = &version["logging"]["client"];
    if let (Some(url), Some(file_id), Some(arg)) =
        (log_cfg["file"]["url"].as_str(), log_cfg["file"]["id"].as_str(), log_cfg["argument"].as_str())
    {
        let path = assets_root.join("log_configs").join(file_id);
        jobs.push(Job {
            url: url.into(),
            path: path.clone(),
            sha1: log_cfg["file"]["sha1"].as_str().map(String::from),
            size: log_cfg["file"]["size"].as_u64(),
        });
        logging_arg = Some(arg.replace("${path}", &path.to_string_lossy()));
    }

    download::run(jobs, "Downloading game files", reporter)?;
    if launch_jar != client_jar
        && !http::file_ok(&launch_jar, None, fs::metadata(&client_jar).ok().map(|m| m.len()))
    {
        let _ = fs::remove_file(&launch_jar);
        if let Some(parent) = launch_jar.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::hard_link(&client_jar, &launch_jar).is_err() {
            fs::copy(&client_jar, &launch_jar)
                .with_context(|| format!("copying {}", client_jar.display()))?;
        }
    }

    // Old versions read assets from a flat "virtual" tree instead of the hashed store.
    let mut game_assets = assets_root.clone();
    let is_virtual = index["virtual"].as_bool() == Some(true);
    let map_to_resources = index["map_to_resources"].as_bool() == Some(true);
    if is_virtual || map_to_resources {
        game_assets = if map_to_resources {
            game_dir.join("resources")
        } else {
            assets_root.join("virtual").join(&assets_index)
        };
        reporter.status("Preparing legacy assets");
        for (name, obj) in &objects {
            let Some(hash) = obj["hash"].as_str() else { continue };
            let target = game_assets.join(name);
            if !target.is_file() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(assets_root.join("objects").join(&hash[..2]).join(hash), &target)?;
            }
        }
    }

    // Natives
    let natives_dir = root.join("versions").join(id).join("natives");
    let _ = fs::remove_dir_all(&natives_dir);
    fs::create_dir_all(&natives_dir)?;
    for (jar, exclude) in &native_jars {
        extract_natives(jar, &natives_dir, exclude)
            .with_context(|| format!("extracting {}", jar.display()))?;
    }

    Ok(Prepared {
        id: id.to_string(),
        java_component: version["javaVersion"]["component"].as_str().unwrap_or("jre-legacy").to_string(),
        java_major: version["javaVersion"]["majorVersion"].as_u64().unwrap_or(8) as u32,
        version,
        classpath,
        natives_dir,
        assets_root,
        assets_index,
        game_assets,
        logging_arg,
    })
}

const MAVEN_CENTRAL: &str = "https://repo1.maven.org/maven2/";
const MACOS_JNA: &str = "5.17.0";

/// Replaces libraries that are known to break on the current platform.
///
/// JNA before 5.13 asserts in `LOAD_ERROR` ("snprintf() output has been truncated") when
/// `dlerror()` is long, and recent macOS lists every path dyld tried, so the game aborts
/// while mods probe for native libraries (e.g. Minecraft 1.20.1 ships JNA 5.12.1).
fn library_override(lib: &Value) -> Option<Value> {
    if platform::os_name() != "osx" {
        return None;
    }
    let name = lib["name"].as_str()?;
    let (artifact, ver) = name.strip_prefix("net.java.dev.jna:")?.split_once(':')?;
    if !matches!(artifact, "jna" | "jna-platform") || version_lt(ver, MACOS_JNA) != Some(true) {
        return None;
    }
    Some(serde_json::json!({
        "name": format!("net.java.dev.jna:{artifact}:{MACOS_JNA}"),
        "url": MAVEN_CENTRAL,
    }))
}

/// Compares dotted numeric versions; `None` if either is not purely numeric.
fn version_lt(a: &str, b: &str) -> Option<bool> {
    let parse = |s: &str| s.split('.').map(|p| p.parse::<u32>().ok()).collect::<Option<Vec<_>>>();
    Some(parse(a)? < parse(b)?)
}

fn extract_natives(jar: &Path, dest: &Path, exclude: &[String]) -> Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(jar)?)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        if entry.is_dir()
            || name.starts_with("META-INF/")
            || exclude.iter().any(|e| name.starts_with(e.as_str()))
        {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest.join(rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}
