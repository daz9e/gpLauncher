//! A window for one instance: its console, mods and packs, worlds, screenshots, log files and
//! settings. One window per instance; opening it again brings the existing one to the front.

use std::collections::HashMap;

use gplauncher::content::Kind;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Global,
    KeyBinding, SharedString, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, actions,
    div, img, prelude::*, px, size, svg,
};

use crate::content_page::{self, ContentEvent, ContentPage};
use crate::instance_settings::InstanceSettings;
use crate::log_view::{LogView, Source};
use crate::pages::{LogFilesPage, ScreenshotsPage, WorldsPage};
use crate::state::{Phase, State};
use crate::theme::Theme;
use crate::ui::{self, Style};

actions!(instance_window, [CloseWindow]);

const CONTEXT: &str = "InstanceWindow";
const NAV_WIDTH: f32 = 220.;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-w", CloseWindow, Some(CONTEXT))]);
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Page {
    Console,
    Content(Kind),
    Worlds,
    Screenshots,
    LogFiles,
    Settings,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Console => "Console",
            Page::Content(kind) => kind.label(),
            Page::Worlds => "Worlds",
            Page::Screenshots => "Screenshots",
            Page::LogFiles => "Log files",
            Page::Settings => "Settings",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Page::Console => "icons/terminal.svg",
            Page::Content(kind) => content_page::kind_icon(kind),
            Page::Worlds => "icons/earth.svg",
            Page::Screenshots => "icons/image.svg",
            Page::LogFiles => "icons/file-text.svg",
            Page::Settings => "icons/settings.svg",
        }
    }
}

const NAV: [Page; 8] = [
    Page::Console,
    Page::Content(Kind::Mods),
    Page::Content(Kind::ResourcePacks),
    Page::Content(Kind::ShaderPacks),
    Page::Worlds,
    Page::Screenshots,
    Page::LogFiles,
    Page::Settings,
];

/// Open instance windows by instance id.
#[derive(Default)]
struct Windows(HashMap<String, WindowHandle<InstanceWindow>>);

impl Global for Windows {}

/// Shows the window of instance `id` on `page`, opening it if needed.
pub fn open(state: &State, id: &str, page: Page, cx: &mut App) {
    if let Some(handle) = cx.default_global::<Windows>().0.get(id).copied() {
        let shown = handle.update(cx, |view, window, cx| {
            view.show(page, window, cx);
            window.activate_window();
        });
        if shown.is_ok() {
            return;
        }
    }
    let name = state.read(cx).instance(id).map(|i| i.name.clone()).unwrap_or_default();
    let bounds = Bounds::centered(None, size(px(1000.), px(680.)), cx);
    let (state, key) = (state.clone(), id.to_string());
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(name.into()), ..Default::default() }),
            window_min_size: Some(size(px(720.), px(460.))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| InstanceWindow::new(state, key.clone(), page, window, cx)),
    );
    if let Ok(handle) = handle {
        cx.default_global::<Windows>().0.insert(id.to_string(), handle);
    }
}

/// Opens `<instance id>:<page>[:browse]`, for trying pages out during development.
#[cfg(debug_assertions)]
pub fn open_debug(state: &State, spec: &str, cx: &mut App) {
    let mut parts = spec.split(':');
    let (Some(id), page) = (parts.next(), parts.next().unwrap_or("console")) else { return };
    let page = match page {
        "mods" => Page::Content(Kind::Mods),
        "resourcepacks" => Page::Content(Kind::ResourcePacks),
        "shaderpacks" => Page::Content(Kind::ShaderPacks),
        "worlds" => Page::Worlds,
        "screenshots" => Page::Screenshots,
        "logs" => Page::LogFiles,
        "settings" => Page::Settings,
        _ => Page::Console,
    };
    open(state, id, page, cx);
    if parts.next() == Some("browse")
        && let Some(handle) = cx.default_global::<Windows>().0.get(id).copied()
    {
        let _ = handle.update(cx, |view, window, cx| {
            if let Page::Content(kind) = page
                && let Some(content) = view.content.get(&kind)
            {
                content.update(cx, |c, cx| c.browse(window, cx));
            }
        });
    }
}

/// Closes the window of instance `id`, e.g. after it was deleted.
pub fn close(id: &str, cx: &mut App) {
    if let Some(handle) = cx.default_global::<Windows>().0.remove(id) {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

pub struct InstanceWindow {
    state: State,
    id: String,
    page: Page,
    focus_handle: FocusHandle,
    console: Entity<LogView>,
    content: HashMap<Kind, Entity<ContentPage>>,
    worlds: Option<Entity<WorldsPage>>,
    screenshots: Option<Entity<ScreenshotsPage>>,
    log_files: Option<Entity<LogFilesPage>>,
    settings: Option<Entity<InstanceSettings>>,
    title: String,
}

impl Focusable for InstanceWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl InstanceWindow {
    fn new(state: State, id: String, page: Page, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, _, cx| cx.notify()).detach();
        cx.observe(&state, |this, state, cx| {
            // The instance was deleted or the launcher folder changed.
            if state.read(cx).instance(&this.id).is_none() {
                let id = this.id.clone();
                cx.defer(move |cx| close(&id, cx));
            }
            cx.notify();
        })
        .detach();
        // Files change while the game runs or in other apps: look again when the window is used.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.reload_page(cx);
            }
        })
        .detach();
        // The play timer ticks while the game runs.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(20)).await;
                let alive = this.update(cx, |this, cx| {
                    if this.state.read(cx).sessions.get(&this.id).is_some_and(|s| s.phase == Phase::Running) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        let console = cx.new(|cx| {
            LogView::new(
                Source::Session { state: state.clone(), id: id.clone() },
                ("No output yet", "Press Play: the game's output appears here while it runs."),
                cx,
            )
        });
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let mut this = InstanceWindow {
            state,
            id,
            page,
            focus_handle,
            console,
            content: HashMap::new(),
            worlds: None,
            screenshots: None,
            log_files: None,
            settings: None,
            title: String::new(),
        };
        this.ensure_page(page, window, cx);
        this
    }

    fn show(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.ensure_page(page, window, cx);
        self.reload_page(cx);
        cx.notify();
    }

    /// Creates the view of `page` on first use.
    fn ensure_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        let (state, id) = (self.state.clone(), self.id.clone());
        match page {
            Page::Content(kind) if !self.content.contains_key(&kind) => {
                let view = cx.new(|cx| ContentPage::new(state, id, kind, cx));
                cx.subscribe_in(&view, window, |this, _, e: &ContentEvent, window, cx| match e {
                    ContentEvent::OpenSettings => this.show(Page::Settings, window, cx),
                })
                .detach();
                self.content.insert(kind, view);
            }
            Page::Worlds if self.worlds.is_none() => {
                self.worlds = Some(cx.new(|cx| WorldsPage::new(state, id, cx)))
            }
            Page::Screenshots if self.screenshots.is_none() => {
                self.screenshots = Some(cx.new(|cx| ScreenshotsPage::new(state, id, cx)))
            }
            Page::LogFiles if self.log_files.is_none() => {
                self.log_files = Some(cx.new(|cx| LogFilesPage::new(state, id, cx)))
            }
            Page::Settings if self.settings.is_none() => {
                self.settings = Some(cx.new(|cx| InstanceSettings::new(state, id, cx)))
            }
            _ => {}
        }
    }

    fn reload_page(&mut self, cx: &mut Context<Self>) {
        match self.page {
            Page::Content(kind) => {
                if let Some(v) = self.content.get(&kind) {
                    v.update(cx, |v, cx| v.reload(cx));
                }
            }
            Page::Worlds => {
                if let Some(v) = &self.worlds {
                    v.update(cx, |v, cx| v.reload(cx));
                }
            }
            Page::Screenshots => {
                if let Some(v) = &self.screenshots {
                    v.update(cx, |v, cx| v.reload(cx));
                }
            }
            Page::LogFiles => {
                if let Some(v) = &self.log_files {
                    v.update(cx, |v, cx| v.reload(cx));
                }
            }
            Page::Console | Page::Settings => {}
        }
    }

    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.id.clone();
        self.state.update(cx, |s, cx| s.launch(&id, cx));
        self.show(Page::Console, window, cx);
    }

    fn kill(&mut self, cx: &mut Context<Self>) {
        let id = self.id.clone();
        self.state.update(cx, |s, cx| s.kill(&id, cx));
    }

    // ---- rendering ---------------------------------------------------------------

    fn nav(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let Some(inst) = state.instance(&self.id) else { return div().into_any_element() };
        let session = state.sessions.get(&self.id);
        let phase = session.map(|s| s.phase);
        let icon = div()
            .size(px(44.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .overflow_hidden()
            .bg(t.tile)
            .border_1()
            .border_color(if phase.is_some_and(|p| p != Phase::Finished) { t.success } else { t.tile_edge })
            .map(|d| match &inst.icon {
                Some(p) => d.child(img(p.clone()).size_full()),
                None => d.child(svg().path("icons/box.svg").size(px(20.)).text_color(t.subtle)),
            });

        let play = match phase {
            Some(Phase::Running) => ui::button("stop", Some("icons/stop.svg"), "Stop", Style::Stop, true, t)
                .w_full()
                .h(px(34.))
                .on_click(cx.listener(|this, _, _, cx| this.kill(cx))),
            Some(Phase::Preparing) => {
                ui::button("preparing", None, "Starting…", Style::Secondary, false, t).w_full().h(px(34.))
            }
            _ => ui::button("play", Some("icons/play.svg"), "Play", Style::Play, true, t)
                .w_full()
                .h(px(34.))
                .on_click(cx.listener(|this, _, window, cx| this.launch(window, cx))),
        };
        let status = session.map(|s| {
            let text = match (s.phase, s.played) {
                (Phase::Running, _) => format!("Playing for {}", ui::duration(s.started.elapsed())),
                (Phase::Finished, Some(played)) if !s.status.starts_with("Error") => {
                    format!("{} · played {}", s.status, ui::duration(played))
                }
                _ => s.status.clone(),
            };
            let error = s.status.starts_with("Error") || s.status.starts_with("Game crashed");
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(if error { t.danger } else { t.muted })
                        .line_clamp(2)
                        .child(text),
                )
                .when_some(s.progress, |d, (done, total)| {
                    d.child(ui::progress_bar(done as f32 / total as f32, t))
                })
        });

        let running = phase == Some(Phase::Running);
        let dir = inst.game_dir.clone();
        div()
            .w(px(NAV_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.panel)
            .border_r_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .pb_4()
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div().flex().items_center().gap_2p5().child(icon).child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(t.text)
                                        .line_clamp(2)
                                        .child(inst.name.clone()),
                                )
                                .child(
                                    div().truncate().text_xs().text_color(t.muted).child(inst.description()),
                                ),
                        ),
                    )
                    .child(play)
                    .children(status),
            )
            .child(
                div()
                    .id("nav-items")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .p_2()
                    .children(NAV.map(|page| {
                        let active = self.page == page;
                        let live = page == Page::Console && running;
                        div()
                            .id(SharedString::from(page.label()))
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .h(px(30.))
                            .px_2()
                            .rounded_md()
                            .text_sm()
                            .cursor_pointer()
                            .when(active, |d| {
                                d.bg(t.accent_soft).text_color(t.accent).font_weight(FontWeight::MEDIUM)
                            })
                            .when(!active, |d| d.text_color(t.text).hover(|d| d.bg(t.hover)))
                            .on_click(cx.listener(move |this, _, window, cx| this.show(page, window, cx)))
                            .child(svg().path(page.icon()).size(px(15.)).text_color(if active {
                                t.accent
                            } else {
                                t.muted
                            }))
                            .child(div().flex_1().child(page.label()))
                            .when(live, |d| d.child(div().size(px(7.)).rounded_full().bg(t.success)))
                    })),
            )
            .child(
                div().p_2().border_t_1().border_color(t.border).child(
                    ui::button(
                        "open-instance-folder",
                        Some("icons/folder.svg"),
                        "Open game folder",
                        Style::Ghost,
                        true,
                        t,
                    )
                    .w_full()
                    .justify_start()
                    .text_color(t.muted)
                    .on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&dir);
                        cx.open_with_system(&dir);
                    }),
                ),
            )
            .into_any_element()
    }

    fn body(&self) -> AnyElement {
        match self.page {
            Page::Console => Some(self.console.clone().into_any_element()),
            Page::Content(kind) => self.content.get(&kind).map(|v| v.clone().into_any_element()),
            Page::Worlds => self.worlds.clone().map(|v| v.into_any_element()),
            Page::Screenshots => self.screenshots.clone().map(|v| v.into_any_element()),
            Page::LogFiles => self.log_files.clone().map(|v| v.into_any_element()),
            Page::Settings => self.settings.clone().map(|v| v.into_any_element()),
        }
        .unwrap_or_else(|| div().into_any_element())
    }
}

impl Render for InstanceWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let name = self.state.read(cx).instance(&self.id).map(|i| i.name.clone()).unwrap_or_default();
        if name != self.title {
            window.set_window_title(&name);
            self.title = name;
        }
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .size_full()
            .flex()
            .bg(t.bg)
            .text_color(t.text)
            .child(self.nav(t, cx))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().child(self.body()))
    }
}
