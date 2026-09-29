//! Main window: toolbar, instance grid, sidebar with actions for the selected instance, status bar.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use futures::channel::mpsc;
use gplauncher::auth::Account;
use gplauncher::import::{self, Imported};
use gplauncher::instance::{self, Instance, Loader};
use gplauncher::launch::{self, GameHandle};
use gplauncher::settings::{OnLaunch, Settings};
use gplauncher::{Event, Reporter, export, modpack, version};
use gpui::{
    AnyElement, AnyView, App, Context, Entity, ExternalPaths, FocusHandle, Focusable, Font, FontWeight,
    IntoElement, LineFragment, ParentElement, Pixels, PromptLevel, Render, SharedString, Styled, TextRun,
    Window, actions, div, img, prelude::*, px, relative, rems, svg,
};

use crate::accounts::{self, Accounts, AccountsEvent};
use crate::add_instance::{self, AddInstance, AddInstanceEvent};
use crate::edit_instance::{self, EditInstance, EditInstanceEvent};
use crate::settings_page::{SettingsEvent, SettingsPage};
use crate::shortcut;
use crate::text_input::{self, TextInput};
use crate::theme::{self, Theme};

actions!(launcher, [NewInstance, FocusSearch, OpenSettings]);

const TOOLBAR_HEIGHT: f32 = 44.;
const SIDEBAR_WIDTH: f32 = 248.;
const TILE_WIDTH: f32 = 128.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    Running,
    Finished,
}

/// One launch of an instance, from preparing files until the game exits.
struct Session {
    phase: Phase,
    handle: GameHandle,
    status: String,
    progress: Option<(u64, u64)>,
}

enum Msg {
    Event(Event),
    Done(Result<(), String>),
}

/// Modpack import or install running in the background (not tied to an instance).
struct Job {
    active: bool,
    status: String,
    progress: Option<(u64, u64)>,
}

enum JobMsg {
    Event(Event),
    Imported(Imported),
    /// Final status line, or the error.
    Done(Result<String, String>),
}

pub struct Launcher {
    focus_handle: FocusHandle,
    settings: Settings,
    instances: Vec<Instance>,
    selected: Option<String>,
    sessions: HashMap<String, Session>,
    search: Entity<TextInput>,
    add_dialog: Option<Entity<AddInstance>>,
    edit_dialog: Option<Entity<EditInstance>>,
    accounts_dialog: Option<Entity<Accounts>>,
    settings_page: Option<Entity<SettingsPage>>,
    job: Option<Job>,
    /// Groups folded in the grid.
    collapsed: HashSet<String>,
    /// Instances with files that have to be downloaded by hand (see [`import::BLOCKED_LIST`]).
    manual_downloads: HashSet<String>,
}

impl Launcher {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, _, cx| cx.notify()).detach();
        let settings = Settings::load();
        theme::set_appearance(settings.appearance);
        let instances = instance::list(&settings.data_dir);
        let selected = instances.first().map(|i| i.id.clone());
        let manual_downloads = instances
            .iter()
            .filter(|i| i.dir.join(import::BLOCKED_LIST).is_file())
            .map(|i| i.id.clone())
            .collect();
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let search = cx.new(|cx| TextInput::new("Search instances", cx).with_icon("icons/search.svg"));
        cx.subscribe(&search, |_, _, _: &text_input::Changed, cx| cx.notify()).detach();
        Launcher {
            focus_handle,
            settings,
            instances,
            selected,
            sessions: HashMap::new(),
            search,
            add_dialog: None,
            edit_dialog: None,
            accounts_dialog: None,
            settings_page: None,
            job: None,
            collapsed: HashSet::new(),
            manual_downloads,
        }
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = Some(id);
        // A finished import's message has been seen once the user moves on.
        if self.job.as_ref().is_some_and(|j| !j.active) {
            self.job = None;
        }
        cx.notify();
    }

    fn open_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_dialog.is_some() {
            return;
        }
        self.close_settings(window, cx);
        let (data_dir, key) = (self.settings.data_dir.clone(), self.settings.curseforge_key());
        let dialog = cx.new(|cx| AddInstance::new(data_dir, key, window, cx));
        cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            match event {
                AddInstanceEvent::Created(inst) => this.add_instance(inst.clone(), cx),
                AddInstanceEvent::Import(paths) => this.import(paths.clone(), cx),
                AddInstanceEvent::Install(pack, version) => this.install(pack.clone(), version.clone(), cx),
                AddInstanceEvent::SaveCurseForgeKey(key) => {
                    this.settings.curseforge_api_key = key.clone();
                    if let Err(e) = this.settings.save() {
                        this.job = Some(Job {
                            active: false,
                            status: format!("Could not save settings: {e:#}"),
                            progress: None,
                        });
                    }
                    return;
                }
                AddInstanceEvent::Dismissed => {}
            }
            this.add_dialog = None;
            window.focus(&this.focus_handle);
            cx.notify();
        })
        .detach();
        self.add_dialog = Some(dialog);
        cx.notify();
    }

    fn open_accounts_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.accounts_dialog.is_some() {
            return;
        }
        let s = &self.settings;
        let (accounts, selected, client_id) =
            (s.accounts.clone(), s.selected_account, s.ms_client_id.clone());
        let dialog = cx.new(|cx| Accounts::new(accounts, selected, client_id, window, cx));
        cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            match event {
                AccountsEvent::Changed { accounts, selected } => {
                    this.settings.accounts = accounts.clone();
                    this.settings.selected_account = *selected;
                }
                AccountsEvent::SaveClientId(id) => this.settings.ms_client_id = id.clone(),
                AccountsEvent::Dismissed => {
                    this.accounts_dialog = None;
                    window.focus(&this.focus_handle);
                    cx.notify();
                    return;
                }
            }
            if let Err(e) = this.settings.save() {
                this.notice(format!("Could not save settings: {e:#}"), cx);
            }
            cx.notify();
        })
        .detach();
        self.accounts_dialog = Some(dialog);
        cx.notify();
    }

    /// Games or jobs are running.
    fn busy(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.active) || self.sessions.keys().any(|id| self.is_running(id))
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_page.is_some() || self.modal().is_some() {
            return;
        }
        let (settings, busy) = (self.settings.clone(), self.busy());
        let page = cx.new(|cx| SettingsPage::new(settings, busy, window, cx));
        cx.subscribe_in(&page, window, |this, _, event, window, cx| match event {
            SettingsEvent::Changed(settings) => this.apply_settings(settings.clone(), window, cx),
            SettingsEvent::Closed => this.close_settings(window, cx),
        })
        .detach();
        self.settings_page = Some(page);
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_page.take().is_some() {
            window.focus(&self.focus_handle);
            cx.notify();
        }
    }

    fn apply_settings(&mut self, mut settings: Settings, window: &mut Window, cx: &mut Context<Self>) {
        // Accounts may have been refreshed by a running launch meanwhile.
        settings.accounts = std::mem::take(&mut self.settings.accounts);
        settings.selected_account = self.settings.selected_account;
        let moved = settings.data_dir != self.settings.data_dir;
        self.settings = settings;
        theme::set_appearance(self.settings.appearance);
        window.refresh();
        if let Err(e) = self.settings.save() {
            self.notice(format!("Could not save settings: {e:#}"), cx);
        }
        if moved {
            self.reload_instances(cx);
        }
    }

    /// Reads the instances again, after the launcher folder changed.
    fn reload_instances(&mut self, cx: &mut Context<Self>) {
        self.instances = instance::list(&self.settings.data_dir);
        self.sessions.clear();
        self.collapsed.clear();
        self.manual_downloads = self
            .instances
            .iter()
            .filter(|i| i.dir.join(import::BLOCKED_LIST).is_file())
            .map(|i| i.id.clone())
            .collect();
        self.selected = self.instances.first().map(|i| i.id.clone());
        let count = self.instances.len();
        self.notice(format!("Using {} ({count} instances)", self.settings.data_dir.display()), cx);
    }

    /// Puts a new instance at the front of the grid and selects it.
    fn add_instance(&mut self, inst: Instance, cx: &mut Context<Self>) {
        self.instances.retain(|i| i.id != inst.id);
        let id = inst.id.clone();
        self.instances.insert(0, inst);
        self.select(id, cx);
    }

    /// Selects the instance with `id` and launches it (for `--launch` shortcuts).
    pub fn launch_id(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.instances.iter().any(|i| i.id == id) {
            true => {
                self.select(id.to_string(), cx);
                self.launch(window, cx);
            }
            false => self.notice(format!("No instance \"{id}\" to launch"), cx),
        }
    }

    /// Shows `status` in the status bar until the user selects something.
    fn notice(&mut self, status: String, cx: &mut Context<Self>) {
        self.job = Some(Job { active: false, status, progress: None });
        cx.notify();
    }

    fn is_running(&self, id: &str) -> bool {
        self.sessions.get(id).is_some_and(|s| s.phase != Phase::Finished)
    }

    fn open_edit_dialog(&mut self, focus: edit_instance::Focus, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inst) = self.selected().cloned() else { return };
        if self.edit_dialog.is_some() || self.is_running(&inst.id) {
            return;
        }
        let settings = self.settings.clone();
        let dialog = cx.new(|cx| EditInstance::new(inst, settings, focus, window, cx));
        cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            if let EditInstanceEvent::Saved(inst) = event
                && let Some(slot) = this.instances.iter_mut().find(|i| i.id == inst.id)
            {
                *slot = inst.clone();
            }
            this.edit_dialog = None;
            window.focus(&this.focus_handle);
            cx.notify();
        })
        .detach();
        self.edit_dialog = Some(dialog);
        cx.notify();
    }

    fn open_folder(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = self.selected() else { return };
        let dir = inst.game_dir.clone();
        let _ = std::fs::create_dir_all(&dir);
        cx.open_with_system(&dir);
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = self.selected().cloned() else { return };
        let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_default();
        let path = cx.prompt_for_new_path(&dir, Some(&export::file_name(&inst)));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(dest))) = path.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.run_job(&format!("Exporting {}", inst.name), cx, move |reporter, _| {
                    export::export(&inst, &dest, reporter)?;
                    Ok(format!("Exported to {}", dest.display()))
                });
            });
        })
        .detach();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = self.selected().cloned() else { return };
        let data_dir = self.settings.data_dir.clone();
        self.run_job(&format!("Copying {}", inst.name), cx, move |_, imported| {
            let copy = instance::duplicate(&data_dir, &inst, &format!("{} (copy)", inst.name))?;
            let status = format!("Copied to \"{}\"", copy.name);
            imported(Imported { instance: copy, blocked: Vec::new() });
            Ok(status)
        });
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inst) = self.selected().cloned() else { return };
        if self.is_running(&inst.id) {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Delete \"{}\"?", inst.name),
            Some("The instance folder with its worlds, mods and settings will be deleted permanently."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = instance::delete(&inst) {
                    return this.notice(format!("Error: {e:#}"), cx);
                }
                let index = this.instances.iter().position(|i| i.id == inst.id).unwrap_or(0);
                this.instances.retain(|i| i.id != inst.id);
                this.sessions.remove(&inst.id);
                this.manual_downloads.remove(&inst.id);
                let next = this.instances.get(index).or(this.instances.last()).map(|i| i.id.clone());
                this.selected = next;
                this.notice(format!("Deleted \"{}\"", inst.name), cx);
            });
        })
        .detach();
    }

    fn create_shortcut(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = self.selected() else { return };
        let status = match shortcut::create(inst) {
            Ok(path) => format!("Created {}", path.display()),
            Err(e) => format!("Could not create a shortcut: {e:#}"),
        };
        self.notice(status, cx);
    }

    fn toggle_group(&mut self, group: String, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&group) {
            self.collapsed.insert(group);
        }
        cx.notify();
    }

    fn import(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let files = add_instance::importable(&paths);
        if files.is_empty() {
            self.job = Some(Job {
                active: false,
                status: "Nothing to import: expected .mrpack or .zip modpacks".into(),
                progress: None,
            });
            cx.notify();
            return;
        }
        let settings = self.settings.clone();
        self.run_job("Importing", cx, move |reporter, imported| {
            let mut summary = Summary::default();
            for file in &files {
                let result = import::import(&settings, file, reporter)?;
                summary.add(&result);
                imported(result);
            }
            Ok(summary.status())
        });
    }

    fn install(&mut self, pack: modpack::Pack, version: modpack::PackVersion, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        self.run_job(&format!("Installing {}", pack.title), cx, move |reporter, imported| {
            let result = modpack::install(&settings, &pack, &version, reporter)?;
            let mut summary = Summary::default();
            summary.add(&result);
            imported(result);
            Ok(summary.status())
        });
    }

    /// Runs `work` on a worker thread, showing its progress in the status bar.
    /// Only one job runs at a time.
    fn run_job(
        &mut self,
        status: &str,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Reporter, &dyn Fn(Imported)) -> anyhow::Result<String> + Send + 'static,
    ) {
        if self.job.as_ref().is_some_and(|j| j.active) {
            return;
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

    fn selected(&self) -> Option<&Instance> {
        let id = self.selected.as_ref()?;
        self.instances.iter().find(|i| &i.id == id)
    }

    fn account_name(&self) -> String {
        self.settings.account().map(|a| a.name.clone()).unwrap_or_else(|| "Player".into())
    }

    fn can_launch(&self) -> bool {
        self.selected().is_some_and(|i| self.sessions.get(&i.id).is_none_or(|s| s.phase == Phase::Finished))
    }

    fn can_kill(&self) -> bool {
        self.selected().is_some_and(|i| self.sessions.get(&i.id).is_some_and(|s| s.phase == Phase::Running))
    }

    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_launch() {
            return;
        }
        let Some(mut inst) = self.selected().cloned() else { return };
        let id = inst.id.clone();
        let handle = GameHandle::default();
        self.sessions.insert(
            id.clone(),
            Session {
                phase: Phase::Preparing,
                handle: handle.clone(),
                status: "Starting".into(),
                progress: None,
            },
        );

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

        cx.spawn_in(window, async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let alive = this.update_in(cx, |this, window, cx| {
                    this.apply(&id, msg, window);
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

    fn kill(&mut self, cx: &mut Context<Self>) {
        if !self.can_kill() {
            return;
        }
        if let Some(session) = self.selected.as_ref().and_then(|id| self.sessions.get_mut(id)) {
            session.handle.kill();
            session.status = "Stopping".into();
            cx.notify();
        }
    }

    fn apply(&mut self, id: &str, msg: Msg, window: &mut Window) {
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
                let Some(session) = self.sessions.get_mut(id) else { return };
                match msg {
                    Msg::Event(Event::Status(s)) => session.status = s,
                    Msg::Event(Event::Progress { done, total }) => {
                        session.progress = (total > 0).then_some((done, total));
                    }
                    Msg::Event(Event::GameStarted) => {
                        session.phase = Phase::Running;
                        session.status = "Playing".into();
                        session.progress = None;
                        if self.settings.on_launch == OnLaunch::Minimize {
                            window.minimize_window();
                        }
                    }
                    Msg::Event(Event::GameExited(code)) => {
                        if self.settings.on_launch == OnLaunch::Minimize {
                            window.activate_window();
                        }
                        session.status = match code {
                            Some(0) => "Game closed".into(),
                            Some(code) => format!("Game exited with code {code}"),
                            None => "Game was stopped".into(),
                        };
                    }
                    Msg::Done(result) => {
                        session.phase = Phase::Finished;
                        session.progress = None;
                        if let Err(e) = result {
                            session.status = format!("Error: {e}");
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ---- rendering -----------------------------------------------------------

    fn toolbar(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let data_dir = self.settings.data_dir.clone();
        let account = self.account_name();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(TOOLBAR_HEIGHT))
            .px_3()
            .bg(t.panel)
            .border_b_1()
            .border_color(t.border)
            .child(
                toolbar_button("add", "icons/plus.svg", "Add Instance", true, t)
                    .on_click(cx.listener(|this, _, window, cx| this.open_add_dialog(window, cx))),
            )
            .child(toolbar_button("folders", "icons/folder.svg", "Folders", true, t).on_click(
                move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&data_dir);
                    cx.open_with_system(&data_dir);
                },
            ))
            .child(
                toolbar_button("settings", "icons/settings.svg", "Settings", true, t)
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            )
            .child(div().flex_1())
            .child(div().w(px(240.)).min_w(px(120.)).flex_shrink().child(self.search.clone()))
            .child(
                div()
                    .id("account")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .ml_2()
                    .pl_1()
                    .pr_2()
                    .py_1()
                    .rounded_full()
                    .border_1()
                    .border_color(t.border)
                    .bg(t.bg)
                    .cursor_pointer()
                    .hover(|d| d.bg(t.hover))
                    .on_click(cx.listener(|this, _, window, cx| this.open_accounts_dialog(window, cx)))
                    .child(accounts::avatar(&account, 22., t))
                    .child(div().text_sm().text_color(t.text).whitespace_nowrap().child(account))
                    .child(svg().path("icons/chevron-down.svg").size(px(12.)).text_color(t.muted)),
            )
    }

    fn grid(&self, t: Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.instances.is_empty() {
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(svg().path("icons/box.svg").size(px(28.)).text_color(t.subtle))
                .child(div().text_color(t.text).font_weight(FontWeight::MEDIUM).child("No instances yet"))
                .child(
                    div()
                        .text_sm()
                        .text_color(t.muted)
                        .child("Create one with Add Instance, or drop a modpack (.mrpack, .zip) here"),
                )
                .into_any_element();
        }
        let query = self.search.read(cx).text().trim().to_lowercase();
        let visible: Vec<&Instance> = self
            .instances
            .iter()
            .filter(|i| query.is_empty() || i.name.to_lowercase().contains(&query))
            .collect();
        if visible.is_empty() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(t.muted)
                .child("No instances match the search")
                .into_any_element();
        }
        let mut name_font = window.text_style().font();
        name_font.weight = FontWeight::MEDIUM;
        let name_size = rems(0.875).to_pixels(window.rem_size());
        let tiles = |instances: &[&Instance], cx: &mut Context<Self>| {
            // Tiles keep their own height: the selected one grows with its full name.
            div().flex().flex_wrap().items_start().gap_2().children(
                instances
                    .iter()
                    .map(|inst| self.tile(inst, name_font.clone(), name_size, t, cx))
                    .collect::<Vec<_>>(),
            )
        };

        // Ungrouped instances first, then groups by name.
        let mut groups: BTreeMap<(bool, &str), Vec<&Instance>> = BTreeMap::new();
        for inst in visible.iter().copied() {
            groups.entry((!inst.group.is_empty(), inst.group.as_str())).or_default().push(inst);
        }
        let content = if groups.keys().all(|(named, _)| !named) {
            div()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap_2()
                        .mb_3()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.text)
                                .child("Instances"),
                        )
                        .child(div().text_xs().text_color(t.subtle).child(visible.len().to_string())),
                )
                .child(tiles(&visible, cx))
        } else {
            div().flex().flex_col().gap_4().children(
                groups
                    .into_iter()
                    .map(|((_, group), instances)| {
                        let collapsed = self.collapsed.contains(group);
                        let key = group.to_string();
                        let label = if group.is_empty() { "Ungrouped".to_string() } else { key.clone() };
                        div()
                            .child(
                                div()
                                    .id(SharedString::from(format!("group-{group}")))
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .mb_2()
                                    .cursor_pointer()
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| this.toggle_group(key.clone(), cx)),
                                    )
                                    .child(
                                        svg()
                                            .path(if collapsed {
                                                "icons/chevron-right.svg"
                                            } else {
                                                "icons/chevron-down.svg"
                                            })
                                            .size(px(14.))
                                            .text_color(t.muted),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(t.text)
                                            .child(label),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(t.subtle)
                                            .child(instances.len().to_string()),
                                    )
                                    .child(div().flex_1().ml_1().h(px(1.)).bg(t.border)),
                            )
                            .when(!collapsed, |d| d.child(tiles(&instances, cx)))
                    })
                    .collect::<Vec<_>>(),
            )
        };
        div()
            .id("instances")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .child(content)
            .into_any_element()
    }

    fn tile(
        &self,
        inst: &Instance,
        name_font: Font,
        name_size: Pixels,
        t: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected.as_deref() == Some(inst.id.as_str());
        let running = self.is_running(&inst.id);
        let id = inst.id.clone();
        div()
            .id(SharedString::from(format!("inst-{}", inst.id)))
            .w(px(TILE_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .pt_3()
            .pb_2p5()
            .px_2()
            .rounded_lg()
            .border_1()
            .cursor_pointer()
            .when(selected, |d| d.bg(t.accent_soft).border_color(t.accent.opacity(0.35)))
            .when(!selected, |d| d.border_color(gpui::transparent_black()).hover(|d| d.bg(t.hover)))
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                this.select(id.clone(), cx);
                if e.click_count() == 2 {
                    this.launch(window, cx);
                }
            }))
            .child(instance_icon(64., running, inst.icon.as_deref(), t))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_0p5()
                    // The selected tile shows the whole name, others two lines.
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.text)
                            .text_center()
                            .when(selected, |d| d.child(inst.name.clone()))
                            .when(!selected, |d| {
                                d.children(
                                    clamp_lines(&inst.name, name_font, name_size, 2, cx)
                                        .into_iter()
                                        .map(|line| div().whitespace_nowrap().child(line)),
                                )
                            }),
                    )
                    .child(div().text_xs().text_color(t.muted).child(short_description(inst))),
            )
            .into_any_element()
    }

    fn sidebar(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (can_launch, can_kill) = (self.can_launch(), self.can_kill());
        let memory = self.selected().and_then(|i| i.memory_mb).unwrap_or(self.settings.memory_mb);
        div()
            .id("sidebar")
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .bg(t.panel)
            .border_l_1()
            .border_color(t.border)
            .when_some(self.selected(), |d, inst| {
                let running = self.is_running(&inst.id);
                let loader = match inst.loader {
                    Loader::Vanilla => "None".to_string(),
                    l if inst.loader_version.is_empty() => format!("{} (latest)", l.label()),
                    l => format!("{} {}", l.label(), inst.loader_version),
                };
                let group = if inst.group.is_empty() { "None".to_string() } else { inst.group.clone() };
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .pt_5()
                        .pb_4()
                        .child(instance_icon(72., running, inst.icon.as_deref(), t))
                        .child(
                            div()
                                .w_full()
                                .text_center()
                                .text_color(t.text)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(inst.name.clone()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .px_4()
                        .pb_3()
                        .child(
                            action_button("launch", "icons/play.svg", "Launch", can_launch, true, t)
                                .flex_1()
                                .when(can_launch, |b| {
                                    b.on_click(cx.listener(|this, _, window, cx| this.launch(window, cx)))
                                }),
                        )
                        .child(
                            action_button("kill", "icons/stop.svg", "Kill", can_kill, false, t)
                                .px_3()
                                .when(can_kill, |b| b.on_click(cx.listener(|this, _, _, cx| this.kill(cx)))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .mx_2()
                        .py_2()
                        .border_t_1()
                        .border_color(t.border)
                        .child(menu_item("edit", "icons/pencil.svg", "Edit", !running, false, t).when(
                            !running,
                            |b| {
                                b.on_click(cx.listener(|this, _, window, cx| {
                                    this.open_edit_dialog(edit_instance::Focus::Name, window, cx)
                                }))
                            },
                        ))
                        .child(menu_item("group", "icons/tag.svg", "Change Group", !running, false, t).when(
                            !running,
                            |b| {
                                b.on_click(cx.listener(|this, _, window, cx| {
                                    this.open_edit_dialog(edit_instance::Focus::Group, window, cx)
                                }))
                            },
                        ))
                        .child(
                            menu_item("folder", "icons/folder.svg", "Folder", true, false, t)
                                .on_click(cx.listener(|this, _, _, cx| this.open_folder(cx))),
                        )
                        .child(
                            menu_item("export", "icons/share.svg", "Export", true, false, t)
                                .on_click(cx.listener(|this, _, _, cx| this.export(cx))),
                        )
                        .child(
                            menu_item("copy", "icons/copy.svg", "Copy", true, false, t)
                                .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                        )
                        .child(
                            menu_item("shortcut", "icons/shortcut.svg", "Create Shortcut", true, false, t)
                                .on_click(cx.listener(|this, _, _, cx| this.create_shortcut(cx))),
                        )
                        .child(
                            menu_item("delete", "icons/trash.svg", "Delete", !running, true, t)
                                .when(!running, |b| {
                                    b.on_click(cx.listener(|this, _, window, cx| this.delete(window, cx)))
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .mx_4()
                        .pt_3()
                        .pb_4()
                        .border_t_1()
                        .border_color(t.border)
                        .child(detail("Minecraft", inst.minecraft.clone(), t))
                        .child(detail("Loader", loader, t))
                        .child(detail("Group", group, t))
                        .child(detail("Memory", format!("{memory} MB"), t))
                        .child(detail("Last played", last_played(inst.last_played), t)),
                )
                .when(self.manual_downloads.contains(&inst.id), |d| {
                    let list = inst.dir.join(import::BLOCKED_LIST);
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .mx_4()
                            .mb_4()
                            .p_3()
                            .rounded_md()
                            .border_1()
                            .border_color(t.danger.opacity(0.4))
                            .bg(t.danger.opacity(0.08))
                            .text_xs()
                            .child(div().text_color(t.text).child("Some mods must be downloaded by hand"))
                            .child(
                                div()
                                    .id("manual-list")
                                    .text_color(t.accent)
                                    .cursor_pointer()
                                    .hover(|d| d.underline())
                                    .on_click(move |_, _, cx| cx.open_with_system(&list))
                                    .child("Show the list"),
                            ),
                    )
                })
            })
    }

    fn status_bar(&self, t: Theme) -> impl IntoElement {
        let inst = self.selected();
        let session = inst.and_then(|i| self.sessions.get(&i.id));
        // An active import wins; a finished one stays until the user selects something.
        let (text, active, progress) = match (&self.job, session) {
            (Some(job), _) if job.active => (job.status.clone(), true, job.progress),
            (_, Some(s)) => (s.status.clone(), s.phase != Phase::Finished, s.progress),
            (Some(job), None) => (job.status.clone(), false, None),
            (None, None) if inst.is_some() => ("Ready".into(), false, None),
            (None, None) => (String::new(), false, None),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h(px(28.))
            .px_4()
            .bg(t.panel)
            .border_t_1()
            .border_color(t.border)
            .text_xs()
            .text_color(t.muted)
            .child(div().size(px(6.)).rounded_full().bg(if active { t.accent } else { t.subtle }))
            .child(div().flex_1().truncate().child(text))
            .when_some(progress, |d, (done, total)| {
                let ratio = (done as f32 / total as f32).clamp(0., 1.);
                d.child(
                    div()
                        .w(px(160.))
                        .h(px(4.))
                        .rounded_full()
                        .bg(t.border)
                        .child(div().h_full().w(relative(ratio)).rounded_full().bg(t.accent)),
                )
                .child(div().w(px(32.)).text_right().child(format!("{:.0}%", ratio * 100.)))
            })
    }
}

impl Launcher {
    fn modal(&self) -> Option<AnyView> {
        let edit = self.edit_dialog.clone().map(AnyView::from);
        edit.or_else(|| self.add_dialog.clone().map(AnyView::from))
            .or_else(|| self.accounts_dialog.clone().map(AnyView::from))
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &NewInstance, window, cx| this.open_add_dialog(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)))
            .on_action(
                cx.listener(|this, _: &FocusSearch, window, cx| window.focus(&this.search.focus_handle(cx))),
            )
            .on_drop(
                cx.listener(|this, paths: &ExternalPaths, _, cx| this.import(paths.paths().to_vec(), cx)),
            )
            .map(|d| match self.settings_page.clone() {
                Some(page) => {
                    let busy = self.busy();
                    if page.read(cx).busy() != busy {
                        page.update(cx, |p, cx| p.set_busy(busy, cx));
                    }
                    d.child(div().flex_1().min_h_0().child(page))
                }
                None => d.child(self.toolbar(t, cx)).child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(self.grid(t, window, cx))
                        .child(self.sidebar(t, cx)),
                ),
            })
            .child(self.status_bar(t))
            .when_some(self.modal(), |d, dialog| {
                d.child(
                    div()
                        .id("modal")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_6()
                        .bg(gpui::black().opacity(0.35))
                        .occlude()
                        .child(dialog),
                )
            })
    }
}

/// Final status line of an import job.
#[derive(Default)]
struct Summary {
    names: Vec<String>,
    blocked: usize,
}

impl Summary {
    fn add(&mut self, imported: &Imported) {
        self.names.push(imported.instance.name.clone());
        self.blocked += imported.blocked.len();
    }

    fn status(&self) -> String {
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

const TILE_TEXT_WIDTH: f32 = TILE_WIDTH - 2. * 8. - 2.;

/// `text` wrapped to the tile width and cut to `max_lines`, the last line ending with "…".
/// gpui's `line_clamp` only estimates this and lets a long last line overflow.
fn clamp_lines(text: &str, font: Font, size: Pixels, max_lines: usize, cx: &App) -> Vec<SharedString> {
    let width = px(TILE_TEXT_WIDTH);
    let mut wrapper = cx.text_system().line_wrapper(font.clone(), size);
    let fragments = [LineFragment::text(text)];
    let mut starts = vec![0];
    starts.extend(wrapper.wrap_line(&fragments, width).map(|b| b.ix));
    let line = |range: std::ops::Range<usize>| SharedString::from(text[range].trim().to_string());
    if starts.len() <= max_lines {
        starts.push(text.len());
        return starts.windows(2).map(|w| line(w[0]..w[1])).collect();
    }
    let mut lines: Vec<SharedString> =
        starts.windows(2).take(max_lines - 1).map(|w| line(w[0]..w[1])).collect();
    let rest = text[starts[max_lines - 1]..].trim();
    let mut runs = vec![TextRun {
        len: rest.len(),
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    lines.push(wrapper.truncate_line(rest.to_string().into(), width, "…", &mut runs));
    lines
}

/// `1.21.1 · Fabric`, without the loader version.
fn short_description(inst: &Instance) -> String {
    match inst.loader {
        Loader::Vanilla => inst.minecraft.clone(),
        loader => format!("{} · {}", inst.minecraft, loader.label()),
    }
}

fn last_played(ts: u64) -> String {
    if ts == 0 {
        return "Never".into();
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(ts);
    match now.saturating_sub(ts) / 86400 {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        days => format!("{days} days ago"),
    }
}

/// The instance's own icon, or a placeholder. A running instance gets an accent border.
fn instance_icon(size: f32, running: bool, icon: Option<&Path>, t: Theme) -> impl IntoElement {
    let frame = div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.24))
        .bg(t.tile)
        .border_1()
        .border_color(if running { t.accent } else { t.tile_edge })
        .overflow_hidden();
    match icon {
        Some(path) => frame.child(img(path.to_path_buf()).size_full()),
        None => frame.child(svg().path("icons/box.svg").size(px(size * 0.42)).text_color(if running {
            t.accent
        } else {
            t.subtle
        })),
    }
}

fn detail(label: &'static str, value: String, t: Theme) -> impl IntoElement {
    div()
        .flex()
        .justify_between()
        .gap_2()
        .text_xs()
        .child(div().text_color(t.muted).child(label))
        .child(div().text_color(t.text).truncate().child(value))
}

/// A row in the sidebar's action list.
fn menu_item(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    enabled: bool,
    danger: bool,
    t: Theme,
) -> gpui::Stateful<gpui::Div> {
    let color = match (enabled, danger) {
        (false, _) => t.subtle,
        (true, true) => t.danger,
        (true, false) => t.text,
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2p5()
        .h(px(30.))
        .px_2()
        .rounded_md()
        .text_sm()
        .text_color(color)
        .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(t.hover)))
        .child(svg().path(icon).size(px(15.)).text_color(if enabled && !danger { t.muted } else { color }))
        .child(label)
}

fn toolbar_button(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    enabled: bool,
    t: Theme,
) -> gpui::Stateful<gpui::Div> {
    let color = if enabled { t.text } else { t.subtle };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .px_2p5()
        .py_1()
        .rounded_md()
        .text_sm()
        .text_color(color)
        .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(t.hover)))
        .child(svg().path(icon).size(px(15.)).text_color(color))
        .child(label)
}

fn action_button(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    enabled: bool,
    primary: bool,
    t: Theme,
) -> gpui::Stateful<gpui::Div> {
    let (bg, fg, border) = match (enabled, primary) {
        (true, true) => (t.accent, t.on_accent, t.accent),
        (true, false) => (t.bg, t.text, t.border),
        (false, _) => (gpui::transparent_black(), t.subtle, t.border),
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .h(px(34.))
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .when(enabled, |d| d.cursor_pointer().hover(|d| d.opacity(0.88)))
        .child(svg().path(icon).size(px(14.)).text_color(fg))
        .child(label)
}
