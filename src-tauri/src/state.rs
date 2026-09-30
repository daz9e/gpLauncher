//! Launcher state shared by every window: settings, instances, running games with their logs,
//! and background jobs. Windows read it through [`AppState::snapshot`] and follow the `state`
//! and `log` events that [`Shared::run_emitter`] sends when it changes.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gplauncher::auth::{Account, AccountKind};
use gplauncher::import::{self, Imported};
use gplauncher::instance::{self, Instance};
use gplauncher::launch::{self, GameHandle};
use gplauncher::settings::{OnLaunch, Settings};
use gplauncher::{Event, Reporter, version};
use serde::Serialize;

/// Lines kept per game; older ones are dropped.
pub const MAX_LOG_LINES: usize = 100_000;
/// How often the emitter looks for changes to send to the windows.
const EMIT_INTERVAL: Duration = Duration::from_millis(60);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Preparing,
    Running,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Written by the launcher itself: steps, the command line, warnings.
    Launcher,
    Info,
    Warn,
    Error,
    Debug,
}

#[derive(Clone, Debug, Serialize)]
pub struct LogLine {
    pub text: String,
    pub level: Level,
}

/// One launch of an instance, from preparing files until the game exits. Kept afterwards so
/// its log can still be read.
pub struct Session {
    pub phase: Phase,
    handle: GameHandle,
    pub status: String,
    pub progress: Option<(u64, u64)>,
    pub log: VecDeque<LogLine>,
    /// Lines dropped from the front to stay under [`MAX_LOG_LINES`].
    pub dropped: usize,
    /// When the game started (or the launch began, while preparing).
    started: Instant,
    started_unix_ms: u64,
    /// When the launch began, in Unix milliseconds; identifies the session.
    pub key: u64,
    /// How long the game ran, once it exited.
    pub played: Option<Duration>,
    /// Level of the last game line, for continuation lines such as stack traces.
    last_level: Level,
    /// Absolute index of the first line not sent to the windows yet.
    emitted: usize,
}

impl Session {
    fn new(handle: GameHandle) -> Session {
        let now = unix_ms();
        Session {
            phase: Phase::Preparing,
            handle,
            status: "Starting".into(),
            progress: None,
            log: VecDeque::new(),
            dropped: 0,
            started: Instant::now(),
            started_unix_ms: now,
            key: now,
            played: None,
            last_level: Level::Info,
            emitted: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.phase != Phase::Finished
    }

    /// Adds `text` line by line; `level` = `None` classifies game output.
    pub fn push(&mut self, text: &str, level: Option<Level>) {
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            let level = level.unwrap_or_else(|| {
                let level = classify(line, self.last_level);
                self.last_level = level;
                level
            });
            self.log.push_back(LogLine { text: line.to_string(), level });
        }
        if self.log.len() > MAX_LOG_LINES {
            let extra = self.log.len() - MAX_LOG_LINES;
            self.log.drain(..extra);
            self.dropped += extra;
        }
    }

    fn view(&self) -> SessionView {
        SessionView {
            phase: self.phase,
            status: self.status.clone(),
            progress: self.progress,
            played: self.played.map(|d| d.as_secs()),
            started_at: self.started_unix_ms,
            key: self.key,
        }
    }

    /// Lines added since the last call, if any.
    fn take_new_lines(&mut self, id: &str) -> Option<LogChunk> {
        let end = self.dropped + self.log.len();
        if self.emitted >= end {
            return None;
        }
        let start = self.emitted.max(self.dropped);
        self.emitted = end;
        Some(LogChunk {
            id: id.to_string(),
            key: self.key,
            start,
            lines: self.log.iter().skip(start - self.dropped).cloned().collect(),
        })
    }

    pub fn chunk(&self, id: &str) -> LogChunk {
        LogChunk {
            id: id.to_string(),
            key: self.key,
            start: self.dropped,
            lines: self.log.iter().cloned().collect(),
        }
    }
}

/// Level of a game log line; stack trace lines inherit the level of the line before.
pub fn classify(line: &str, previous: Level) -> Level {
    let head: String = line.chars().take(96).collect::<String>().to_ascii_uppercase();
    if head.contains("/ERROR]")
        || head.contains("/FATAL]")
        || head.contains("[ERROR]")
        || head.contains(" ERROR ")
    {
        Level::Error
    } else if head.contains("/WARN]") || head.contains("[WARN]") || head.contains("[WARNING]") {
        Level::Warn
    } else if head.contains("/DEBUG]") || head.contains("/TRACE]") {
        Level::Debug
    } else if line.starts_with(['\t', ' ']) || line.starts_with("Caused by") || line.starts_with("Exception")
    {
        match previous {
            Level::Launcher => Level::Info,
            level => level,
        }
    } else {
        Level::Info
    }
}

/// Splits a log file into lines with their levels.
pub fn log_lines(text: &str) -> Vec<LogLine> {
    let mut previous = Level::Info;
    text.lines()
        .map(|line| {
            let level = classify(line, previous);
            previous = level;
            LogLine { text: line.to_string(), level }
        })
        .collect()
}

/// Modpack import, install or copy running in the background (not tied to a game).
#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub active: bool,
    pub status: String,
    pub progress: Option<(u64, u64)>,
}

/// What the windows know about a session.
#[derive(Clone, Debug, Serialize)]
pub struct SessionView {
    pub phase: Phase,
    pub status: String,
    pub progress: Option<(u64, u64)>,
    /// Seconds the game ran, once it exited.
    pub played: Option<u64>,
    /// Unix milliseconds when the game started, for the play timer.
    pub started_at: u64,
    pub key: u64,
}

/// Lines of a session's log starting at absolute index `start`.
#[derive(Clone, Debug, Serialize)]
pub struct LogChunk {
    pub id: String,
    pub key: u64,
    pub start: usize,
    pub lines: Vec<LogLine>,
}

/// An instance with the fields `instance.json` does not store.
#[derive(Clone, Debug, Serialize)]
pub struct InstanceView {
    pub id: String,
    pub dir: PathBuf,
    pub game_dir: PathBuf,
    pub icon: Option<PathBuf>,
    pub description: String,
    #[serde(flatten)]
    pub data: Instance,
}

impl From<&Instance> for InstanceView {
    fn from(inst: &Instance) -> Self {
        InstanceView {
            id: inst.id.clone(),
            dir: inst.dir.clone(),
            game_dir: inst.game_dir.clone(),
            icon: inst.icon.clone(),
            description: inst.description(),
            data: inst.clone(),
        }
    }
}

/// An account without its tokens: the windows only show and pick accounts.
#[derive(Clone, Debug, Serialize)]
pub struct AccountView {
    pub kind: AccountKind,
    pub name: String,
    pub uuid: String,
}

/// Everything the windows show, sent with every change.
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    /// The settings without accounts; see `accounts`.
    pub settings: Settings,
    pub accounts: Vec<AccountView>,
    pub selected_account: usize,
    pub instances: Vec<InstanceView>,
    pub sessions: HashMap<String, SessionView>,
    pub job: Option<Job>,
    pub manual_downloads: Vec<String>,
    pub running: usize,
    pub busy: bool,
}

/// Side effects on windows, run after the state lock is released.
pub enum Effect {
    MinimizeAll,
    RestoreMain,
    /// Selects this instance in the main window (e.g. after an import).
    Reveal(String),
    CloseInstanceWindow(String),
}

/// Where the state sends its events and window effects: the Tauri app, or a recorder in tests.
pub trait Ui: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: serde_json::Value);
    fn apply(&self, effect: Effect);
}

pub struct AppState {
    pub settings: Settings,
    pub instances: Vec<Instance>,
    pub sessions: HashMap<String, Session>,
    pub job: Option<Job>,
    /// Instances with files that have to be downloaded by hand (see [`import::BLOCKED_LIST`]).
    pub manual_downloads: HashSet<String>,
    effects: Vec<Effect>,
}

impl AppState {
    pub fn new(settings: Settings) -> AppState {
        let mut state = AppState {
            settings,
            instances: Vec::new(),
            sessions: HashMap::new(),
            job: None,
            manual_downloads: HashSet::new(),
            effects: Vec::new(),
        };
        state.load_instances();
        state
    }

    fn load_instances(&mut self) {
        self.instances = instance::list(&self.settings.data_dir);
        self.manual_downloads = self
            .instances
            .iter()
            .filter(|i| i.dir.join(import::BLOCKED_LIST).is_file())
            .map(|i| i.id.clone())
            .collect();
    }

    /// Reads the instances again, after the launcher folder changed.
    pub fn reload_instances(&mut self) {
        self.load_instances();
        self.sessions.retain(|_, s| s.is_active());
        let count = self.instances.len();
        self.notice(format!("Using {} ({count} instances)", self.settings.data_dir.display()));
    }

    pub fn instance(&self, id: &str) -> Option<&Instance> {
        self.instances.iter().find(|i| i.id == id)
    }

    /// Saves `inst` and replaces the copy in the list.
    pub fn save_instance(&mut self, inst: Instance) -> anyhow::Result<()> {
        inst.save()?;
        if let Some(slot) = self.instances.iter_mut().find(|i| i.id == inst.id) {
            *slot = inst;
        }
        Ok(())
    }

    /// Puts a new instance at the front and asks the main window to select it.
    pub fn add_instance(&mut self, inst: Instance) {
        self.instances.retain(|i| i.id != inst.id);
        self.effects.push(Effect::Reveal(inst.id.clone()));
        self.instances.insert(0, inst);
    }

    pub fn delete_instance(&mut self, id: &str) -> anyhow::Result<String> {
        let Some(inst) = self.instance(id).cloned() else { anyhow::bail!("No instance \"{id}\"") };
        if self.is_running(id) {
            anyhow::bail!("{} is running", inst.name);
        }
        instance::delete(&inst)?;
        self.instances.retain(|i| i.id != id);
        self.sessions.remove(id);
        self.manual_downloads.remove(id);
        self.effects.push(Effect::CloseInstanceWindow(id.to_string()));
        Ok(inst.name)
    }

    pub fn save_settings(&mut self) {
        if let Err(e) = self.settings.save() {
            self.notice(format!("Could not save settings: {e:#}"));
        }
    }

    /// Shows `status` in the status bar until the next job or selection.
    pub fn notice(&mut self, status: String) {
        self.job = Some(Job { active: false, status, progress: None });
    }

    pub fn clear_notice(&mut self) {
        if self.job.as_ref().is_some_and(|j| !j.active) {
            self.job = None;
        }
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.sessions.get(id).is_some_and(Session::is_active)
    }

    pub fn running_count(&self) -> usize {
        self.sessions.values().filter(|s| s.is_active()).count()
    }

    /// Games or jobs are running.
    pub fn busy(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.active) || self.running_count() > 0
    }

    /// Stops every running game, e.g. before quitting.
    pub fn kill_all(&mut self) {
        for session in self.sessions.values() {
            session.handle.kill();
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut settings = self.settings.clone();
        let accounts = std::mem::take(&mut settings.accounts);
        let mut manual_downloads: Vec<String> = self.manual_downloads.iter().cloned().collect();
        manual_downloads.sort();
        Snapshot {
            settings,
            accounts: accounts
                .iter()
                .map(|a| AccountView { kind: a.kind.clone(), name: a.name.clone(), uuid: a.uuid.clone() })
                .collect(),
            selected_account: self.settings.selected_account,
            instances: self.instances.iter().map(InstanceView::from).collect(),
            sessions: self.sessions.iter().map(|(id, s)| (id.clone(), s.view())).collect(),
            job: self.job.clone(),
            manual_downloads,
            running: self.running_count(),
            busy: self.busy(),
        }
    }

    // ---- accounts --------------------------------------------------------------

    /// Adds `account` and selects it; an account with the same UUID is replaced.
    pub fn add_account(&mut self, account: Account) {
        let accounts = &mut self.settings.accounts;
        match accounts.iter().position(|a| a.kind == account.kind && a.uuid == account.uuid) {
            Some(i) => {
                accounts[i] = account;
                self.settings.selected_account = i;
            }
            None => {
                accounts.push(account);
                self.settings.selected_account = accounts.len() - 1;
            }
        }
        self.save_settings();
    }

    pub fn remove_account(&mut self, index: usize) {
        let accounts = &mut self.settings.accounts;
        if index >= accounts.len() {
            return;
        }
        accounts.remove(index);
        let selected = &mut self.settings.selected_account;
        if *selected > index {
            *selected -= 1;
        }
        *selected = (*selected).min(accounts.len().saturating_sub(1));
        self.save_settings();
    }

    pub fn select_account(&mut self, index: usize) {
        if index < self.settings.accounts.len() {
            self.settings.selected_account = index;
            self.save_settings();
        }
    }

    // ---- games -------------------------------------------------------------------

    fn apply(&mut self, id: &str, msg: Msg) {
        match msg {
            Msg::Event(Event::AccountRefreshed(account)) => {
                // Matched by UUID: the selection may have changed since the launch started.
                let accounts = &mut self.settings.accounts;
                if let Some(slot) =
                    accounts.iter_mut().find(|a| a.kind == account.kind && a.uuid == account.uuid)
                {
                    *slot = account;
                    let _ = self.settings.save();
                }
            }
            Msg::Event(Event::InstanceUpdated(updated)) => {
                if let Some(slot) = self.instances.iter_mut().find(|i| i.id == updated.id) {
                    *slot = updated;
                }
            }
            msg => {
                let minimize = self.settings.on_launch == OnLaunch::Minimize;
                let Some(session) = self.sessions.get_mut(id) else { return };
                match msg {
                    Msg::Event(Event::Status(s)) => {
                        session.push(&s, Some(Level::Launcher));
                        session.status = s;
                    }
                    Msg::Event(Event::Progress { done, total }) => {
                        session.progress = (total > 0).then_some((done, total));
                    }
                    Msg::Event(Event::Log(line)) => {
                        let level = (line.starts_with("> ")
                            || line.starts_with("[!]")
                            || line.starts_with("[installer]"))
                        .then_some(Level::Launcher);
                        session.push(&line, level);
                    }
                    Msg::Event(Event::GameStarted) => {
                        session.phase = Phase::Running;
                        session.status = "Playing".into();
                        session.progress = None;
                        session.started = Instant::now();
                        session.started_unix_ms = unix_ms();
                        if minimize {
                            self.effects.push(Effect::MinimizeAll);
                        }
                    }
                    Msg::Event(Event::GameExited(code)) => {
                        let played = session.started.elapsed();
                        session.played = Some(played);
                        session.status = match code {
                            Some(0) => "Game closed".into(),
                            Some(code) => format!("Game crashed (exit code {code})"),
                            None => "Game was stopped".into(),
                        };
                        let level =
                            if matches!(code, Some(0) | None) { Level::Launcher } else { Level::Error };
                        let status = session.status.clone();
                        session.push(&status, Some(level));
                        if minimize {
                            self.effects.push(Effect::RestoreMain);
                        }
                        if let Some(inst) = self.instances.iter_mut().find(|i| i.id == id) {
                            // Re-read: the launch may have saved a resolved loader version meanwhile.
                            if let Ok(fresh) = instance::load(&inst.dir) {
                                *inst = fresh;
                            }
                            inst.play_time += played.as_secs();
                            let _ = inst.save();
                        }
                    }
                    Msg::Done(result) => {
                        session.phase = Phase::Finished;
                        session.progress = None;
                        if let Err(e) = result {
                            session.push(&format!("Error: {e}"), Some(Level::Error));
                            session.status = format!("Error: {e}");
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn apply_job(&mut self, msg: JobMsg) {
        match msg {
            JobMsg::Imported(imported) => {
                if !imported.blocked.is_empty() {
                    self.manual_downloads.insert(imported.instance.id.clone());
                }
                self.add_instance(imported.instance);
            }
            msg => {
                let Some(job) = self.job.as_mut() else { return };
                match msg {
                    JobMsg::Event(Event::Status(s)) => job.status = s,
                    JobMsg::Event(Event::Progress { done, total }) => {
                        job.progress = (total > 0).then_some((done, total));
                    }
                    JobMsg::Done(result) => {
                        job.active = false;
                        job.progress = None;
                        job.status = match result {
                            Ok(status) => status,
                            Err(e) => format!("Error: {e}"),
                        };
                    }
                    _ => {}
                }
            }
        }
    }
}

// Messages live only between a worker thread and the state; boxing them buys nothing.
#[allow(clippy::large_enum_variant)]
enum Msg {
    Event(Event),
    Done(Result<(), String>),
}

enum JobMsg {
    Event(Event),
    Imported(Imported),
    /// Final status line, or the error.
    Done(Result<String, String>),
}

/// The state behind a lock, the flag that tells the emitter to send it, and the windows.
pub struct Shared {
    state: Mutex<AppState>,
    dirty: AtomicBool,
    ui: Box<dyn Ui>,
}

impl Shared {
    pub fn new(state: AppState, ui: impl Ui) -> Arc<Shared> {
        Arc::new(Shared { state: Mutex::new(state), dirty: AtomicBool::new(true), ui: Box::new(ui) })
    }

    /// Read-only access; use [`Shared::update`] for changes.
    pub fn read(&self) -> MutexGuard<'_, AppState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes the state, then marks it for sending and runs the window effects it asked for.
    pub fn update<R>(&self, f: impl FnOnce(&mut AppState) -> R) -> R {
        self.change(true, f)
    }

    /// Like [`Shared::update`]; `dirty` = false for changes the snapshot does not show (log lines).
    fn change<R>(&self, dirty: bool, f: impl FnOnce(&mut AppState) -> R) -> R {
        let (result, effects) = {
            let mut state = self.read();
            let result = f(&mut state);
            (result, std::mem::take(&mut state.effects))
        };
        if dirty {
            self.dirty.store(true, Ordering::Release);
        }
        for effect in effects {
            self.ui.apply(effect);
        }
        result
    }

    pub fn emit(&self, event: &str, payload: impl Serialize) {
        if let Ok(value) = serde_json::to_value(payload) {
            self.ui.emit(event, value);
        }
    }

    /// Sends what changed: the snapshot when the state changed, and new log lines.
    pub fn flush(&self) {
        let snapshot = self.dirty.swap(false, Ordering::AcqRel).then(|| self.read().snapshot());
        let chunks: Vec<LogChunk> = {
            let mut state = self.read();
            state.sessions.iter_mut().filter_map(|(id, s)| s.take_new_lines(id)).collect()
        };
        if let Some(snapshot) = snapshot {
            self.emit("state", snapshot);
        }
        for chunk in chunks {
            self.emit("log", chunk);
        }
    }

    /// Flushes changes to the windows until the app exits.
    pub fn run_emitter(self: &Arc<Self>) {
        let shared = Arc::downgrade(self);
        std::thread::spawn(move || {
            while let Some(shared) = shared.upgrade() {
                shared.flush();
                drop(shared);
                std::thread::sleep(EMIT_INTERVAL);
            }
        });
    }

    /// Starts instance `id` on a worker thread; does nothing if it is already running.
    pub fn launch(self: &Arc<Self>, id: &str) -> bool {
        let handle = GameHandle::default();
        let prepared = self.update(|s| {
            if s.is_running(id) {
                return None;
            }
            let inst = s.instance(id).cloned()?;
            let mut session = Session::new(handle.clone());
            session.push(&format!("Launching {} ({})", inst.name, inst.description()), Some(Level::Launcher));
            s.sessions.insert(id.to_string(), session);
            let mut settings = s.settings.clone();
            if settings.account().is_none() {
                settings.accounts = vec![Account::offline("Player")];
                settings.selected_account = 0;
            }
            Some((inst, settings))
        });
        let Some((mut inst, settings)) = prepared else { return false };
        let (shared, id) = (self.clone(), id.to_string());
        std::thread::spawn(move || {
            let events = shared.clone();
            let event_id = id.clone();
            let reporter = Reporter::new(move |e| {
                // Games print thousands of lines a second; the lines go out on their own.
                let quiet = matches!(e, Event::Log(_));
                events.change(!quiet, |s| s.apply(&event_id, Msg::Event(e)))
            });
            inst.touch();
            let result = version::list(&settings.data_dir)
                .and_then(|manifest| launch::run(&settings, &mut inst, &manifest, &reporter, &handle));
            shared.update(|s| s.apply(&id, Msg::Done(result.map_err(|e| format!("{e:#}")))));
        });
        true
    }

    pub fn kill(&self, id: &str) {
        self.update(|s| {
            if let Some(session) = s.sessions.get_mut(id)
                && session.phase == Phase::Running
            {
                session.handle.kill();
                session.status = "Stopping".into();
                session.push("Stopping the game", Some(Level::Launcher));
            }
        });
    }

    /// Runs `work` on a worker thread, showing its progress in the status bar.
    /// Only one job runs at a time; returns false if one is already running.
    pub fn run_job(
        self: &Arc<Self>,
        status: &str,
        work: impl FnOnce(&Reporter, &dyn Fn(Imported)) -> anyhow::Result<String> + Send + 'static,
    ) -> bool {
        let started = self.update(|s| {
            if s.job.as_ref().is_some_and(|j| j.active) {
                return false;
            }
            s.job = Some(Job { active: true, status: status.into(), progress: None });
            true
        });
        if !started {
            return false;
        }
        let shared = self.clone();
        std::thread::spawn(move || {
            let events = shared.clone();
            let reporter = Reporter::new(move |e| events.update(|s| s.apply_job(JobMsg::Event(e))));
            let imported = |i| shared.update(|s| s.apply_job(JobMsg::Imported(i)));
            let result = work(&reporter, &imported);
            shared.update(|s| s.apply_job(JobMsg::Done(result.map_err(|e| format!("{e:#}")))));
        });
        true
    }
}

fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Final status line of an import job.
#[derive(Default)]
pub struct Summary {
    names: Vec<String>,
    blocked: usize,
}

impl Summary {
    pub fn add(&mut self, imported: &Imported) {
        self.names.push(imported.instance.name.clone());
        self.blocked += imported.blocked.len();
    }

    pub fn status(&self) -> String {
        let done = match self.names.as_slice() {
            [name] => format!("Added \"{name}\""),
            names => format!("Added {} instances", names.len()),
        };
        match self.blocked {
            0 => done,
            n => format!("{done}; {n} file(s) must be downloaded by hand, see the sidebar"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records events and window effects.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl Ui for Recorder {
        fn emit(&self, event: &str, _: serde_json::Value) {
            self.0.lock().unwrap().push(format!("emit:{event}"));
        }

        fn apply(&self, effect: Effect) {
            self.0.lock().unwrap().push(match effect {
                Effect::MinimizeAll => "minimize".into(),
                Effect::RestoreMain => "restore".into(),
                Effect::Reveal(id) => format!("reveal:{id}"),
                Effect::CloseInstanceWindow(id) => format!("close:{id}"),
            });
        }
    }

    fn shared_with(dir: &std::path::Path, on_launch: OnLaunch) -> (Arc<Shared>, Recorder) {
        let settings = Settings { data_dir: dir.to_path_buf(), on_launch, ..Settings::default() };
        let recorder = Recorder::default();
        (Shared::new(AppState::new(settings), recorder.clone()), recorder)
    }

    #[test]
    fn a_launch_goes_through_its_phases() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, recorder) = shared_with(dir.path(), OnLaunch::Minimize);
        let inst = instance::create(dir.path(), "Game", "1.21.1", gplauncher::instance::Loader::Vanilla, "")
            .unwrap();
        shared.update(|s| s.add_instance(inst.clone()));
        let id = inst.id.clone();
        shared.update(|s| {
            s.sessions.insert(id.clone(), Session::new(GameHandle::default()));
        });
        assert!(shared.read().is_running(&id));
        assert!(shared.read().busy());

        let apply = |msg| shared.update(|s| s.apply(&id, msg));
        apply(Msg::Event(Event::Status("Downloading".into())));
        apply(Msg::Event(Event::Progress { done: 5, total: 10 }));
        apply(Msg::Event(Event::Log("> java -jar".into())));
        apply(Msg::Event(Event::Log("[main/WARN]: hmm".into())));
        {
            let state = shared.read();
            let s = &state.sessions[&id];
            assert_eq!(s.status, "Downloading");
            assert_eq!(s.progress, Some((5, 10)));
            let levels: Vec<Level> = s.log.iter().map(|l| l.level).collect();
            assert_eq!(levels, [Level::Launcher, Level::Launcher, Level::Warn]);
        }
        apply(Msg::Event(Event::GameStarted));
        assert_eq!(shared.read().sessions[&id].phase, Phase::Running);
        apply(Msg::Event(Event::GameExited(Some(1))));
        apply(Msg::Done(Ok(())));
        let state = shared.read();
        let s = &state.sessions[&id];
        assert_eq!(s.phase, Phase::Finished);
        assert_eq!(s.status, "Game crashed (exit code 1)");
        assert_eq!(s.log.back().unwrap().level, Level::Error);
        assert!(s.played.is_some());
        assert!(!state.busy());
        let log = recorder.0.lock().unwrap().clone();
        assert!(log.contains(&format!("reveal:{id}")));
        let minimize = log.iter().position(|e| e == "minimize").unwrap();
        let restore = log.iter().position(|e| e == "restore").unwrap();
        assert!(minimize < restore);
    }

    #[test]
    fn a_failed_launch_shows_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, recorder) = shared_with(dir.path(), OnLaunch::KeepOpen);
        let inst =
            instance::create(dir.path(), "Broken", "1.21.1", gplauncher::instance::Loader::Vanilla, "")
                .unwrap();
        let id = inst.id.clone();
        shared.update(|s| {
            s.add_instance(inst);
            s.sessions.insert(id.clone(), Session::new(GameHandle::default()));
            s.apply(&id, Msg::Event(Event::GameStarted));
            s.apply(&id, Msg::Done(Err("no Java".into())));
        });
        let state = shared.read();
        assert_eq!(state.sessions[&id].status, "Error: no Java");
        assert!(!recorder.0.lock().unwrap().iter().any(|e| e == "minimize"));
    }

    #[test]
    fn deleting_closes_the_instance_window_and_refuses_running_games() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, recorder) = shared_with(dir.path(), OnLaunch::KeepOpen);
        let inst = instance::create(dir.path(), "Gone", "1.21.1", gplauncher::instance::Loader::Vanilla, "")
            .unwrap();
        let id = inst.id.clone();
        shared.update(|s| {
            s.add_instance(inst);
            s.sessions.insert(id.clone(), Session::new(GameHandle::default()));
        });
        assert!(shared.update(|s| s.delete_instance(&id)).unwrap_err().to_string().contains("is running"));
        shared.update(|s| s.apply(&id, Msg::Done(Ok(()))));
        assert_eq!(shared.update(|s| s.delete_instance(&id)).unwrap(), "Gone");
        assert!(recorder.0.lock().unwrap().contains(&format!("close:{id}")));
        assert!(shared.read().instances.is_empty());
    }

    #[test]
    fn jobs_run_one_at_a_time_and_report_their_result() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, _) = shared_with(dir.path(), OnLaunch::KeepOpen);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        assert!(shared.run_job("Working", move |reporter, _| {
            reporter.progress(1, 4);
            rx.recv().unwrap();
            Ok("Done".into())
        }));
        assert!(!shared.run_job("Second", |_, _| Ok(String::new())));
        tx.send(()).unwrap();
        let start = Instant::now();
        while shared.read().job.as_ref().is_some_and(|j| j.active) {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        let job = shared.read().job.clone().unwrap();
        assert_eq!((job.status.as_str(), job.progress), ("Done", None));
        assert!(shared.run_job("Failing", |_, _| anyhow::bail!("broken")));
        let start = Instant::now();
        while shared.read().job.as_ref().is_some_and(|j| j.active) {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(shared.read().job.as_ref().unwrap().status, "Error: broken");
    }

    #[test]
    fn flush_sends_state_and_new_lines_once() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, recorder) = shared_with(dir.path(), OnLaunch::KeepOpen);
        shared.flush();
        shared.update(|s| {
            let mut session = Session::new(GameHandle::default());
            session.push("hello", None);
            s.sessions.insert("x".into(), session);
        });
        shared.flush();
        shared.flush();
        let log = recorder.0.lock().unwrap().clone();
        assert_eq!(log, ["emit:state", "emit:state", "emit:log"]);
    }

    #[test]
    fn classifies_levels_and_continuations() {
        assert_eq!(classify("[12:00:00] [main/INFO]: hi", Level::Info), Level::Info);
        assert_eq!(classify("[12:00:00] [main/WARN]: careful", Level::Info), Level::Warn);
        assert_eq!(classify("[12:00:00] [Render thread/ERROR]: boom", Level::Info), Level::Error);
        assert_eq!(classify("[12:00:00] [main/FATAL]: dead", Level::Info), Level::Error);
        assert_eq!(classify("[12:00:00] [main/DEBUG]: x", Level::Info), Level::Debug);
        assert_eq!(classify("\tat net.minecraft.Foo.bar(Foo.java:1)", Level::Error), Level::Error);
        assert_eq!(classify("Caused by: java.lang.NullPointerException", Level::Warn), Level::Warn);
        // A trace after a launcher line is game output.
        assert_eq!(classify("\tat x", Level::Launcher), Level::Info);
    }

    #[test]
    fn log_lines_carry_levels() {
        let lines = log_lines("[a/ERROR]: x\n\tat y\nplain");
        let levels: Vec<Level> = lines.iter().map(|l| l.level).collect();
        assert_eq!(levels, [Level::Error, Level::Error, Level::Info]);
    }

    #[test]
    fn session_drops_old_lines_and_emits_only_new_ones() {
        let mut s = Session::new(GameHandle::default());
        s.push("one\ntwo\r", None);
        assert_eq!(s.log.len(), 2);
        assert_eq!(s.log[1].text, "two");
        let chunk = s.take_new_lines("i").unwrap();
        assert_eq!((chunk.start, chunk.lines.len()), (0, 2));
        assert!(s.take_new_lines("i").is_none());

        let many: Vec<String> = (0..MAX_LOG_LINES).map(|i| i.to_string()).collect();
        s.push(&many.join("\n"), Some(Level::Info));
        assert_eq!(s.log.len(), MAX_LOG_LINES);
        assert_eq!(s.dropped, 2);
        let chunk = s.take_new_lines("i").unwrap();
        assert_eq!(chunk.start, 2);
        assert_eq!(chunk.lines.len(), MAX_LOG_LINES);
        assert_eq!(chunk.lines.last().unwrap().text, (MAX_LOG_LINES - 1).to_string());
    }

    #[test]
    fn summary_status() {
        let inst = |name: &str| Imported {
            instance: Instance { name: name.into(), ..Default::default() },
            blocked: Vec::new(),
        };
        let mut s = Summary::default();
        s.add(&inst("A"));
        assert_eq!(s.status(), "Added \"A\"");
        s.add(&inst("B"));
        assert_eq!(s.status(), "Added 2 instances");
    }
}
