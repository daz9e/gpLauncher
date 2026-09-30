//! Launcher state shared by every window: settings, instances, running games with their logs,
//! and background jobs. Views observe the entity and redraw when it changes.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use gplauncher::auth::Account;
use gplauncher::import::{self, Imported};
use gplauncher::instance::{self, Instance};
use gplauncher::launch::{self, GameHandle};
use gplauncher::settings::{OnLaunch, Settings};
use gplauncher::{Event, Reporter, version};
use gpui::{App, AppContext, Context, Entity, SharedString};

/// Lines kept per game; older ones are dropped.
const MAX_LOG_LINES: usize = 100_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Preparing,
    Running,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Written by the launcher itself: steps, the command line, warnings.
    Launcher,
    Info,
    Warn,
    Error,
    Debug,
}

#[derive(Clone)]
pub struct LogLine {
    pub text: SharedString,
    pub level: Level,
}

/// One launch of an instance, from preparing files until the game exits. Kept afterwards so
/// its log can still be read.
pub struct Session {
    pub phase: Phase,
    handle: GameHandle,
    pub status: String,
    pub progress: Option<(u64, u64)>,
    pub log: Vec<LogLine>,
    /// Lines dropped from the front to stay under [`MAX_LOG_LINES`]; views use it to keep
    /// their position.
    pub dropped: usize,
    pub started: Instant,
    /// When the launch began; identifies the session.
    created: Instant,
    /// How long the game ran, once it exited.
    pub played: Option<Duration>,
    /// Level of the last game line, for continuation lines such as stack traces.
    last_level: Level,
}

impl Session {
    pub fn is_active(&self) -> bool {
        self.phase != Phase::Finished
    }

    pub fn started_at(&self) -> Instant {
        self.created
    }

    fn push(&mut self, text: String, level: Option<Level>) {
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            let level = level.unwrap_or_else(|| {
                let level = classify(line, self.last_level);
                self.last_level = level;
                level
            });
            self.log.push(LogLine { text: SharedString::from(line.to_string()), level });
        }
        if self.log.len() > MAX_LOG_LINES {
            let extra = self.log.len() - MAX_LOG_LINES;
            self.log.drain(..extra);
            self.dropped += extra;
        }
    }
}

/// Level of a game log line; stack trace lines inherit the level of the line before.
fn classify(line: &str, previous: Level) -> Level {
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

/// Modpack import, install or copy running in the background (not tied to a game).
pub struct Job {
    pub active: bool,
    pub status: String,
    pub progress: Option<(u64, u64)>,
}

enum JobMsg {
    Event(Event),
    Imported(Imported),
    /// Final status line, or the error.
    Done(Result<String, String>),
}

enum Msg {
    Event(Event),
    Done(Result<(), String>),
}

pub struct AppState {
    pub settings: Settings,
    pub instances: Vec<Instance>,
    pub sessions: HashMap<String, Session>,
    pub job: Option<Job>,
    /// Instances with files that have to be downloaded by hand (see [`import::BLOCKED_LIST`]).
    pub manual_downloads: HashSet<String>,
    /// Instance whose new state the main window should select, e.g. after an import.
    pub reveal: Option<String>,
}

pub type State = Entity<AppState>;

impl AppState {
    pub fn new(cx: &mut App) -> State {
        let settings = Settings::load();
        let mut state = AppState {
            settings,
            instances: Vec::new(),
            sessions: HashMap::new(),
            job: None,
            manual_downloads: HashSet::new(),
            reveal: None,
        };
        state.load_instances();
        cx.new(|_| state)
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
    pub fn reload_instances(&mut self, cx: &mut Context<Self>) {
        self.load_instances();
        self.sessions.retain(|_, s| s.is_active());
        let count = self.instances.len();
        self.notice(format!("Using {} ({count} instances)", self.settings.data_dir.display()), cx);
    }

    pub fn instance(&self, id: &str) -> Option<&Instance> {
        self.instances.iter().find(|i| i.id == id)
    }

    /// Saves `inst` and replaces the copy in the list.
    pub fn save_instance(&mut self, inst: Instance, cx: &mut Context<Self>) -> anyhow::Result<()> {
        inst.save()?;
        if let Some(slot) = self.instances.iter_mut().find(|i| i.id == inst.id) {
            *slot = inst;
        }
        cx.notify();
        Ok(())
    }

    /// Puts a new instance at the front and asks the main window to select it.
    pub fn add_instance(&mut self, inst: Instance, cx: &mut Context<Self>) {
        self.instances.retain(|i| i.id != inst.id);
        self.reveal = Some(inst.id.clone());
        self.instances.insert(0, inst);
        cx.notify();
    }

    pub fn delete_instance(&mut self, id: &str, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let Some(inst) = self.instance(id).cloned() else { return Ok(()) };
        if self.is_running(id) {
            anyhow::bail!("{} is running", inst.name);
        }
        instance::delete(&inst)?;
        self.instances.retain(|i| i.id != id);
        self.sessions.remove(id);
        self.manual_downloads.remove(id);
        cx.notify();
        Ok(())
    }

    pub fn save_settings(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.settings.save() {
            self.notice(format!("Could not save settings: {e:#}"), cx);
        }
        cx.notify();
    }

    /// Shows `status` in the status bar until the next job or selection.
    pub fn notice(&mut self, status: String, cx: &mut Context<Self>) {
        self.job = Some(Job { active: false, status, progress: None });
        cx.notify();
    }

    pub fn clear_notice(&mut self, cx: &mut Context<Self>) {
        if self.job.as_ref().is_some_and(|j| !j.active) {
            self.job = None;
            cx.notify();
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

    pub fn account_name(&self) -> String {
        self.settings.account().map(|a| a.name.clone()).unwrap_or_else(|| "Player".into())
    }

    // ---- games ---------------------------------------------------------------

    pub fn launch(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.is_running(id) {
            return;
        }
        let Some(mut inst) = self.instance(id).cloned() else { return };
        let handle = GameHandle::default();
        let mut session = Session {
            phase: Phase::Preparing,
            handle: handle.clone(),
            status: "Starting".into(),
            progress: None,
            log: Vec::new(),
            dropped: 0,
            started: Instant::now(),
            created: Instant::now(),
            played: None,
            last_level: Level::Info,
        };
        session.push(format!("Launching {} ({})", inst.name, inst.description()), Some(Level::Launcher));
        self.sessions.insert(id.to_string(), session);

        let mut settings = self.settings.clone();
        if settings.account().is_none() {
            settings.accounts = vec![Account::offline("Player")];
            settings.selected_account = 0;
        }
        let (tx, mut rx) = mpsc::unbounded();
        std::thread::spawn(move || {
            let events = tx.clone();
            let reporter = Reporter::new(move |e| {
                let _ = events.unbounded_send(Msg::Event(e));
            });
            inst.touch();
            let result = version::list(&settings.data_dir)
                .and_then(|manifest| launch::run(&settings, &mut inst, &manifest, &reporter, &handle));
            let _ = tx.unbounded_send(Msg::Done(result.map_err(|e| format!("{e:#}"))));
        });

        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                // Games can print thousands of lines a second: apply what has queued up at once.
                let mut batch = vec![first];
                while let Ok(msg) = rx.try_recv() {
                    batch.push(msg);
                }
                let alive = this.update(cx, |this, cx| {
                    for msg in batch {
                        this.apply(&id, msg, cx);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn kill(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.sessions.get_mut(id)
            && session.phase == Phase::Running
        {
            session.handle.kill();
            session.status = "Stopping".into();
            session.push("Stopping the game".into(), Some(Level::Launcher));
            cx.notify();
        }
    }

    /// Stops every running game, e.g. before quitting.
    pub fn kill_all(&mut self) {
        for session in self.sessions.values() {
            session.handle.kill();
        }
    }

    fn apply(&mut self, id: &str, msg: Msg, cx: &mut Context<Self>) {
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
                        session.push(s.clone(), Some(Level::Launcher));
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
                        session.push(line, level);
                    }
                    Msg::Event(Event::GameStarted) => {
                        session.phase = Phase::Running;
                        session.status = "Playing".into();
                        session.progress = None;
                        session.started = Instant::now();
                        if minimize {
                            for window in cx.windows() {
                                let _ = window.update(cx, |_, window, _| window.minimize_window());
                            }
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
                        session.push(session.status.clone(), Some(level));
                        if minimize && let Some(window) = cx.windows().into_iter().next() {
                            let _ = window.update(cx, |_, window, _| window.activate_window());
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
                            session.push(format!("Error: {e}"), Some(Level::Error));
                            session.status = format!("Error: {e}");
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ---- background jobs --------------------------------------------------------

    /// Runs `work` on a worker thread, showing its progress in the status bar.
    /// Only one job runs at a time; returns false if one is already running.
    pub fn run_job(
        &mut self,
        status: &str,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Reporter, &dyn Fn(Imported)) -> anyhow::Result<String> + Send + 'static,
    ) -> bool {
        if self.job.as_ref().is_some_and(|j| j.active) {
            return false;
        }
        self.job = Some(Job { active: true, status: status.into(), progress: None });
        let (tx, mut rx) = mpsc::unbounded();
        std::thread::spawn(move || {
            let events = tx.clone();
            let reporter = Reporter::new(move |e| {
                let _ = events.unbounded_send(JobMsg::Event(e));
            });
            let imported = |i| {
                let _ = tx.unbounded_send(JobMsg::Imported(i));
            };
            let result = work(&reporter, &imported);
            let _ = tx.unbounded_send(JobMsg::Done(result.map_err(|e| format!("{e:#}"))));
        });

        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let alive = this.update(cx, |this, cx| {
                    this.apply_job(msg, cx);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
        true
    }

    fn apply_job(&mut self, msg: JobMsg, cx: &mut Context<Self>) {
        match msg {
            JobMsg::Imported(imported) => {
                if !imported.blocked.is_empty() {
                    self.manual_downloads.insert(imported.instance.id.clone());
                }
                self.add_instance(imported.instance, cx);
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

/// Splits a log file into lines with their levels.
pub fn log_lines(text: &str) -> Vec<LogLine> {
    let mut previous = Level::Info;
    text.lines()
        .map(|line| {
            let level = classify(line, previous);
            previous = level;
            LogLine { text: SharedString::from(line.to_string()), level }
        })
        .collect()
}
