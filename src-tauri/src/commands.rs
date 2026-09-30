//! Commands the windows call. Anything that touches the network or many files runs on a
//! blocking thread; errors reach the windows as display strings.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gplauncher::addons::{self, Update};
use gplauncher::auth::{self, Account, DeviceCode};
use gplauncher::content::{self, Item, Kind, World};
use gplauncher::forge::LoaderVersion;
use gplauncher::instance::{self, Instance, Loader};
use gplauncher::modpack::{Filters, Pack, PackVersion, Page, Platform, Sort, Source};
use gplauncher::modrinth::{self, Project, ProjectPage, Version};
use gplauncher::settings::Settings;
use gplauncher::version::VersionEntry;
use gplauncher::{Event, Reporter, export, http, import, java, loader, modpack};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::shortcut;
use crate::state::{InstanceView, LogChunk, LogLine, Shared, Snapshot, Summary, log_lines};

type Res<T> = Result<T, String>;
type Launcher<'a> = State<'a, Arc<Shared>>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn anyhow_err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Runs `f` on a thread that may block, e.g. on the network.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?.map_err(anyhow_err)
}

fn instance(shared: &Shared, id: &str) -> Res<Instance> {
    shared.read().instance(id).cloned().ok_or_else(|| format!("No instance \"{id}\""))
}

/// Whether `path` is inside `dir` (both must exist).
fn inside(path: &Path, dir: &Path) -> bool {
    match (path.canonicalize(), dir.canonicalize()) {
        (Ok(path), Ok(dir)) => path != dir && path.starts_with(dir),
        _ => false,
    }
}

// ---- start-up ---------------------------------------------------------------------

/// What the process was started with; the launch request is handed out once.
#[derive(Default)]
pub struct StartInfo {
    pub launch: Mutex<Option<String>>,
}

#[derive(Serialize)]
pub struct Boot {
    /// Instance to launch right away (`--launch <id>` from a desktop shortcut).
    launch: Option<String>,
    /// Development aids: `GPLAUNCHER_THEME` and `GPLAUNCHER_OPEN`.
    theme: Option<String>,
    open: Option<String>,
    /// `CURSEFORGE_API_KEY` is set and wins over the stored key.
    env_curseforge_key: bool,
    os: &'static str,
}

#[tauri::command]
pub fn boot(start: State<'_, StartInfo>) -> Boot {
    let debug = |name: &str| if cfg!(debug_assertions) { std::env::var(name).ok() } else { None };
    Boot {
        launch: start.launch.lock().unwrap_or_else(|e| e.into_inner()).take(),
        theme: debug("GPLAUNCHER_THEME"),
        open: debug("GPLAUNCHER_OPEN"),
        env_curseforge_key: std::env::var("CURSEFORGE_API_KEY").is_ok_and(|k| !k.trim().is_empty()),
        os: std::env::consts::OS,
    }
}

#[tauri::command]
pub fn snapshot(shared: Launcher<'_>) -> Snapshot {
    shared.read().snapshot()
}

#[tauri::command]
pub fn notice(shared: Launcher<'_>, text: String) {
    shared.update(|s| s.notice(text));
}

#[tauri::command]
pub fn clear_notice(shared: Launcher<'_>) {
    shared.update(|s| s.clear_notice());
}

// ---- settings and accounts ------------------------------------------------------

/// Applies settings from the settings page. Accounts are kept: they change through their own
/// commands, and a running launch may have refreshed one meanwhile.
#[tauri::command]
pub fn set_settings<R: Runtime>(app: AppHandle<R>, shared: Launcher<'_>, settings: Settings) -> Res<()> {
    let moved = shared.update(|s| {
        let mut settings = settings;
        settings.accounts = std::mem::take(&mut s.settings.accounts);
        settings.selected_account = s.settings.selected_account;
        let moved = settings.data_dir != s.settings.data_dir;
        if moved && s.busy() {
            settings.data_dir = s.settings.data_dir.clone();
            s.settings = settings;
            s.save_settings();
            return Err("The launcher folder can not change while a game or a job is running".to_string());
        }
        s.settings = settings;
        s.save_settings();
        if moved {
            s.reload_instances();
        }
        Ok(moved)
    })?;
    if moved {
        crate::allow_data_dir(&app, &shared.read().settings.data_dir);
    }
    Ok(())
}

#[tauri::command]
pub fn set_curseforge_key(shared: Launcher<'_>, key: String) {
    shared.update(|s| {
        s.settings.curseforge_api_key = key.trim().to_string();
        s.save_settings();
    });
}

#[tauri::command]
pub fn set_client_id(shared: Launcher<'_>, id: String) {
    shared.update(|s| {
        s.settings.ms_client_id = id.trim().to_string();
        s.save_settings();
    });
}

#[tauri::command]
pub fn select_account(shared: Launcher<'_>, index: usize) {
    shared.update(|s| s.select_account(index));
}

#[tauri::command]
pub fn remove_account(shared: Launcher<'_>, index: usize) {
    shared.update(|s| s.remove_account(index));
}

#[tauri::command]
pub fn add_offline_account(shared: Launcher<'_>, name: String) -> Res<()> {
    let name = name.trim();
    if !auth::valid_offline_name(name) {
        return Err("Use 3–16 characters: letters, digits and _".into());
    }
    shared.update(|s| s.add_account(Account::offline(name)));
    Ok(())
}

/// The Microsoft sign-in in progress: its code, and the flag that stops waiting for it.
#[derive(Default)]
pub struct Login(Mutex<Option<(DeviceCode, Arc<AtomicBool>)>>);

#[derive(Serialize)]
pub struct CodeView {
    user_code: String,
    verification_uri: String,
}

/// Starts a Microsoft sign-in; finish it with [`ms_complete`].
#[tauri::command]
pub async fn ms_request_code(shared: Launcher<'_>, login: State<'_, Login>) -> Res<CodeView> {
    ms_cancel(login.clone());
    let client_id = shared.read().settings.ms_client_id.clone();
    let code = blocking(move || auth::request_device_code(&client_id)).await?;
    let view =
        CodeView { user_code: code.user_code.clone(), verification_uri: code.verification_uri.clone() };
    *login.0.lock().unwrap_or_else(|e| e.into_inner()) = Some((code, Arc::new(AtomicBool::new(false))));
    Ok(view)
}

/// Waits until the user signed in, then adds and selects the account.
#[tauri::command]
pub async fn ms_complete(shared: Launcher<'_>, login: State<'_, Login>) -> Res<()> {
    let (code, cancel) =
        login.0.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("No sign-in to finish")?;
    let client_id = shared.read().settings.ms_client_id.clone();
    let flag = cancel.clone();
    let account =
        blocking(move || auth::complete_device_code(&client_id, &code, || flag.load(Ordering::Relaxed)))
            .await?;
    if cancel.load(Ordering::Relaxed) {
        return Err("sign-in was cancelled".into());
    }
    shared.update(|s| s.add_account(account));
    Ok(())
}

#[tauri::command]
pub fn ms_cancel(login: State<'_, Login>) {
    if let Some((_, cancel)) = login.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
        cancel.store(true, Ordering::Relaxed);
    }
}

// ---- versions and new instances -------------------------------------------------

#[tauri::command]
pub async fn list_versions(shared: Launcher<'_>) -> Res<Vec<VersionEntry>> {
    let dir = shared.read().settings.data_dir.clone();
    blocking(move || gplauncher::version::list(&dir)).await
}

/// Minecraft versions the loader supports; `None` = no restriction.
#[tauri::command]
pub async fn supported_versions(loader: Loader) -> Res<Option<Vec<String>>> {
    blocking(move || Ok(loader::supported_versions(loader)?.map(|set| set.into_iter().collect()))).await
}

#[tauri::command]
pub async fn loader_versions(loader: Loader, minecraft: String) -> Res<Vec<LoaderVersion>> {
    blocking(move || loader::versions(loader, &minecraft)).await
}

#[tauri::command]
pub fn create_instance(
    shared: Launcher<'_>,
    name: String,
    minecraft: String,
    loader: Loader,
    loader_version: String,
) -> Res<InstanceView> {
    let data_dir = shared.read().settings.data_dir.clone();
    let inst = instance::create(&data_dir, &name, &minecraft, loader, &loader_version).map_err(anyhow_err)?;
    let view = InstanceView::from(&inst);
    shared.update(|s| s.add_instance(inst));
    Ok(view)
}

/// Imports modpack files as new instances in the background.
#[tauri::command]
pub fn import_files(shared: Launcher<'_>, paths: Vec<PathBuf>) -> Res<()> {
    let files: Vec<PathBuf> = paths.into_iter().filter(|p| import::is_importable(p)).collect();
    if files.is_empty() {
        let text = "Nothing to import: expected .mrpack or .zip modpacks".to_string();
        shared.update(|s| s.notice(text.clone()));
        return Err(text);
    }
    let settings = shared.read().settings.clone();
    let started = shared.run_job("Importing", move |reporter, imported| {
        let mut summary = Summary::default();
        for file in &files {
            let result = import::import(&settings, file, reporter)?;
            summary.add(&result);
            imported(result);
        }
        Ok(summary.status())
    });
    started.then_some(()).ok_or_else(busy_job)
}

fn busy_job() -> String {
    "Another job is running; try again when it is done".into()
}

#[tauri::command]
pub fn install_modpack(shared: Launcher<'_>, pack: Pack, version: PackVersion) -> Res<()> {
    let settings = shared.read().settings.clone();
    let started = shared.run_job(&format!("Installing {}", pack.title), move |reporter, imported| {
        let result = modpack::install(&settings, &pack, &version, reporter)?;
        let mut summary = Summary::default();
        summary.add(&result);
        imported(result);
        Ok(summary.status())
    });
    started.then_some(()).ok_or_else(busy_job)
}

fn source(shared: &Shared, platform: Platform) -> Res<Source> {
    Ok(match platform {
        Platform::Modrinth => Source::Modrinth,
        Platform::CurseForge => Source::CurseForge {
            api_key: shared.read().settings.curseforge_key().ok_or("CurseForge API key needed")?,
        },
    })
}

#[tauri::command]
pub async fn search_modpacks(
    shared: Launcher<'_>,
    platform: Platform,
    query: String,
    filters: Filters,
    offset: u64,
    limit: u64,
) -> Res<Page> {
    let source = source(&shared, platform)?;
    blocking(move || source.search(&query, &filters, offset, limit)).await
}

#[tauri::command]
pub async fn modpack_versions(shared: Launcher<'_>, platform: Platform, id: String) -> Res<Vec<PackVersion>> {
    let source = source(&shared, platform)?;
    blocking(move || source.versions(&id)).await
}

// ---- instance actions -------------------------------------------------------------

#[tauri::command]
pub fn launch(shared: Launcher<'_>, id: String) -> Res<bool> {
    if shared.read().instance(&id).is_none() {
        let text = format!("No instance \"{id}\" to launch");
        shared.update(|s| s.notice(text.clone()));
        return Err(text);
    }
    Ok(shared.launch(&id))
}

#[tauri::command]
pub fn kill(shared: Launcher<'_>, id: String) {
    shared.kill(&id);
}

#[tauri::command]
pub fn open_instance_window<R: Runtime>(
    app: AppHandle<R>,
    shared: Launcher<'_>,
    id: String,
    page: String,
) -> Res<()> {
    let name = instance(&shared, &id)?.name;
    crate::open_instance_window(&app, &id, &name, &page).map_err(err)
}

/// Opens a folder in the file manager, creating it first.
#[tauri::command]
pub fn open_folder(path: PathBuf) -> Res<()> {
    let _ = std::fs::create_dir_all(&path);
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(err)
}

/// Opens a file with its default app.
#[tauri::command]
pub fn open_file(path: PathBuf) -> Res<()> {
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(err)
}

#[derive(Serialize)]
pub struct ExportTarget {
    dir: PathBuf,
    file_name: String,
}

/// Where the export dialog starts, and the suggested file name.
#[tauri::command]
pub fn export_target(shared: Launcher<'_>, id: String) -> Res<ExportTarget> {
    let inst = instance(&shared, &id)?;
    let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_default();
    Ok(ExportTarget { dir, file_name: export::file_name(&inst) })
}

#[tauri::command]
pub fn export_instance(shared: Launcher<'_>, id: String, dest: PathBuf) -> Res<()> {
    let inst = instance(&shared, &id)?;
    let started = shared.run_job(&format!("Exporting {}", inst.name), move |reporter, _| {
        export::export(&inst, &dest, reporter)?;
        Ok(format!("Exported to {}", dest.display()))
    });
    started.then_some(()).ok_or_else(busy_job)
}

#[tauri::command]
pub fn duplicate_instance(shared: Launcher<'_>, id: String) -> Res<()> {
    let inst = instance(&shared, &id)?;
    let data_dir = shared.read().settings.data_dir.clone();
    let started = shared.run_job(&format!("Copying {}", inst.name), move |_, imported| {
        let copy = instance::duplicate(&data_dir, &inst, &format!("{} (copy)", inst.name))?;
        let status = format!("Copied to \"{}\"", copy.name);
        imported(import::Imported { instance: copy, blocked: Vec::new() });
        Ok(status)
    });
    started.then_some(()).ok_or_else(busy_job)
}

#[tauri::command]
pub fn delete_instance(shared: Launcher<'_>, id: String) -> Res<()> {
    shared.update(|s| match s.delete_instance(&id) {
        Ok(name) => {
            s.notice(format!("Deleted \"{name}\""));
            Ok(())
        }
        Err(e) => {
            let text = format!("Error: {e:#}");
            s.notice(text.clone());
            Err(text)
        }
    })
}

#[tauri::command]
pub fn create_shortcut(shared: Launcher<'_>, id: String) -> Res<String> {
    let inst = instance(&shared, &id)?;
    let status = match shortcut::create(&inst) {
        Ok(path) => format!("Created {}", path.display()),
        Err(e) => format!("Could not create a shortcut: {e:#}"),
    };
    shared.update(|s| s.notice(status.clone()));
    Ok(status)
}

/// Saves the editable fields of an instance; the folder fields stay as they are.
#[tauri::command]
pub fn save_instance(shared: Launcher<'_>, id: String, data: Instance) -> Res<InstanceView> {
    let current = instance(&shared, &id)?;
    let inst = Instance {
        id: current.id,
        dir: current.dir,
        game_dir: current.game_dir,
        icon: current.icon,
        name: match data.name.trim() {
            "" => data.minecraft.clone(),
            name => name.to_string(),
        },
        group: data.group.trim().to_string(),
        ..data
    };
    let view = InstanceView::from(&inst);
    shared.update(|s| s.save_instance(inst)).map_err(|e| format!("Could not save: {e:#}"))?;
    Ok(view)
}

#[tauri::command]
pub async fn set_instance_icon(shared: Launcher<'_>, id: String, path: PathBuf) -> Res<()> {
    let mut inst = instance(&shared, &id)?;
    let inst = blocking(move || inst.set_icon(&path).map(|()| inst)).await?;
    shared.update(|s| s.save_instance(inst)).map_err(anyhow_err)
}

#[tauri::command]
pub fn clear_instance_icon(shared: Launcher<'_>, id: String) -> Res<()> {
    let mut inst = instance(&shared, &id)?;
    inst.clear_icon().map_err(anyhow_err)?;
    shared.update(|s| s.save_instance(inst)).map_err(anyhow_err)
}

#[tauri::command]
pub async fn java_version(path: PathBuf) -> Res<String> {
    blocking(move || java::version(&path)).await
}

#[tauri::command]
pub fn is_file(path: PathBuf) -> bool {
    path.is_file()
}

// ---- mods, resource packs and shader packs ---------------------------------------

/// The instance and the kind's folder; `item` must be in that folder.
fn content_item(shared: &Shared, id: &str, kind: Kind, item: &Item) -> Res<Instance> {
    let inst = instance(shared, id)?;
    if !inside(&item.path, &kind.dir(&inst)) {
        return Err(format!("{} is not in the {} folder", item.path.display(), kind.folder()));
    }
    Ok(inst)
}

#[tauri::command]
pub async fn content_list(shared: Launcher<'_>, id: String, kind: Kind) -> Res<Vec<Item>> {
    let inst = instance(&shared, &id)?;
    let cache = shared.read().settings.data_dir.join("cache/icons/content");
    blocking(move || Ok(content::list(&inst, kind, &cache))).await
}

/// Modrinth versions of the files, by path.
#[tauri::command]
pub async fn content_identify(items: Vec<Item>) -> Res<HashMap<PathBuf, Version>> {
    blocking(move || addons::identify(&items)).await
}

#[tauri::command]
pub fn content_set_enabled(
    shared: Launcher<'_>,
    id: String,
    kind: Kind,
    item: Item,
    enabled: bool,
) -> Res<PathBuf> {
    content_item(&shared, &id, kind, &item)?;
    content::set_enabled(&item, enabled).map_err(anyhow_err)
}

#[tauri::command]
pub fn content_delete(shared: Launcher<'_>, id: String, kind: Kind, item: Item) -> Res<()> {
    content_item(&shared, &id, kind, &item)?;
    content::delete(&item).map_err(anyhow_err)
}

/// Copies files into the kind's folder; returns how many were added.
#[tauri::command]
pub fn content_add_files(shared: Launcher<'_>, id: String, kind: Kind, paths: Vec<PathBuf>) -> Res<usize> {
    let inst = instance(&shared, &id)?;
    let accepted: Vec<PathBuf> = paths.into_iter().filter(|p| kind.accepts(p)).collect();
    if accepted.is_empty() {
        let expected = if kind == Kind::Mods { ".jar files" } else { ".zip files" };
        return Err(format!("Nothing to add: {} takes {expected}", kind.label()));
    }
    content::add_files(&inst, kind, &accepted).map_err(anyhow_err)
}

#[tauri::command]
pub async fn content_check_updates(
    shared: Launcher<'_>,
    id: String,
    kind: Kind,
    items: Vec<Item>,
) -> Res<Vec<Update>> {
    let inst = instance(&shared, &id)?;
    blocking(move || addons::check_updates(&inst, kind, &items)).await
}

#[derive(Clone, Serialize)]
struct TaskProgress {
    task: String,
    status: Option<String>,
    progress: Option<(u64, u64)>,
}

/// Reports a content task's progress to the window that runs it.
fn task_reporter<R: Runtime>(app: &AppHandle<R>, task: String) -> Reporter {
    let app = app.clone();
    Reporter::new(move |event| {
        let payload = match event {
            Event::Status(s) => TaskProgress { task: task.clone(), status: Some(s), progress: None },
            Event::Progress { done, total } => TaskProgress {
                task: task.clone(),
                status: None,
                progress: (total > 0).then_some((done, total)),
            },
            _ => return,
        };
        let _ = app.emit("task-progress", payload);
    })
}

#[derive(Serialize)]
pub struct AddonScope {
    loaders: Vec<&'static str>,
    game_version: Option<String>,
}

/// The loaders and game version Modrinth results must fit for this instance.
#[tauri::command]
pub fn addon_scope(shared: Launcher<'_>, id: String, kind: Kind) -> Res<AddonScope> {
    let inst = instance(&shared, &id)?;
    Ok(AddonScope { loaders: addons::loaders(&inst, kind), game_version: addons::game_version(&inst, kind) })
}

/// Installs a project (the newest fitting version when `version` is `None`) with its dependencies.
/// `installed` holds the Modrinth project ids already in the instance.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn content_install<R: Runtime>(
    app: AppHandle<R>,
    shared: Launcher<'_>,
    id: String,
    kind: Kind,
    project: Project,
    version: Option<Version>,
    installed: Vec<String>,
    task: String,
) -> Res<String> {
    let inst = instance(&shared, &id)?;
    let reporter = task_reporter(&app, task);
    blocking(move || {
        let version = match version {
            Some(v) => v,
            None => modrinth::project_versions(
                &project.id,
                &addons::loaders(&inst, kind),
                addons::game_version(&inst, kind).as_deref(),
            )?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("{} has no version for this instance", project.title))?,
        };
        let installed: HashSet<String> = installed.into_iter().collect();
        let added = addons::install(&inst, kind, &version, &installed, &reporter)?;
        Ok(match added.len() {
            0 => format!("{} is already installed", project.title),
            1 => format!("Installed {}", project.title),
            n => format!("Installed {} with {} dependencies", project.title, n - 1),
        })
    })
    .await
}

#[tauri::command]
pub async fn content_update<R: Runtime>(
    app: AppHandle<R>,
    shared: Launcher<'_>,
    id: String,
    kind: Kind,
    updates: Vec<Update>,
    task: String,
) -> Res<String> {
    let inst = instance(&shared, &id)?;
    for u in &updates {
        content_item(&shared, &id, kind, &u.item)?;
    }
    let reporter = task_reporter(&app, task);
    blocking(move || {
        for u in &updates {
            addons::apply_update(&inst, kind, u, &reporter)?;
        }
        Ok(format!("Updated {}", updates.iter().map(|u| u.item.name.as_str()).collect::<Vec<_>>().join(", ")))
    })
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn search_projects(
    kind: Kind,
    query: String,
    game_version: Option<String>,
    loaders: Vec<String>,
    sort: Sort,
    offset: u64,
    limit: u64,
) -> Res<ProjectPage> {
    blocking(move || {
        let loaders: Vec<&str> = loaders.iter().map(String::as_str).collect();
        modrinth::search_projects(kind, &query, game_version.as_deref(), &loaders, sort, offset, limit)
    })
    .await
}

#[tauri::command]
pub async fn project_versions(
    project: String,
    loaders: Vec<String>,
    game_version: Option<String>,
) -> Res<Vec<Version>> {
    blocking(move || {
        let loaders: Vec<&str> = loaders.iter().map(String::as_str).collect();
        modrinth::project_versions(&project, &loaders, game_version.as_deref())
    })
    .await
}

// ---- worlds, screenshots and logs -----------------------------------------------

#[tauri::command]
pub async fn worlds(shared: Launcher<'_>, id: String) -> Res<Vec<World>> {
    let inst = instance(&shared, &id)?;
    blocking(move || Ok(content::worlds(&inst))).await
}

#[tauri::command]
pub async fn dir_size(path: PathBuf) -> Res<u64> {
    blocking(move || Ok(content::dir_size(&path))).await
}

#[tauri::command]
pub fn delete_world(shared: Launcher<'_>, id: String, path: PathBuf) -> Res<()> {
    let inst = instance(&shared, &id)?;
    if shared.read().is_running(&id) {
        return Err("Close the game to delete worlds".into());
    }
    if !inside(&path, &inst.game_dir.join("saves")) {
        return Err(format!("{} is not a world of this instance", path.display()));
    }
    std::fs::remove_dir_all(&path).map_err(err)
}

#[tauri::command]
pub async fn screenshots(shared: Launcher<'_>, id: String) -> Res<Vec<PathBuf>> {
    let inst = instance(&shared, &id)?;
    blocking(move || Ok(content::screenshots(&inst))).await
}

#[derive(Serialize)]
pub struct LogFile {
    path: PathBuf,
    name: String,
    crash: bool,
    modified: u64,
}

#[tauri::command]
pub async fn log_files(shared: Launcher<'_>, id: String) -> Res<Vec<LogFile>> {
    let inst = instance(&shared, &id)?;
    blocking(move || {
        Ok(content::log_files(&inst)
            .into_iter()
            .map(|path| {
                let modified = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                LogFile {
                    name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                    crash: path.parent().is_some_and(|p| p.ends_with("crash-reports")),
                    modified,
                    path,
                }
            })
            .collect())
    })
    .await
}

/// Lines of a log file of the instance, with their levels.
#[tauri::command]
pub async fn read_log(shared: Launcher<'_>, id: String, path: PathBuf) -> Res<Vec<LogLine>> {
    let inst = instance(&shared, &id)?;
    if !inside(&path, &inst.game_dir) {
        return Err(format!("{} is not a log of this instance", path.display()));
    }
    blocking(move || Ok(log_lines(&content::read_log(&path)?))).await
}

/// `logs/latest.log` of the instance, for a console that has not run since the launcher started.
#[tauri::command]
pub async fn latest_log(shared: Launcher<'_>, id: String) -> Res<Option<Vec<LogLine>>> {
    let inst = instance(&shared, &id)?;
    let path = inst.game_dir.join("logs/latest.log");
    blocking(move || Ok(content::read_log(&path).ok().map(|text| log_lines(&text)))).await
}

/// Everything the last launch of the instance printed so far.
#[tauri::command]
pub fn get_log(shared: Launcher<'_>, id: String) -> Option<LogChunk> {
    shared.read().sessions.get(&id).map(|s| s.chunk(&id))
}

#[tauri::command]
pub async fn upload_log(text: String) -> Res<String> {
    blocking(move || http::upload_log(&text)).await
}

/// Errors the windows could not show, for the launcher's output.
#[tauri::command]
pub fn report_error(text: String) {
    eprintln!("[ui] {text}");
}

#[tauri::command]
pub fn quit<R: Runtime>(app: AppHandle<R>) {
    crate::request_quit(&app);
}
