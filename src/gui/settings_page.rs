//! "Settings" page: launcher folder and behavior, Java, game window, service keys.
//! It fills the launcher window and applies every valid change right away.

use std::path::{Path, PathBuf};

use gplauncher::settings::{Appearance, OnLaunch, Settings};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    PathPromptOptions, SharedString, Window, actions, div, prelude::*, px, svg,
};

use crate::add_instance::button;
use crate::java_field::JavaField;
use crate::text_input::{self, TextInput};
use crate::theme::Theme;

actions!(settings_page, [Close]);

const CONTEXT: &str = "Settings";
const CURSEFORGE_CONSOLE: &str = "https://console.curseforge.com/";
pub const MIN_MEMORY_MB: u32 = 512;
pub const MEMORY_PRESETS: [u32; 4] = [2048, 4096, 6144, 8192];

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Close, Some(CONTEXT))]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    Java,
    Game,
    Services,
}

const SECTIONS: [(Section, &str, &str); 4] = [
    (Section::General, "General", "icons/settings.svg"),
    (Section::Java, "Java", "icons/coffee.svg"),
    (Section::Game, "Game", "icons/monitor.svg"),
    (Section::Services, "Services", "icons/globe.svg"),
];

pub enum SettingsEvent {
    /// The settings after a valid change; the launcher applies and saves them.
    Changed(Settings),
    Closed,
}

pub struct SettingsPage {
    focus_handle: FocusHandle,
    /// The settings as last applied; fields not shown here are kept.
    settings: Settings,
    /// The launcher folder when the page was opened, to offer going back to it.
    original_dir: PathBuf,
    section: Section,
    /// Games or jobs are running, so the launcher folder can not change.
    busy: bool,
    java: Entity<JavaField>,
    memory: Entity<TextInput>,
    jvm_args: Entity<TextInput>,
    width: Entity<TextInput>,
    height: Entity<TextInput>,
    curseforge_key: Entity<TextInput>,
    client_id: Entity<TextInput>,
    // Fields holding a value that is not applied, and why.
    memory_error: Option<String>,
    java_error: Option<String>,
    size_error: Option<String>,
}

impl EventEmitter<SettingsEvent> for SettingsPage {}

impl Focusable for SettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SettingsPage {
    pub fn new(settings: Settings, busy: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &str, text: String, cx: &mut Context<Self>| {
            let input = cx.new(|cx| TextInput::new(placeholder.to_string(), cx));
            input.update(cx, |i, cx| i.set_text(text, cx));
            cx.subscribe(&input, |this: &mut Self, _, _: &text_input::Changed, cx| this.commit(cx)).detach();
            input
        };
        let size = |v: Option<u32>| v.map(|v| v.to_string()).unwrap_or_default();
        let s = &settings;
        let java = cx.new(|cx| {
            JavaField::new(
                "Automatic".into(),
                s.java_path.clone(),
                "Each Minecraft version gets the Java it needs, downloaded from Mojang. \
                 Choose a binary only to use your own Java for every instance.",
                "Use automatic Java",
                cx,
            )
        });
        // Typing, Browse and the reset link all redraw the field.
        cx.observe(&java, |this, _, cx| this.commit(cx)).detach();
        let memory = input(&Settings::default().memory_mb.to_string(), s.memory_mb.to_string(), cx);
        let jvm_args = input("None", s.jvm_args.clone(), cx);
        let width = input("854", size(s.window_width), cx);
        let height = input("480", size(s.window_height), cx);
        let curseforge_key = input("API key", s.curseforge_api_key.clone(), cx);
        let client_id = input("Application (client) ID", s.ms_client_id.clone(), cx);
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        SettingsPage {
            focus_handle,
            original_dir: s.data_dir.clone(),
            section: Section::General,
            busy,
            java,
            memory,
            jvm_args,
            width,
            height,
            curseforge_key,
            client_id,
            memory_error: None,
            java_error: None,
            size_error: None,
            settings,
        }
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        cx.notify();
    }

    /// Takes every valid field into the settings and hands them to the launcher.
    /// Invalid fields keep their last good value and show why.
    fn commit(&mut self, cx: &mut Context<Self>) {
        let s = &mut self.settings;

        self.memory_error = None;
        match self.memory.read(cx).text().trim() {
            "" => s.memory_mb = Settings::default().memory_mb,
            m => match m.parse::<u32>() {
                Ok(mb) if mb >= MIN_MEMORY_MB => s.memory_mb = mb,
                _ => self.memory_error = Some(format!("At least {MIN_MEMORY_MB} MB")),
            },
        }

        let java = self.java.read(cx).text(cx);
        self.java_error = None;
        if java.is_empty() || Path::new(&java).is_file() {
            s.java_path = java;
        } else {
            self.java_error = Some(format!("No Java at {java}"));
        }
        s.jvm_args = self.jvm_args.read(cx).text().trim().to_string();

        let size = |text: &str| match text.trim() {
            "" => Ok(None),
            t => t.parse::<u32>().ok().filter(|&v| v > 0).map(Some).ok_or(()),
        };
        self.size_error = None;
        match (size(self.width.read(cx).text()), size(self.height.read(cx).text())) {
            (Ok(w), Ok(h)) if w.is_some() == h.is_some() => (s.window_width, s.window_height) = (w, h),
            (Ok(_), Ok(_)) => self.size_error = Some("Set both width and height, or neither".into()),
            _ => self.size_error = Some("Width and height are numbers of pixels".into()),
        }

        s.curseforge_api_key = self.curseforge_key.read(cx).text().trim().to_string();
        s.ms_client_id = self.client_id.read(cx).text().trim().to_string();
        cx.emit(SettingsEvent::Changed(self.settings.clone()));
        cx.notify();
    }

    fn close(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Closed);
    }

    fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    fn set_data_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        if !self.busy && dir != self.settings.data_dir {
            self.settings.data_dir = dir;
            self.commit(cx);
        }
    }

    fn pick_data_dir(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(dir) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |this, cx| this.set_data_dir(dir, cx));
        })
        .detach();
    }

    // ---- rendering -----------------------------------------------------------

    fn nav(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        nav_panel(t)
            .w(px(200.))
            .pt_3()
            .child(
                div()
                    .id("back")
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(30.))
                    .px_2()
                    .mb_2()
                    .rounded_md()
                    .text_sm()
                    .text_color(t.muted)
                    .cursor_pointer()
                    .hover(|d| d.bg(t.hover).text_color(t.text))
                    .on_click(cx.listener(|this, _, window, cx| this.close(&Close, window, cx)))
                    .child(svg().path("icons/chevron-left.svg").size(px(15.)).text_color(t.muted))
                    .child("Instances"),
            )
            .children(SECTIONS.into_iter().map(|(section, label, icon)| {
                nav_item(label, icon, self.section == section, t)
                    .on_click(cx.listener(move |this, _, _, cx| this.show(section, cx)))
            }))
    }

    fn general(&self, t: Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = &self.settings;
        let dir = s.data_dir.clone();
        let moved = s.data_dir != self.original_dir;
        let appearance = segments(
            [Appearance::System, Appearance::Light, Appearance::Dark].map(|a| {
                let label = match a {
                    Appearance::System => "System",
                    Appearance::Light => "Light",
                    Appearance::Dark => "Dark",
                };
                segment(label, s.appearance == a, t).px_3().on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.appearance = a;
                    this.commit(cx);
                }))
            }),
            t,
        )
        .w(px(240.));
        let minimize = s.on_launch == OnLaunch::Minimize;
        let folder_note = if self.busy {
            "Can not be changed while a game or a job is running."
        } else if moved {
            "Instances, versions and Java runtimes were not moved: the launcher starts over in the new folder."
        } else {
            "Holds instances, versions, libraries, assets and Java runtimes."
        };
        vec![
            group(
                [
                    row("Appearance", None, t).child(appearance).into_any_element(),
                    row(
                        "Minimize while playing",
                        Some(hint("The launcher comes back when the game closes.", t)),
                        t,
                    )
                    .child(switch("minimize", minimize, t).on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.on_launch =
                            if minimize { OnLaunch::KeepOpen } else { OnLaunch::Minimize };
                        this.commit(cx);
                    })))
                    .into_any_element(),
                ],
                t,
            )
            .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(group(
                    [row("Launcher folder", Some(hint(dir.display().to_string(), t).truncate()), t)
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .gap_2()
                                .child(
                                    button("change-data-dir", "Change…", !self.busy, false, t)
                                        .when(!self.busy, |b| {
                                            b.on_click(cx.listener(|this, _, _, cx| this.pick_data_dir(cx)))
                                        }),
                                )
                                .child(button("open-data-dir", "Open", true, false, t).on_click(
                                    move |_, _, cx| {
                                        let _ = std::fs::create_dir_all(&dir);
                                        cx.open_with_system(&dir);
                                    },
                                )),
                        )
                        .into_any_element()],
                    t,
                ))
                .child(div().flex().gap_2().px_1().child(hint(folder_note, t)).when(
                    moved && !self.busy,
                    |d| {
                        d.child(link("reset-data-dir", "Use the previous folder", t).flex_none().on_click(
                            cx.listener(|this, _, _, cx| {
                                let dir = this.original_dir.clone();
                                this.set_data_dir(dir, cx);
                            }),
                        ))
                    },
                ))
                .into_any_element(),
        ]
    }

    fn java(&self, t: Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let memory = self.memory.read(cx).text().trim().to_string();
        vec![
            group(
                [stacked("Java executable", t)
                    .child(self.java.clone())
                    .children(self.java_error.clone().map(|e| error(e, t)))
                    .into_any_element()],
                t,
            )
            .into_any_element(),
            group(
                [
                    row(
                        "Memory",
                        Some(match &self.memory_error {
                            Some(e) => error(e.clone(), t),
                            None => hint("For instances without their own setting.", t),
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
                        .child(hint("Passed to every instance, before the instance's own arguments.", t))
                        .into_any_element(),
                ],
                t,
            )
            .into_any_element(),
        ]
    }

    fn game(&self, t: Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let fullscreen = self.settings.fullscreen;
        vec![
            group(
                [
                    row(
                        "Window size",
                        Some(match &self.size_error {
                            Some(e) => error(e.clone(), t),
                            None => hint("Leave empty for the game's default.", t),
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
                    row("Start in fullscreen", None, t)
                        .child(switch("fullscreen", fullscreen, t).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.settings.fullscreen = !fullscreen;
                                this.commit(cx);
                            },
                        )))
                        .into_any_element(),
                ],
                t,
            )
            .into_any_element(),
        ]
    }

    fn services(&self, t: Theme, _: &mut Context<Self>) -> Vec<AnyElement> {
        let env_key = std::env::var("CURSEFORGE_API_KEY").is_ok_and(|k| !k.trim().is_empty());
        vec![
            group(
                [
                    stacked("CurseForge API key", t)
                        .child(self.curseforge_key.clone())
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(hint(
                                    if env_key {
                                        "CURSEFORGE_API_KEY is set in the environment and is used instead."
                                    } else {
                                        "Needed to browse and install CurseForge modpacks."
                                    },
                                    t,
                                ))
                                .child(
                                    link("curseforge-console", "Get a key", t)
                                        .flex_none()
                                        .on_click(|_, _, cx| cx.open_url(CURSEFORGE_CONSOLE)),
                                ),
                        )
                        .into_any_element(),
                    stacked("Microsoft client ID", t)
                        .child(self.client_id.clone())
                        .child(hint(
                            "The Azure application used to sign in with Microsoft accounts. \
                             Mojang has to allow it to use the Minecraft API.",
                            t,
                        ))
                        .into_any_element(),
                ],
                t,
            )
            .into_any_element(),
        ]
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let title =
            SECTIONS.iter().find(|(s, ..)| *s == self.section).map(|(_, l, _)| *l).unwrap_or_default();
        let content = match self.section {
            Section::General => self.general(t, cx),
            Section::Java => self.java(t, cx),
            Section::Game => self.game(t, cx),
            Section::Services => self.services(t, cx),
        };
        div()
            .id("settings")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .flex()
            .bg(t.bg)
            .child(self.nav(t, cx))
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(680.))
                            .flex()
                            .flex_col()
                            .gap_5()
                            .px_8()
                            .py_6()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child(title),
                            )
                            .children(content),
                    ),
            )
    }
}

/// A rounded card of rows, divided by hairlines.
fn group(rows: impl IntoIterator<Item = AnyElement>, t: Theme) -> gpui::Div {
    div().flex().flex_col().rounded_lg().border_1().border_color(t.border).bg(t.panel).children(
        rows.into_iter().enumerate().map(|(i, row)| {
            div().flex().flex_col().when(i > 0, |d| d.border_t_1().border_color(t.border)).child(row)
        }),
    )
}

/// A setting with its label and `detail` on the left; add the control as a child.
fn row(label: &'static str, detail: Option<gpui::Div>, t: Theme) -> gpui::Div {
    div().flex().items_center().gap_4().min_h(px(52.)).px_4().py_2p5().child(
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(div().text_sm().text_color(t.text).child(label))
            .children(detail),
    )
}

/// A setting whose control is too wide for a row: the label on top, then the children.
fn stacked(label: &'static str, t: Theme) -> gpui::Div {
    div().flex().flex_col().gap_2().px_4().py_3().child(div().text_sm().text_color(t.text).child(label))
}

fn error(text: impl Into<SharedString>, t: Theme) -> gpui::Div {
    div().text_xs().text_color(t.danger).child(text.into())
}

/// An on/off switch.
fn switch(id: &'static str, on: bool, t: Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(36.))
        .h(px(20.))
        .flex()
        .items_center()
        .when(on, |d| d.justify_end())
        .p(px(2.))
        .rounded_full()
        .bg(if on { t.accent } else { t.subtle.opacity(0.45) })
        .cursor_pointer()
        .child(div().size(px(16.)).rounded_full().bg(gpui::white()).shadow_sm())
}

pub fn hint(text: impl Into<SharedString>, t: Theme) -> gpui::Div {
    div().text_xs().text_color(t.subtle).child(text.into())
}

pub fn link(id: &'static str, label: &'static str, t: Theme) -> gpui::Stateful<gpui::Div> {
    div().id(id).text_xs().text_color(t.accent).cursor_pointer().hover(|d| d.underline()).child(label)
}

/// Left column listing the sections of a dialog.
pub fn nav_panel(t: Theme) -> gpui::Div {
    div()
        .w(px(150.))
        .flex_none()
        .flex()
        .flex_col()
        .gap_0p5()
        .p_2()
        .bg(t.panel)
        .border_r_1()
        .border_color(t.border)
}

pub fn nav_item(
    label: &'static str,
    icon: &'static str,
    active: bool,
    t: Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(label)
        .flex()
        .items_center()
        .gap_2p5()
        .h(px(30.))
        .px_2()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .when(active, |d| d.bg(t.accent_soft).text_color(t.accent).font_weight(FontWeight::MEDIUM))
        .when(!active, |d| d.text_color(t.text).hover(|d| d.bg(t.hover)))
        .child(svg().path(icon).size(px(15.)).text_color(if active { t.accent } else { t.muted }))
        .child(label)
}

/// A read-only path.
pub fn path_box(path: String, t: Theme) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .h(px(30.))
        .flex()
        .items_center()
        .px_2p5()
        .rounded_md()
        .border_1()
        .border_color(t.border)
        .bg(t.panel)
        .text_sm()
        .text_color(t.text)
        .child(div().truncate().child(path))
}

pub fn segments(items: impl IntoIterator<Item = gpui::Stateful<gpui::Div>>, t: Theme) -> gpui::Div {
    div().flex().p_0p5().gap_0p5().rounded_md().bg(t.tile).children(items)
}

/// One option of a segmented control; matches the loader picker of "Add Instance".
pub fn segment(label: &'static str, active: bool, t: Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(label)
        .flex_1()
        .flex()
        .justify_center()
        .py_1()
        .rounded(px(5.))
        .text_sm()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(t.bg).text_color(t.text).font_weight(FontWeight::MEDIUM).border_1().border_color(t.border)
        })
        .when(!active, |d| d.text_color(t.muted).hover(|d| d.text_color(t.text)))
        .child(label)
}

/// Memory preset, e.g. `4 GB`.
pub fn chip(mb: u32, active: bool, t: Theme) -> gpui::Stateful<gpui::Div> {
    let label = match mb % 1024 {
        0 => format!("{} GB", mb / 1024),
        _ => format!("{mb} MB"),
    };
    div()
        .id(SharedString::from(format!("memory-{mb}")))
        .flex_none()
        .h(px(26.))
        .flex()
        .items_center()
        .px_2()
        .rounded_md()
        .border_1()
        .text_xs()
        .cursor_pointer()
        .when(active, |d| d.bg(t.accent_soft).border_color(t.accent.opacity(0.35)).text_color(t.accent))
        .when(!active, |d| d.border_color(t.border).text_color(t.muted).hover(|d| d.bg(t.hover)))
        .child(label)
}
