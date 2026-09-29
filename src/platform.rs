use std::path::PathBuf;

/// OS name as used in Mojang's version JSON rules.
pub fn os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

/// Architecture name as used in Mojang's version JSON rules.
pub fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86" => "x86",
        "aarch64" => "arm64",
        "arm" => "arm32",
        _ => "x86_64",
    }
}

pub fn is_64bit() -> bool {
    cfg!(target_pointer_width = "64")
}

pub fn classpath_separator() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}

/// Default launcher directory:
/// - Windows: `%APPDATA%\.gplauncher`
/// - macOS: `~/Library/Application Support/gplauncher`
/// - Linux: `~/.gplauncher`
pub fn default_data_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "windows") {
        dirs::data_dir().unwrap_or(home).join(".gplauncher")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/gplauncher")
    } else {
        home.join(".gplauncher")
    }
}
