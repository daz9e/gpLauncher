//! Main window: toolbar, instance grid, sidebar with actions for the selected instance, status bar.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::*, px, relative, svg,
};
use gplauncher::auth::Account;
use gplauncher::instance::{self, Instance, Loader};
use gplauncher::launch::{self, GameHandle};
use gplauncher::settings::Settings;
use gplauncher::{Event, Reporter, version};

use crate::theme::Theme;

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

pub struct Launcher {
    settings: Settings,
    instances: Vec<Instance>,
    selected: Option<String>,
    sessions: HashMap<String, Session>,
}

impl Launcher {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, _, cx| cx.notify()).detach();
        let settings = Settings::load();
        let instances = instance::list(&settings.data_dir);
        let selected = instances.first().map(|i| i.id.clone());
        Launcher { settings, instances, selected, sessions: HashMap::new() }
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
            Session { phase: Phase::Preparing, handle: handle.clone(), status: "Starting".into(), progress: None },
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
            .child(toolbar_button("add", "icons/plus.svg", "Add Instance", false, t))
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

    fn grid(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
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
                .child(div().text_sm().text_color(t.muted).child("Import a modpack with `gplauncher import FILE`"))
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
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child("Instances"))
                    .child(div().text_xs().text_color(t.subtle).child(self.instances.len().to_string())),
            )
            .child(div().flex().flex_wrap().gap_2().children(self.instances.iter().map(|inst| {
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
                    .when(!selected, |d| d.border_color(gpui::transparent_black()).hover(|d| d.bg(t.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(id.clone());
                        cx.notify();
                    }))
                    .child(instance_icon(64., running, t))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_0p5()
                            .child(
                                div()
                                    .w_full()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(t.text)
                                    .text_center()
                                    .line_clamp(2)
                                    .child(inst.name.clone()),
                            )
                            .child(div().text_xs().text_color(t.muted).child(short_description(inst))),
                    )
            })))
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
                        .child(instance_icon(80., running, t))
                        .child(
                            div()
                                .w_full()
                                .text_center()
                                .text_color(t.text)
                                .font_weight(FontWeight::SEMIBOLD)
                                .line_clamp(2)
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
                        .child(action_button("launch", "icons/play.svg", "Launch", can_launch, true, t).when(
                            can_launch,
                            |b| b.on_click(cx.listener(|this, _, _, cx| this.launch(cx))),
                        ))
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
            })
    }

    fn status_bar(&self, t: Theme) -> impl IntoElement {
        let inst = self.selected();
        let session = inst.and_then(|i| self.sessions.get(&i.id));
        let text = match (inst, session) {
            (_, Some(s)) => s.status.clone(),
            (Some(_), None) => "Ready".into(),
            (None, None) => String::new(),
        };
        let active = session.is_some_and(|s| s.phase != Phase::Finished);
        let progress = session.and_then(|s| s.progress);
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
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .child(self.toolbar(t, cx))
            .child(div().flex_1().min_h_0().flex().child(self.grid(t, cx)).child(self.sidebar(t, cx)))
            .child(self.status_bar(t))
    }
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

/// Placeholder until instances get their own icons.
fn instance_icon(size: f32, running: bool, t: Theme) -> impl IntoElement {
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.24))
        .bg(t.tile)
        .border_1()
        .border_color(if running { t.accent } else { t.tile_edge })
        .child(svg().path("icons/box.svg").size(px(size * 0.42)).text_color(if running { t.accent } else { t.subtle }))
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
