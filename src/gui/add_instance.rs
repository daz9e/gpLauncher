//! "Add Instance" dialog. Pages on the left: a custom instance (name, loader, Minecraft version),
//! importing a modpack file, and browsing modpacks on Modrinth and CurseForge.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use gplauncher::instance::{self, Instance, Loader};
use gplauncher::loader;
use gplauncher::modpack::{Pack, PackVersion, Platform, Source};
use gplauncher::version::{self, VersionEntry};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable, FontWeight,
    KeyBinding, PathPromptOptions, ScrollStrategy, SharedString, UniformListScrollHandle, Window, actions,
    div, prelude::*, px, svg, uniform_list,
};

use crate::modpack_browser::ModpackBrowser;
use crate::text_input::{self, TextInput};
use crate::theme::Theme;

actions!(add_instance, [Cancel, Confirm, SelectPrev, SelectNext]);

const CONTEXT: &str = "AddInstance";
const LOADERS: [Loader; 3] = [Loader::Vanilla, Loader::Fabric, Loader::Quilt];
const ROW_HEIGHT: f32 = 30.;
const NAV_WIDTH: f32 = 176.;
const CURSEFORGE_CONSOLE: &str = "https://console.curseforge.com/";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Custom,
    Import,
    Browse(Platform),
}

const PAGES: [(Page, &str, &str); 4] = [
    (Page::Custom, "Custom", "icons/box.svg"),
    (Page::Import, "Import", "icons/folder.svg"),
    (Page::Browse(Platform::Modrinth), "Modrinth", "icons/globe.svg"),
    (Page::Browse(Platform::CurseForge), "CurseForge", "icons/globe.svg"),
];

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm, Some(CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
    ]);
}

pub enum AddInstanceEvent {
    Created(Instance),
    Import(Vec<PathBuf>),
    Install(Pack, PackVersion),
    /// The user entered a CurseForge API key; the dialog stays open.
    SaveCurseForgeKey(String),
    Dismissed,
}

/// Result of a background lookup; `None` while it is still running.
type Lookup<T> = Option<Result<T, String>>;

pub struct AddInstance {
    focus_handle: FocusHandle,
    data_dir: PathBuf,
    page: Page,
    browsers: HashMap<Platform, Entity<ModpackBrowser>>,
    curseforge_key: Option<String>,
    key_input: Entity<TextInput>,
    name: Entity<TextInput>,
    search: Entity<TextInput>,
    loader: Loader,
    snapshots: bool,
    old: bool,
    versions: Lookup<Vec<VersionEntry>>,
    /// Game versions each loader supports (`None` inside = no restriction).
    supported: HashMap<Loader, Lookup<Option<HashSet<String>>>>,
    /// Indices into `versions` that pass the current filters.
    visible: Vec<usize>,
    selected: Option<String>,
    scroll: UniformListScrollHandle,
    error: Option<String>,
}

impl EventEmitter<AddInstanceEvent> for AddInstance {}

impl Focusable for AddInstance {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AddInstance {
    pub fn new(
        data_dir: PathBuf,
        curseforge_key: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| TextInput::new("Instance name", cx));
        let search = cx.new(|cx| TextInput::new("Search versions", cx));
        cx.subscribe(&search, |this, _, _: &text_input::Changed, cx| this.refilter(cx)).detach();
        window.focus(&name.focus_handle(cx));

        let dir = data_dir.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { version::list(&dir) }).await;
            let _ = this.update(cx, |this, cx| {
                this.versions = Some(result.map_err(|e| format!("{e:#}")));
                let releases = this.releases();
                for browser in this.browsers.values() {
                    browser.update(cx, |b, cx| b.set_game_versions(releases.clone(), cx));
                }
                this.refilter(cx);
            });
        })
        .detach();

        let this = AddInstance {
            focus_handle: cx.focus_handle(),
            data_dir,
            page: Page::Custom,
            browsers: HashMap::new(),
            curseforge_key,
            key_input: cx.new(|cx| TextInput::new("Paste your API key", cx)),
            name,
            search,
            loader: Loader::Vanilla,
            snapshots: false,
            old: false,
            versions: None,
            supported: HashMap::from([(Loader::Vanilla, Some(Ok(None)))]),
            visible: Vec::new(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            error: None,
        };
        this.update_placeholder(cx);
        this
    }

    fn set_loader(&mut self, loader: Loader, cx: &mut Context<Self>) {
        self.loader = loader;
        if let std::collections::hash_map::Entry::Vacant(slot) = self.supported.entry(loader) {
            slot.insert(None);
            cx.spawn(async move |this, cx| {
                let result = cx.background_spawn(async move { loader::supported_versions(loader) }).await;
                let _ = this.update(cx, |this, cx| {
                    this.supported.insert(loader, Some(result.map_err(|e| format!("{e:#}"))));
                    this.refilter(cx);
                });
            })
            .detach();
        }
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).text().trim().to_lowercase();
        // While the loader list is loading (or failed to load) show everything.
        let supported = match self.supported.get(&self.loader) {
            Some(Some(Ok(set))) => set.as_ref(),
            _ => None,
        };
        let versions = match &self.versions {
            Some(Ok(v)) => v.as_slice(),
            _ => &[],
        };
        self.visible = versions
            .iter()
            .enumerate()
            .filter(|(_, v)| match v.kind.as_str() {
                "release" => true,
                "snapshot" => self.snapshots,
                "old_beta" | "old_alpha" => self.old,
                // Local profiles (Fabric, Forge, ...) are what loaders produce, not something to pick here.
                _ => false,
            })
            .filter(|(_, v)| supported.is_none_or(|s| s.contains(&v.id)))
            .filter(|(_, v)| query.is_empty() || v.id.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();

        let still_visible = self.selected.as_ref().is_some_and(|id| self.visible_ids().any(|v| v == id));
        if !still_visible {
            let first = self.visible_ids().next().map(String::from);
            self.selected = first;
        }
        self.scroll_to_selected();
        self.update_placeholder(cx);
        cx.notify();
    }

    fn visible_ids(&self) -> impl Iterator<Item = &str> {
        let versions = match &self.versions {
            Some(Ok(v)) => v.as_slice(),
            _ => &[],
        };
        self.visible.iter().map(move |&i| versions[i].id.as_str())
    }

    fn selected_position(&self) -> Option<usize> {
        let id = self.selected.as_deref()?;
        self.visible_ids().position(|v| v == id)
    }

    fn scroll_to_selected(&self) {
        if let Some(ix) = self.selected_position() {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        }
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.visible.len();
        if len == 0 {
            return;
        }
        let ix = match self.selected_position() {
            Some(ix) => ix.saturating_add_signed(delta).min(len - 1),
            None => 0,
        };
        let id = self.visible_ids().nth(ix).map(String::from);
        self.selected = id;
        self.scroll_to_selected();
        self.update_placeholder(cx);
        cx.notify();
    }

    fn default_name(&self) -> String {
        match (&self.selected, self.loader) {
            (None, _) => String::new(),
            (Some(v), Loader::Vanilla) => v.clone(),
            (Some(v), loader) => format!("{v} {}", loader.label()),
        }
    }

    fn update_placeholder(&self, cx: &mut Context<Self>) {
        let placeholder = match self.default_name() {
            name if name.is_empty() => "Instance name".to_string(),
            name => name,
        };
        self.name.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
    }

    fn can_create(&self) -> bool {
        self.selected.is_some()
    }

    fn set_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        let focus = match page {
            Page::Custom => Some(self.name.focus_handle(cx)),
            Page::Import => None,
            Page::Browse(platform) => match self.browser(platform, cx) {
                Some(browser) => Some(browser.read(cx).search.focus_handle(cx)),
                None => Some(self.key_input.focus_handle(cx)),
            },
        };
        window.focus(&focus.unwrap_or_else(|| self.focus_handle.clone()));
        cx.notify();
    }

    /// The browser for a platform, created on first use; `None` while CurseForge has no key.
    fn browser(&mut self, platform: Platform, cx: &mut Context<Self>) -> Option<Entity<ModpackBrowser>> {
        if let Some(browser) = self.browsers.get(&platform) {
            return Some(browser.clone());
        }
        let source = match platform {
            Platform::Modrinth => Source::Modrinth,
            Platform::CurseForge => Source::CurseForge { api_key: self.curseforge_key.clone()? },
        };
        let (data_dir, releases) = (self.data_dir.clone(), self.releases());
        let browser = cx.new(|cx| ModpackBrowser::new(source, data_dir, releases, cx));
        // The footer's Install button depends on the browser's selection.
        cx.observe(&browser, |_, _, cx| cx.notify()).detach();
        self.browsers.insert(platform, browser.clone());
        Some(browser)
    }

    /// Minecraft releases for the modpack version filters.
    fn releases(&self) -> Vec<String> {
        match &self.versions {
            Some(Ok(v)) => v.iter().filter(|v| v.kind == "release").map(|v| v.id.clone()).collect(),
            _ => Vec::new(),
        }
    }

    fn selection(&self, cx: &App) -> Option<(Pack, PackVersion)> {
        let Page::Browse(platform) = self.page else { return None };
        self.browsers.get(&platform)?.read(cx).selection()
    }

    fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key_input.read(cx).text().trim().to_string();
        if key.is_empty() {
            return;
        }
        self.curseforge_key = Some(key.clone());
        cx.emit(AddInstanceEvent::SaveCurseForgeKey(key));
        self.set_page(Page::Browse(Platform::CurseForge), window, cx);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Custom => self.create(cx),
            Page::Import => self.pick_modpack(cx),
            Page::Browse(platform) if !self.browsers.contains_key(&platform) => self.save_key(window, cx),
            Page::Browse(_) => {
                if let Some((pack, version)) = self.selection(cx)
                    && version.url.is_some()
                {
                    cx.emit(AddInstanceEvent::Install(pack, version));
                }
            }
        }
    }

    fn move_in_page(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.page {
            Page::Custom => self.move_selection(delta, cx),
            Page::Browse(platform) => {
                if let Some(browser) = self.browsers.get(&platform) {
                    browser.update(cx, |b, cx| b.move_selection(delta, cx));
                }
            }
            Page::Import => {}
        }
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let Some(minecraft) = self.selected.clone() else { return };
        let name = match self.name.read(cx).text().trim() {
            "" => self.default_name(),
            name => name.to_string(),
        };
        match instance::create(&self.data_dir, &name, &minecraft, self.loader, "") {
            Ok(inst) => cx.emit(AddInstanceEvent::Created(inst)),
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                cx.notify();
            }
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(AddInstanceEvent::Dismissed);
    }

    fn pick_modpack(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let _ = this.update(cx, |_, cx| cx.emit(AddInstanceEvent::Import(paths)));
        })
        .detach();
    }

    // ---- rendering -----------------------------------------------------------

    fn loader_picker(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().p_0p5().gap_0p5().rounded_md().bg(t.tile).children(LOADERS.into_iter().map(|loader| {
            let active = self.loader == loader;
            div()
                .id(SharedString::from(format!("loader-{}", loader.label())))
                .flex_1()
                .flex()
                .justify_center()
                .py_1()
                .rounded(px(5.))
                .text_sm()
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.bg)
                        .text_color(t.text)
                        .font_weight(FontWeight::MEDIUM)
                        .border_1()
                        .border_color(t.border)
                })
                .when(!active, |d| d.text_color(t.muted).hover(|d| d.text_color(t.text)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_loader(loader, cx)))
                .child(loader.label())
        }))
    }

    fn version_list(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let placeholder = |text: String| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .px_4()
                .text_sm()
                .text_color(t.muted)
                .text_center()
                .child(text)
                .into_any_element()
        };
        let body = match (&self.versions, self.supported.get(&self.loader)) {
            (None, _) => placeholder("Loading versions…".into()),
            (Some(Err(e)), _) => placeholder(format!("Could not load the version list: {e}")),
            (_, Some(None)) => placeholder(format!("Checking {} support…", self.loader.label())),
            _ if self.visible.is_empty() => placeholder("No matching versions".into()),
            _ => uniform_list(
                "versions",
                self.visible.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    let Some(Ok(versions)) = &this.versions else { return Vec::new() };
                    range
                        .map(|ix| {
                            let v = &versions[this.visible[ix]];
                            let selected = this.selected.as_deref() == Some(v.id.as_str());
                            let id = v.id.clone();
                            div()
                                .id(ix)
                                .w_full()
                                .h(px(ROW_HEIGHT))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .text_sm()
                                .cursor_pointer()
                                .when(selected, |d| d.bg(t.accent_soft))
                                .when(!selected, |d| d.hover(|d| d.bg(t.hover)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.selected = Some(id.clone());
                                    this.update_placeholder(cx);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex_1()
                                        .truncate()
                                        .text_color(t.text)
                                        .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                                        .child(v.id.clone()),
                                )
                                .when(v.installed, |d| {
                                    d.child(div().text_xs().text_color(t.subtle).child("installed"))
                                })
                                .child(
                                    div()
                                        .w(px(64.))
                                        .text_right()
                                        .text_xs()
                                        .text_color(t.muted)
                                        .child(kind_label(&v.kind)),
                                )
                        })
                        .collect()
                }),
            )
            .track_scroll(self.scroll.clone())
            .size_full()
            .into_any_element(),
        };
        div()
            .flex_1()
            .min_h(px(160.))
            .rounded_md()
            .border_1()
            .border_color(t.border)
            .bg(t.bg)
            .overflow_hidden()
            .child(body)
    }

    fn loader_note(&self, t: Theme) -> Option<impl IntoElement> {
        let text = match self.supported.get(&self.loader) {
            Some(Some(Err(_))) => {
                format!("Could not check which versions {} supports; showing all.", self.loader.label())
            }
            _ if self.loader != Loader::Vanilla => {
                format!("The latest stable {} is installed on first launch.", self.loader.label())
            }
            _ => return None,
        };
        Some(div().text_xs().text_color(t.muted).child(text))
    }
}

impl AddInstance {
    fn nav(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(NAV_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .gap_0p5()
            .p_2()
            .bg(t.panel)
            .border_r_1()
            .border_color(t.border)
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_3()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child("Add Instance"),
            )
            .children(PAGES.into_iter().map(|(page, label, icon)| {
                let active = self.page == page;
                let color = if active { t.text } else { t.muted };
                div()
                    .id(label)
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(30.))
                    .px_2()
                    .rounded_md()
                    .text_sm()
                    .text_color(color)
                    .cursor_pointer()
                    .when(active, |d| d.bg(t.accent_soft).font_weight(FontWeight::MEDIUM))
                    .when(!active, |d| d.hover(|d| d.bg(t.hover)))
                    .on_click(cx.listener(move |this, _, window, cx| this.set_page(page, window, cx)))
                    .child(svg().path(icon).size(px(14.)).text_color(if active {
                        t.accent
                    } else {
                        t.subtle
                    }))
                    .child(label)
            }))
    }

    fn custom_page(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .child(field("Name", t).child(self.name.clone()))
            .child(field("Mod loader", t).child(self.loader_picker(t, cx)).children(self.loader_note(t)))
            .child(
                field("Minecraft version", t)
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().child(self.search.clone()))
                            .child(toggle("snapshots", "Snapshots", self.snapshots, t).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.snapshots = !this.snapshots;
                                    this.refilter(cx);
                                },
                            )))
                            .child(toggle("old", "Old", self.old, t).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.old = !this.old;
                                    this.refilter(cx);
                                },
                            ))),
                    )
                    .child(self.version_list(t, cx)),
            )
            .when_some(self.error.clone(), |d, e| d.child(div().text_xs().text_color(t.danger).child(e)))
    }

    fn import_page(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let cf_note =
            if self.curseforge_key.is_some() { "" } else { " (needs an API key, see the CurseForge page)" };
        let format = |name: &'static str, ext: &'static str, note: &str| {
            div()
                .flex()
                .gap_2()
                .text_sm()
                .child(div().w(px(150.)).flex_none().text_color(t.text).child(name))
                .child(div().text_color(t.muted).child(format!("{ext}{note}")))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .p_5()
            .child(
                div()
                    .id("drop-zone")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .rounded_lg()
                    .border_2()
                    .border_dashed()
                    .border_color(t.border)
                    .bg(t.bg)
                    .cursor_pointer()
                    .hover(|d| d.border_color(t.accent.opacity(0.5)))
                    .drag_over::<ExternalPaths>(move |d, _, _, _| d.border_color(t.accent).bg(t.accent_soft))
                    .on_click(cx.listener(|this, _, _, cx| this.pick_modpack(cx)))
                    .child(svg().path("icons/folder.svg").size(px(28.)).text_color(t.subtle))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child("Drop modpack files here"),
                    )
                    .child(div().text_sm().text_color(t.muted).child("or click to choose them")),
            )
            .child(
                field("Supported formats", t)
                    .gap_2()
                    .child(format("Modrinth", ".mrpack", ""))
                    .child(format("CurseForge", ".zip", cf_note))
                    .child(format("MultiMC / Prism", ".zip export", "")),
            )
    }

    fn curseforge_key_page(&self, t: Theme) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .gap_3()
            .px_10()
            .child(
                div().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child("CurseForge API key needed"),
            )
            .child(div().text_sm().text_color(t.muted).child(
                "CurseForge only answers apps that send an API key. You can get one for free \
                 in the CurseForge console; it is stored in the launcher settings. \
                 The CURSEFORGE_API_KEY environment variable works too.",
            ))
            .child(self.key_input.clone())
            .child(
                div()
                    .id("cf-console")
                    .text_sm()
                    .text_color(t.accent)
                    .cursor_pointer()
                    .hover(|d| d.underline())
                    .on_click(|_, _, cx| cx.open_url(CURSEFORGE_CONSOLE))
                    .child("Open console.curseforge.com"),
            )
    }

    fn footer(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut note = None;
        let (label, enabled) = match self.page {
            Page::Custom => ("Create", self.can_create()),
            Page::Import => ("Choose files…", true),
            Page::Browse(p) if !self.browsers.contains_key(&p) => {
                ("Save key", !self.key_input.read(cx).text().trim().is_empty())
            }
            Page::Browse(_) => match self.selection(cx) {
                Some((_, v)) if v.url.is_none() => {
                    note = Some("The author does not allow launchers to download this version");
                    ("Install", false)
                }
                selection => ("Install", selection.is_some()),
            },
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_5()
            .py_3()
            .border_t_1()
            .border_color(t.border)
            .child(div().flex_1().min_w_0().truncate().text_xs().text_color(t.muted).children(note))
            .child(
                button("cancel", "Cancel", true, false, t)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx))),
            )
            .child(button("confirm", label, enabled, true, t).when(enabled, |b| {
                b.on_click(cx.listener(|this, _, window, cx| this.confirm(&Confirm, window, cx)))
            }))
    }
}

impl Render for AddInstance {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let content: AnyElement = match self.page {
            Page::Custom => self.custom_page(t, cx).into_any_element(),
            Page::Import => self.import_page(t, cx).into_any_element(),
            Page::Browse(platform) => match self.browsers.get(&platform) {
                Some(browser) => browser.clone().into_any_element(),
                None => self.curseforge_key_page(t).into_any_element(),
            },
        };
        div()
            .id("add-instance")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_in_page(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_in_page(1, cx)))
            .on_drop(cx.listener(|_, paths: &ExternalPaths, _, cx| {
                cx.emit(AddInstanceEvent::Import(paths.paths().to_vec()))
            }))
            .w(px(860.))
            .h(px(600.))
            .max_w_full()
            .max_h_full()
            .flex()
            .rounded_xl()
            .border_1()
            .border_color(t.border)
            .bg(t.bg)
            .shadow_lg()
            .overflow_hidden()
            .child(self.nav(t, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().flex_1().min_h_0().child(content))
                    .child(self.footer(t, cx)),
            )
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "release" => "Release",
        "snapshot" => "Snapshot",
        "old_beta" => "Beta",
        "old_alpha" => "Alpha",
        _ => "",
    }
}

fn field(label: &'static str, t: Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(t.muted).child(label))
}

fn toggle(id: &'static str, label: &'static str, on: bool, t: Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(32.))
        .flex()
        .items_center()
        .px_2p5()
        .rounded_md()
        .border_1()
        .text_sm()
        .cursor_pointer()
        .when(on, |d| d.bg(t.accent_soft).border_color(t.accent.opacity(0.35)).text_color(t.accent))
        .when(!on, |d| d.border_color(t.border).text_color(t.muted).hover(|d| d.bg(t.hover)))
        .child(label)
}

fn button(
    id: &'static str,
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
        .h(px(30.))
        .flex()
        .items_center()
        .px_3()
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .when(enabled, |d| d.cursor_pointer().hover(|d| d.opacity(0.88)))
        .child(label)
}

/// Keeps only files the importer understands.
pub fn importable(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths.iter().filter(|p| gplauncher::import::is_importable(Path::new(p))).cloned().collect()
}
