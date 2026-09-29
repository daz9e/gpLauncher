//! Modrinth search for mods, resource packs or shaders that fit one instance, with the versions
//! of the selected project. Installing is left to the owner: the browser emits [`Install`].

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use gplauncher::content::Kind;
use gplauncher::modpack::{self, Sort};
use gplauncher::modrinth::{self, Project, Version};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, Focusable, FontWeight, ScrollStrategy, SharedString,
    Task, UniformListScrollHandle, Window, div, img, prelude::*, px, svg, uniform_list,
};

use crate::dropdown::Dropdown;
use crate::text_input::{self, TextInput};
use crate::theme::Theme;
use crate::ui::{self, Style};

const PAGE_SIZE: u64 = 30;
const ROW_HEIGHT: f32 = 64.;
const DETAILS_WIDTH: f32 = 280.;
const VERSION_ROW_HEIGHT: f32 = 40.;
const SEARCH_DELAY: Duration = Duration::from_millis(300);

type Lookup<T> = Option<Result<T, String>>;

/// Asks the owner to install a project; `None` = the newest fitting version.
pub struct Install(pub Project, pub Option<Version>);

/// What the owner knows about a project, for the row's button.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProjectState {
    Available,
    Queued,
    Installed,
}

pub struct AddonBrowser {
    kind: Kind,
    data_dir: PathBuf,
    loaders: Vec<&'static str>,
    game_version: Option<String>,
    pub search: Entity<TextInput>,
    projects: Vec<Project>,
    total: u64,
    loading: bool,
    error: Option<String>,
    generation: u64,
    pending_search: Option<Task<()>>,
    sort: Sort,
    sort_open: bool,
    selected: Option<usize>,
    versions: HashMap<String, Lookup<Vec<Version>>>,
    icons: HashMap<String, Option<PathBuf>>,
    installed: HashSet<String>,
    queued: HashSet<String>,
    scroll: UniformListScrollHandle,
    version_scroll: UniformListScrollHandle,
}

impl EventEmitter<Install> for AddonBrowser {}

impl AddonBrowser {
    pub fn new(
        kind: Kind,
        data_dir: PathBuf,
        loaders: Vec<&'static str>,
        game_version: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            TextInput::new(format!("Search {} on Modrinth", kind.label().to_lowercase()), cx)
                .with_icon("icons/search.svg")
        });
        cx.subscribe(&search, |this, _, _: &text_input::Changed, cx| {
            this.pending_search = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SEARCH_DELAY).await;
                let _ = this.update(cx, |this, cx| this.run_search(cx));
            }));
        })
        .detach();
        let mut this = AddonBrowser {
            kind,
            data_dir,
            loaders,
            game_version,
            search,
            projects: Vec::new(),
            total: 0,
            loading: false,
            error: None,
            generation: 0,
            pending_search: None,
            sort: Sort::Relevance,
            sort_open: false,
            selected: None,
            versions: HashMap::new(),
            icons: HashMap::new(),
            installed: HashSet::new(),
            queued: HashSet::new(),
            scroll: UniformListScrollHandle::new(),
            version_scroll: UniformListScrollHandle::new(),
        };
        this.run_search(cx);
        this
    }

    /// Project ids that are installed, and those waiting to be.
    pub fn set_known(&mut self, installed: HashSet<String>, queued: HashSet<String>, cx: &mut Context<Self>) {
        if installed != self.installed || queued != self.queued {
            self.installed = installed;
            self.queued = queued;
            cx.notify();
        }
    }

    fn project_state(&self, id: &str) -> ProjectState {
        if self.installed.contains(id) {
            ProjectState::Installed
        } else if self.queued.contains(id) {
            ProjectState::Queued
        } else {
            ProjectState::Available
        }
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.projects.clear();
        self.total = 0;
        self.selected = None;
        self.fetch(0, cx);
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if !self.loading && self.error.is_none() && (self.projects.len() as u64) < self.total {
            self.fetch(self.projects.len() as u64, cx);
        }
    }

    fn fetch(&mut self, offset: u64, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        let generation = self.generation;
        let query = self.search.read(cx).text().to_string();
        let (kind, sort, loaders, game) =
            (self.kind, self.sort, self.loaders.clone(), self.game_version.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    modrinth::search_projects(
                        kind,
                        &query,
                        game.as_deref(),
                        &loaders,
                        sort,
                        offset,
                        PAGE_SIZE,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(page) => {
                        this.total = page.total;
                        this.projects.extend(page.projects);
                        if this.selected.is_none() && !this.projects.is_empty() {
                            this.select(0, cx);
                        }
                    }
                    Err(e) => this.error = Some(format!("{e:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(project) = self.projects.get(ix) else { return };
        self.selected = Some(ix);
        self.version_scroll.scroll_to_item(0, ScrollStrategy::Top);
        let id = project.id.clone();
        if !self.versions.contains_key(&id) {
            self.versions.insert(id.clone(), None);
            let (loaders, game) = (self.loaders.clone(), self.game_version.clone());
            cx.spawn(async move |this, cx| {
                let project = id.clone();
                let result = cx
                    .background_spawn(async move {
                        modrinth::project_versions(&project, &loaders, game.as_deref())
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.versions.insert(id, Some(result.map_err(|e| format!("{e:#}"))));
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn request_icon(&mut self, url: &str, cx: &mut Context<Self>) {
        if self.icons.contains_key(url) {
            return;
        }
        self.icons.insert(url.to_string(), None);
        let (url, dir) = (url.to_string(), self.data_dir.clone());
        cx.spawn(async move |this, cx| {
            let icon_url = url.clone();
            let path = cx.background_spawn(async move { modpack::icon(&dir, &icon_url) }).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.icons.insert(url, path);
                cx.notify();
            });
        })
        .detach();
    }

    fn install(&mut self, ix: usize, version: Option<Version>, cx: &mut Context<Self>) {
        if let Some(project) = self.projects.get(ix) {
            cx.emit(Install(project.clone(), version));
        }
    }

    // ---- rendering -----------------------------------------------------------

    fn icon(&self, url: Option<&str>, size: f32, t: Theme) -> AnyElement {
        let path = url.and_then(|u| self.icons.get(u).cloned().flatten());
        let frame = div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size * 0.22))
            .overflow_hidden()
            .bg(t.tile);
        match path {
            Some(path) => frame.child(img(path).size_full()).into_any_element(),
            None => frame
                .child(svg().path("icons/puzzle.svg").size(px(size * 0.45)).text_color(t.subtle))
                .into_any_element(),
        }
    }

    fn state_button(
        &self,
        id: impl Into<gpui::ElementId>,
        state: ProjectState,
        t: Theme,
    ) -> gpui::Stateful<gpui::Div> {
        match state {
            ProjectState::Installed => {
                ui::button(id, Some("icons/check.svg"), "Installed", Style::Ghost, false, t)
            }
            ProjectState::Queued => ui::button(id, None, "Queued…", Style::Ghost, false, t),
            ProjectState::Available => {
                ui::button(id, Some("icons/download.svg"), "Install", Style::Secondary, true, t)
            }
        }
    }

    fn list(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.projects.is_empty() {
            return match (&self.error, self.loading) {
                (Some(e), _) => {
                    ui::empty_state("icons/triangle-alert.svg", "Could not search Modrinth", e.clone(), t)
                }
                (None, true) => ui::empty_state("icons/search.svg", "Searching…", "", t),
                (None, false) => ui::empty_state("icons/search.svg", "Nothing found", "Try other words.", t),
            }
            .into_any_element();
        }
        let more = (self.projects.len() as u64) < self.total || self.error.is_some();
        let count = self.projects.len() + usize::from(more);
        uniform_list(
            "addons",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                if range.end >= this.projects.len() {
                    this.load_more(cx);
                }
                let icons: Vec<String> =
                    range.clone().filter_map(|ix| this.projects.get(ix)?.icon_url.clone()).collect();
                for url in icons {
                    this.request_icon(&url, cx);
                }
                range.map(|ix| this.row(ix, t, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .size_full()
        .into_any_element()
    }

    fn row(&self, ix: usize, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(project) = self.projects.get(ix) else {
            let text = match &self.error {
                Some(e) => format!("Could not load more: {e}"),
                None => "Loading more…".into(),
            };
            return div()
                .id(ix)
                .h(px(ROW_HEIGHT))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(t.muted)
                .child(text)
                .into_any_element();
        };
        let selected = self.selected == Some(ix);
        let state = self.project_state(&project.id);
        div()
            .id(ix)
            .h(px(ROW_HEIGHT))
            .w_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .cursor_pointer()
            .border_b_1()
            .border_color(t.border.opacity(0.5))
            .when(selected, |d| d.bg(t.accent_soft))
            .when(!selected, |d| d.hover(|d| d.bg(t.hover)))
            .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
            .child(self.icon(project.icon_url.as_deref(), 40., t))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_1p5()
                            .child(
                                div()
                                    .flex_shrink()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(t.text)
                                    .child(project.title.clone()),
                            )
                            .when(!project.author.is_empty(), |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(t.subtle)
                                        .child(format!("by {}", project.author)),
                                )
                            }),
                    )
                    .child(div().truncate().text_xs().text_color(t.muted).child(one_line(&project.summary)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.subtle)
                            .child(format!("{} downloads", short_number(project.downloads))),
                    ),
            )
            .child(self.state_button(SharedString::from(format!("install-{ix}")), state, t).when(
                state == ProjectState::Available,
                |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.install(ix, None, cx);
                    }))
                },
            ))
            .into_any_element()
    }

    fn details(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(DETAILS_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(t.border)
            .when_some(self.selected.and_then(|ix| Some((ix, self.projects.get(ix)?))), |d, (ix, project)| {
                let url = project.url(self.kind);
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_4()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(self.icon(project.icon_url.as_deref(), 48., t))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(t.text)
                                        .line_clamp(2)
                                        .child(project.title.clone()),
                                ),
                        )
                        .child(div().text_xs().text_color(t.muted).child(project.summary.clone()))
                        .child(
                            div()
                                .id("project-page")
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(t.accent)
                                .cursor_pointer()
                                .hover(|d| d.underline())
                                .on_click(move |_, _, cx| cx.open_url(&url))
                                .child("Open on Modrinth")
                                .child(
                                    svg().path("icons/external-link.svg").size(px(11.)).text_color(t.accent),
                                ),
                        ),
                )
                .child(self.version_list(ix, project, t, cx))
            })
    }

    fn version_list(&self, ix: usize, project: &Project, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let header = div()
            .flex_none()
            .px_4()
            .pt_3()
            .pb_1p5()
            .border_t_1()
            .border_color(t.border)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.muted)
            .child("Versions for this instance");
        let note =
            |text: String| div().px_4().py_2().text_xs().text_color(t.muted).child(text).into_any_element();
        let state = self.project_state(&project.id);
        let body = match self.versions.get(&project.id) {
            None | Some(None) => note("Loading…".into()),
            Some(Some(Err(e))) => note(format!("Could not load versions: {e}")),
            Some(Some(Ok(v))) if v.is_empty() => {
                note("No version fits this instance's Minecraft version and loader.".into())
            }
            Some(Some(Ok(v))) => {
                let count = v.len();
                uniform_list(
                    "addon-versions",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                        let t = Theme::for_appearance(window.appearance());
                        let Some(project) = this.projects.get(ix) else { return Vec::new() };
                        let Some(Some(Ok(versions))) = this.versions.get(&project.id) else {
                            return Vec::new();
                        };
                        range
                            .filter_map(|vix| {
                                let v = versions.get(vix)?.clone();
                                let meta = [v.game_versions.last().cloned(), v.loaders.first().cloned()]
                                    .into_iter()
                                    .flatten()
                                    .collect::<Vec<_>>()
                                    .join(" · ");
                                let enabled = state == ProjectState::Available;
                                Some(
                                    div()
                                        .id(vix)
                                        .h(px(VERSION_ROW_HEIGHT))
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .px_4()
                                        .hover(|d| d.bg(t.hover))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .flex()
                                                .flex_col()
                                                .child(
                                                    div()
                                                        .truncate()
                                                        .text_xs()
                                                        .text_color(t.text)
                                                        .child(v.number.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .truncate()
                                                        .text_xs()
                                                        .text_color(t.subtle)
                                                        .child(meta),
                                                ),
                                        )
                                        .child(
                                            ui::icon_button(
                                                SharedString::from(format!("iv-{vix}")),
                                                "icons/download.svg",
                                                enabled,
                                                t,
                                            )
                                            .tooltip(ui::tooltip("Install this version"))
                                            .when(enabled, |b| {
                                                b.on_click(cx.listener(move |this, _, _, cx| {
                                                    this.install(ix, Some(v.clone()), cx)
                                                }))
                                            }),
                                        )
                                        .into_any_element(),
                                )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.version_scroll.clone())
                .flex_1()
                .into_any_element()
            }
        };
        div().flex_1().min_h_0().flex().flex_col().child(header).child(body).into_any_element()
    }
}

impl Render for AddonBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let sort = self.sort;
        let sort_dropdown = Dropdown::new("addon-sort", "Sort", sort.label(), t)
            .options(Sort::ALL.iter().map(|s| (SharedString::from(s.label()), *s == sort)))
            .open(self.sort_open)
            .on_toggle(cx.listener(|this, _, _, cx| {
                this.sort_open = !this.sort_open;
                cx.notify();
            }))
            .on_dismiss(cx.listener(|this, _, _, cx| {
                this.sort_open = false;
                cx.notify();
            }))
            .on_select(cx.processor(|this, ix: usize, _, cx| {
                this.sort_open = false;
                let pick = Sort::ALL.get(ix).copied().unwrap_or_default();
                if pick != this.sort {
                    this.sort = pick;
                    this.run_search(cx);
                }
                cx.notify();
            }));
        let scope = match (&self.game_version, self.loaders.first()) {
            (Some(g), Some(l)) => {
                format!("Showing {} for {g} · {}", self.kind.label().to_lowercase(), capitalize(l))
            }
            (Some(g), None) => format!("Showing {} for {g}", self.kind.label().to_lowercase()),
            (None, _) => format!("Showing all {}", self.kind.label().to_lowercase()),
        };
        let count = match (self.total, self.loading) {
            (0, true) => String::new(),
            (n, _) => format!("{} results", short_number(n)),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(t.border)
                    .child(div().flex_1().child(self.search.clone()))
                    .child(sort_dropdown),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .justify_between()
                    .px_4()
                    .py_1p5()
                    .bg(t.panel)
                    .border_b_1()
                    .border_color(t.border)
                    .text_xs()
                    .text_color(t.subtle)
                    .child(scope)
                    .child(count),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(div().flex_1().min_w_0().child(self.list(t, cx)))
                    .child(self.details(t, cx)),
            )
    }
}

fn capitalize(s: &str) -> String {
    match s {
        "neoforge" => "NeoForge".into(),
        _ => s.chars().take(1).flat_map(char::to_uppercase).chain(s.chars().skip(1)).collect(),
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn short_number(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.0}K", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

impl AddonBrowser {
    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(&self.search.focus_handle(cx));
    }
}
