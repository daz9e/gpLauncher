//! Instance pages that show files the game made: worlds, screenshots and log files.

use std::path::PathBuf;

use gplauncher::content::{self, World};
use gplauncher::instance::Instance;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, ObjectFit, PromptLevel, SharedString, StyledImage, Window,
    div, img, prelude::*, px, svg,
};

use crate::log_view::{LogView, Source};
use crate::state::{self, State};
use crate::theme::Theme;
use crate::ui::{self, Style, tooltip};

fn instance(state: &State, id: &str, cx: &App) -> Option<Instance> {
    state.read(cx).instance(id).cloned()
}

fn page_toolbar(title: impl Into<SharedString>, detail: impl Into<SharedString>, t: Theme) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .py_2()
        .min_h(px(46.))
        .border_b_1()
        .border_color(t.border)
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(t.text).child(title.into()))
        .child(div().text_xs().text_color(t.subtle).child(detail.into()))
        .child(div().flex_1())
}

// ---- worlds ---------------------------------------------------------------------------

pub struct WorldsPage {
    state: State,
    id: String,
    worlds: Option<Vec<World>>,
    sizes: Vec<(PathBuf, u64)>,
}

impl WorldsPage {
    pub fn new(state: State, id: String, cx: &mut Context<Self>) -> Self {
        let mut this = WorldsPage { state, id, worlds: None, sizes: Vec::new() };
        this.reload(cx);
        this
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = instance(&self.state, &self.id, cx) else { return };
        cx.spawn(async move |this, cx| {
            let worlds = cx.background_spawn(async move { content::worlds(&inst) }).await;
            let paths: Vec<PathBuf> = worlds.iter().map(|w| w.path.clone()).collect();
            let _ = this.update(cx, |this, cx| {
                this.worlds = Some(worlds);
                cx.notify();
            });
            let sizes = cx
                .background_spawn(async move {
                    paths.into_iter().map(|p| (content::dir_size(&p), p)).map(|(s, p)| (p, s)).collect()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.sizes = sizes;
                cx.notify();
            });
        })
        .detach();
    }

    fn delete(&mut self, world: World, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Delete the world \"{}\"?", world.name),
            Some("The world folder is deleted permanently. This can not be undone."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = std::fs::remove_dir_all(&world.path);
            let _ = this.update(cx, |this, cx| this.reload(cx));
        })
        .detach();
    }
}

impl Render for WorldsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let saves = instance(&self.state, &self.id, cx).map(|i| i.game_dir.join("saves"));
        let running = self.state.read(cx).is_running(&self.id);
        let count = self.worlds.as_ref().map_or(0, Vec::len);
        let toolbar = page_toolbar("Worlds", if count > 0 { format!("{count}") } else { String::new() }, t)
            .when_some(saves, |d, dir| {
                d.child(
                    ui::button("open-saves", Some("icons/folder.svg"), "Open folder", Style::Ghost, true, t)
                        .on_click(move |_, _, cx| {
                            let _ = std::fs::create_dir_all(&dir);
                            cx.open_with_system(&dir);
                        }),
                )
            });
        let body: AnyElement = match &self.worlds {
            None => div().into_any_element(),
            Some(list) if list.is_empty() => ui::empty_state(
                "icons/earth.svg",
                "No worlds yet",
                "Worlds you create in the game appear here. Drop a world folder into the saves folder to add one.",
                t,
            )
            .into_any_element(),
            Some(list) => div()
                .id("worlds")
                .size_full()
                .overflow_y_scroll()
                .children(list.iter().enumerate().map(|(ix, w)| {
                    let size = self.sizes.iter().find(|(p, _)| *p == w.path).map(|(_, s)| content::human_size(*s));
                    let detail = [w.game_mode.map(String::from), Some(format!("Played {}", ui::ago(w.last_played).to_lowercase())), size]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ");
                    let (dir, world) = (w.path.clone(), w.clone());
                    div()
                        .id(ix)
                        .group("world")
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .py_2p5()
                        .border_b_1()
                        .border_color(t.border.opacity(0.5))
                        .hover(|d| d.bg(t.hover))
                        .child(
                            div()
                                .size(px(44.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_md()
                                .overflow_hidden()
                                .bg(t.tile)
                                .map(|d| match &w.icon {
                                    Some(icon) => d.child(img(icon.clone()).size_full()),
                                    None => d.child(svg().path("icons/earth.svg").size(px(20.)).text_color(t.subtle)),
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .child(div().truncate().text_sm().font_weight(FontWeight::MEDIUM).text_color(t.text).child(w.name.clone()))
                                .child(div().truncate().text_xs().text_color(t.muted).child(detail))
                                .when(w.name != w.folder, |d| d.child(div().truncate().text_xs().text_color(t.subtle).child(w.folder.clone()))),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_0p5()
                                .invisible()
                                .group_hover("world", |d| d.visible())
                                .child(
                                    ui::icon_button(SharedString::from(format!("wf-{ix}")), "icons/folder.svg", true, t)
                                        .tooltip(tooltip("Open folder"))
                                        .on_click(move |_, _, cx| cx.open_with_system(&dir)),
                                )
                                .child(
                                    ui::icon_button(SharedString::from(format!("wd-{ix}")), "icons/trash.svg", !running, t)
                                        .tooltip(tooltip(if running { "Close the game to delete worlds" } else { "Delete" }))
                                        .when(!running, |b| {
                                            b.on_click(cx.listener(move |this, _, window, cx| this.delete(world.clone(), window, cx)))
                                        }),
                                ),
                        )
                }))
                .into_any_element(),
        };
        div().size_full().flex().flex_col().child(toolbar).child(div().flex_1().min_h_0().child(body))
    }
}

// ---- screenshots ------------------------------------------------------------------------

pub struct ScreenshotsPage {
    state: State,
    id: String,
    shots: Option<Vec<PathBuf>>,
}

impl ScreenshotsPage {
    pub fn new(state: State, id: String, cx: &mut Context<Self>) -> Self {
        let mut this = ScreenshotsPage { state, id, shots: None };
        this.reload(cx);
        this
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = instance(&self.state, &self.id, cx) else { return };
        cx.spawn(async move |this, cx| {
            let shots = cx.background_spawn(async move { content::screenshots(&inst) }).await;
            let _ = this.update(cx, |this, cx| {
                this.shots = Some(shots);
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for ScreenshotsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let dir = instance(&self.state, &self.id, cx).map(|i| i.game_dir.join("screenshots"));
        let count = self.shots.as_ref().map_or(0, Vec::len);
        let toolbar =
            page_toolbar("Screenshots", if count > 0 { count.to_string() } else { String::new() }, t)
                .when_some(dir, |d, dir| {
                    d.child(
                        ui::button(
                            "open-shots",
                            Some("icons/folder.svg"),
                            "Open folder",
                            Style::Ghost,
                            true,
                            t,
                        )
                        .on_click(move |_, _, cx| {
                            let _ = std::fs::create_dir_all(&dir);
                            cx.open_with_system(&dir);
                        }),
                    )
                });
        let body: AnyElement = match &self.shots {
            None => div().into_any_element(),
            Some(list) if list.is_empty() => ui::empty_state(
                "icons/image.svg",
                "No screenshots yet",
                "Press F2 in the game to take one.",
                t,
            )
            .into_any_element(),
            Some(list) => div()
                .id("shots")
                .size_full()
                .overflow_y_scroll()
                .p_4()
                .child(div().flex().flex_wrap().gap_3().children(list.iter().enumerate().map(
                    |(ix, path)| {
                        let open = path.clone();
                        let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                        div()
                            .id(ix)
                            .w(px(200.))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .cursor_pointer()
                            .on_click(move |_, _, cx| cx.open_with_system(&open))
                            .child(
                                div()
                                    .w(px(200.))
                                    .h(px(112.))
                                    .rounded_md()
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(t.border)
                                    .bg(t.tile)
                                    .hover(|d| d.border_color(t.accent))
                                    .child(img(path.clone()).size_full().object_fit(ObjectFit::Cover)),
                            )
                            .child(div().truncate().text_xs().text_color(t.muted).child(name))
                    },
                )))
                .into_any_element(),
        };
        div().size_full().flex().flex_col().child(toolbar).child(div().flex_1().min_h_0().child(body))
    }
}

// ---- log files ------------------------------------------------------------------------

pub struct LogFilesPage {
    state: State,
    id: String,
    files: Option<Vec<PathBuf>>,
    selected: Option<PathBuf>,
    view: Entity<LogView>,
    error: Option<String>,
}

impl LogFilesPage {
    pub fn new(state: State, id: String, cx: &mut Context<Self>) -> Self {
        let view = cx.new(|cx| {
            LogView::new(
                Source::Lines(Default::default()),
                ("Pick a file", "Choose a log or crash report on the left."),
                cx,
            )
        });
        let mut this = LogFilesPage { state, id, files: None, selected: None, view, error: None };
        this.reload(cx);
        this
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = instance(&self.state, &self.id, cx) else { return };
        cx.spawn(async move |this, cx| {
            let files = cx.background_spawn(async move { content::log_files(&inst) }).await;
            let _ = this.update(cx, |this, cx| {
                let first = files.first().cloned();
                this.files = Some(files);
                if this.selected.is_none()
                    && let Some(first) = first
                {
                    this.open(first, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.selected = Some(path.clone());
        self.error = None;
        cx.spawn(async move |this, cx| {
            let text = cx.background_spawn(async move { content::read_log(&path) }).await;
            let _ = this.update(cx, |this, cx| {
                match text {
                    Ok(text) => this.view.update(cx, |v, cx| v.set_lines(state::log_lines(&text), cx)),
                    Err(e) => this.error = Some(format!("{e:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for LogFilesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let files = self.files.clone().unwrap_or_default();
        if self.files.is_some() && files.is_empty() {
            return div().size_full().child(ui::empty_state(
                "icons/file-text.svg",
                "No log files",
                "The game writes its logs and crash reports here once it has run.",
                t,
            ));
        }
        let list = div()
            .id("log-files")
            .w(px(230.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(t.border)
            .py_1()
            .children(files.iter().enumerate().map(|(ix, path)| {
                let selected = self.selected.as_ref() == Some(path);
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                let crash = path.parent().is_some_and(|p| p.ends_with("crash-reports"));
                let modified = std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                let path = path.clone();
                div()
                    .id(ix)
                    .mx_1()
                    .px_2()
                    .py_1p5()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.accent_soft))
                    .when(!selected, |d| d.hover(|d| d.bg(t.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| this.open(path.clone(), cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                svg()
                                    .path(if crash {
                                        "icons/triangle-alert.svg"
                                    } else {
                                        "icons/file-text.svg"
                                    })
                                    .size(px(13.))
                                    .flex_none()
                                    .text_color(if crash { t.danger } else { t.subtle }),
                            )
                            .child(div().truncate().text_sm().text_color(t.text).child(name)),
                    )
                    .child(div().pl(px(19.)).text_xs().text_color(t.subtle).child(ui::ago(modified)))
            }));
        let selected = self.selected.clone();
        div().size_full().flex().child(list).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .when_some(selected, |d, path| {
                    let open = path.clone();
                    d.child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_4()
                            .py_1p5()
                            .border_b_1()
                            .border_color(t.border)
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_xs()
                                    .text_color(t.muted)
                                    .child(path.display().to_string()),
                            )
                            .child(
                                ui::icon_button("open-log-file", "icons/external-link.svg", true, t)
                                    .tooltip(tooltip("Open in another app"))
                                    .on_click(move |_, _, cx| cx.open_with_system(&open)),
                            ),
                    )
                })
                .when_some(self.error.clone(), |d, e| {
                    d.child(div().p_4().text_sm().text_color(t.danger).child(e))
                })
                .child(div().flex_1().min_h_0().child(self.view.clone())),
        )
    }
}
