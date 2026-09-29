//! Mods, resource packs or shader packs of one instance: turn them on and off, remove them,
//! add files, find updates, and browse Modrinth for more.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use futures::StreamExt;
use futures::channel::mpsc;
use gplauncher::addons::{self, Update};
use gplauncher::content::{self, Item, Kind};
use gplauncher::instance::{Instance, Loader};
use gplauncher::modrinth::{Project, Version};
use gplauncher::{Event, Reporter};
use gpui::{
    AnyElement, Context, Entity, EventEmitter, ExternalPaths, FontWeight, PathPromptOptions, PromptLevel,
    SharedString, UniformListScrollHandle, Window, div, img, prelude::*, px, svg, uniform_list,
};

use crate::addon_browser::{AddonBrowser, Install};
use crate::state::State;
use crate::text_input::{self, TextInput};
use crate::theme::Theme;
use crate::ui::{self, Style, tooltip};

const ROW_HEIGHT: f32 = 56.;

pub enum ContentEvent {
    /// The user wants to pick a mod loader first.
    OpenSettings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Installed,
    Browse,
}

enum Updates {
    Idle,
    Checking,
    Found(Vec<Update>),
    Failed(String),
}

/// Work for the background thread, one at a time.
enum Task {
    Install(Project, Option<Version>),
    Update(Vec<Update>),
}

enum TaskMsg {
    Event(Event),
    Done(Result<String, String>),
}

pub struct ContentPage {
    state: State,
    id: String,
    kind: Kind,
    items: Vec<Item>,
    loaded: bool,
    filter: Entity<TextInput>,
    /// Modrinth versions of installed files, by path.
    identified: HashMap<PathBuf, Version>,
    updates: Updates,
    mode: Mode,
    browser: Option<Entity<AddonBrowser>>,
    queue: VecDeque<Task>,
    /// What the background thread is doing, with its progress.
    working: Option<(String, Option<(u64, u64)>)>,
    /// Result of the last action, until the next one.
    notice: Option<(String, bool)>,
    scroll: UniformListScrollHandle,
    generation: u64,
}

impl EventEmitter<ContentEvent> for ContentPage {}

impl ContentPage {
    pub fn new(state: State, id: String, kind: Kind, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| {
            TextInput::new(format!("Filter {}", kind.label().to_lowercase()), cx)
                .with_icon("icons/search.svg")
        });
        cx.subscribe(&filter, |_, _, _: &text_input::Changed, cx| cx.notify()).detach();
        let mut this = ContentPage {
            state,
            id,
            kind,
            items: Vec::new(),
            loaded: false,
            filter,
            identified: HashMap::new(),
            updates: Updates::Idle,
            mode: Mode::Installed,
            browser: None,
            queue: VecDeque::new(),
            working: None,
            notice: None,
            scroll: UniformListScrollHandle::new(),
            generation: 0,
        };
        this.reload(cx);
        this
    }

    fn instance(&self, cx: &gpui::App) -> Option<Instance> {
        self.state.read(cx).instance(&self.id).cloned()
    }

    /// Reads the folder again in the background, then looks the files up on Modrinth.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = self.instance(cx) else { return };
        let kind = self.kind;
        let cache = self.state.read(cx).settings.data_dir.join("cache/icons/content");
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let items = cx.background_spawn(async move { content::list(&inst, kind, &cache) }).await;
            // Only files not looked up yet: the rest keep what Modrinth said before.
            let known = this.update(cx, |this, _| this.identified.clone()).unwrap_or_default();
            let to_identify: Vec<Item> =
                items.iter().filter(|i| !known.contains_key(&i.path)).cloned().collect();
            let alive = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return false;
                }
                this.items = items;
                this.loaded = true;
                // Updates found before refer to files that may be gone.
                if let Updates::Found(list) = &mut this.updates {
                    list.retain(|u| u.item.path.exists());
                }
                this.sync_browser(cx);
                cx.notify();
                true
            });
            if !matches!(alive, Ok(true)) {
                return;
            }
            let identified = cx.background_spawn(async move { addons::identify(&to_identify) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation
                    && let Ok(found) = identified
                {
                    let paths: HashSet<&PathBuf> = this.items.iter().map(|i| &i.path).collect();
                    this.identified.retain(|p, _| paths.contains(p));
                    this.identified.extend(found);
                    this.sync_browser(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn installed_projects(&self) -> HashSet<String> {
        self.identified.values().map(|v| v.project_id.clone()).collect()
    }

    fn queued_projects(&self) -> HashSet<String> {
        self.queue
            .iter()
            .filter_map(|t| match t {
                Task::Install(p, _) => Some(p.id.clone()),
                Task::Update(_) => None,
            })
            .collect()
    }

    fn sync_browser(&self, cx: &mut Context<Self>) {
        if let Some(browser) = &self.browser {
            let (installed, queued) = (self.installed_projects(), self.queued_projects());
            browser.update(cx, |b, cx| b.set_known(installed, queued, cx));
        }
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode == Mode::Browse && self.browser.is_none() {
            let Some(inst) = self.instance(cx) else { return };
            let data_dir = self.state.read(cx).settings.data_dir.clone();
            let (loaders, game) = (addons::loaders(&inst, self.kind), addons::game_version(&inst, self.kind));
            let kind = self.kind;
            let browser = cx.new(|cx| AddonBrowser::new(kind, data_dir, loaders, game, cx));
            cx.subscribe(&browser, |this, _, Install(project, version), cx| {
                this.enqueue(Task::Install(project.clone(), version.clone()), cx);
            })
            .detach();
            self.browser = Some(browser);
            self.sync_browser(cx);
        }
        if mode == Mode::Browse
            && let Some(browser) = &self.browser
        {
            browser.read(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// Switches to browsing Modrinth.
    pub fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(Mode::Browse, window, cx);
    }

    // ---- actions ---------------------------------------------------------------

    fn toggle(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(ix) else { return };
        match content::set_enabled(item, !item.enabled) {
            Ok(path) => {
                let item = &mut self.items[ix];
                if let Some(v) = self.identified.remove(&item.path) {
                    self.identified.insert(path.clone(), v);
                }
                item.path = path;
                item.enabled = !item.enabled;
            }
            Err(e) => self.notice = Some((format!("{e:#}"), true)),
        }
        cx.notify();
    }

    fn set_all(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let visible = self.visible(cx);
        for ix in visible {
            if self.items[ix].enabled != enabled {
                self.toggle(ix, cx);
            }
        }
    }

    fn delete(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(ix).cloned() else { return };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Remove {}?", item.name),
            Some(&format!("{} will be deleted from the instance.", item.file_name)),
            &["Remove", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                match content::delete(&item) {
                    Ok(()) => {
                        this.items.retain(|i| i.path != item.path);
                        this.identified.remove(&item.path);
                        if let Updates::Found(list) = &mut this.updates {
                            list.retain(|u| u.item.path != item.path);
                        }
                        this.notice = Some((format!("Removed {}", item.name), false));
                        this.sync_browser(cx);
                    }
                    Err(e) => this.notice = Some((format!("{e:#}"), true)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn add_files(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(inst) = self.instance(cx) else { return };
        let kind = self.kind;
        let accepted: Vec<PathBuf> = paths.into_iter().filter(|p| kind.accepts(p)).collect();
        if accepted.is_empty() {
            let expected = if kind == Kind::Mods { ".jar files" } else { ".zip files" };
            self.notice = Some((format!("Nothing to add: {} takes {expected}", kind.label()), true));
            cx.notify();
            return;
        }
        self.notice = Some(match content::add_files(&inst, kind, &accepted) {
            Ok(n) => (
                format!(
                    "Added {n} {}",
                    if n == 1 { kind.noun().to_string() } else { format!("{}s", kind.noun()) }
                ),
                false,
            ),
            Err(e) => (format!("{e:#}"), true),
        });
        self.reload(cx);
        cx.notify();
    }

    fn pick_files(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let _ = this.update(cx, |this, cx| this.add_files(paths, cx));
        })
        .detach();
    }

    fn open_folder(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = self.instance(cx) else { return };
        let dir = self.kind.dir(&inst);
        let _ = std::fs::create_dir_all(&dir);
        cx.open_with_system(&dir);
    }

    fn check_updates(&mut self, cx: &mut Context<Self>) {
        let Some(inst) = self.instance(cx) else { return };
        if matches!(self.updates, Updates::Checking) {
            return;
        }
        self.updates = Updates::Checking;
        let (kind, items) = (self.kind, self.items.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { addons::check_updates(&inst, kind, &items) }).await;
            let _ = this.update(cx, |this, cx| {
                this.updates = match result {
                    Ok(list) => Updates::Found(list),
                    Err(e) => Updates::Failed(format!("{e:#}")),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn update_items(&mut self, list: Vec<Update>, cx: &mut Context<Self>) {
        // The new file may reuse the name; look it up again afterwards.
        for u in &list {
            self.identified.remove(&u.item.path);
        }
        if let Updates::Found(found) = &mut self.updates {
            found.retain(|u| !list.iter().any(|x| x.item.path == u.item.path));
        }
        self.enqueue(Task::Update(list), cx);
    }

    fn enqueue(&mut self, task: Task, cx: &mut Context<Self>) {
        self.queue.push_back(task);
        self.sync_browser(cx);
        self.run_next(cx);
        cx.notify();
    }

    fn run_next(&mut self, cx: &mut Context<Self>) {
        if self.working.is_some() {
            return;
        }
        let Some(inst) = self.instance(cx) else { return };
        let Some(task) = self.queue.pop_front() else { return };
        let kind = self.kind;
        let installed = self.installed_projects();
        let label = match &task {
            Task::Install(p, _) => format!("Installing {}", p.title),
            Task::Update(list) => format!(
                "Updating {} {}",
                list.len(),
                if list.len() == 1 { kind.noun().to_string() } else { format!("{}s", kind.noun()) }
            ),
        };
        self.working = Some((label, None));
        let (tx, mut rx) = mpsc::unbounded();
        std::thread::spawn(move || {
            let events = tx.clone();
            let reporter = Reporter::new(move |e| {
                let _ = events.unbounded_send(TaskMsg::Event(e));
            });
            let result = match task {
                Task::Install(project, version) => (|| {
                    let version = match version {
                        Some(v) => v,
                        None => gplauncher::modrinth::project_versions(
                            &project.id,
                            &addons::loaders(&inst, kind),
                            addons::game_version(&inst, kind).as_deref(),
                        )?
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            anyhow::anyhow!("{} has no version for this instance", project.title)
                        })?,
                    };
                    let added = addons::install(&inst, kind, &version, &installed, &reporter)?;
                    Ok(match added.len() {
                        0 => format!("{} is already installed", project.title),
                        1 => format!("Installed {}", project.title),
                        n => format!("Installed {} with {} dependencies", project.title, n - 1),
                    })
                })(),
                Task::Update(list) => (|| {
                    for u in &list {
                        addons::apply_update(&inst, kind, u, &reporter)?;
                    }
                    Ok(format!(
                        "Updated {}",
                        list.iter().map(|u| u.item.name.as_str()).collect::<Vec<_>>().join(", ")
                    ))
                })(),
            };
            let _ = tx.unbounded_send(TaskMsg::Done(result.map_err(|e: anyhow::Error| format!("{e:#}"))));
        });
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let alive = this.update(cx, |this, cx| {
                    match msg {
                        TaskMsg::Event(Event::Status(s)) => {
                            if let Some(w) = &mut this.working {
                                w.0 = s;
                            }
                        }
                        TaskMsg::Event(Event::Progress { done, total }) => {
                            if let Some(w) = &mut this.working {
                                w.1 = (total > 0).then_some((done, total));
                            }
                        }
                        TaskMsg::Event(_) => {}
                        TaskMsg::Done(result) => {
                            this.working = None;
                            this.notice = Some(match result {
                                Ok(s) => (s, false),
                                Err(e) => (e, true),
                            });
                            this.reload(cx);
                            this.sync_browser(cx);
                            this.run_next(cx);
                        }
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    // ---- rendering -------------------------------------------------------------

    /// Indices of items passing the filter.
    fn visible(&self, cx: &gpui::App) -> Vec<usize> {
        let query = self.filter.read(cx).text().trim().to_lowercase();
        (0..self.items.len())
            .filter(|&i| {
                let item = &self.items[i];
                query.is_empty()
                    || item.name.to_lowercase().contains(&query)
                    || item.file_name.to_lowercase().contains(&query)
                    || item.id.to_lowercase().contains(&query)
            })
            .collect()
    }

    fn update_for(&self, item: &Item) -> Option<&Update> {
        match &self.updates {
            Updates::Found(list) => list.iter().find(|u| u.item.path == item.path),
            _ => None,
        }
    }

    fn row(&self, ix: usize, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.items[ix];
        let update = self.update_for(item).cloned();
        let project = self.identified.get(&item.path).map(|v| v.project_id.clone());
        let icon = div()
            .size(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .overflow_hidden()
            .bg(t.tile)
            .map(|d| match &item.icon {
                Some(path) => d.child(img(path.clone()).size_full()),
                None => d.child(svg().path(kind_icon(self.kind)).size(px(16.)).text_color(t.subtle)),
            });
        let mut meta: Vec<String> = Vec::new();
        if !item.version.is_empty() {
            meta.push(item.version.clone());
        }
        if !item.authors.is_empty() {
            meta.push(format!("by {}", item.authors.iter().take(2).cloned().collect::<Vec<_>>().join(", ")));
        }
        div()
            .id(SharedString::from(format!("item-{}", item.file_name)))
            .group("row")
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .border_b_1()
            .border_color(t.border.opacity(0.5))
            .hover(|d| d.bg(t.hover))
            .child(
                ui::switch(SharedString::from(format!("toggle-{ix}")), item.enabled, t)
                    .tooltip(tooltip(if item.enabled { "Turn off" } else { "Turn on" }))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(ix, cx))),
            )
            .child(icon.when(!item.enabled, |d| d.opacity(0.45)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .when(!item.enabled, |d| d.opacity(0.55))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_shrink()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(t.text)
                                    .child(item.name.clone()),
                            )
                            .when(!meta.is_empty(), |d| {
                                d.child(
                                    div()
                                        .flex_shrink()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .text_color(t.subtle)
                                        .child(meta.join(" · ")),
                                )
                            }),
                    )
                    .child(div().truncate().text_xs().text_color(t.muted).child(
                        if item.description.is_empty() {
                            item.file_name.clone()
                        } else {
                            item.description.clone()
                        },
                    )),
            )
            .when(!item.enabled, |d| d.child(ui::badge("Off", t.subtle)))
            .when_some(update, |d, u| {
                let label = format!("Update to {}", u.latest.number);
                d.child(
                    ui::button(
                        SharedString::from(format!("update-{ix}")),
                        Some("icons/circle-arrow-up.svg"),
                        "Update",
                        Style::Secondary,
                        true,
                        t,
                    )
                    .tooltip(tooltip(label))
                    .on_click(cx.listener(move |this, _, _, cx| this.update_items(vec![u.clone()], cx))),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_0p5()
                    .invisible()
                    .group_hover("row", |d| d.visible())
                    .when_some(project, |d, project| {
                        let url = format!("https://modrinth.com/project/{project}");
                        d.child(
                            ui::icon_button(
                                SharedString::from(format!("web-{ix}")),
                                "icons/external-link.svg",
                                true,
                                t,
                            )
                            .tooltip(tooltip("Open on Modrinth"))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                    })
                    .child(
                        ui::icon_button(
                            SharedString::from(format!("delete-{ix}")),
                            "icons/trash.svg",
                            true,
                            t,
                        )
                        .tooltip(tooltip("Remove"))
                        .on_click(cx.listener(move |this, _, window, cx| this.delete(ix, window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn installed_view(&self, t: Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let _ = window;
        let inst = self.instance(cx);
        let running = self.state.read(cx).is_running(&self.id);
        let visible = self.visible(cx);
        let no_loader = self.kind == Kind::Mods && inst.as_ref().is_some_and(|i| i.loader == Loader::Vanilla);
        let disabled = self.items.iter().filter(|i| !i.enabled).count();
        let total_size: u64 = self.items.iter().map(|i| i.size).sum();
        let checking = matches!(self.updates, Updates::Checking);
        let has_items = !self.items.is_empty();

        let toolbar = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(t.border)
            .child(div().w(px(240.)).child(self.filter.clone()))
            .child(div().flex_1())
            .child(
                ui::button(
                    "check-updates",
                    Some("icons/refresh-cw.svg"),
                    if checking { "Checking…" } else { "Check for updates" },
                    Style::Ghost,
                    has_items && !checking,
                    t,
                )
                .when(has_items && !checking, |b| {
                    b.on_click(cx.listener(|this, _, _, cx| this.check_updates(cx)))
                }),
            )
            .child(
                ui::icon_button("add-file", "icons/plus.svg", true, t)
                    .tooltip(tooltip("Add files…"))
                    .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx))),
            )
            .child(
                ui::icon_button("open-folder", "icons/folder.svg", true, t)
                    .tooltip(tooltip("Open folder"))
                    .on_click(cx.listener(|this, _, _, cx| this.open_folder(cx))),
            )
            .child(
                ui::button(
                    "browse",
                    Some("icons/download.svg"),
                    format!("Get {}", self.kind.label().to_lowercase()),
                    Style::Primary,
                    !no_loader,
                    t,
                )
                .when(!no_loader, |b| {
                    b.on_click(cx.listener(|this, _, window, cx| this.set_mode(Mode::Browse, window, cx)))
                }),
            );

        let mut banners: Vec<AnyElement> = Vec::new();
        if running {
            banners.push(
                banner(
                    "icons/info.svg",
                    "The game is running: changes apply the next time it starts.",
                    t.accent,
                    t,
                )
                .into_any_element(),
            );
        }
        match &self.updates {
            Updates::Found(list) if !list.is_empty() => {
                let all = list.clone();
                let text =
                    format!("{} update{} available", list.len(), if list.len() == 1 { "" } else { "s" });
                banners.push(
                    banner("icons/circle-arrow-up.svg", text, t.success, t)
                        .child(
                            ui::button("update-all", None, "Update all", Style::Primary, true, t).on_click(
                                cx.listener(move |this, _, _, cx| this.update_items(all.clone(), cx)),
                            ),
                        )
                        .into_any_element(),
                );
            }
            Updates::Found(_) => banners.push(
                banner("icons/check.svg", "Everything is up to date.", t.success, t).into_any_element(),
            ),
            Updates::Failed(e) => banners.push(
                banner("icons/triangle-alert.svg", format!("Could not check for updates: {e}"), t.danger, t)
                    .into_any_element(),
            ),
            _ => {}
        }

        let body = if no_loader {
            ui::empty_state(
                "icons/puzzle.svg",
                "This instance has no mod loader",
                "Mods need Fabric, Quilt, Forge or NeoForge. Pick one in the instance settings.",
                t,
            )
            .child(
                ui::button(
                    "pick-loader",
                    Some("icons/settings.svg"),
                    "Choose a mod loader",
                    Style::Primary,
                    true,
                    t,
                )
                .mt_2()
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ContentEvent::OpenSettings))),
            )
            .into_any_element()
        } else if !self.loaded {
            div().into_any_element()
        } else if self.items.is_empty() {
            ui::empty_state(
                kind_icon(self.kind),
                format!("No {} yet", self.kind.label().to_lowercase()),
                format!(
                    "Get them from Modrinth, or drop {} files here.",
                    if self.kind == Kind::Mods { ".jar" } else { ".zip" }
                ),
                t,
            )
            .child(
                ui::button(
                    "browse-empty",
                    Some("icons/download.svg"),
                    format!("Get {}", self.kind.label().to_lowercase()),
                    Style::Primary,
                    true,
                    t,
                )
                .mt_2()
                .on_click(cx.listener(|this, _, window, cx| this.set_mode(Mode::Browse, window, cx))),
            )
            .into_any_element()
        } else if visible.is_empty() {
            ui::empty_state("icons/search.svg", "Nothing matches the filter", "", t).into_any_element()
        } else {
            let rows = visible.clone();
            uniform_list(
                "content-items",
                rows.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                    let t = Theme::for_appearance(window.appearance());
                    range
                        .filter_map(|i| rows.get(i).copied())
                        .filter(|&ix| ix < this.items.len())
                        .map(|ix| this.row(ix, t, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(self.scroll.clone())
            .size_full()
            .into_any_element()
        };

        let noun =
            |n: usize| if n == 1 { self.kind.noun().to_string() } else { format!("{}s", self.kind.noun()) };
        let footer = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .h(px(34.))
            .px_4()
            .border_t_1()
            .border_color(t.border)
            .text_xs()
            .text_color(t.muted)
            .child(format!(
                "{} {}{} · {}",
                self.items.len(),
                noun(self.items.len()),
                if disabled > 0 { format!(", {disabled} off") } else { String::new() },
                content::human_size(total_size)
            ))
            .child(div().flex_1())
            .when(has_items, |d| {
                d.child(
                    div()
                        .id("enable-all")
                        .cursor_pointer()
                        .hover(|d| d.text_color(t.text))
                        .on_click(cx.listener(|this, _, _, cx| this.set_all(true, cx)))
                        .child("Turn all on"),
                )
                .child(
                    div()
                        .id("disable-all")
                        .cursor_pointer()
                        .hover(|d| d.text_color(t.text))
                        .on_click(cx.listener(|this, _, _, cx| this.set_all(false, cx)))
                        .child("Turn all off"),
                )
            });

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .children(banners)
            .child(div().flex_1().min_h_0().child(body))
            .child(footer)
            .into_any_element()
    }

    fn browse_view(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(browser) = self.browser.clone() else { return div().into_any_element() };
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
                    .px_2()
                    .py_1p5()
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        ui::button(
                            "back",
                            Some("icons/chevron-left.svg"),
                            format!("Installed {}", self.kind.label().to_lowercase()),
                            Style::Ghost,
                            true,
                            t,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_mode(Mode::Installed, window, cx);
                        })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.subtle)
                            .pr_2()
                            .child(format!("{} installed", self.items.len())),
                    ),
            )
            .child(div().flex_1().min_h_0().child(browser))
            .into_any_element()
    }
}

/// A one-line message across the page with an icon; add buttons as children.
fn banner(icon: &'static str, text: impl Into<SharedString>, color: gpui::Hsla, t: Theme) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .py_2()
        .bg(color.opacity(0.08))
        .border_b_1()
        .border_color(t.border)
        .text_sm()
        .text_color(t.text)
        .child(svg().path(icon).size(px(15.)).flex_none().text_color(color))
        .child(div().flex_1().min_w_0().child(text.into()))
}

pub fn kind_icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Mods => "icons/puzzle.svg",
        Kind::ResourcePacks => "icons/image.svg",
        Kind::ShaderPacks => "icons/sparkles.svg",
    }
}

impl Render for ContentPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let content = match self.mode {
            Mode::Installed => self.installed_view(t, window, cx),
            Mode::Browse => self.browse_view(t, cx),
        };
        let status = self
            .working
            .clone()
            .map(|(text, progress)| (text, progress, false))
            .or_else(|| self.notice.clone().map(|(text, error)| (text, None, error)));
        div()
            .id("content-page")
            .size_full()
            .flex()
            .flex_col()
            .on_drop(
                cx.listener(|this, paths: &ExternalPaths, _, cx| this.add_files(paths.paths().to_vec(), cx)),
            )
            .drag_over::<ExternalPaths>(move |d, _, _, _| d.bg(t.accent_soft))
            .child(div().flex_1().min_h_0().child(content))
            .when_some(status, |d, (text, progress, error)| {
                let working = self.working.is_some();
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_3()
                        .h(px(30.))
                        .px_4()
                        .border_t_1()
                        .border_color(t.border)
                        .bg(t.panel)
                        .text_xs()
                        .text_color(if error { t.danger } else { t.muted })
                        .child(div().size(px(6.)).rounded_full().bg(if working {
                            t.accent
                        } else if error {
                            t.danger
                        } else {
                            t.success
                        }))
                        .child(div().flex_1().truncate().child(text))
                        .when(self.queue.len() > 0, |d| d.child(format!("{} queued", self.queue.len())))
                        .when_some(progress, |d, (done, total)| {
                            d.child(div().w(px(140.)).child(ui::progress_bar(done as f32 / total as f32, t)))
                        })
                        .when(!working, |d| {
                            d.child(ui::icon_button("dismiss-notice", "icons/x.svg", true, t).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.notice = None;
                                    cx.notify();
                                }),
                            ))
                        }),
                )
            })
    }
}
