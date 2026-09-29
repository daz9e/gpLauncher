//! Main window: toolbar, instance grid, sidebar with actions for the selected instance, status bar.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use futures::channel::mpsc;
use gplauncher::auth::Account;
use gplauncher::import::{self, Imported};
use gplauncher::instance::{self, Instance, Loader};
use gplauncher::launch::{self, GameHandle};
use gplauncher::settings::Settings;
use gplauncher::{Event, Reporter, modpack, version};
use gpui::{
    AnyElement, App, Context, Entity, ExternalPaths, FocusHandle, Font, FontWeight, IntoElement,
    LineFragment, ParentElement, Pixels, Render, SharedString, Styled, TextRun, Window, actions, div, img,
    prelude::*, px, relative, rems, svg,
};

use crate::add_instance::{self, AddInstance, AddInstanceEvent};
use crate::theme::Theme;

actions!(launcher, [NewInstance]);

pub const TOOLBAR_HEIGHT: f32 = 52.;
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
    add_dialog: Option<Entity<AddInstance>>,
    job: Option<Job>,
    /// Instances with files that have to be downloaded by hand (see [`import::BLOCKED_LIST`]).
    manual_downloads: HashSet<String>,
}

impl Launcher {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, _, cx| cx.notify()).detach();
        let settings = Settings::load();
        let instances = instance::list(&settings.data_dir);
        let selected = instances.first().map(|i| i.id.clone());
        let manual_downloads = instances
            .iter()
            .filter(|i| i.dir.join(import::BLOCKED_LIST).is_file())
            .map(|i| i.id.clone())
            .collect();
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        Launcher {
            focus_handle,
            settings,
            instances,
            selected,
            sessions: HashMap::new(),
            add_dialog: None,
            job: None,
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

    /// Puts a new instance at the front of the grid and selects it.
    fn add_instance(&mut self, inst: Instance, cx: &mut Context<Self>) {
        self.instances.retain(|i| i.id != inst.id);
        let id = inst.id.clone();
        self.instances.insert(0, inst);
        self.select(id, cx);
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

    fn launch(&mut self, cx: &mut Context<Self>) {
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

        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let alive = this.update(cx, |this, cx| {
                    this.apply(&id, msg);
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

    fn apply(&mut self, id: &str, msg: Msg) {
        match msg {
            Msg::Event(Event::AccountRefreshed(account)) => {
                if let Some(slot) = self.settings.accounts.get_mut(self.settings.selected_account) {
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
                    }
                    Msg::Event(Event::GameExited(code)) => {
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
            .id("toolbar")
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(TOOLBAR_HEIGHT))
            // Room for the traffic lights, which sit inside the toolbar.
            .pl(px(84.))
            .pr_3()
            .bg(t.panel)
            .border_b_1()
            .border_color(t.border)
            .on_click(|e, window, _| {
                if e.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(
                toolbar_button("add", "icons/plus.svg", "Add Instance", true, t)
                    .on_click(cx.listener(|this, _, window, cx| this.open_add_dialog(window, cx))),
            )
            .child(toolbar_button("folders", "icons/folder.svg", "Folders", true, t).on_click(cx.listener(
                move |_, _, _, cx| {
                    let _ = std::fs::create_dir_all(&data_dir);
                    cx.open_with_system(&data_dir);
                },
            )))
            .child(toolbar_button("settings", "icons/settings.svg", "Settings", false, t))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl_1()
                    .pr_3()
                    .py_1()
                    .rounded_full()
                    .border_1()
                    .border_color(t.border)
                    .bg(t.bg)
                    .child(avatar(&account, 22., t))
                    .child(div().text_sm().text_color(t.text).child(account)),
            )
    }

    fn grid(&self, t: Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut name_font = window.text_style().font();
        name_font.weight = FontWeight::MEDIUM;
        let name_size = rems(0.875).to_pixels(window.rem_size());
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
        div()
            .id("instances")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .px_5()
            .py_4()
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
                    .child(div().text_xs().text_color(t.subtle).child(self.instances.len().to_string())),
            )
            // Tiles keep their own height: the selected one grows with its full name.
            .child(div().flex().flex_wrap().items_start().gap_2().children(self.instances.iter().map(
                |inst| {
                    let selected = self.selected.as_deref() == Some(inst.id.as_str());
                    let running = self.sessions.get(&inst.id).is_some_and(|s| s.phase != Phase::Finished);
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
                        .when(!selected, |d| {
                            d.border_color(gpui::transparent_black()).hover(|d| d.bg(t.hover))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
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
                                                clamp_lines(&inst.name, name_font.clone(), name_size, 2, cx)
                                                    .into_iter()
                                                    .map(|line| div().whitespace_nowrap().child(line)),
                                            )
                                        }),
                                )
                                .child(div().text_xs().text_color(t.muted).child(short_description(inst))),
                        )
                },
            )))
            .into_any_element()
    }

    fn sidebar(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (can_launch, can_kill) = (self.can_launch(), self.can_kill());
        let memory = self.selected().and_then(|i| i.memory_mb).unwrap_or(self.settings.memory_mb);
        div()
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .bg(t.panel)
            .border_l_1()
            .border_color(t.border)
            .when_some(self.selected(), |d, inst| {
                let running = self.sessions.get(&inst.id).is_some_and(|s| s.phase != Phase::Finished);
                let loader = match inst.loader {
                    Loader::Vanilla => "None".to_string(),
                    l if inst.loader_version.is_empty() => format!("{} (latest)", l.label()),
                    l => format!("{} {}", l.label(), inst.loader_version),
                };
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .pt_6()
                        .pb_4()
                        .child(instance_icon(80., running, inst.icon.as_deref(), t))
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
                        .flex_col()
                        .gap_2()
                        .px_4()
                        .pb_4()
                        .child(
                            action_button("launch", "icons/play.svg", "Launch", can_launch, true, t)
                                .when(can_launch, |b| {
                                    b.on_click(cx.listener(|this, _, _, cx| this.launch(cx)))
                                }),
                        )
                        .child(
                            action_button("kill", "icons/stop.svg", "Kill", can_kill, false, t)
                                .when(can_kill, |b| b.on_click(cx.listener(|this, _, _, cx| this.kill(cx)))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .mx_4()
                        .pt_4()
                        .border_t_1()
                        .border_color(t.border)
                        .child(detail("Minecraft", inst.minecraft.clone(), t))
                        .child(detail("Loader", loader, t))
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
                            .m_4()
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
            .on_drop(
                cx.listener(|this, paths: &ExternalPaths, _, cx| this.import(paths.paths().to_vec(), cx)),
            )
            .child(self.toolbar(t, cx))
            .child(div().flex_1().min_h_0().flex().child(self.grid(t, window, cx)).child(self.sidebar(t, cx)))
            .child(self.status_bar(t))
            .when_some(self.add_dialog.clone(), |d, dialog| {
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

fn avatar(name: &str, size: f32, t: Theme) -> impl IntoElement {
    let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    div()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(t.tile)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.muted)
        .child(initial)
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
