//! gpLauncher desktop app: the windows are web pages (see `ui/`), this crate runs the launcher
//! core behind them and keeps the state they share.

pub mod commands;
pub mod shortcut;
pub mod state;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gplauncher::settings::Settings;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::commands::{Login, StartInfo};
use crate::state::{AppState, Effect, Shared, Ui};

pub const MAIN_WINDOW: &str = "main";

/// Set once the user agreed to stop running games, so quitting does not ask again.
static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

/// Sends state events to every window and applies window effects.
struct TauriUi<R: Runtime>(AppHandle<R>);

impl<R: Runtime> Ui for TauriUi<R> {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.0.emit(event, payload);
    }

    fn apply(&self, effect: Effect) {
        match effect {
            Effect::MinimizeAll => {
                for window in self.0.webview_windows().values() {
                    let _ = window.minimize();
                }
            }
            Effect::RestoreMain => {
                if let Some(window) = self.0.get_webview_window(MAIN_WINDOW) {
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            Effect::Reveal(id) => {
                let _ = self.0.emit_to(MAIN_WINDOW, "reveal", id);
            }
            Effect::CloseInstanceWindow(id) => {
                if let Some(window) = self.0.get_webview_window(&instance_label(&id)) {
                    let _ = window.destroy();
                }
            }
        }
    }
}

/// Window label of an instance window; labels only allow a few characters, ids allow more.
pub fn instance_label(id: &str) -> String {
    let hex: String = id.bytes().map(|b| format!("{b:02x}")).collect();
    format!("instance-{hex}")
}

/// Shows the window of instance `id` on `page`, opening it if needed.
pub fn open_instance_window<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    name: &str,
    page: &str,
) -> tauri::Result<()> {
    let label = instance_label(id);
    if let Some(window) = app.get_webview_window(&label) {
        window.emit_to(&label, "show-page", page)?;
        let _ = window.unminimize();
        window.set_focus()?;
        return Ok(());
    }
    let url = format!("index.html#/instance/{}/{}", encode(id), encode(page));
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(name)
        .inner_size(1000., 680.)
        .min_inner_size(720., 460.)
        .center()
        .build()?;
    Ok(())
}

/// Percent-encodes everything but unreserved characters, for the window URL.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b => format!("%{b:02X}"),
        })
        .collect()
}

/// Lets the windows show images from the launcher folder (instance icons, screenshots, ...).
pub fn allow_data_dir<R: Runtime>(app: &AppHandle<R>, dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = app.asset_protocol_scope().allow_directory(dir, true);
}

/// Quits, after asking whether to stop games that are still running.
pub fn request_quit<R: Runtime>(app: &AppHandle<R>) {
    let shared = app.state::<Arc<Shared>>().inner().clone();
    let running = shared.read().running_count();
    if running == 0 || QUIT_CONFIRMED.load(Ordering::Relaxed) {
        app.exit(0);
        return;
    }
    let app = app.clone();
    let title = if running == 1 { "A game is still running" } else { "Games are still running" };
    let mut dialog = app
        .dialog()
        .message("Quitting the launcher stops them. Unsaved progress in the game may be lost.")
        .title(title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom("Stop and Quit".into(), "Cancel".into()));
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |stop| {
        if stop {
            QUIT_CONFIRMED.store(true, Ordering::Relaxed);
            shared.update(|s| s.kill_all());
            app.exit(0);
        }
    });
}

/// The macOS menu bar; other platforms use the shortcuts the windows handle themselves.
fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let app_menu = SubmenuBuilder::new(app, "gpLauncher")
        .item(&PredefinedMenuItem::about(app, Some("About gpLauncher"), None)?)
        .separator()
        .item(&MenuItemBuilder::with_id("settings", "Settings…").accelerator("CmdOrCtrl+,").build(app)?)
        .separator()
        .item(&PredefinedMenuItem::hide(app, Some("Hide gpLauncher"))?)
        .item(&PredefinedMenuItem::hide_others(app, None)?)
        .item(&PredefinedMenuItem::show_all(app, None)?)
        .separator()
        .item(&MenuItemBuilder::with_id("quit", "Quit gpLauncher").accelerator("CmdOrCtrl+Q").build(app)?)
        .build()?;
    let file = SubmenuBuilder::new(app, "File")
        .item(
            &MenuItemBuilder::with_id("add-instance", "Add Instance…")
                .accelerator("CmdOrCtrl+N")
                .build(app)?,
        )
        .separator()
        .item(
            &MenuItemBuilder::with_id("play-selected", "Play Selected")
                .accelerator("CmdOrCtrl+Enter")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("open-selected", "Open Selected")
                .accelerator("CmdOrCtrl+O")
                .build(app)?,
        )
        .separator()
        .item(&PredefinedMenuItem::close_window(app, None)?)
        .build()?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let window = SubmenuBuilder::new(app, "Window")
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::maximize(app, None)?)
        .build()?;
    MenuBuilder::new(app).items(&[&app_menu, &file, &edit, &window]).build()
}

/// Starts the launcher state behind a built app: `run` calls it on setup, tests right after building.
pub fn init<R: Runtime>(handle: &AppHandle<R>, settings: Settings) -> tauri::Result<()> {
    let shared = Shared::new(AppState::new(settings), TauriUi(handle.clone()));
    allow_data_dir(handle, &shared.read().settings.data_dir);
    shared.run_emitter();
    handle.manage(shared);
    Ok(())
}

/// The app with its plugins and commands; `run` builds it for real, tests on a mock runtime.
pub fn app<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(StartInfo { launch: std::sync::Mutex::new(shortcut::launch_arg()) })
        .manage(Login::default())
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => request_quit(app),
            id => {
                if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
                let _ = app.emit_to(MAIN_WINDOW, "menu", id);
            }
        })
        .on_window_event(|window, event| {
            // Closing the main window quits, which may stop running games.
            if window.label() == MAIN_WINDOW
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                request_quit(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::boot,
            commands::snapshot,
            commands::notice,
            commands::clear_notice,
            commands::set_settings,
            commands::set_curseforge_key,
            commands::set_client_id,
            commands::select_account,
            commands::remove_account,
            commands::add_offline_account,
            commands::ms_request_code,
            commands::ms_complete,
            commands::ms_cancel,
            commands::list_versions,
            commands::supported_versions,
            commands::loader_versions,
            commands::create_instance,
            commands::import_files,
            commands::install_modpack,
            commands::search_modpacks,
            commands::modpack_versions,
            commands::launch,
            commands::kill,
            commands::open_instance_window,
            commands::open_folder,
            commands::open_file,
            commands::export_target,
            commands::export_instance,
            commands::duplicate_instance,
            commands::delete_instance,
            commands::create_shortcut,
            commands::save_instance,
            commands::set_instance_icon,
            commands::clear_instance_icon,
            commands::java_version,
            commands::is_file,
            commands::content_list,
            commands::content_identify,
            commands::content_set_enabled,
            commands::content_delete,
            commands::content_add_files,
            commands::content_check_updates,
            commands::addon_scope,
            commands::content_install,
            commands::content_update,
            commands::search_projects,
            commands::project_versions,
            commands::worlds,
            commands::dir_size,
            commands::delete_world,
            commands::screenshots,
            commands::log_files,
            commands::read_log,
            commands::latest_log,
            commands::get_log,
            commands::upload_log,
            commands::report_error,
            commands::quit,
        ])
}

pub fn run() {
    let app = app(tauri::Builder::default())
        .setup(|app| {
            init(app.handle(), Settings::load())?;
            if cfg!(target_os = "macos") {
                app.set_menu(menu(app.handle())?)?;
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start gpLauncher");

    app.run(|app, event| {
        // Cmd+Q from the system (Dock, app switcher) and the last window closing end up here.
        if let RunEvent::ExitRequested { api, code: None, .. } = event
            && !QUIT_CONFIRMED.load(Ordering::Relaxed)
            && app.state::<Arc<Shared>>().read().running_count() > 0
        {
            api.prevent_exit();
            request_quit(app);
        }
    });
}
