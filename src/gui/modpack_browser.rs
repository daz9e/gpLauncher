//! Searchable list of modpacks on one platform, with details and versions of the selected one.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use gplauncher::instance::Loader;
use gplauncher::modpack::{self, Filters, Pack, PackVersion, Sort, Source};
use gpui::{
    AnyElement, Context, Entity, FontWeight, ScrollStrategy, SharedString, Task, UniformListScrollHandle,
    Window, div, img, prelude::*, px, svg, uniform_list,
};

use crate::dropdown::Dropdown;
use crate::text_input::{self, TextInput};
use crate::theme::Theme;

const PAGE_SIZE: u64 = 25;
const ROW_HEIGHT: f32 = 60.;
const DETAILS_WIDTH: f32 = 250.;
const VERSION_ROW_HEIGHT: f32 = 28.;
const SEARCH_DELAY: Duration = Duration::from_millis(350);

const LOADERS: [Loader; 4] = [Loader::Fabric, Loader::Forge, Loader::NeoForge, Loader::Quilt];

/// Result of a background lookup; `None` while it is still running.
type Lookup<T> = Option<Result<T, String>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    GameVersion,
    Loader,
    Sort,
}

pub struct ModpackBrowser {
    source: Source,
    data_dir: PathBuf,
    pub search: Entity<TextInput>,
    packs: Vec<Pack>,
    total: u64,
    loading: bool,
    error: Option<String>,
    /// Bumped by every new query so results of older ones are dropped.
    generation: u64,
    pending_search: Option<Task<()>>,
    filters: Filters,
    /// Minecraft releases offered by the version filter, newest first.
    game_versions: Vec<String>,
    open_menu: Option<Menu>,
    selected: Option<usize>,
    versions: HashMap<String, Lookup<Vec<PackVersion>>>,
    selected_version: usize,
    /// Icon files by URL; `None` while downloading or when it failed.
    icons: HashMap<String, Option<PathBuf>>,
    scroll: UniformListScrollHandle,
    version_scroll: UniformListScrollHandle,
}

impl ModpackBrowser {
    pub fn new(
        source: Source,
        data_dir: PathBuf,
        game_versions: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| TextInput::new("Search modpacks", cx));
        cx.subscribe(&search, |this, _, _: &text_input::Changed, cx| {
            // Debounce: dropping the previous task cancels it.
            this.pending_search = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SEARCH_DELAY).await;
                let _ = this.update(cx, |this, cx| this.run_search(cx));
            }));
        })
        .detach();
        let mut this = ModpackBrowser {
            source,
            data_dir,
            search,
            packs: Vec::new(),
            total: 0,
            loading: false,
            error: None,
            generation: 0,
            pending_search: None,
            filters: Filters::default(),
            game_versions,
            open_menu: None,
            selected: None,
            versions: HashMap::new(),
            selected_version: 0,
            icons: HashMap::new(),
            scroll: UniformListScrollHandle::new(),
            version_scroll: UniformListScrollHandle::new(),
        };
        this.run_search(cx);
        this
    }

    pub fn set_game_versions(&mut self, versions: Vec<String>, cx: &mut Context<Self>) {
        self.game_versions = versions;
        cx.notify();
    }

    fn update_filters(&mut self, f: impl FnOnce(&mut Filters), cx: &mut Context<Self>) {
        self.open_menu = None;
        let before = self.filters.clone();
        f(&mut self.filters);
        if self.filters != before {
            self.selected_version = 0;
            self.run_search(cx);
        }
        cx.notify();
    }

    fn close_menu(&mut self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        cx.notify();
    }

    fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.open_menu = if self.open_menu == Some(menu) { None } else { Some(menu) };
        cx.notify();
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.packs.clear();
        self.total = 0;
        self.selected = None;
        self.fetch(0, cx);
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if !self.loading && self.error.is_none() && (self.packs.len() as u64) < self.total {
            self.fetch(self.packs.len() as u64, cx);
        }
    }

    fn fetch(&mut self, offset: u64, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        let generation = self.generation;
        let query = self.search.read(cx).text().to_string();
        let (source, filters) = (self.source.clone(), self.filters.clone());
        cx.spawn(async move |this, cx| {
            let result =
                cx.background_spawn(async move { source.search(&query, &filters, offset, PAGE_SIZE) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(page) => {
                        this.total = page.total;
                        this.packs.extend(page.packs);
                        if this.selected.is_none() && !this.packs.is_empty() {
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
        let Some(pack) = self.packs.get(ix) else { return };
        self.selected = Some(ix);
        self.selected_version = 0;
        self.version_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        let id = pack.id.clone();
        if !self.versions.contains_key(&id) {
            self.versions.insert(id.clone(), None);
            let source = self.source.clone();
            cx.spawn(async move |this, cx| {
                let pack_id = id.clone();
                let result = cx.background_spawn(async move { source.versions(&pack_id) }).await;
                let _ = this.update(cx, |this, cx| {
                    this.versions.insert(id, Some(result.map_err(|e| format!("{e:#}"))));
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.packs.is_empty() {
            return;
        }
        let ix = match self.selected {
            Some(ix) => ix.saturating_add_signed(delta).min(self.packs.len() - 1),
            None => 0,
        };
        self.select(ix, cx);
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

    fn selected_pack(&self) -> Option<&Pack> {
        self.packs.get(self.selected?)
    }

    /// Versions of the selected pack that fit the Minecraft version and loader filters.
    fn matching_versions(&self) -> Option<Vec<&PackVersion>> {
        match self.versions.get(&self.selected_pack()?.id) {
            Some(Some(Ok(v))) => Some(v.iter().filter(|v| self.filters.matches(v)).collect()),
            _ => None,
        }
    }

    /// What the Install button would install.
    pub fn selection(&self) -> Option<(Pack, PackVersion)> {
        let version = *self.matching_versions()?.get(self.selected_version)?;
        Some((self.selected_pack()?.clone(), version.clone()))
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
            .rounded(px(size * 0.2))
            .overflow_hidden()
            .bg(t.tile);
        match path {
            Some(path) => frame.child(img(path).size_full()).into_any_element(),
            None => frame
                .child(svg().path("icons/box.svg").size(px(size * 0.45)).text_color(t.subtle))
                .into_any_element(),
        }
    }

    fn list(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let message = |text: String| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .px_6()
                .text_sm()
                .text_color(t.muted)
                .text_center()
                .child(text)
                .into_any_element()
        };
        if self.packs.is_empty() {
            return match (&self.error, self.loading) {
                (Some(e), _) => message(format!("Could not load modpacks: {e}")),
                (None, true) => message("Loading…".into()),
                (None, false) => message("Nothing found".into()),
            };
        }
        let more = (self.packs.len() as u64) < self.total || self.error.is_some();
        let count = self.packs.len() + usize::from(more);
        uniform_list(
            "packs",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                // Reaching the footer row fetches the next page.
                if range.end >= this.packs.len() {
                    this.load_more(cx);
                }
                let icons: Vec<String> =
                    range.clone().filter_map(|ix| this.packs.get(ix)?.icon_url.clone()).collect();
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
        let Some(pack) = self.packs.get(ix) else {
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
        div()
            .id(ix)
            .h(px(ROW_HEIGHT))
            .w_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .cursor_pointer()
            .when(selected, |d| d.bg(t.accent_soft))
            .when(!selected, |d| d.hover(|d| d.bg(t.hover)))
            .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
            .child(self.icon(pack.icon_url.as_deref(), 40., t))
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
                                    .child(pack.title.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(t.subtle)
                                    .child(format!("{} downloads", short_number(pack.downloads))),
                            ),
                    )
                    .child(div().truncate().text_xs().text_color(t.muted).child(one_line(&pack.summary))),
            )
            .into_any_element()
    }

    fn details(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("pack-details")
            .w(px(DETAILS_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(t.border)
            .when_some(self.selected_pack(), |d, pack| {
                let website = pack.website.clone();
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
                                .child(self.icon(pack.icon_url.as_deref(), 48., t))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(t.text)
                                                .line_clamp(2)
                                                .text_ellipsis()
                                                .child(pack.title.clone()),
                                        )
                                        .when(!pack.author.is_empty(), |d| {
                                            d.child(
                                                div()
                                                    .truncate()
                                                    .text_xs()
                                                    .text_color(t.muted)
                                                    .child(format!("by {}", pack.author)),
                                            )
                                        }),
                                ),
                        )
                        .child(div().text_xs().text_color(t.muted).child(pack.summary.clone()))
                        .when_some(website, |d, url| {
                            d.child(
                                div()
                                    .id("website")
                                    .text_xs()
                                    .text_color(t.accent)
                                    .cursor_pointer()
                                    .hover(|d| d.underline())
                                    .on_click(move |_, _, cx| cx.open_url(&url))
                                    .child(format!("Open on {}", pack.platform.label())),
                            )
                        }),
                )
                .child(self.version_list(pack, t, cx))
            })
    }

    fn version_list(&self, pack: &Pack, t: Theme, cx: &mut Context<Self>) -> AnyElement {
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
            .child("Version");
        let note =
            |text: String| div().px_4().py_2().text_xs().text_color(t.muted).child(text).into_any_element();
        let body = match self.versions.get(&pack.id) {
            None | Some(None) => note("Loading…".into()),
            Some(Some(Err(e))) => note(format!("Could not load versions: {e}")),
            Some(Some(Ok(v))) if v.is_empty() => note("No versions available".into()),
            Some(Some(Ok(_))) => match self.matching_versions().unwrap_or_default().len() {
                0 => note("No versions match the filters".into()),
                // Packs can have hundreds of versions, so only the visible rows are built.
                count => uniform_list(
                    "versions",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        let versions = this.matching_versions().unwrap_or_default();
                        range
                            .filter_map(|ix| Some(this.version_row(ix, versions.get(ix)?, t, cx)))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.version_scroll.clone())
                .flex_1()
                .into_any_element(),
            },
        };
        div().flex_1().min_h_0().flex().flex_col().child(header).child(body).into_any_element()
    }

    fn version_row(&self, ix: usize, v: &PackVersion, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let selected = ix == self.selected_version;
        let mut meta = v.game_versions.first().cloned().unwrap_or_default();
        if let Some(loader) = v.loaders.first() {
            meta = if meta.is_empty() { loader.clone() } else { format!("{meta} · {loader}") };
        }
        div()
            .id(ix)
            .w_full()
            .h(px(VERSION_ROW_HEIGHT))
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .text_xs()
            .cursor_pointer()
            .when(selected, |d| d.bg(t.accent_soft))
            .when(!selected, |d| d.hover(|d| d.bg(t.hover)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_version = ix;
                cx.notify();
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.text)
                    .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                    .child(v.name.clone()),
            )
            .child(div().flex_none().text_color(t.muted).child(meta))
            .into_any_element()
    }

    fn filter_bar(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.open_menu;
        let any = |selected: bool| (SharedString::from("Any"), selected);

        let version = self.filters.game_version.clone();
        let versions = self.game_versions.clone();
        let version_dropdown =
            Dropdown::new("filter-version", "Minecraft", version.clone().unwrap_or("Any".into()), t)
                .options(std::iter::once(any(version.is_none())).chain(
                    versions.iter().map(|v| (SharedString::from(v.clone()), version.as_ref() == Some(v))),
                ))
                .open(open == Some(Menu::GameVersion))
                .on_toggle(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::GameVersion, cx)))
                .on_dismiss(cx.listener(Self::close_menu))
                .on_select(cx.processor(move |this, ix: usize, _, cx| {
                    let pick = ix.checked_sub(1).and_then(|i| versions.get(i).cloned());
                    this.update_filters(|f| f.game_version = pick, cx);
                }));

        let loader = self.filters.loader;
        let loader_dropdown =
            Dropdown::new("filter-loader", "Loader", loader.map_or("Any", |l| l.label()), t)
                .options(
                    std::iter::once(any(loader.is_none()))
                        .chain(LOADERS.iter().map(|l| (SharedString::from(l.label()), loader == Some(*l)))),
                )
                .open(open == Some(Menu::Loader))
                .on_toggle(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Loader, cx)))
                .on_dismiss(cx.listener(Self::close_menu))
                .on_select(cx.processor(|this, ix: usize, _, cx| {
                    let pick = ix.checked_sub(1).and_then(|i| LOADERS.get(i).copied());
                    this.update_filters(|f| f.loader = pick, cx);
                }));

        let sort = self.filters.sort;
        let sort_dropdown = Dropdown::new("filter-sort", "Sort", sort.label(), t)
            .options(Sort::ALL.iter().map(|s| (SharedString::from(s.label()), *s == sort)))
            .open(open == Some(Menu::Sort))
            .on_toggle(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Sort, cx)))
            .on_dismiss(cx.listener(Self::close_menu))
            .on_select(cx.processor(|this, ix: usize, _, cx| {
                let pick = Sort::ALL.get(ix).copied().unwrap_or_default();
                this.update_filters(|f| f.sort = pick, cx);
            }));

        let filtered = self.filters.game_version.is_some() || self.filters.loader.is_some();
        div()
            .flex()
            .items_start()
            .gap_2()
            .child(version_dropdown)
            .child(loader_dropdown)
            .child(sort_dropdown)
            .when(filtered, |d| {
                d.child(
                    div()
                        .id("filter-reset")
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .px_1()
                        .text_xs()
                        .text_color(t.accent)
                        .cursor_pointer()
                        .hover(|d| d.underline())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.update_filters(
                                |f| {
                                    f.game_version = None;
                                    f.loader = None;
                                },
                                cx,
                            )
                        }))
                        .child("Reset"),
                )
            })
    }
}

impl Render for ModpackBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
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
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().flex_1().child(self.search.clone()))
                            .child(div().flex_none().text_xs().text_color(t.subtle).child(count)),
                    )
                    .child(self.filter_bar(t, cx)),
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

/// Summaries often span several lines; the list shows them as one.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `17.7M`, `950K`, `42`.
fn short_number(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.0}K", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}
