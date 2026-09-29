//! Settings of one instance: name, group and icon, game and loader versions, Java and window.
//! Every valid change is saved right away; fields left empty follow the launcher settings.

use std::path::Path;

use gplauncher::forge::LoaderVersion;
use gplauncher::instance::{Instance, Loader};
use gplauncher::loader;
use gplauncher::version;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, PathPromptOptions, SharedString, Window, div, img,
    prelude::*, px, svg,
};

use crate::dropdown::Dropdown;
use crate::java_field::JavaField;
use crate::settings_page::{
    MEMORY_PRESETS, MIN_MEMORY_MB, chip, error, group, hint, row, segment, segments, stacked,
};
use crate::state::State;
use crate::text_input::{self, TextInput};
use crate::theme::Theme;
use crate::ui::{self, Style};

const LOADERS: [(Loader, &str); 5] = [
    (Loader::Vanilla, "None"),
    (Loader::Fabric, "Fabric"),
    (Loader::Quilt, "Quilt"),
    (Loader::Forge, "Forge"),
    (Loader::NeoForge, "NeoForge"),
];

type Lookup<T> = Option<Result<T, String>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Minecraft,
    LoaderVersion,
}

pub struct InstanceSettings {
    state: State,
    id: String,
    name: Entity<TextInput>,
    group: Entity<TextInput>,
    java: Entity<JavaField>,
    memory: Entity<TextInput>,
    jvm_args: Entity<TextInput>,
    width: Entity<TextInput>,
    height: Entity<TextInput>,
    releases: Lookup<Vec<String>>,
    loader_versions: Option<(Loader, String, Lookup<Vec<LoaderVersion>>)>,
    open_menu: Option<Menu>,
    memory_error: Option<String>,
    java_error: Option<String>,
    size_error: Option<String>,
    save_error: Option<String>,
}

impl InstanceSettings {
    pub fn new(state: State, id: String, cx: &mut Context<Self>) -> Self {
        let inst = state.read(cx).instance(&id).cloned().unwrap_or_default();
        let settings = state.read(cx).settings.clone();
        let input = |placeholder: String, text: String, cx: &mut Context<Self>| {
            let input = cx.new(|cx| TextInput::new(placeholder, cx));
            input.update(cx, |i, cx| i.set_text(text, cx));
            cx.subscribe(&input, |this: &mut Self, _, _: &text_input::Changed, cx| this.commit(cx)).detach();
            input
        };
        let size = |v: Option<u32>| v.map(|v| v.to_string()).unwrap_or_default();
        let java = cx.new(|cx| {
            let global = match settings.java_path.trim() {
                "" => "Automatic".to_string(),
                path => path.to_string(),
            };
            JavaField::new(
                format!("Default ({global})"),
                inst.java_path.clone(),
                "Uses the Java from the launcher settings. Choose a binary only if this instance needs a different one.",
                "Use the default Java",
                cx,
            )
        });
        cx.observe(&java, |this, _, cx| this.commit(cx)).detach();
        let (dw, dh) = settings.resolution().unwrap_or((854, 480));
        let this = InstanceSettings {
            name: input(inst.minecraft.clone(), inst.name.clone(), cx),
            group: input("Ungrouped".into(), inst.group.clone(), cx),
            java,
            memory: input(format!("{}", settings.memory_mb), size(inst.memory_mb), cx),
            jvm_args: input("None".into(), inst.jvm_args.clone(), cx),
            width: input(dw.to_string(), size(inst.window_width), cx),
            height: input(dh.to_string(), size(inst.window_height), cx),
            releases: None,
            loader_versions: None,
            open_menu: None,
            memory_error: None,
            java_error: None,
            size_error: None,
            save_error: None,
            state,
            id,
        };
        let data_dir = this.state.read(cx).settings.data_dir.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { version::list(&data_dir) }).await;
            let _ = this.update(cx, |this, cx| {
                this.releases = Some(
                    result
                        .map(|list| list.into_iter().filter(|v| v.kind == "release").map(|v| v.id).collect())
                        .map_err(|e| format!("{e:#}")),
                );
                cx.notify();
            });
        })
        .detach();
        cx.observe(&this.state, |_, _, cx| cx.notify()).detach();
        this
    }

    fn instance(&self, cx: &App) -> Option<Instance> {
        self.state.read(cx).instance(&self.id).cloned()
    }

    fn save(&mut self, inst: Instance, cx: &mut Context<Self>) {
        let result = self.state.update(cx, |s, cx| s.save_instance(inst, cx));
        self.save_error = result.err().map(|e| format!("Could not save: {e:#}"));
        cx.notify();
    }

    /// Takes every valid field into the instance and saves it.
    fn commit(&mut self, cx: &mut Context<Self>) {
        let Some(mut inst) = self.instance(cx) else { return };
        let text = |input: &Entity<TextInput>, cx: &App| input.read(cx).text().trim().to_string();
        let name = text(&self.name, cx);
        inst.name = if name.is_empty() { inst.minecraft.clone() } else { name };
        inst.group = text(&self.group, cx);

        self.memory_error = None;
        match text(&self.memory, cx).as_str() {
            "" => inst.memory_mb = None,
            m => match m.parse::<u32>() {
                Ok(mb) if mb >= MIN_MEMORY_MB => inst.memory_mb = Some(mb),
                _ => self.memory_error = Some(format!("At least {MIN_MEMORY_MB} MB")),
            },
        }
        let java = self.java.read(cx).text(cx);
        self.java_error = None;
        if java.is_empty() || Path::new(&java).is_file() {
            inst.java_path = java;
        } else {
            self.java_error = Some(format!("No Java at {java}"));
        }
        inst.jvm_args = text(&self.jvm_args, cx);

        let size = |t: String| match t.as_str() {
            "" => Ok(None),
            t => t.parse::<u32>().ok().filter(|&v| v > 0).map(Some).ok_or(()),
        };
        self.size_error = None;
        match (size(text(&self.width, cx)), size(text(&self.height, cx))) {
            (Ok(w), Ok(h)) if w.is_some() == h.is_some() => (inst.window_width, inst.window_height) = (w, h),
            (Ok(_), Ok(_)) => self.size_error = Some("Set both width and height, or neither".into()),
            _ => self.size_error = Some("Width and height are numbers of pixels".into()),
        }
        self.save(inst, cx);
    }

    fn change(&mut self, f: impl FnOnce(&mut Instance), cx: &mut Context<Self>) {
        let Some(mut inst) = self.instance(cx) else { return };
        f(&mut inst);
        self.open_menu = None;
        self.save(inst, cx);
    }

    fn set_loader(&mut self, loader: Loader, cx: &mut Context<Self>) {
        self.change(
            |i| {
                if i.loader != loader {
                    i.loader = loader;
                    i.loader_version.clear();
                }
            },
            cx,
        );
    }

    fn set_minecraft(&mut self, mc: String, cx: &mut Context<Self>) {
        self.change(
            |i| {
                if i.minecraft != mc {
                    i.minecraft = mc;
                    i.loader_version.clear();
                }
            },
            cx,
        );
    }

    /// Loads the loader versions for the instance's current loader and game version, once.
    fn ensure_loader_versions(&mut self, inst: &Instance, cx: &mut Context<Self>) {
        if inst.loader == Loader::Vanilla {
            return;
        }
        let key = (inst.loader, inst.minecraft.clone());
        if self.loader_versions.as_ref().is_some_and(|(l, m, _)| (*l, m.clone()) == key) {
            return;
        }
        self.loader_versions = Some((key.0, key.1.clone(), None));
        cx.spawn(async move |this, cx| {
            let (l, mc) = key.clone();
            let result = cx.background_spawn(async move { loader::versions(l, &mc) }).await;
            let _ = this.update(cx, |this, cx| {
                if let Some((l, m, slot)) = &mut this.loader_versions
                    && (*l, m.clone()) == key
                {
                    *slot = Some(result.map_err(|e| format!("{e:#}")));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn pick_icon(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use as icon".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |this, cx| {
                let Some(mut inst) = this.instance(cx) else { return };
                match inst.set_icon(&path) {
                    Ok(()) => this.save(inst, cx),
                    Err(e) => {
                        this.save_error = Some(format!("{e:#}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn clear_icon(&mut self, cx: &mut Context<Self>) {
        let Some(mut inst) = self.instance(cx) else { return };
        match inst.clear_icon() {
            Ok(()) => self.save(inst, cx),
            Err(e) => self.save_error = Some(format!("{e:#}")),
        }
        cx.notify();
    }

    // ---- rendering ---------------------------------------------------------------

    fn general(&self, inst: &Instance, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let dir = inst.game_dir.clone();
        let icon = div()
            .size(px(56.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .overflow_hidden()
            .bg(t.tile)
            .border_1()
            .border_color(t.tile_edge)
            .map(|d| match &inst.icon {
                Some(p) => d.child(img(p.clone()).size_full()),
                None => d.child(svg().path("icons/box.svg").size(px(24.)).text_color(t.subtle)),
            });
        let has_icon = inst.icon.is_some();
        group(
            [
                row("Icon", Some(hint("PNG or JPEG; shown in the launcher.", t)), t)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(has_icon, |d| {
                                d.child(
                                    ui::button("clear-icon", None, "Remove", Style::Ghost, true, t)
                                        .on_click(cx.listener(|this, _, _, cx| this.clear_icon(cx))),
                                )
                            })
                            .child(
                                ui::button("pick-icon", None, "Choose…", Style::Secondary, true, t)
                                    .on_click(cx.listener(|this, _, _, cx| this.pick_icon(cx))),
                            )
                            .child(icon),
                    )
                    .into_any_element(),
                stacked("Name", t).child(self.name.clone()).into_any_element(),
                stacked("Group", t)
                    .child(self.group.clone())
                    .child(hint("Instances with the same group are shown together.", t))
                    .into_any_element(),
                row("Game folder", Some(hint(dir.display().to_string(), t).truncate()), t)
                    .child(
                        ui::button(
                            "open-game-dir",
                            Some("icons/folder.svg"),
                            "Open",
                            Style::Secondary,
                            true,
                            t,
                        )
                        .on_click(move |_, _, cx| {
                            let _ = std::fs::create_dir_all(&dir);
                            cx.open_with_system(&dir);
                        }),
                    )
                    .into_any_element(),
            ],
            t,
        )
        .into_any_element()
    }

    fn versions(&mut self, inst: &Instance, running: bool, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        self.ensure_loader_versions(inst, cx);
        let open = self.open_menu;
        let releases = match &self.releases {
            Some(Ok(list)) => list.clone(),
            _ => Vec::new(),
        };
        let current = inst.minecraft.clone();
        let mc_dropdown = Dropdown::new("mc-version", "Minecraft", current.clone(), t)
            .options(releases.iter().map(|v| (SharedString::from(v.clone()), *v == current)))
            .open(open == Some(Menu::Minecraft) && !running)
            .on_toggle(cx.listener(move |this, _, _, cx| {
                if !running {
                    this.open_menu =
                        if this.open_menu == Some(Menu::Minecraft) { None } else { Some(Menu::Minecraft) };
                    cx.notify();
                }
            }))
            .on_dismiss(cx.listener(|this, _, _, cx| {
                this.open_menu = None;
                cx.notify();
            }))
            .on_select(cx.processor(move |this, ix: usize, _, cx| {
                if let Some(v) = releases.get(ix) {
                    this.set_minecraft(v.clone(), cx);
                }
            }));

        let loader_picker = segments(
            LOADERS.map(|(loader, label)| {
                segment(label, inst.loader == loader, t).px_2p5().when(!running, |s| {
                    s.on_click(cx.listener(move |this, _, _, cx| this.set_loader(loader, cx)))
                })
            }),
            t,
        )
        .flex_none();

        let mut rows = vec![
            row("Minecraft version", None, t).child(mc_dropdown).into_any_element(),
            stacked("Mod loader", t).child(loader_picker).into_any_element(),
        ];
        if inst.loader != Loader::Vanilla {
            let (list, note): (Vec<LoaderVersion>, Option<gpui::Div>) =
                match self.loader_versions.as_ref().map(|(.., l)| l) {
                    Some(Some(Ok(list))) if list.is_empty() => (
                        Vec::new(),
                        Some(error(
                            format!("{} has no builds for Minecraft {}", inst.loader.label(), inst.minecraft),
                            t,
                        )),
                    ),
                    Some(Some(Ok(list))) => (list.iter().take(80).cloned().collect(), None),
                    Some(Some(Err(e))) => {
                        (Vec::new(), Some(error(format!("Could not load versions: {e}"), t)))
                    }
                    _ => (Vec::new(), Some(hint("Loading versions…", t))),
                };
            let value = if inst.loader_version.is_empty() {
                "Latest stable".to_string()
            } else {
                inst.loader_version.clone()
            };
            let current = inst.loader_version.clone();
            let picked = list.clone();
            let dropdown = Dropdown::new("loader-version", "Version", value, t)
                .options(std::iter::once((SharedString::from("Latest stable"), current.is_empty())).chain(
                    list.iter().map(|v| {
                        let label =
                            if v.stable { v.version.clone() } else { format!("{} (beta)", v.version) };
                        (SharedString::from(label), v.version == current)
                    }),
                ))
                .open(open == Some(Menu::LoaderVersion) && !running)
                .on_toggle(cx.listener(move |this, _, _, cx| {
                    if !running {
                        this.open_menu = if this.open_menu == Some(Menu::LoaderVersion) {
                            None
                        } else {
                            Some(Menu::LoaderVersion)
                        };
                        cx.notify();
                    }
                }))
                .on_dismiss(cx.listener(|this, _, _, cx| {
                    this.open_menu = None;
                    cx.notify();
                }))
                .on_select(cx.processor(move |this, ix: usize, _, cx| {
                    let version = ix
                        .checked_sub(1)
                        .and_then(|i| picked.get(i))
                        .map(|v| v.version.clone())
                        .unwrap_or_default();
                    this.change(|i| i.loader_version = version, cx);
                }));
            rows.push(
                row(
                    "Loader version",
                    Some(note.unwrap_or_else(|| {
                        hint(
                            if inst.loader_version.is_empty() {
                                "The newest stable build is installed on the next launch."
                            } else {
                                "Installed on the next launch if needed."
                            },
                            t,
                        )
                    })),
                    t,
                )
                .child(dropdown)
                .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(group(rows, t))
            .child(div().px_1().child(hint(
                if running {
                    "Close the game to change versions."
                } else {
                    "Changing versions can break worlds and mods: make a copy of the instance first if unsure."
                },
                t,
            )))
            .into_any_element()
    }

    fn java(&self, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let memory = self.memory.read(cx).text().trim().to_string();
        let global_args = self.state.read(cx).settings.jvm_args.trim().to_string();
        group(
            [
                stacked("Java executable", t)
                    .child(self.java.clone())
                    .children(self.java_error.clone().map(|e| error(e, t)))
                    .into_any_element(),
                row(
                    "Memory",
                    Some(match &self.memory_error {
                        Some(e) => error(e.clone(), t),
                        None => hint("Empty = the launcher setting.", t),
                    }),
                    t,
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1p5()
                        .children(MEMORY_PRESETS.map(|mb| {
                            chip(mb, memory == mb.to_string(), t).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.memory.update(cx, |i, cx| i.set_text(mb.to_string(), cx));
                                    this.commit(cx);
                                },
                            ))
                        }))
                        .child(div().w(px(76.)).ml_1().child(self.memory.clone()))
                        .child(div().text_sm().text_color(t.muted).child("MB")),
                )
                .into_any_element(),
                stacked("JVM arguments", t)
                    .child(self.jvm_args.clone())
                    .child(hint(
                        match global_args.as_str() {
                            "" => "Added to the JVM arguments from the launcher settings.".to_string(),
                            args => format!("Added after the launcher's arguments: {args}"),
                        },
                        t,
                    ))
                    .into_any_element(),
            ],
            t,
        )
        .into_any_element()
    }

    fn window(&self, inst: &Instance, t: Theme, cx: &mut Context<Self>) -> AnyElement {
        let settings = &self.state.read(cx).settings;
        let global_mode = if settings.fullscreen { "fullscreen" } else { "windowed" };
        let fullscreen = inst.fullscreen;
        group(
            [
                row(
                    "Window size",
                    Some(match &self.size_error {
                        Some(e) => error(e.clone(), t),
                        None => hint("Empty = the launcher setting.", t),
                    }),
                    t,
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(76.)).child(self.width.clone()))
                        .child(div().text_sm().text_color(t.muted).child("×"))
                        .child(div().w(px(76.)).child(self.height.clone())),
                )
                .into_any_element(),
                row(
                    "Display",
                    fullscreen.is_none().then(|| hint(format!("Follows the launcher: {global_mode}."), t)),
                    t,
                )
                .child(
                    segments(
                        [(None, "Default"), (Some(false), "Windowed"), (Some(true), "Fullscreen")].map(
                            |(mode, label)| {
                                segment(label, fullscreen == mode, t).px_2p5().on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.change(|i| i.fullscreen = mode, cx)
                                    }),
                                )
                            },
                        ),
                        t,
                    )
                    .flex_none(),
                )
                .into_any_element(),
            ],
            t,
        )
        .into_any_element()
    }
}

fn heading(text: &'static str, t: Theme) -> gpui::Div {
    div().px_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child(text)
}

impl Render for InstanceSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let Some(inst) = self.instance(cx) else { return div().into_any_element() };
        let running = self.state.read(cx).is_running(&self.id);
        div()
            .id("instance-settings")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(680.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .px_6()
                    .py_5()
                    .when_some(self.save_error.clone(), |d, e| d.child(error(e, t)))
                    .child(heading("GENERAL", t))
                    .child(self.general(&inst, t, cx))
                    .child(heading("VERSION", t).mt_3())
                    .child(self.versions(&inst, running, t, cx))
                    .child(heading("JAVA", t).mt_3())
                    .child(self.java(t, cx))
                    .child(heading("GAME WINDOW", t).mt_3())
                    .child(self.window(&inst, t, cx)),
            )
            .into_any_element()
    }
}
