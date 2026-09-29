//! Downloads the Java runtime Mojang ships for each version (`javaVersion.component`).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::download::{self, Job};
use crate::{Reporter, http};

const RUNTIMES_URL: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Platform keys to try in order. Apple Silicon and Windows on ARM fall back
/// to x64 runtimes (Rosetta / emulation) for components without native builds.
fn platform_keys() -> &'static [&'static str] {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => &["mac-os-arm64", "mac-os"],
        ("macos", _) => &["mac-os"],
        ("windows", "aarch64") => &["windows-arm64", "windows-x64"],
        ("windows", "x86") => &["windows-x86"],
        ("windows", _) => &["windows-x64"],
        ("linux", "x86") => &["linux-i386"],
        ("linux", "x86_64") => &["linux"],
        _ => &[],
    }
}

fn java_exe(runtime_dir: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        runtime_dir.join("jre.bundle/Contents/Home/bin/java")
    } else if cfg!(windows) {
        runtime_dir.join("bin/java.exe")
    } else {
        runtime_dir.join("bin/java")
    }
}

/// First line of `java -version`, e.g. `openjdk version "21.0.3" 2024-04-16`.
pub fn version(java: &Path) -> Result<String> {
    let mut cmd = std::process::Command::new(java);
    cmd.arg("-version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().with_context(|| format!("failed to run {}", java.display()))?;
    // Java prints the version to stderr.
    let text = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr });
    match text.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(line) if out.status.success() => Ok(line.to_string()),
        Some(line) => bail!("{line}"),
        None => bail!("{} printed no version", java.display()),
    }
}

/// Ensures runtime `component` is installed under `<root>/runtimes` and returns the java binary.
pub fn ensure(root: &Path, component: &str, major: u32, reporter: &Reporter) -> Result<PathBuf> {
    let dir = root.join("runtimes").join(component);
    let marker = dir.join(".installed");
    let exe = java_exe(&dir);

    reporter.status(format!("Checking Java ({component})"));
    let all = match http::get_json(RUNTIMES_URL) {
        Ok(v) => v,
        Err(e) if marker.is_file() && exe.is_file() => {
            reporter.log(format!("[!] offline, using the installed Java: {e:#}"));
            return Ok(exe);
        }
        Err(e) => return Err(e),
    };

    let manifest_ref =
        platform_keys().iter().find_map(|key| all[*key][component][0]["manifest"].as_object().cloned());
    let Some(manifest_ref) = manifest_ref else {
        bail!(
            "Mojang does not provide Java {major} ({component}) for this platform. \
             Install Java {major} manually and choose it in Settings → Java."
        );
    };
    let manifest_sha1 = manifest_ref["sha1"].as_str().unwrap_or_default().to_string();
    if exe.is_file() && fs::read_to_string(&marker).ok().as_deref() == Some(&manifest_sha1) {
        return Ok(exe);
    }

    reporter.status(format!("Downloading Java {major}"));
    let manifest = http::get_json(manifest_ref["url"].as_str().unwrap_or_default())?;
    let files = manifest["files"].as_object().context("empty Java runtime manifest")?;

    let mut jobs = Vec::new();
    let mut executables = Vec::new();
    let mut links = Vec::new();
    for (rel, info) in files {
        let path = dir.join(rel);
        match info["type"].as_str() {
            Some("directory") => fs::create_dir_all(&path)?,
            Some("file") => {
                let raw: &Value = &info["downloads"]["raw"];
                jobs.push(Job {
                    url: raw["url"].as_str().unwrap_or_default().into(),
                    path: path.clone(),
                    sha1: raw["sha1"].as_str().map(String::from),
                    size: raw["size"].as_u64(),
                });
                if info["executable"].as_bool() == Some(true) {
                    executables.push(path);
                }
            }
            Some("link") => {
                if let Some(target) = info["target"].as_str() {
                    links.push((path, target.to_string()));
                }
            }
            _ => {}
        }
    }
    download::run(jobs, &format!("Downloading Java {major}"), reporter)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in &executables {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
        }
        for (path, target) in &links {
            let _ = fs::remove_file(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            std::os::unix::fs::symlink(target, path)?;
        }
    }
    #[cfg(not(unix))]
    let _ = (&executables, &links);

    if !exe.is_file() {
        bail!("Java was installed, but {} is missing", exe.display());
    }
    fs::write(&marker, manifest_sha1)?;
    Ok(exe)
}
