use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::auth::{self, Account};
use crate::install::{self, Prepared};
use crate::instance::Instance;
use crate::settings::Settings;
use crate::version::{self, VersionEntry};
use crate::{Event, Reporter, java, loader, platform};

/// Shared handle to the game process, so another thread can stop it.
#[derive(Clone, Default)]
pub struct GameHandle(Arc<Mutex<Option<Child>>>);

impl GameHandle {
    /// Kills the game if it is running. Returns whether there was a process to kill.
    pub fn kill(&self) -> bool {
        match self.0.lock().unwrap().as_mut() {
            Some(child) => child.kill().is_ok(),
            None => false,
        }
    }
}

/// Full pipeline: refresh account -> install (with mod loader) -> Java -> start the game ->
/// stream its output. Blocks until the game exits; `handle` can kill it meanwhile.
pub fn run(
    settings: &Settings,
    instance: &mut Instance,
    manifest: &[VersionEntry],
    reporter: &Reporter,
    handle: &GameHandle,
) -> Result<()> {
    let mut account = settings.account().cloned().context("no account selected")?;
    if account.needs_refresh() {
        reporter.status("Refreshing Microsoft sign-in");
        account = auth::refresh(&settings.ms_client_id, &account)?;
        reporter.send(Event::AccountRefreshed(account.clone()));
    }

    let root = &settings.data_dir;
    std::fs::create_dir_all(&instance.game_dir)?;
    let java_path = [&instance.java_path, &settings.java_path]
        .map(|p| p.trim())
        .into_iter()
        .find(|p| !p.is_empty())
        .map(PathBuf::from);
    let had_loader_version = !instance.loader_version.is_empty();
    let version_id = loader::prepare(root, instance, manifest, java_path.as_deref(), reporter)?;
    if !had_loader_version && !instance.loader_version.is_empty() {
        reporter.send(Event::InstanceUpdated(instance.clone()));
    }
    let prepared = install::install(root, &instance.game_dir, &version_id, manifest, reporter)?;

    let java = match java_path {
        Some(path) => path,
        None => java::ensure(root, &prepared.java_component, prepared.java_major, reporter)?,
    };

    let mut cmd = build_command(&java, &prepared, settings, instance, &account);
    reporter.status(format!("Launching {} ({version_id})", instance.name));
    reporter.log(format!("> {}", redact(&cmd, &account)));
    let mut child = spawn(&mut cmd).with_context(|| format!("failed to start {}", java.display()))?;
    reporter.send(Event::GameStarted);
    reporter.progress(0, 0);

    let out = child.stdout.take().map(|s| pipe_lines(s, reporter.clone()));
    let err = child.stderr.take().map(|s| pipe_lines(s, reporter.clone()));
    *handle.0.lock().unwrap() = Some(child);
    let status = loop {
        let status = handle.0.lock().unwrap().as_mut().map(Child::try_wait).transpose()?.flatten();
        if let Some(status) = status {
            break status;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    *handle.0.lock().unwrap() = None;
    for t in [out, err].into_iter().flatten() {
        let _ = t.join();
    }
    reporter.send(Event::GameExited(status.code()));
    Ok(())
}

fn spawn(cmd: &mut Command) -> std::io::Result<Child> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
}

fn pipe_lines(stream: impl Read + Send + 'static, reporter: Reporter) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut event: Option<String> = None;
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        // Read raw bytes: output in a non-UTF-8 code page (cp1251 on Windows) must not stop
        // the reader, or the pipe fills up and the game blocks.
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string();
            // The logging config Mojang ships emits log4j XML events; render them as plain lines.
            if line.trim_start().starts_with("<log4j:Event") {
                event = Some(line);
            } else if let Some(buf) = event.as_mut() {
                buf.push('\n');
                buf.push_str(&line);
                if line.trim_start().starts_with("</log4j:Event>") {
                    reporter.log(format_log4j(&event.take().unwrap()));
                }
            } else {
                reporter.log(line);
            }
        }
    })
}

fn format_log4j(xml: &str) -> String {
    let attr = |name: &str| {
        let key = format!("{name}=\"");
        xml.find(&key).and_then(|i| {
            let rest = &xml[i + key.len()..];
            rest.find('"').map(|end| rest[..end].to_string())
        })
    };
    let cdata = |tag: &str| {
        let start = xml.find(&format!("<log4j:{tag}><![CDATA["))?;
        let rest = &xml[start + tag.len() + 17..];
        rest.find("]]>").map(|end| rest[..end].to_string())
    };
    let time = attr("timestamp")
        .and_then(|t| t.parse::<u64>().ok())
        .map(|ms| {
            let s = ms / 1000;
            format!("{:02}:{:02}:{:02}", s / 3600 % 24, s / 60 % 60, s % 60)
        })
        .unwrap_or_default();
    let mut line = format!(
        "[{time}] [{}/{}]: {}",
        attr("thread").unwrap_or_default(),
        attr("level").unwrap_or_default(),
        cdata("Message").unwrap_or_default()
    );
    if let Some(t) = cdata("Throwable") {
        line.push('\n');
        line.push_str(&t);
    }
    line
}

pub fn build_command(
    java: &Path,
    p: &Prepared,
    settings: &Settings,
    instance: &Instance,
    account: &Account,
) -> Command {
    let v = &p.version;
    let game_dir = &instance.game_dir;
    let classpath = p
        .classpath
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(platform::classpath_separator());

    let mut vars: HashMap<&str, String> = HashMap::from([
        ("auth_player_name", account.name.clone()),
        ("version_name", p.id.clone()),
        ("game_directory", game_dir.to_string_lossy().into()),
        ("assets_root", p.assets_root.to_string_lossy().into()),
        ("assets_index_name", p.assets_index.clone()),
        ("game_assets", p.game_assets.to_string_lossy().into()),
        ("auth_uuid", account.uuid.clone()),
        ("auth_access_token", account.access_token.clone()),
        ("auth_session", format!("token:{}:{}", account.access_token, account.uuid)),
        ("auth_xuid", account.xuid.clone().unwrap_or_default()),
        ("clientid", String::new()),
        ("user_type", account.user_type().into()),
        ("user_properties", "{}".into()),
        ("version_type", v["type"].as_str().unwrap_or("release").into()),
        ("natives_directory", p.natives_dir.to_string_lossy().into()),
        ("launcher_name", "gplauncher".into()),
        ("launcher_version", env!("CARGO_PKG_VERSION").into()),
        ("classpath", classpath),
        ("classpath_separator", platform::classpath_separator().into()),
        ("library_directory", settings.data_dir.join("libraries").to_string_lossy().into()),
    ]);
    let mut features: HashMap<&str, bool> = HashMap::new();
    let resolution = instance.resolution().or(settings.resolution());
    if let Some((width, height)) = resolution {
        features.insert("has_custom_resolution", true);
        vars.insert("resolution_width", width.to_string());
        vars.insert("resolution_height", height.to_string());
    }

    let memory = instance.memory_mb.unwrap_or(settings.memory_mb);
    let mut jvm = vec![
        format!("-Xmx{memory}M"),
        format!("-Xms{}M", memory.min(1024)),
        // Java 19+ otherwise writes piped output in the console code page.
        "-Dstdout.encoding=UTF-8".into(),
        "-Dstderr.encoding=UTF-8".into(),
    ];
    jvm.extend(settings.jvm_args.split_whitespace().map(String::from));
    jvm.extend(instance.jvm_args.split_whitespace().map(String::from));
    let mut game = Vec::new();

    if v["arguments"].is_object() {
        collect_args(&v["arguments"]["jvm"], &features, &mut jvm);
        collect_args(&v["arguments"]["game"], &features, &mut game);
    } else {
        // Pre-1.13 format.
        if platform::os_name() == "osx" {
            jvm.push("-XstartOnFirstThread".into());
        }
        jvm.push("-Djava.library.path=${natives_directory}".into());
        jvm.push("-cp".into());
        jvm.push("${classpath}".into());
        game.extend(
            v["minecraftArguments"].as_str().unwrap_or_default().split_whitespace().map(String::from),
        );
        if resolution.is_some() {
            game.extend(
                ["--width", "${resolution_width}", "--height", "${resolution_height}"].map(String::from),
            );
        }
    }
    if instance.fullscreen.unwrap_or(settings.fullscreen) {
        game.push("--fullscreen".into());
    }
    if let Some(arg) = &p.logging_arg {
        jvm.push(arg.clone());
    }

    let mut cmd = Command::new(java);
    cmd.current_dir(game_dir);
    cmd.args(jvm.iter().map(|a| substitute(a, &vars)));
    cmd.arg(v["mainClass"].as_str().unwrap_or("net.minecraft.client.main.Main"));
    cmd.args(game.iter().map(|a| substitute(a, &vars)));
    cmd
}

fn collect_args(list: &Value, features: &HashMap<&str, bool>, out: &mut Vec<String>) {
    for arg in list.as_array().into_iter().flatten() {
        match arg {
            Value::String(s) => out.push(s.clone()),
            Value::Object(_) if version::rules_allow(&arg["rules"], features) => match &arg["value"] {
                Value::String(s) => out.push(s.clone()),
                Value::Array(items) => out.extend(items.iter().filter_map(|i| i.as_str().map(String::from))),
                _ => {}
            },
            _ => {}
        }
    }
}

fn substitute(arg: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match vars.get(key) {
                    Some(value) => out.push_str(value),
                    None => out.push_str(&rest[start..start + 2 + end + 1]),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Command line for the log, with the access token hidden and the classpath shortened.
fn redact(cmd: &Command, account: &Account) -> String {
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    for arg in cmd.get_args() {
        let mut a = arg.to_string_lossy().into_owned();
        if account.access_token.len() > 4 {
            a = a.replace(&account.access_token, "<token>");
        }
        if a.len() > 300 {
            a = format!("{}…", &a[..a.floor_char_boundary(120)]);
        }
        parts.push(a);
    }
    parts.join(" ")
}
