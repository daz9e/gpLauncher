//! Installs Forge and NeoForge the way their installers do, without the installer UI:
//! the version profile is written into `versions/`, libraries are downloaded or unpacked from the
//! installer, and the install processors (which patch the game jar) are run with Java.
//!
//! Old Forge installers (up to 1.12) carry a `versionInfo` profile and the universal jar instead
//! of processors; both kinds are handled.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use zip::ZipArchive;

use crate::download::{self, Job};
use crate::instance::Loader;
use crate::version::{self, VersionEntry};
use crate::{Reporter, http, install, java};

const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/";
/// Written next to an installed profile once every step succeeded.
const MARKER: &str = ".gplauncher-installed";

/// Maven coordinates of the installer artifact, e.g. `net.minecraftforge:forge:1.20.1-47.2.0`.
fn installer_coords(loader: Loader, mc: &str, version: &str) -> (String, &'static str) {
    match loader {
        // NeoForge for 1.20.1 still used Forge's artifact name and version scheme.
        Loader::NeoForge if mc == "1.20.1" => (format!("net.neoforged:forge:{mc}-{version}"), NEOFORGE_MAVEN),
        Loader::NeoForge => (format!("net.neoforged:neoforge:{version}"), NEOFORGE_MAVEN),
        _ => (format!("net.minecraftforge:forge:{mc}-{version}"), FORGE_MAVEN),
    }
}

/// File remembering which profile id an installed (loader, mc, version) produced.
fn alias_path(root: &Path, loader: Loader, mc: &str, version: &str) -> PathBuf {
    root.join("versions/.loaders").join(format!("{}-{mc}-{version}", loader.label().to_lowercase()))
}

/// Profile id of an already installed loader version.
pub fn installed_id(root: &Path, loader: Loader, mc: &str, version: &str) -> Option<String> {
    let id = fs::read_to_string(alias_path(root, loader, mc, version)).ok()?.trim().to_string();
    let dir = root.join("versions").join(&id);
    (dir.join(format!("{id}.json")).is_file() && dir.join(MARKER).is_file()).then_some(id)
}

/// Installs Forge/NeoForge `version` for Minecraft `mc` and returns the profile id to launch.
/// `java` overrides the runtime the processors run with.
#[allow(clippy::too_many_arguments)]
pub fn install(
    root: &Path,
    game_dir: &Path,
    loader: Loader,
    mc: &str,
    version: &str,
    manifest: &[VersionEntry],
    java: Option<&Path>,
    reporter: &Reporter,
) -> Result<String> {
    if let Some(id) = installed_id(root, loader, mc, version) {
        return Ok(id);
    }
    let label = loader.label();
    let (coords, maven) = installer_coords(loader, mc, version);
    let rel = version::maven_path(&format!("{coords}:installer")).context("bad installer coordinates")?;
    let installer = root.join("cache/installers").join(Path::new(&rel).file_name().unwrap_or_default());
    if !installer.is_file() {
        reporter.status(format!("Downloading the {label} {version} installer"));
        http::download(&format!("{maven}{rel}"), &installer, None, &|_| {})
            .with_context(|| format!("{label} {version} has no installer for Minecraft {mc}"))?;
    }
    let mut zip = ZipArchive::new(fs::File::open(&installer)?).context("the installer is not a jar")?;
    let profile: Value = serde_json::from_str(&entry_text(&mut zip, "install_profile.json")?)
        .context("parsing install_profile.json")?;

    // The vanilla game has to be there first: processors patch its jar.
    reporter.status(format!("Installing Minecraft {mc}"));
    let prepared = install::install(root, game_dir, mc, manifest, reporter)?;

    let id = if profile["versionInfo"].is_object() {
        install_legacy(root, mc, &profile, &mut zip, reporter)?
    } else {
        let java = match java {
            Some(path) => path.to_path_buf(),
            None => java::ensure(root, &prepared.java_component, prepared.java_major, reporter)?,
        };
        install_modern(root, mc, &profile, &mut zip, &installer, &java, reporter)?
    };
    fs::write(root.join("versions").join(&id).join(MARKER), "")?;
    let alias = alias_path(root, loader, mc, version);
    fs::create_dir_all(alias.parent().unwrap())?;
    fs::write(alias, &id)?;
    Ok(id)
}

fn install_modern(
    root: &Path,
    mc: &str,
    profile: &Value,
    zip: &mut ZipArchive<fs::File>,
    installer: &Path,
    java: &Path,
    reporter: &Reporter,
) -> Result<String> {
    let libraries = root.join("libraries");
    let json_entry = profile["json"].as_str().unwrap_or("/version.json").trim_start_matches('/');
    let version_json: Value =
        serde_json::from_str(&entry_text(zip, json_entry)?).context("parsing the version profile")?;
    let id = version_json["id"].as_str().context("the version profile has no id")?.to_string();
    reporter.status(format!("Installing {id}"));

    // Jars shipped inside the installer (the loader itself on newer versions).
    extract_prefix(zip, "maven/", &libraries)?;

    // Libraries the processors need.
    let jobs: Vec<Job> = profile["libraries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|lib| {
            let art = &lib["downloads"]["artifact"];
            let url = art["url"].as_str().filter(|u| !u.is_empty())?;
            let path = art["path"]
                .as_str()
                .map(String::from)
                .or_else(|| version::maven_path(lib["name"].as_str()?))?;
            Some(Job {
                url: url.to_string(),
                path: libraries.join(path),
                sha1: art["sha1"].as_str().map(String::from),
                size: art["size"].as_u64(),
            })
        })
        .collect();
    download::run(jobs, "Downloading installer libraries", reporter)?;

    // Values the processor arguments refer to as `{NAME}`.
    let work = root.join("cache/installers/work").join(&id);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    let mut data: HashMap<String, String> = HashMap::new();
    for (key, value) in profile["data"].as_object().into_iter().flatten() {
        let Some(client) = value["client"].as_str() else { continue };
        let resolved = if let Some(coords) = client.strip_prefix('[').and_then(|c| c.strip_suffix(']')) {
            lib_path(&libraries, coords)?.to_string_lossy().into_owned()
        } else if let Some(literal) = client.strip_prefix('\'').and_then(|c| c.strip_suffix('\'')) {
            literal.to_string()
        } else if let Some(entry) = client.strip_prefix('/') {
            let out = work.join(entry);
            fs::create_dir_all(out.parent().unwrap())?;
            let mut file = fs::File::create(&out)?;
            std::io::copy(
                &mut zip.by_name(entry).with_context(|| format!("{entry} missing in installer"))?,
                &mut file,
            )?;
            out.to_string_lossy().into_owned()
        } else {
            client.to_string()
        };
        data.insert(key.clone(), resolved);
    }
    let mc_jar = root.join("versions").join(mc).join(format!("{mc}.jar"));
    for (key, value) in [
        ("SIDE", "client".to_string()),
        ("MINECRAFT_JAR", mc_jar.to_string_lossy().into_owned()),
        ("MINECRAFT_VERSION", mc.to_string()),
        ("ROOT", root.to_string_lossy().into_owned()),
        ("INSTALLER", installer.to_string_lossy().into_owned()),
        ("LIBRARY_DIR", libraries.to_string_lossy().into_owned()),
    ] {
        data.insert(key.into(), value);
    }

    let processors: Vec<&Value> = profile["processors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["sides"].as_array().is_none_or(|s| s.iter().any(|s| s == "client")))
        .collect();
    for (n, p) in processors.iter().enumerate() {
        reporter.status(format!("Running {id} installer step {}/{}", n + 1, processors.len()));
        reporter.progress(n as u64, processors.len() as u64);
        run_processor(p, &libraries, &data, java, reporter)
            .with_context(|| format!("installer step {} failed", n + 1))?;
    }
    reporter.progress(0, 0);
    let _ = fs::remove_dir_all(&work);

    write_profile(root, &id, &version_json)?;
    Ok(id)
}

fn run_processor(
    p: &Value,
    libraries: &Path,
    data: &HashMap<String, String>,
    java: &Path,
    reporter: &Reporter,
) -> Result<()> {
    let substitute = |arg: &str| -> Result<String> {
        if let Some(key) = arg.strip_prefix('{').and_then(|a| a.strip_suffix('}')) {
            return data.get(key).cloned().with_context(|| format!("unknown installer value {key}"));
        }
        if let Some(coords) = arg.strip_prefix('[').and_then(|a| a.strip_suffix(']')) {
            return Ok(lib_path(libraries, coords)?.to_string_lossy().into_owned());
        }
        Ok(arg.to_string())
    };

    // Skip steps whose outputs are already in place.
    let outputs: Vec<(String, String)> = p["outputs"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            Ok((substitute(k)?, substitute(v.as_str().unwrap_or_default())?.trim_matches('\'').to_string()))
        })
        .collect::<Result<_>>()?;
    if !outputs.is_empty()
        && outputs.iter().all(|(path, sha1)| http::file_ok(Path::new(path), Some(sha1), None))
    {
        return Ok(());
    }

    let jar = lib_path(libraries, p["jar"].as_str().context("processor without a jar")?)?;
    let main_class = main_class(&jar)?;
    let mut classpath = vec![jar];
    for coords in p["classpath"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        classpath.push(lib_path(libraries, coords)?);
    }
    let classpath = std::env::join_paths(&classpath)?;
    let args: Vec<String> = p["args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(substitute)
        .collect::<Result<_>>()?;

    let mut cmd = Command::new(java);
    cmd.arg("-cp").arg(classpath).arg(&main_class).args(&args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().with_context(|| format!("starting {}", java.display()))?;
    let err = child.stderr.take().map(|s| {
        let reporter = reporter.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(s).lines().map_while(Result::ok) {
                reporter.log(format!("[installer] {line}"));
            }
        })
    });
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            reporter.log(format!("[installer] {line}"));
        }
    }
    if let Some(t) = err {
        let _ = t.join();
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("{main_class} exited with {status}; see the log for details");
    }
    for (path, sha1) in &outputs {
        if !http::file_ok(Path::new(path), Some(sha1), None) {
            bail!("{main_class} produced a damaged {path}");
        }
    }
    Ok(())
}

fn install_legacy(
    root: &Path,
    mc: &str,
    profile: &Value,
    zip: &mut ZipArchive<fs::File>,
    reporter: &Reporter,
) -> Result<String> {
    let mut version_json = profile["versionInfo"].clone();
    let id = version_json["id"].as_str().context("the version profile has no id")?.to_string();
    reporter.status(format!("Installing {id}"));

    // The universal jar ships inside the installer.
    let install = &profile["install"];
    if let (Some(file), Some(coords)) = (install["filePath"].as_str(), install["path"].as_str()) {
        let dest = lib_path(&root.join("libraries"), coords)?;
        if !dest.is_file() {
            fs::create_dir_all(dest.parent().unwrap())?;
            let mut out = fs::File::create(&dest)?;
            std::io::copy(
                &mut zip.by_name(file).with_context(|| format!("{file} missing in installer"))?,
                &mut out,
            )?;
        }
    }

    // Keep client libraries only, and point them at the current Forge maven.
    if let Some(libs) = version_json["libraries"].as_array_mut() {
        libs.retain(|l| l["clientreq"].as_bool() != Some(false));
        for lib in libs.iter_mut() {
            if let Some(url) = lib["url"].as_str()
                && url.contains("files.minecraftforge.net")
            {
                lib["url"] = Value::String(FORGE_MAVEN.into());
            }
        }
    }
    // Very old profiles repeat the whole game instead of inheriting it.
    if version_json["inheritsFrom"].is_null() {
        version_json["inheritsFrom"] = Value::String(mc.into());
    }
    if version_json["jar"].is_null() {
        version_json["jar"] = Value::String(mc.into());
    }
    write_profile(root, &id, &version_json)?;
    Ok(id)
}

fn write_profile(root: &Path, id: &str, json: &Value) -> Result<()> {
    let path = root.join("versions").join(id).join(format!("{id}.json"));
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, serde_json::to_vec_pretty(json)?)?;
    Ok(())
}

fn lib_path(libraries: &Path, coords: &str) -> Result<PathBuf> {
    Ok(libraries.join(version::maven_path(coords).with_context(|| format!("bad library name {coords}"))?))
}

fn main_class(jar: &Path) -> Result<String> {
    let mut zip =
        ZipArchive::new(fs::File::open(jar).with_context(|| format!("opening {}", jar.display()))?)?;
    let manifest = entry_text(&mut zip, "META-INF/MANIFEST.MF")?;
    manifest
        .lines()
        .find_map(|l| l.strip_prefix("Main-Class:"))
        .map(|c| c.trim().to_string())
        .with_context(|| format!("{} has no Main-Class", jar.display()))
}

fn entry_text(zip: &mut ZipArchive<fs::File>, name: &str) -> Result<String> {
    let mut text = String::new();
    zip.by_name(name).with_context(|| format!("{name} missing"))?.read_to_string(&mut text)?;
    Ok(text)
}

/// Extracts entries under `prefix` into `dest`, keeping files that already exist.
fn extract_prefix(zip: &mut ZipArchive<fs::File>, prefix: &str, dest: &Path) -> Result<()> {
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else { continue };
        let Ok(rel) = rel.strip_prefix(prefix.trim_end_matches('/')) else { continue };
        let out = dest.join(rel);
        if out.is_file() {
            continue;
        }
        fs::create_dir_all(out.parent().unwrap())?;
        let mut file = fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

// ---- versions --------------------------------------------------------------------

/// A loader version offered for a Minecraft version.
#[derive(Clone, Debug, Serialize)]
pub struct LoaderVersion {
    pub version: String,
    /// Recommended (Forge) or not a beta.
    pub stable: bool,
}

/// Forge or NeoForge versions for Minecraft `mc`, newest first.
pub fn versions(loader: Loader, mc: &str) -> Result<Vec<LoaderVersion>> {
    match loader {
        Loader::Forge => {
            let all = maven_versions(&format!("{FORGE_MAVEN}net/minecraftforge/forge/maven-metadata.xml"))?;
            let promos = http::get_json(
                "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json",
            )
            .unwrap_or_default();
            let recommended = promos["promos"][format!("{mc}-recommended")].as_str().map(String::from);
            let prefix = format!("{mc}-");
            let mut out: Vec<LoaderVersion> = all
                .iter()
                .filter_map(|v| v.strip_prefix(&prefix))
                .map(|v| LoaderVersion { stable: recommended.as_deref() == Some(v), version: v.to_string() })
                .collect();
            out.sort_by(|a, b| compare_versions(&b.version, &a.version));
            Ok(out)
        }
        Loader::NeoForge if mc == "1.20.1" => {
            let all = maven_versions(&format!("{NEOFORGE_MAVEN}net/neoforged/forge/maven-metadata.xml"))?;
            let mut out: Vec<LoaderVersion> = all
                .iter()
                .filter_map(|v| v.strip_prefix("1.20.1-"))
                .map(|v| LoaderVersion { version: v.to_string(), stable: !v.contains("beta") })
                .collect();
            out.sort_by(|a, b| compare_versions(&b.version, &a.version));
            Ok(out)
        }
        Loader::NeoForge => {
            let all = maven_versions(&format!("{NEOFORGE_MAVEN}net/neoforged/neoforge/maven-metadata.xml"))?;
            let mut out: Vec<LoaderVersion> = all
                .into_iter()
                .filter(|v| neoforge_minecraft(v).as_deref() == Some(mc))
                .map(|v| LoaderVersion { stable: !v.contains("beta") && !v.contains("alpha"), version: v })
                .collect();
            out.sort_by(|a, b| compare_versions(&b.version, &a.version));
            Ok(out)
        }
        _ => Ok(Vec::new()),
    }
}

/// Minecraft versions Forge or NeoForge has builds for.
pub fn supported_minecraft(loader: Loader) -> Result<std::collections::HashSet<String>> {
    Ok(match loader {
        Loader::Forge => {
            maven_versions(&format!("{FORGE_MAVEN}net/minecraftforge/forge/maven-metadata.xml"))?
                .iter()
                .filter_map(|v| v.split_once('-').map(|(mc, _)| mc.to_string()))
                .collect()
        }
        Loader::NeoForge => {
            let mut set: std::collections::HashSet<String> =
                maven_versions(&format!("{NEOFORGE_MAVEN}net/neoforged/neoforge/maven-metadata.xml"))?
                    .iter()
                    .filter_map(|v| neoforge_minecraft(v))
                    .collect();
            set.insert("1.20.1".into());
            set
        }
        _ => Default::default(),
    })
}

/// The Minecraft version a NeoForge version is for: `20.4.237` → `1.20.4`, `21.0.1` → `1.21`,
/// and from the year-based scheme on `26.1.2.5` → `26.1.2`, `26.3.0.21-beta` → `26.3`.
fn neoforge_minecraft(version: &str) -> Option<String> {
    let parts: Vec<u32> =
        version.split(['-', '+']).next()?.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match parts.as_slice() {
        [major, minor, ..] if *major < 25 => {
            Some(if *minor == 0 { format!("1.{major}") } else { format!("1.{major}.{minor}") })
        }
        [year, drop, patch, _, ..] => {
            Some(if *patch == 0 { format!("{year}.{drop}") } else { format!("{year}.{drop}.{patch}") })
        }
        _ => None,
    }
}

fn maven_versions(url: &str) -> Result<Vec<String>> {
    let resp = http::agent().get(url).call().with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("GET {url}: HTTP {}", resp.status());
    }
    let text = resp.into_body().with_config().limit(16 * 1024 * 1024).read_to_string()?;
    Ok(text
        .split("<version>")
        .skip(1)
        .filter_map(|s| s.split_once("</version>").map(|(v, _)| v.trim().to_string()))
        .collect())
}

/// Compares versions number by number (`47.10.0` > `47.9.1`); text parts compare as text.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> Vec<String> { s.split(['.', '-', '+']).map(String::from).collect() };
    let (a, b) = (split(a), split(b));
    for (x, y) in a.iter().zip(&b) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            // A release sorts after its betas: `1.0` > `1.0-beta`.
            (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
            (Err(_), Ok(_)) => std::cmp::Ordering::Less,
            _ => x.cmp(y),
        };
        if ord.is_ne() {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_versions_map_to_minecraft() {
        assert_eq!(neoforge_minecraft("20.4.237").as_deref(), Some("1.20.4"));
        assert_eq!(neoforge_minecraft("21.0.167").as_deref(), Some("1.21"));
        assert_eq!(neoforge_minecraft("21.1.77").as_deref(), Some("1.21.1"));
        assert_eq!(neoforge_minecraft("26.3.0.21-beta").as_deref(), Some("26.3"));
        assert_eq!(neoforge_minecraft("26.1.2.5").as_deref(), Some("26.1.2"));
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(compare_versions("47.10.0", "47.9.1").is_gt());
        assert!(compare_versions("21.1.77", "21.1.8").is_gt());
    }
}
