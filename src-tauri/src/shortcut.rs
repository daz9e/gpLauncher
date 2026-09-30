//! Desktop shortcuts that start the launcher with `--launch <instance id>`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use gplauncher::instance::Instance;

/// Command-line flag that launches an instance right after the window opens.
pub const LAUNCH_FLAG: &str = "--launch";

/// Instance id passed with [`LAUNCH_FLAG`], if any.
pub fn launch_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == LAUNCH_FLAG {
            return args.next();
        }
    }
    None
}

/// Creates a shortcut on the desktop and returns its path.
pub fn create(inst: &Instance) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locating the launcher executable")?;
    let desktop = dirs::desktop_dir().context("no desktop folder")?;
    let name: String = inst
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_.()[]".contains(c) { c } else { '_' })
        .collect();
    let name = name.trim();
    write(&desktop, name, &exe.to_string_lossy(), inst)
}

#[cfg(target_os = "macos")]
fn write(desktop: &std::path::Path, name: &str, exe: &str, inst: &Instance) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    // A minimal app bundle: opens without a Terminal window, unlike a `.command` script.
    let app = desktop.join(format!("{name}.app"));
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    let script = contents.join("MacOS/launch");
    fs::write(&script, format!("#!/bin/sh\nexec {} {LAUNCH_FLAG} {}\n", quote(exe), quote(&inst.id)))?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    fs::write(
        contents.join("Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>launch</string>
    <key>CFBundleName</key><string>{}</string>
    <key>CFBundleIdentifier</key><string>dev.gplauncher.shortcut.{}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
</dict>
</plist>
"#,
            xml_escape(&inst.name),
            inst.id.chars().filter(char::is_ascii_alphanumeric).collect::<String>(),
        ),
    )?;
    Ok(app)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn write(desktop: &std::path::Path, name: &str, exe: &str, inst: &Instance) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let path = desktop.join(format!("{name}.desktop"));
    // Names come from modpacks too; a line break would add keys to the entry.
    let title = inst.name.replace(char::is_control, " ");
    let icon = inst.icon.as_ref().map(|i| format!("Icon={}\n", i.display())).unwrap_or_default();
    fs::write(
        &path,
        format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec=\"{exe}\" {LAUNCH_FLAG} \"{}\"\n{icon}Terminal=false\n",
            title, inst.id
        ),
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

#[cfg(windows)]
fn write(desktop: &std::path::Path, name: &str, exe: &str, inst: &Instance) -> Result<PathBuf> {
    let path = desktop.join(format!("{name}.bat"));
    fs::write(&path, format!("@start \"\" \"{exe}\" {LAUNCH_FLAG} \"{}\"\r\n", inst.id))?;
    Ok(path)
}

/// Single-quotes `s` for a POSIX shell.
#[cfg(target_os = "macos")]
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
