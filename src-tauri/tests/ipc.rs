//! The commands the windows call, run through Tauri's IPC on a mock runtime against a temporary
//! launcher folder. Tests marked `#[ignore]` talk to Mojang and Modrinth: `cargo test -- --ignored`.
//!
//! Not on Windows: settings are saved under the user's AppData there, which `HOME` does not move.
#![cfg(not(windows))]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, Instant};

use gplauncher::settings::Settings;
use serde_json::{Value, json};
use tauri::test::{INVOKE_KEY, MockRuntime, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{App, Listener, Manager, WebviewWindow, WebviewWindowBuilder};

/// Settings are always saved under `$HOME`; keep them out of the real one.
fn isolate_home() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = std::env::temp_dir().join(format!("gplauncher-test-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: runs once, before any test thread reads the environment through the launcher.
        unsafe { std::env::set_var("HOME", &home) };
    });
    assert!(dirs::home_dir().unwrap().starts_with(std::env::temp_dir()));
}

struct Harness {
    _app: App<MockRuntime>,
    window: WebviewWindow<MockRuntime>,
    dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Harness {
        isolate_home();
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings { data_dir: dir.path().to_path_buf(), ..Settings::default() };
        let app = gplauncher_app::app(mock_builder()).build(mock_context(noop_assets())).unwrap();
        gplauncher_app::init(app.handle(), settings).unwrap();
        let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        Harness { _app: app, window, dir }
    }

    fn data_dir(&self) -> &Path {
        self.dir.path()
    }

    fn call(&self, cmd: &str, body: Value) -> Result<Value, Value> {
        let request = InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        tauri::test::get_ipc_response(&self.window, request).map(|r| r.deserialize::<Value>().unwrap())
    }

    fn ok(&self, cmd: &str, body: Value) -> Value {
        self.call(cmd, body).unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
    }

    fn err(&self, cmd: &str, body: Value) -> String {
        match self.call(cmd, body) {
            Ok(v) => panic!("{cmd} should fail, returned {v}"),
            Err(e) => e.as_str().unwrap_or_default().to_string(),
        }
    }

    fn snapshot(&self) -> Value {
        self.ok("snapshot", json!({}))
    }

    fn instance_ids(&self) -> Vec<String> {
        self.snapshot()["instances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_string())
            .collect()
    }

    fn create(&self, name: &str, minecraft: &str, loader: &str) -> Value {
        self.ok(
            "create_instance",
            json!({ "name": name, "minecraft": minecraft, "loader": loader, "loaderVersion": "" }),
        )
    }

    /// Waits until the background job finished; returns its status line.
    fn wait_job(&self) -> String {
        let start = Instant::now();
        loop {
            let job = self.snapshot()["job"].clone();
            if job.is_object() && job["active"] == false {
                return job["status"].as_str().unwrap().to_string();
            }
            assert!(start.elapsed() < Duration::from_secs(120), "job did not finish: {job}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// A mod jar with Fabric metadata.
fn write_mod(path: &Path, id: &str, name: &str) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap();
    let meta = json!({ "schemaVersion": 1, "id": id, "name": name, "version": "1.0.0", "authors": ["Tester"], "description": "Test mod" });
    zip.write_all(meta.to_string().as_bytes()).unwrap();
    zip.finish().unwrap();
}

fn path_of(v: &Value) -> PathBuf {
    PathBuf::from(v.as_str().unwrap())
}

#[test]
fn boot_and_empty_snapshot() {
    let h = Harness::new();
    let boot = h.ok("boot", json!({}));
    assert_eq!(boot["os"], std::env::consts::OS);
    assert!(boot["launch"].is_null());
    let snap = h.snapshot();
    assert_eq!(snap["instances"], json!([]));
    assert_eq!(snap["running"], 0);
    assert_eq!(snap["busy"], false);
    assert_eq!(snap["settings"]["data_dir"], json!(h.data_dir()));
    // Accounts come without their tokens, in `accounts`.
    assert_eq!(snap["settings"]["accounts"], json!([]));
    assert_eq!(snap["accounts"], json!([]));
}

#[test]
fn create_save_and_delete_instances() {
    let h = Harness::new();
    let revealed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = revealed.clone();
    h.window.listen("reveal", move |e| sink.lock().unwrap().push(e.payload().to_string()));

    let inst = h.create("My World", "1.21.1", "fabric");
    assert_eq!(inst["name"], "My World");
    assert_eq!(inst["loader"], "fabric");
    assert_eq!(inst["description"], "1.21.1 · Fabric");
    let id = inst["id"].as_str().unwrap().to_string();
    assert!(path_of(&inst["dir"]).join("instance.json").is_file());
    assert_eq!(h.instance_ids(), std::slice::from_ref(&id));
    assert_eq!(revealed.lock().unwrap().as_slice(), [format!("\"{id}\"")]);

    // An empty name falls back to the Minecraft version; the folder fields can not be changed.
    let mut data = inst.clone();
    data["name"] = json!("  ");
    data["group"] = json!(" Modded ");
    data["memory_mb"] = json!(6144);
    data["fullscreen"] = json!(true);
    data["dir"] = json!("/somewhere/else");
    let saved = h.ok("save_instance", json!({ "id": id, "data": data }));
    assert_eq!(saved["name"], "1.21.1");
    assert_eq!(saved["group"], "Modded");
    assert_eq!(saved["dir"], inst["dir"]);
    let stored: Value =
        serde_json::from_slice(&std::fs::read(path_of(&inst["dir"]).join("instance.json")).unwrap()).unwrap();
    assert_eq!(stored["memory_mb"], 6144);
    assert_eq!(stored["fullscreen"], true);
    assert_eq!(h.snapshot()["instances"][0]["group"], "Modded");

    assert!(
        h.err(
            "create_instance",
            json!({ "name": "x", "minecraft": "", "loader": "vanilla", "loaderVersion": "" })
        )
        .contains("empty")
    );

    h.ok("delete_instance", json!({ "id": id }));
    assert!(h.instance_ids().is_empty());
    assert!(!path_of(&inst["dir"]).exists());
    assert_eq!(h.snapshot()["job"]["status"], "Deleted \"1.21.1\"");
    assert!(h.err("delete_instance", json!({ "id": id })).contains("No instance"));
}

#[test]
fn icons_are_set_and_cleared() {
    let h = Harness::new();
    let id = h.create("Icons", "1.20.1", "vanilla")["id"].as_str().unwrap().to_string();
    let png = h.data_dir().join("icon-source.png");
    image::RgbaImage::from_pixel(300, 300, image::Rgba([10, 200, 30, 255])).save(&png).unwrap();
    h.ok("set_instance_icon", json!({ "id": id, "path": png }));
    let icon = path_of(&h.snapshot()["instances"][0]["icon"]);
    assert!(icon.is_file());
    assert_eq!(image::image_dimensions(&icon).unwrap(), (256, 256));
    h.ok("clear_instance_icon", json!({ "id": id }));
    assert!(h.snapshot()["instances"][0]["icon"].is_null());
    assert!(!icon.exists());
    let text = h.data_dir().join("not-an-image.png");
    std::fs::write(&text, "hello").unwrap();
    assert!(h.err("set_instance_icon", json!({ "id": id, "path": text })).contains("not an image"));
}

#[test]
fn duplicate_export_and_import_jobs() {
    let h = Harness::new();
    let inst = h.create("Original", "1.20.1", "fabric");
    let id = inst["id"].as_str().unwrap();
    let game = path_of(&inst["game_dir"]);
    std::fs::create_dir_all(game.join("mods")).unwrap();
    write_mod(&game.join("mods/a.jar"), "a", "A Mod");

    h.ok("duplicate_instance", json!({ "id": id }));
    assert_eq!(h.wait_job(), "Copied to \"Original (copy)\"");
    let snap = h.snapshot();
    assert_eq!(snap["instances"].as_array().unwrap().len(), 2);
    let copy = &snap["instances"][0];
    assert_eq!(copy["name"], "Original (copy)");
    assert!(path_of(&copy["game_dir"]).join("mods/a.jar").is_file());

    let target = h.ok("export_target", json!({ "id": id }));
    assert_eq!(target["file_name"], "Original.zip");
    let dest = h.data_dir().join("Original.zip");
    h.ok("export_instance", json!({ "id": id, "dest": dest }));
    assert!(h.wait_job().starts_with("Exported to"));
    assert!(dest.is_file());

    h.ok("clear_notice", json!({}));
    assert!(h.snapshot()["job"].is_null());

    h.ok("import_files", json!({ "paths": [dest] }));
    assert_eq!(h.wait_job(), "Added \"Original\"");
    let snap = h.snapshot();
    assert_eq!(snap["instances"].as_array().unwrap().len(), 3);
    let imported = &snap["instances"][0];
    assert_eq!(imported["loader"], "fabric");
    assert!(path_of(&imported["game_dir"]).join("mods/a.jar").is_file());

    let err = h.err("import_files", json!({ "paths": [h.data_dir().join("readme.txt")] }));
    assert!(err.starts_with("Nothing to import"));
    assert_eq!(h.snapshot()["job"]["status"], err);
}

#[test]
fn only_one_job_at_a_time() {
    let h = Harness::new();
    let inst = h.create("Big", "1.20.1", "vanilla");
    let game = path_of(&inst["game_dir"]);
    std::fs::create_dir_all(game.join("saves/w")).unwrap();
    // Enough data that copying takes a moment.
    std::fs::write(game.join("saves/w/big.bin"), vec![7u8; 64 << 20]).unwrap();
    let id = inst["id"].as_str().unwrap();
    h.ok("duplicate_instance", json!({ "id": id }));
    let second = h.call("duplicate_instance", json!({ "id": id }));
    let status = h.wait_job();
    assert!(status.starts_with("Copied"), "{status}");
    if let Err(e) = second {
        assert!(e.as_str().unwrap().contains("Another job"));
    }
}

#[test]
fn content_files() {
    let h = Harness::new();
    let vanilla = h.create("Plain", "1.21.1", "vanilla");
    let inst = h.create("Mods", "1.21.1", "fabric");
    let id = inst["id"].as_str().unwrap();
    let source = h.data_dir().join("downloads");
    std::fs::create_dir_all(&source).unwrap();
    write_mod(&source.join("sodium.jar"), "sodium", "Sodium");
    write_mod(&source.join("lithium.jar"), "lithium", "Lithium");
    std::fs::write(source.join("notes.txt"), "no").unwrap();

    let err =
        h.err("content_add_files", json!({ "id": id, "kind": "mods", "paths": [source.join("notes.txt")] }));
    assert_eq!(err, "Nothing to add: Mods takes .jar files");
    let added = h.ok(
        "content_add_files",
        json!({ "id": id, "kind": "mods", "paths": [source.join("sodium.jar"), source.join("lithium.jar"), source.join("notes.txt")] }),
    );
    assert_eq!(added, 2);

    let items = h.ok("content_list", json!({ "id": id, "kind": "mods" }));
    let names: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Lithium", "Sodium"]);
    let sodium = items[1].clone();
    assert_eq!(sodium["id"], "sodium");
    assert_eq!(sodium["authors"], json!(["Tester"]));
    assert_eq!(sodium["enabled"], true);

    let off = path_of(
        &h.ok("content_set_enabled", json!({ "id": id, "kind": "mods", "item": sodium, "enabled": false })),
    );
    assert!(off.to_string_lossy().ends_with("sodium.jar.disabled"));
    assert!(off.is_file());
    let items = h.ok("content_list", json!({ "id": id, "kind": "mods" }));
    assert_eq!(items[1]["enabled"], false);
    let on = path_of(
        &h.ok("content_set_enabled", json!({ "id": id, "kind": "mods", "item": items[1], "enabled": true })),
    );
    assert!(on.to_string_lossy().ends_with("sodium.jar"));

    // Files outside the kind's folder are refused.
    let mut outside = sodium.clone();
    outside["path"] = json!(source.join("sodium.jar"));
    assert!(
        h.err("content_delete", json!({ "id": id, "kind": "mods", "item": outside }))
            .contains("not in the mods folder")
    );
    assert!(source.join("sodium.jar").is_file());
    let mut other = sodium.clone();
    other["path"] = json!(path_of(&vanilla["game_dir"]).join("mods/sodium.jar"));
    assert!(h.call("content_delete", json!({ "id": id, "kind": "mods", "item": other })).is_err());

    h.ok("content_delete", json!({ "id": id, "kind": "mods", "item": items[0] }));
    let items = h.ok("content_list", json!({ "id": id, "kind": "mods" }));
    assert_eq!(items.as_array().unwrap().len(), 1);

    let scope = h.ok("addon_scope", json!({ "id": id, "kind": "mods" }));
    assert_eq!(scope, json!({ "loaders": ["fabric"], "game_version": "1.21.1" }));
    let packs = h.ok("addon_scope", json!({ "id": id, "kind": "resourcepacks" }));
    assert_eq!(packs, json!({ "loaders": [], "game_version": "1.21.1" }));
    // Shaders work across game versions.
    let shaders = h.ok("addon_scope", json!({ "id": id, "kind": "shaderpacks" }));
    assert_eq!(shaders, json!({ "loaders": [], "game_version": null }));
}

#[test]
fn worlds_screenshots_and_logs() {
    let h = Harness::new();
    let inst = h.create("Files", "1.21.1", "vanilla");
    let id = inst["id"].as_str().unwrap();
    let game = path_of(&inst["game_dir"]);
    std::fs::create_dir_all(game.join("saves/Survival")).unwrap();
    std::fs::write(game.join("saves/Survival/data.bin"), vec![0u8; 3000]).unwrap();
    std::fs::write(game.join("saves/Survival/level.dat"), b"").unwrap();
    // Folders without level.dat are not worlds.
    std::fs::create_dir_all(game.join("saves/junk")).unwrap();
    std::fs::create_dir_all(game.join("screenshots")).unwrap();
    image::RgbImage::new(4, 4).save(game.join("screenshots/2024-01-01.png")).unwrap();
    std::fs::create_dir_all(game.join("logs")).unwrap();
    std::fs::create_dir_all(game.join("crash-reports")).unwrap();
    std::fs::write(game.join("logs/latest.log"), "[main/INFO]: hi\n[main/ERROR]: oh\n\tat x\n").unwrap();
    std::fs::write(game.join("crash-reports/crash-1.txt"), "---- Minecraft Crash Report ----\n").unwrap();

    let worlds = h.ok("worlds", json!({ "id": id }));
    assert_eq!(worlds[0]["folder"], "Survival");
    let world = path_of(&worlds[0]["path"]);
    assert_eq!(h.ok("dir_size", json!({ "path": world })), 3000);

    let shots = h.ok("screenshots", json!({ "id": id }));
    assert!(shots[0].as_str().unwrap().ends_with("2024-01-01.png"));

    let files = h.ok("log_files", json!({ "id": id }));
    let crash = files.as_array().unwrap().iter().find(|f| f["crash"] == true).unwrap();
    assert_eq!(crash["name"], "crash-1.txt");
    let lines = h.ok("read_log", json!({ "id": id, "path": game.join("logs/latest.log") }));
    let levels: Vec<&str> = lines.as_array().unwrap().iter().map(|l| l["level"].as_str().unwrap()).collect();
    assert_eq!(levels, ["info", "error", "error"]);
    assert_eq!(h.ok("latest_log", json!({ "id": id })).as_array().unwrap().len(), 3);
    assert!(
        h.err("read_log", json!({ "id": id, "path": h.data_dir().join("launcher.json") }))
            .contains("not a log")
    );

    // Only folders inside `saves` can be deleted.
    assert!(h.err("delete_world", json!({ "id": id, "path": game.join("logs") })).contains("not a world"));
    assert!(h.err("delete_world", json!({ "id": id, "path": game.join("saves") })).contains("not a world"));
    h.ok("delete_world", json!({ "id": id, "path": world }));
    assert!(!world.exists());
    assert_eq!(h.ok("worlds", json!({ "id": id })), json!([]));
}

#[test]
fn accounts_and_settings() {
    let h = Harness::new();
    assert!(h.err("add_offline_account", json!({ "name": "no" })).contains("3–16"));
    assert!(h.err("add_offline_account", json!({ "name": "bad name!" })).contains("3–16"));
    h.ok("add_offline_account", json!({ "name": "Steve" }));
    h.ok("add_offline_account", json!({ "name": " Alex_2 " }));
    let snap = h.snapshot();
    assert_eq!(snap["accounts"][1]["name"], "Alex_2");
    assert_eq!(snap["accounts"][1]["kind"], "Offline");
    assert_eq!(snap["selected_account"], 1);
    // Adding the same name again selects it instead of adding a copy.
    h.ok("add_offline_account", json!({ "name": "Steve" }));
    let snap = h.snapshot();
    assert_eq!(snap["accounts"].as_array().unwrap().len(), 2);
    assert_eq!(snap["selected_account"], 0);
    h.ok("select_account", json!({ "index": 1 }));
    h.ok("select_account", json!({ "index": 9 }));
    assert_eq!(h.snapshot()["selected_account"], 1);
    h.ok("remove_account", json!({ "index": 0 }));
    let snap = h.snapshot();
    assert_eq!(snap["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(snap["selected_account"], 0);

    // The settings page never touches accounts.
    let mut settings = snap["settings"].clone();
    settings["memory_mb"] = json!(8192);
    settings["appearance"] = json!("dark");
    settings["on_launch"] = json!("minimize");
    h.ok("set_settings", json!({ "settings": settings }));
    let snap = h.snapshot();
    assert_eq!(snap["settings"]["memory_mb"], 8192);
    assert_eq!(snap["settings"]["appearance"], "dark");
    assert_eq!(snap["accounts"][0]["name"], "Alex_2");
    let saved: Value = serde_json::from_slice(
        &std::fs::read(
            dirs::home_dir().unwrap().join("Library/Application Support/gplauncher/launcher.json"),
        )
        .or_else(|_| std::fs::read(dirs::home_dir().unwrap().join(".gplauncher/launcher.json")))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["accounts"][0]["name"], "Alex_2");

    h.ok("set_curseforge_key", json!({ "key": " abc " }));
    h.ok("set_client_id", json!({ "id": "client" }));
    let snap = h.snapshot();
    assert_eq!(snap["settings"]["curseforge_api_key"], "abc");
    assert_eq!(snap["settings"]["ms_client_id"], "client");

    // Moving the launcher folder starts over with that folder's instances.
    h.create("Here", "1.21.1", "vanilla");
    let other = tempfile::tempdir().unwrap();
    let mut settings = h.snapshot()["settings"].clone();
    settings["data_dir"] = json!(other.path());
    h.ok("set_settings", json!({ "settings": settings.clone() }));
    let snap = h.snapshot();
    assert_eq!(snap["instances"], json!([]));
    assert!(snap["job"]["status"].as_str().unwrap().contains("(0 instances)"));
    settings["data_dir"] = json!(h.data_dir());
    h.ok("set_settings", json!({ "settings": settings }));
    assert_eq!(h.instance_ids().len(), 1);
}

#[test]
fn logs_windows_and_misc() {
    let h = Harness::new();
    let id = h.create("Win", "1.21.1", "vanilla")["id"].as_str().unwrap().to_string();
    assert!(h.ok("get_log", json!({ "id": id })).is_null());
    assert!(h.err("launch", json!({ "id": "missing" })).contains("No instance"));
    assert!(h.snapshot()["job"]["status"].as_str().unwrap().contains("missing"));
    h.ok("kill", json!({ "id": id }));
    assert_eq!(h.ok("is_file", json!({ "path": h.data_dir() })), false);
    assert!(!h.err("java_version", json!({ "path": h.data_dir().join("nope") })).is_empty());
    h.ok("notice", json!({ "text": "Hello" }));
    assert_eq!(h.snapshot()["job"]["status"], "Hello");

    h.ok("open_instance_window", json!({ "id": id, "page": "mods" }));
    let label = gplauncher_app::instance_label(&id);
    assert!(h.window.app_handle().get_webview_window(&label).is_some());
    // Opening again reuses the window.
    h.ok("open_instance_window", json!({ "id": id, "page": "console" }));
    assert_eq!(h.window.app_handle().webview_windows().len(), 2);
    // Closing the window of a deleted instance is covered by the state tests: the mock runtime
    // does not report destroyed windows back to the app.
    h.ok("delete_instance", json!({ "id": id }));
    assert!(h.instance_ids().is_empty());
}

#[test]
fn state_events_follow_changes() {
    let h = Harness::new();
    let states = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let sink = states.clone();
    h.window.listen("state", move |e| sink.lock().unwrap().push(serde_json::from_str(e.payload()).unwrap()));
    h.create("Evented", "1.21.1", "vanilla");
    let start = Instant::now();
    loop {
        let seen = states.lock().unwrap().iter().any(|s| s["instances"][0]["name"] == "Evented");
        if seen {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "no state event");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---- online ---------------------------------------------------------------------

#[test]
#[ignore = "needs the network"]
fn online_versions_and_loaders() {
    let h = Harness::new();
    let versions = h.ok("list_versions", json!({}));
    assert!(versions.as_array().unwrap().iter().any(|v| v["id"] == "1.21.1" && v["kind"] == "release"));
    let supported = h.ok("supported_versions", json!({ "loader": "fabric" }));
    assert!(supported.as_array().unwrap().iter().any(|v| v == "1.21.1"));
    let builds = h.ok("loader_versions", json!({ "loader": "neoforge", "minecraft": "1.21.1" }));
    assert!(!builds.as_array().unwrap().is_empty());
    let page = h.ok(
        "search_modpacks",
        json!({ "platform": "modrinth", "query": "fabulously", "filters": {}, "offset": 0, "limit": 5 }),
    );
    let pack = &page["packs"][0];
    assert!(page["total"].as_u64().unwrap() > 0);
    let versions = h.ok("modpack_versions", json!({ "platform": "modrinth", "id": pack["id"] }));
    assert!(!versions.as_array().unwrap().is_empty());
    assert!(
        h.err(
            "search_modpacks",
            json!({ "platform": "curseforge", "query": "", "filters": {}, "offset": 0, "limit": 5 })
        )
        .contains("API key")
            || std::env::var("CURSEFORGE_API_KEY").is_ok()
    );
}

#[test]
#[ignore = "needs the network"]
fn online_mods_install_identify_and_update() {
    let h = Harness::new();
    let inst = h.create("Online", "1.21.1", "fabric");
    let id = inst["id"].as_str().unwrap();
    let page = h.ok(
        "search_projects",
        json!({ "kind": "mods", "query": "sodium extra", "gameVersion": "1.21.1", "loaders": ["fabric"], "sort": "relevance", "offset": 0, "limit": 10 }),
    );
    let project =
        page["projects"].as_array().unwrap().iter().find(|p| p["slug"] == "sodium-extra").unwrap().clone();

    let progress = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let sink = progress.clone();
    h.window.listen("task-progress", move |_| *sink.lock().unwrap() += 1);
    let status = h.ok(
        "content_install",
        json!({ "id": id, "kind": "mods", "project": project, "version": null, "installed": [], "task": "t1" }),
    );
    // Sodium Extra needs Sodium, which comes along.
    assert!(status.as_str().unwrap().starts_with("Installed Sodium Extra with"), "{status}");
    assert!(*progress.lock().unwrap() > 0);

    let items = h.ok("content_list", json!({ "id": id, "kind": "mods" }));
    assert!(items.as_array().unwrap().len() >= 2);
    let identified = h.ok("content_identify", json!({ "items": items }));
    assert_eq!(identified.as_object().unwrap().len(), items.as_array().unwrap().len());

    // An old version shows up as an update.
    let versions = h.ok(
        "project_versions",
        json!({ "project": project["id"], "loaders": ["fabric"], "gameVersion": "1.21.1" }),
    );
    let oldest = versions.as_array().unwrap().last().unwrap().clone();
    let old_dir = h.data_dir().join("old");
    std::fs::create_dir_all(&old_dir).unwrap();
    let item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| identified[i["path"].as_str().unwrap()]["project_id"] == project["id"])
        .unwrap()
        .clone();
    h.ok("content_delete", json!({ "id": id, "kind": "mods", "item": item }));
    h.ok("content_install", json!({ "id": id, "kind": "mods", "project": project, "version": oldest, "installed": [], "task": "t2" }));
    let items = h.ok("content_list", json!({ "id": id, "kind": "mods" }));
    let updates = h.ok("content_check_updates", json!({ "id": id, "kind": "mods", "items": items }));
    let update = updates
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["latest"]["project_id"] == project["id"])
        .expect("an update")
        .clone();
    let status =
        h.ok("content_update", json!({ "id": id, "kind": "mods", "updates": [update], "task": "t3" }));
    assert!(status.as_str().unwrap().starts_with("Updated"));
    let updates = h.ok("content_check_updates", json!({ "id": id, "kind": "mods", "items": h.ok("content_list", json!({ "id": id, "kind": "mods" })) }));
    assert!(updates.as_array().unwrap().iter().all(|u| u["latest"]["project_id"] != project["id"]));
}
