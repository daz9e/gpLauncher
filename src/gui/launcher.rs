//! Main window: toolbar, instance grid, sidebar for the selected instance, status bar.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use gplauncher::content::Kind;
use gplauncher::import::{self, Imported};
use gplauncher::instance::{self, Instance, Loader};
use gplauncher::{export, modpack};
use gpui::{
    AnyElement, AnyView, App, Context, Corner, Entity, ExternalPaths, FocusHandle, Focusable, Font,
    FontWeight, IntoElement, LineFragment, MouseButton, ParentElement, Pixels, Point, PromptLevel, Render,
    SharedString, Styled, TextRun, Window, actions, anchored, deferred, div, img, prelude::*, px, rems, svg,
};

use crate::accounts::{self, Accounts, AccountsEvent};
use crate::add_instance::{self, AddInstance, AddInstanceEvent};
use crate::dropdown::Dropdown;
use crate::instance_window::{self, Page};
use crate::settings_page::{SettingsEvent, SettingsPage};
use crate::shortcut;
use crate::state::{Phase, State};
use crate::text_input::{self, TextInput};
use crate::theme::{self, Theme};
use crate::ui::{self, Style, tooltip};

actions!(launcher, [NewInstance, FocusSearch, OpenSettings, LaunchSelected, OpenSelected]);

const TOOLBAR_HEIGHT: f32 = 46.;
const SIDEBAR_WIDTH: f32 = 264.;
const TILE_WIDTH: f32 = 132.;
const ICON_SIZE: f32 = 68.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Recent,
    Name,
}

pub struct Launcher {
    focus_handle: FocusHandle,
    state: State,
    selected: Option<String>,
    search: Entity<TextInput>,
    add_dialog: Option<Entity<AddInstance>>,
    accounts_dialog: Option<Entity<Accounts>>,
    settings_page: Option<Entity<SettingsPage>>,
    /// Groups folded in the grid.
    collapsed: HashSet<String>,
    sort: SortBy,
    sort_open: bool,
    /// Right-click menu: the instance and where it was opened.
    context_menu: Option<(String, Point<Pixels>)>,
}

impl Launcher {
    pub fn new(state: State, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, _, cx| cx.notify()).detach();
        cx.observe(&state, |this: &mut Self, state, cx| {
            // Newly added instances get selected.
            if let Some(id) = state.update(cx, |s, _| s.reveal.take()) {
                this.selected = Some(id);
            }
            let s = state.read(cx);
            if this.selected.as_ref().is_none_or(|id| s.instance(id).is_none()) {
                this.selected = s.instances.first().map(|i| i.id.clone());
            }
            cx.notify();
        })
        .detach();
        theme::set_appearance(state.read(cx).settings.appearance);
        let selected = state.read(cx).instances.first().map(|i| i.id.clone());
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let search = cx.new(|cx| TextInput::new("Search instances", cx).with_icon("icons/search.svg"));
        cx.subscribe(&search, |_, _, _: &text_input::Changed, cx| cx.notify()).detach();
        Launcher {
            focus_handle,
            state,
            selected,
            search,
            add_dialog: None,
            accounts_dialog: None,
            settings_page: None,
            collapsed: HashSet::new(),
            sort: SortBy::Recent,
            sort_open: false,
            context_menu: None,
        }
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = Some(id);
        self.context_menu = None;
        // A finished job's message has been seen once the user moves on.
        self.state.update(cx, |s, cx| s.clear_notice(cx));
        cx.notify();
    }

    fn notice(&self, text: String, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.notice(text, cx));
    }

    // ---- dialogs -------------------------------------------------------------------

    fn open_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_dialog.is_some() {
            return;
        }
        self.close_settings(window, cx);
        let s = &self.state.read(cx).settings;
        let (data_dir, key) = (s.data_dir.clone(), s.curseforge_key());
        let dialog = cx.new(|cx| AddInstance::new(data_dir, key, window, cx));
        cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            match event {
                AddInstanceEvent::Created(inst) => {
                    let inst = inst.clone();
                    this.state.update(cx, |s, cx| s.add_instance(inst, cx));
                }
                AddInstanceEvent::Import(paths) => this.import(paths.clone(), cx),
                AddInstanceEvent::Install(pack, version) => this.install(pack.clone(), version.clone(), cx),
                AddInstanceEvent::SaveCurseForgeKey(key) => {
                    let key = key.clone();
                    this.state.update(cx, |s, cx| {
                        s.settings.curseforge_api_key = key;
                        s.save_settings(cx);
                    });
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

    /// `add[:<loader>]`, `accounts` or `settings`: opens that dialog, for development.
    #[cfg(debug_assertions)]
    pub fn open_debug(&mut self, spec: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut parts = spec.split(':');
        match parts.next() {
            Some("add") => {
                self.open_add_dialog(window, cx);
                let loader = match parts.next() {
                    Some("fabric") => Some(Loader::Fabric),
                    Some("forge") => Some(Loader::Forge),
                    Some("neoforge") => Some(Loader::NeoForge),
                    _ => None,
                };
                if let (Some(dialog), Some(loader)) = (&self.add_dialog, loader) {
                    dialog.update(cx, |d, cx| d.set_loader(loader, cx));
                }
            }
            Some("accounts") => self.open_accounts_dialog(window, cx),
            Some("settings") => self.open_settings(window, cx),
            _ => {}
        }
    }

    fn open_accounts_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.accounts_dialog.is_some() {
            return;
        }
        let s = &self.state.read(cx).settings;
        let (accounts, selected, client_id) =
            (s.accounts.clone(), s.selected_account, s.ms_client_id.clone());
        let dialog = cx.new(|cx| Accounts::new(accounts, selected, client_id, window, cx));
        cx.subscribe_in(&dialog, window, |this, _, event, window, cx| match event {
            AccountsEvent::Changed { accounts, selected } => {
                let (accounts, selected) = (accounts.clone(), *selected);
                this.state.update(cx, |s, cx| {
                    s.settings.accounts = accounts;
                    s.settings.selected_account = selected;
                    s.save_settings(cx);
                });
            }
            AccountsEvent::SaveClientId(id) => {
                let id = id.clone();
                this.state.update(cx, |s, cx| {
                    s.settings.ms_client_id = id;
                    s.save_settings(cx);
                });
            }
            AccountsEvent::Dismissed => {
                this.accounts_dialog = None;
                window.focus(&this.focus_handle);
                cx.notify();
            }
        })
        .detach();
        self.accounts_dialog = Some(dialog);
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_page.is_some() || self.modal().is_some() {
            return;
        }
        let (settings, busy) = (self.state.read(cx).settings.clone(), self.state.read(cx).busy());
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

    fn apply_settings(
        &mut self,
        mut settings: gplauncher::settings::Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |s, cx| {
            // Accounts may have been refreshed by a running launch meanwhile.
            settings.accounts = std::mem::take(&mut s.settings.accounts);
            settings.selected_account = s.settings.selected_account;
            let moved = settings.data_dir != s.settings.data_dir;
            s.settings = settings;
            theme::set_appearance(s.settings.appearance);
            s.save_settings(cx);
            if moved {
                s.reload_instances(cx);
            }
        });
        // Every window picks the new appearance up.
        for handle in cx.windows() {
            let _ = handle.update(cx, |_, window, _| window.refresh());
        }
        window.refresh();
    }

    fn modal(&self) -> Option<AnyView> {
        self.add_dialog.clone().map(AnyView::from).or_else(|| self.accounts_dialog.clone().map(AnyView::from))
    }

    // ---- instance actions ----------------------------------------------------------

    /// Selects the instance with `id` and launches it (for `--launch` shortcuts).
    pub fn launch_id(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.state.read(cx).instance(id).is_some() {
            self.select(id.to_string(), cx);
            self.launch(id.to_string(), cx);
        } else {
            self.notice(format!("No instance \"{id}\" to launch"), cx);
        }
    }

    fn launch(&mut self, id: String, cx: &mut Context<Self>) {
        self.context_menu = None;
        self.state.update(cx, |s, cx| s.launch(&id, cx));
    }

    fn kill(&mut self, id: String, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.kill(&id, cx));
    }

    fn open_window(&mut self, id: String, page: Page, cx: &mut Context<Self>) {
        self.context_menu = None;
        cx.notify();
        let state = self.state.clone();
        cx.defer(move |cx| instance_window::open(&state, &id, page, cx));
    }

    fn open_folder(&mut self, id: String, cx: &mut Context<Self>) {
        self.context_menu = None;
        let Some(inst) = self.state.read(cx).instance(&id).cloned() else { return };
        let _ = std::fs::create_dir_all(&inst.game_dir);
        cx.open_with_system(&inst.game_dir);
    }

    fn export(&mut self, id: String, cx: &mut Context<Self>) {
        self.context_menu = None;
        let Some(inst) = self.state.read(cx).instance(&id).cloned() else { return };
        let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_default();
        let path = cx.prompt_for_new_path(&dir, Some(&export::file_name(&inst)));
        let state = self.state.clone();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(dest))) = path.await else { return };
            let _ = state.update(cx, |s, cx| {
                s.run_job(&format!("Exporting {}", inst.name), cx, move |reporter, _| {
                    export::export(&inst, &dest, reporter)?;
                    Ok(format!("Exported to {}", dest.display()))
                })
            });
        })
        .detach();
        cx.notify();
    }

    fn copy(&mut self, id: String, cx: &mut Context<Self>) {
        self.context_menu = None;
        let Some(inst) = self.state.read(cx).instance(&id).cloned() else { return };
        let data_dir = self.state.read(cx).settings.data_dir.clone();
        self.state.update(cx, |s, cx| {
            s.run_job(&format!("Copying {}", inst.name), cx, move |_, imported| {
                let copy = instance::duplicate(&data_dir, &inst, &format!("{} (copy)", inst.name))?;
                let status = format!("Copied to \"{}\"", copy.name);
                imported(Imported { instance: copy, blocked: Vec::new() });
                Ok(status)
            })
        });
        cx.notify();
    }

    fn delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        let Some(inst) = self.state.read(cx).instance(&id).cloned() else { return };
        if self.state.read(cx).is_running(&id) {
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
                let index = this.state.read(cx).instances.iter().position(|i| i.id == id).unwrap_or(0);
                match this.state.update(cx, |s, cx| s.delete_instance(&id, cx)) {
                    Ok(()) => {
                        instance_window::close(&id, cx);
                        let list = &this.state.read(cx).instances;
                        this.selected = list.get(index).or(list.last()).map(|i| i.id.clone());
                        this.notice(format!("Deleted \"{}\"", inst.name), cx);
                    }
                    Err(e) => this.notice(format!("Error: {e:#}"), cx),
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn create_shortcut(&mut self, id: String, cx: &mut Context<Self>) {
        self.context_menu = None;
        let Some(inst) = self.state.read(cx).instance(&id).cloned() else { return };
        let status = match shortcut::create(&inst) {
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
            self.notice("Nothing to import: expected .mrpack or .zip modpacks".into(), cx);
            return;
        }
        let settings = self.state.read(cx).settings.clone();
        self.state.update(cx, |s, cx| {
            s.run_job("Importing", cx, move |reporter, imported| {
                let mut summary = Summary::default();
                for file in &files {
                    let result = import::import(&settings, file, reporter)?;
                    summary.add(&result);
                    imported(result);
                }
                Ok(summary.status())
            })
        });
    }

    fn install(&mut self, pack: modpack::Pack, version: modpack::PackVersion, cx: &mut Context<Self>) {
        let settings = self.state.read(cx).settings.clone();
        self.state.update(cx, |s, cx| {
            s.run_job(&format!("Installing {}", pack.title), cx, move |reporter, imported| {
                let result = modpack::install(&settings, &pack, &version, reporter)?;
                let mut summary = Summary::default();
                summary.add(&result);
                imported(result);
                Ok(summary.status())
            })
        });
    }

    // ---- rendering -----------------------------------------------------------------

    fn toolbar(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let data_dir = self.state.read(cx).settings.data_dir.clone();
        let account = self.state.read(cx).account_name();
        let offline = self
            .state
            .read(cx)
            .settings
            .account()
            .is_none_or(|a| a.kind == gplauncher::auth::AccountKind::Offline);
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
                ui::button("add", Some("icons/plus.svg"), "Add Instance", Style::Primary, true, t)
                    .on_click(cx.listener(|this, _, window, cx| this.open_add_dialog(window, cx))),
            )
            .child(div().w(px(6.)))
            .child(
                ui::button("folders", Some("icons/folder.svg"), "Folder", Style::Ghost, true, t)
                    .tooltip(tooltip("Open the launcher folder"))
                    .on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&data_dir);
                        cx.open_with_system(&data_dir);
                    }),
            )
            .child(
                ui::button("settings", Some("icons/settings.svg"), "Settings", Style::Ghost, true, t)
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
                    .tooltip(tooltip(if offline {
                        "Offline account · switch or sign in"
                    } else {
                        "Microsoft account · switch"
                    }))
                    .on_click(cx.listener(|this, _, window, cx| this.open_accounts_dialog(window, cx)))
                    .child(accounts::avatar(&account, 22., t))
                    .child(div().text_sm().text_color(t.text).whitespace_nowrap().child(account))
                    .child(svg().path("icons/chevron-down.svg").size(px(12.)).text_color(t.muted)),
            )
    }

    fn grid(&self, t: Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        if state.instances.is_empty() {
            return ui::empty_state(
                "icons/box.svg",
                "No instances yet",
                "An instance is a separate game folder with its own version, mods and worlds. \
                 Create one, or drop a modpack (.mrpack, .zip) here.",
                t,
            )
            .child(
                ui::button("add-first", Some("icons/plus.svg"), "Add Instance", Style::Primary, true, t)
                    .mt_2()
                    .on_click(cx.listener(|this, _, window, cx| this.open_add_dialog(window, cx))),
            )
            .into_any_element();
        }
        let query = self.search.read(cx).text().trim().to_lowercase();
        let mut visible: Vec<Instance> = state
            .instances
            .iter()
            .filter(|i| {
                query.is_empty()
                    || i.name.to_lowercase().contains(&query)
                    || i.minecraft.to_lowercase().contains(&query)
                    || i.group.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        if self.sort == SortBy::Name {
            visible.sort_by_key(|i| i.name.to_lowercase());
        }
        if visible.is_empty() {
            return ui::empty_state(
                "icons/search.svg",
                "No instances match",
                "Try another name or version.",
                t,
            )
            .into_any_element();
        }
        let mut name_font = window.text_style().font();
        name_font.weight = FontWeight::MEDIUM;
        let name_size = rems(0.875).to_pixels(window.rem_size());

        // Ungrouped instances first, then groups by name.
        let mut groups: BTreeMap<(bool, String), Vec<Instance>> = BTreeMap::new();
        for inst in visible.iter() {
            groups.entry((!inst.group.is_empty(), inst.group.clone())).or_default().push(inst.clone());
        }
        let grouped = groups.keys().any(|(named, _)| *named);
        let sort = self.sort;
        let sort_dropdown = Dropdown::new(
            "sort-instances",
            "Sort",
            if sort == SortBy::Recent { "Last played" } else { "Name" },
            t,
        )
        .options([
            (SharedString::from("Last played"), sort == SortBy::Recent),
            (SharedString::from("Name"), sort == SortBy::Name),
        ])
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
            this.sort = if ix == 0 { SortBy::Recent } else { SortBy::Name };
            this.sort_open = false;
            cx.notify();
        }));

        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .mb_3()
            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child("Instances"))
            .child(div().text_sm().text_color(t.subtle).child(visible.len().to_string()))
            .child(div().flex_1())
            .child(sort_dropdown);

        let mut sections: Vec<AnyElement> = Vec::new();
        for ((_, group), instances) in groups {
            let tiles = div().flex().flex_wrap().items_start().gap_2().children(
                instances
                    .iter()
                    .map(|inst| self.tile(inst, name_font.clone(), name_size, t, cx))
                    .collect::<Vec<_>>(),
            );
            if !grouped {
                sections.push(tiles.into_any_element());
                continue;
            }
            let collapsed = self.collapsed.contains(&group);
            let key = group.clone();
            let label = if group.is_empty() { "Ungrouped".to_string() } else { group.clone() };
            sections.push(
                div()
                    .child(
                        div()
                            .id(SharedString::from(format!("group-{group}")))
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .mb_2()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(key.clone(), cx)))
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
                            .child(div().text_xs().text_color(t.subtle).child(instances.len().to_string()))
                            .child(div().flex_1().ml_1().h(px(1.)).bg(t.border)),
                    )
                    .when(!collapsed, |d| d.child(tiles))
                    .into_any_element(),
            );
        }
        div()
            .id("instances")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .child(header)
            .child(div().flex().flex_col().gap_4().children(sections))
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
        let phase =
            self.state.read(cx).sessions.get(&inst.id).map(|s| s.phase).filter(|p| *p != Phase::Finished);
        let (id, play_id, menu_id) = (inst.id.clone(), inst.id.clone(), inst.id.clone());
        let subtitle = match phase {
            Some(Phase::Running) => {
                div().text_xs().text_color(t.success).font_weight(FontWeight::MEDIUM).child("Playing")
            }
            Some(_) => div().text_xs().text_color(t.accent).child("Starting…"),
            None => div().text_xs().text_color(t.muted).child(short_description(inst)),
        };
        div()
            .id(SharedString::from(format!("inst-{}", inst.id)))
            .group("tile")
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
            .when(!selected, |d| d.border_color(ui::transparent()).hover(|d| d.bg(t.hover)))
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                this.select(id.clone(), cx);
                if e.click_count() == 2 {
                    this.launch(id.clone(), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    this.select(menu_id.clone(), cx);
                    this.context_menu = Some((menu_id.clone(), e.position));
                    cx.notify();
                }),
            )
            .child(
                div()
                    .relative()
                    .child(instance_icon(ICON_SIZE, phase.is_some(), inst.icon.as_deref(), t))
                    .when(phase.is_none(), |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("play-{}", inst.id)))
                                .absolute()
                                .right(px(-6.))
                                .bottom(px(-6.))
                                .size(px(30.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(t.success)
                                .border_2()
                                .border_color(t.bg)
                                .shadow_md()
                                .invisible()
                                .group_hover("tile", |d| d.visible())
                                .hover(|d| d.opacity(0.9))
                                .tooltip(tooltip("Play"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.select(play_id.clone(), cx);
                                    this.launch(play_id.clone(), cx);
                                }))
                                .child(svg().path("icons/play.svg").size(px(13.)).text_color(t.on_accent)),
                        )
                    }),
            )
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
                            .flex()
                            .flex_col()
                            .items_center()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.text)
                            .text_center()
                            // The selected tile shows the whole name, others two lines.
                            .when(selected, |d| d.child(inst.name.clone()))
                            .when(!selected, |d| {
                                d.children(
                                    clamp_lines(&inst.name, name_font, name_size, 2, cx)
                                        .into_iter()
                                        .map(|line| div().whitespace_nowrap().child(line)),
                                )
                            }),
                    )
                    .child(subtitle),
            )
            .into_any_element()
    }

    fn sidebar(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let inst = self.selected.as_ref().and_then(|id| state.instance(id)).cloned();
        let session = inst.as_ref().and_then(|i| state.sessions.get(&i.id));
        let phase = session.map(|s| s.phase);
        let session_line = session.map(|s| (s.status.clone(), s.progress, s.phase));
        let default_memory = state.settings.memory_mb;
        let manual = inst.as_ref().is_some_and(|i| state.manual_downloads.contains(&i.id));
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
            .when_some(inst, |d, inst| {
                let running = phase.is_some_and(|p| p != Phase::Finished);
                let id = inst.id.clone();
                let loader = match inst.loader {
                    Loader::Vanilla => "None".to_string(),
                    l if inst.loader_version.is_empty() => format!("{} (latest)", l.label()),
                    l => format!("{} {}", l.label(), inst.loader_version),
                };
                let memory = inst.memory_mb.unwrap_or(default_memory);
                let play = {
                    let id = id.clone();
                    match phase {
                        Some(Phase::Running) => {
                            ui::button("kill", Some("icons/stop.svg"), "Stop", Style::Stop, true, t)
                                .flex_1()
                                .h(px(36.))
                                .on_click(cx.listener(move |this, _, _, cx| this.kill(id.clone(), cx)))
                        }
                        Some(Phase::Preparing) => {
                            ui::button("starting", None, "Starting…", Style::Secondary, false, t)
                                .flex_1()
                                .h(px(36.))
                        }
                        _ => ui::button("launch", Some("icons/play.svg"), "Play", Style::Play, true, t)
                            .flex_1()
                            .h(px(36.))
                            .on_click(cx.listener(move |this, _, _, cx| this.launch(id.clone(), cx))),
                    }
                };
                let link = |key: &'static str,
                            icon: &'static str,
                            label: &'static str,
                            page: Page,
                            cx: &mut Context<Self>| {
                    let id = id.clone();
                    menu_item(key, icon, label, true, false, t)
                        .on_click(cx.listener(move |this, _, _, cx| this.open_window(id.clone(), page, cx)))
                };
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .pt_5()
                        .pb_4()
                        .child(instance_icon(76., running, inst.icon.as_deref(), t))
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
                                        .text_center()
                                        .text_color(t.text)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(inst.name.clone()),
                                )
                                .child(div().text_xs().text_color(t.muted).child(inst.description())),
                        ),
                )
                .child(div().flex().gap_2().px_4().child(play).child({
                    let id = id.clone();
                    ui::button("manage", Some("icons/sliders-horizontal.svg"), "", Style::Secondary, true, t)
                        .h(px(36.))
                        .w(px(40.))
                        .px_0()
                        .tooltip(tooltip("Open the instance window"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_window(id.clone(), Page::Content(Kind::Mods), cx)
                        }))
                }))
                .when_some(session_line, |d, (status, progress, phase)| {
                    let error = status.starts_with("Error") || status.starts_with("Game crashed");
                    let id = id.clone();
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .mx_4()
                            .mt_3()
                            .p_2p5()
                            .rounded_md()
                            .bg(if error { t.danger.opacity(0.08) } else { t.bg })
                            .border_1()
                            .border_color(if error { t.danger.opacity(0.3) } else { t.border })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if error { t.danger } else { t.muted })
                                    .line_clamp(3)
                                    .child(status),
                            )
                            .when_some(progress, |d, (done, total)| {
                                d.child(ui::progress_bar(done as f32 / total as f32, t))
                            })
                            .child(
                                div()
                                    .id("show-console")
                                    .text_xs()
                                    .text_color(t.accent)
                                    .cursor_pointer()
                                    .hover(|d| d.underline())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_window(id.clone(), Page::Console, cx)
                                    }))
                                    .child(if phase == Phase::Finished {
                                        "Show the log"
                                    } else {
                                        "Show the console"
                                    }),
                            ),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .mx_2()
                        .mt_3()
                        .py_2()
                        .border_t_1()
                        .border_color(t.border)
                        .child(link("mods", "icons/puzzle.svg", "Mods", Page::Content(Kind::Mods), cx))
                        .child(link(
                            "packs",
                            "icons/image.svg",
                            "Resource packs",
                            Page::Content(Kind::ResourcePacks),
                            cx,
                        ))
                        .child(link("worlds", "icons/earth.svg", "Worlds", Page::Worlds, cx))
                        .child(link("console", "icons/terminal.svg", "Console & logs", Page::Console, cx))
                        .child(link("edit", "icons/settings.svg", "Settings", Page::Settings, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .mx_2()
                        .py_2()
                        .border_t_1()
                        .border_color(t.border)
                        .child({
                            let id = id.clone();
                            menu_item("folder", "icons/folder.svg", "Open folder", true, false, t)
                                .on_click(cx.listener(move |this, _, _, cx| this.open_folder(id.clone(), cx)))
                        })
                        .child({
                            let id = id.clone();
                            menu_item("export", "icons/share.svg", "Export…", true, false, t)
                                .on_click(cx.listener(move |this, _, _, cx| this.export(id.clone(), cx)))
                        })
                        .child({
                            let id = id.clone();
                            menu_item("copy", "icons/copy.svg", "Duplicate", true, false, t)
                                .on_click(cx.listener(move |this, _, _, cx| this.copy(id.clone(), cx)))
                        })
                        .child({
                            let id = id.clone();
                            menu_item("shortcut", "icons/shortcut.svg", "Desktop shortcut", true, false, t)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.create_shortcut(id.clone(), cx)),
                                )
                        })
                        .child({
                            let id = id.clone();
                            menu_item("delete", "icons/trash.svg", "Delete", !running, true, t).when(
                                !running,
                                |b| {
                                    b.on_click(cx.listener(move |this, _, window, cx| {
                                        this.delete(id.clone(), window, cx)
                                    }))
                                },
                            )
                        }),
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
                        .child(detail("Memory", format!("{memory} MB"), t))
                        .child(detail("Last played", ui::ago(inst.last_played), t))
                        .when(inst.play_time > 0, |d| {
                            d.child(detail(
                                "Time played",
                                ui::duration(std::time::Duration::from_secs(inst.play_time)),
                                t,
                            ))
                        }),
                )
                .when(manual, |d| {
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
                            .border_color(t.warning.opacity(0.4))
                            .bg(t.warning.opacity(0.08))
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

    fn status_bar(&self, t: Theme, cx: &App) -> impl IntoElement {
        let state = self.state.read(cx);
        let session = self.selected.as_ref().and_then(|id| state.sessions.get(id));
        // An active job wins; a finished one stays until the user selects something.
        let (text, active, progress) = match (&state.job, session) {
            (Some(job), _) if job.active => (job.status.clone(), true, job.progress),
            (Some(job), _) => (job.status.clone(), false, None),
            (None, Some(s)) if s.is_active() => (s.status.clone(), true, s.progress),
            _ => (String::from("Ready"), false, None),
        };
        let running = state.running_count();
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
                d.child(div().w(px(160.)).child(ui::progress_bar(ratio, t)))
                    .child(div().w(px(32.)).text_right().child(format!("{:.0}%", ratio * 100.)))
            })
            .when(running > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(div().size(px(6.)).rounded_full().bg(t.success))
                        .child(if running == 1 {
                            "1 game running".to_string()
                        } else {
                            format!("{running} games running")
                        }),
                )
            })
    }

    fn context_menu(&self, t: Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, position) = self.context_menu.clone()?;
        let running = self.state.read(cx).is_running(&id);
        let item =
            |key: &'static str, icon: &'static str, label: &'static str, danger: bool, enabled: bool| {
                menu_item(key, icon, label, enabled, danger, t).mx_1()
            };
        let menu = div()
            .id("context-menu")
            .occlude()
            .w(px(200.))
            .py_1()
            .rounded_lg()
            .border_1()
            .border_color(t.border)
            .bg(t.bg)
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.context_menu = None;
                cx.notify();
            }))
            .child({
                let id = id.clone();
                if running {
                    item("cm-stop", "icons/stop.svg", "Stop", true, true).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.context_menu = None;
                            this.kill(id.clone(), cx)
                        },
                    ))
                } else {
                    item("cm-play", "icons/play.svg", "Play", false, true)
                        .on_click(cx.listener(move |this, _, _, cx| this.launch(id.clone(), cx)))
                }
            })
            .child({
                let id = id.clone();
                item("cm-mods", "icons/puzzle.svg", "Mods", false, true).on_click(cx.listener(
                    move |this, _, _, cx| this.open_window(id.clone(), Page::Content(Kind::Mods), cx),
                ))
            })
            .child({
                let id = id.clone();
                item("cm-console", "icons/terminal.svg", "Console", false, true).on_click(
                    cx.listener(move |this, _, _, cx| this.open_window(id.clone(), Page::Console, cx)),
                )
            })
            .child({
                let id = id.clone();
                item("cm-settings", "icons/settings.svg", "Settings", false, true).on_click(
                    cx.listener(move |this, _, _, cx| this.open_window(id.clone(), Page::Settings, cx)),
                )
            })
            .child(div().my_1().h(px(1.)).bg(t.border))
            .child({
                let id = id.clone();
                item("cm-folder", "icons/folder.svg", "Open folder", false, true)
                    .on_click(cx.listener(move |this, _, _, cx| this.open_folder(id.clone(), cx)))
            })
            .child({
                let id = id.clone();
                item("cm-copy", "icons/copy.svg", "Duplicate", false, true)
                    .on_click(cx.listener(move |this, _, _, cx| this.copy(id.clone(), cx)))
            })
            .child({
                let id = id.clone();
                item("cm-export", "icons/share.svg", "Export…", false, true)
                    .on_click(cx.listener(move |this, _, _, cx| this.export(id.clone(), cx)))
            })
            .child(div().my_1().h(px(1.)).bg(t.border))
            .child({
                let id = id.clone();
                item("cm-delete", "icons/trash.svg", "Delete", true, !running).when(!running, |b| {
                    b.on_click(cx.listener(move |this, _, window, cx| this.delete(id.clone(), window, cx)))
                })
            });
        Some(
            deferred(
                anchored()
                    .position(position)
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(menu),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let main = match self.settings_page.clone() {
            Some(page) => {
                let busy = self.state.read(cx).busy();
                if page.read(cx).busy() != busy {
                    page.update(cx, |p, cx| p.set_busy(busy, cx));
                }
                div().flex_1().min_h_0().child(page).into_any_element()
            }
            None => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.toolbar(t, cx))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(self.grid(t, window, cx))
                        .child(self.sidebar(t, cx)),
                )
                .into_any_element(),
        };
        let selected = self.selected.clone();
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
            .on_action(cx.listener(move |this, _: &LaunchSelected, _, cx| {
                if let Some(id) = selected.clone() {
                    this.launch(id, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSelected, _, cx| {
                if let Some(id) = this.selected.clone() {
                    this.open_window(id, Page::Content(Kind::Mods), cx);
                }
            }))
            .on_drop(
                cx.listener(|this, paths: &ExternalPaths, _, cx| this.import(paths.paths().to_vec(), cx)),
            )
            .child(main)
            .child(self.status_bar(t, cx))
            .children(self.context_menu(t, cx))
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

/// The instance's own icon, or a placeholder. A running instance gets a green ring.
pub fn instance_icon(size: f32, running: bool, icon: Option<&Path>, t: Theme) -> impl IntoElement {
    let frame = div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.24))
        .bg(t.tile)
        .border_1()
        .when(running, |d| d.border_2())
        .border_color(if running { t.success } else { t.tile_edge })
        .overflow_hidden();
    match icon {
        Some(path) => frame.child(img(path.to_path_buf()).size_full()),
        None => frame.child(svg().path("icons/box.svg").size(px(size * 0.42)).text_color(if running {
            t.success
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
