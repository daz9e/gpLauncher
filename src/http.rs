use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha1::{Digest, Sha1};

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .user_agent(concat!("gpLauncher/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into()
});

pub fn agent() -> &'static ureq::Agent {
    &AGENT
}

/// Reads a response body as JSON, failing with the body text on non-2xx statuses.
pub fn read_json(mut resp: ureq::http::Response<ureq::Body>, what: &str) -> Result<Value> {
    let status = resp.status();
    let text = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .with_context(|| format!("{what}: failed to read response"))?;
    if !status.is_success() {
        bail!("{what}: HTTP {status}: {}", text.chars().take(300).collect::<String>());
    }
    serde_json::from_str(&text).with_context(|| format!("{what}: invalid JSON"))
}

pub fn get_json(url: &str) -> Result<Value> {
    let resp = agent().get(url).call().with_context(|| format!("GET {url}"))?;
    read_json(resp, url)
}

pub fn sha1_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Cheap check whether a file is already present and valid.
/// Size is compared when known; the hash is only computed when there is no size.
pub fn file_ok(path: &Path, sha1: Option<&str>, size: Option<u64>) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if let Some(size) = size {
        return meta.len() == size;
    }
    match sha1 {
        Some(expected) => sha1_file(path).is_ok_and(|h| h.eq_ignore_ascii_case(expected)),
        None => true,
    }
}

/// Downloads `url` to `path` atomically (via a temp file), verifying SHA-1 if given.
/// `on_bytes` is called with the number of bytes received for each chunk.
pub fn download(url: &str, path: &Path, sha1: Option<&str>, on_bytes: &dyn Fn(u64)) -> Result<()> {
    let mut last_err = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500 * attempt));
        }
        match try_download(url, path, sha1, on_bytes) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap())
}

fn try_download(url: &str, path: &Path, sha1: Option<&str>, on_bytes: &dyn Fn(u64)) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut resp = agent().get(url).call().with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("GET {url}: HTTP {}", resp.status());
    }
    let tmp = path.with_extension(format!(
        "{}part",
        path.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()
    ));
    let mut out = fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    let mut reader = resp.body_mut().as_reader();
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).with_context(|| format!("reading {url}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        on_bytes(n as u64);
    }
    out.flush()?;
    drop(out);
    if let Some(expected) = sha1 {
        let actual = hex(&hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&tmp);
            bail!("{url}: checksum mismatch (expected {expected}, got {actual})");
        }
    }
    fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}

/// Uploads a log to mclo.gs and returns the link to it. Their service hides IP addresses;
/// the launcher already keeps access tokens out of its log.
pub fn upload_log(text: &str) -> Result<String> {
    let url = "https://api.mclo.gs/1/log";
    let resp = agent().post(url).send_form([("content", text)]).with_context(|| format!("POST {url}"))?;
    let v = read_json(resp, url)?;
    match v["url"].as_str() {
        Some(link) if v["success"].as_bool() == Some(true) => Ok(link.to_string()),
        _ => bail!("mclo.gs: {}", v["error"].as_str().unwrap_or("upload failed")),
    }
}
