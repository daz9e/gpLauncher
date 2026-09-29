//! "Settings" dialog: launcher folder and behavior, Java, game window, service keys.

use std::path::{Path, PathBuf};

use gplauncher::java;
use gplauncher::settings::{Appearance, OnLaunch, Settings};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, PathPromptOptions,
    SharedString, Window, actions, div, prelude::*, px, svg,
};

use crate::add_instance::{button, field};
use crate::text_input::{self, TextInput};
use crate::theme::Theme;

actions!(settings_dialog, [Cancel, Save]);

const CONTEXT: &str = "Settings";
const CURSEFORGE_CONSOLE: &str = "https://console.curseforge.com/";
const MIN_MEMORY_MB: u32 = 512;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("enter", Save, Some(CONTEXT)),
    ]);
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
    Saved(Settings),
    Dismissed,
}

/// Result of running `java -version` on the chosen binary.
enum JavaCheck {
    Idle,
    Running,
    Done(Result<String, String>),
}

pub struct SettingsDialog {
    focus_handle: FocusHandle,
    /// The settings the dialog was opened with; fields not shown here are kept.
    settings: Settings,
    section: Section,
    /// Games or jobs are running, so the launcher folder can not change.
    busy: bool,
    data_dir: PathBuf,
    appearance: Appearance,
    on_launch: OnLaunch,
    fullscreen: bool,
    java_path: Entity<TextInput>,
    memory: Entity<TextInput>,
    jvm_args: Entity<TextInput>,
    width: Entity<TextInput>,
    height: Entity<TextInput>,
    curseforge_key: Entity<TextInput>,
    client_id: Entity<TextInput>,
    java_check: JavaCheck,
    error: Option<String>,
}

impl EventEmitter<SettingsEvent> for SettingsDialog {}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SettingsDialog {
    pub fn new(settings: Settings, busy: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &str, text: String, cx: &mut Context<Self>| {
            let input = cx.new(|cx| TextInput::new(placeholder.to_string(), cx));
            input.update(cx, |i, cx| i.set_text(text, cx));
            cx.subscribe(&input, |this: &mut Self, _, _: &text_input::Changed, cx| {
                this.error = None;
                cx.notify();
            })
            .detach();
            input
        };
        let size = |v: Option<u32>| v.map(|v| v.to_string()).unwrap_or_default();
        let s = &settings;
        let java_path = input("Automatic", s.java_path.clone(), cx);
        cx.subscribe(&java_path, |this: &mut Self, _, _: &text_input::Changed, _| {
            this.java_check = JavaCheck::Idle;
        })
        .detach();
        let memory = input(&Settings::default().memory_mb.to_string(), s.memory_mb.to_string(), cx);
        let jvm_args = input("None", s.jvm_args.clone(), cx);
        let width = input("854", size(s.window_width), cx);
        let height = input("480", size(s.window_height), cx);
        let curseforge_key = input("API key", s.curseforge_api_key.clone(), cx);
        let client_id = input("Application (client) ID", s.ms_client_id.clone(), cx);
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        SettingsDialog {
            focus_handle,
            section: Section::General,
            busy,
            data_dir: s.data_dir.clone(),
            appearance: s.appearance,
            on_launch: s.on_launch,
            fullscreen: s.fullscreen,
            java_path,
            memory,
            jvm_args,
            width,
            height,
            curseforge_key,
            client_id,
            java_check: JavaCheck::Idle,
            error: None,
            settings,
        }
    }

    fn text(&self, input: &Entity<TextInput>, cx: &App) -> String {
        input.read(cx).text().trim().to_string()
    }

    /// Shows `error` on the section it belongs to.
    fn fail(&mut self, section: Section, error: impl Into<String>, cx: &mut Context<Self>) {
        self.section = section;
        self.error = Some(error.into());
        cx.notify();
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        let mut s = self.settings.clone();

        s.memory_mb = match self.text(&self.memory, cx).as_str() {
            "" => Settings::default().memory_mb,
            m => match m.parse::<u32>() {
                Ok(mb) if mb >= MIN_MEMORY_MB => mb,
                _ => {
                    let error = format!("Memory must be a number of megabytes, at least {MIN_MEMORY_MB}");
                    return self.fail(Section::Java, error, cx);
                }
            },
        };
        s.java_path = self.text(&self.java_path, cx);
        if !s.java_path.is_empty() && !Path::new(&s.java_path).is_file() {
            return self.fail(Section::Java, format!("No Java at {}", s.java_path), cx);
        }
        s.jvm_args = self.text(&self.jvm_args, cx);

        let size = |text: String| match text.as_str() {
            "" => Ok(None),
            t => t.parse::<u32>().ok().filter(|&v| v > 0).map(Some).ok_or(()),
        };
        match (size(self.text(&self.width, cx)), size(self.text(&self.height, cx))) {
            (Ok(w), Ok(h)) if w.is_some() == h.is_some() => (s.window_width, s.window_height) = (w, h),
            (Ok(_), Ok(_)) => return self.fail(Section::Game, "Set both width and height, or neither", cx),
            _ => return self.fail(Section::Game, "Window size must be a number of pixels", cx),
        }
        s.fullscreen = self.fullscreen;

        s.data_dir = self.data_dir.clone();
        s.appearance = self.appearance;
        s.on_launch = self.on_launch;
        s.curseforge_api_key = self.text(&self.curseforge_key, cx);
        s.ms_client_id = self.text(&self.client_id, cx);
        cx.emit(SettingsEvent::Saved(s));
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Dismissed);
    }

    fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
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
            let _ = this.update(cx, |this, cx| {
                this.data_dir = dir;
                cx.notify();
            });
        })
        .detach();
    }

    fn pick_java(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |this, cx| {
                this.java_path.update(cx, |i, cx| i.set_text(path.to_string_lossy().into_owned(), cx));
                this.test_java(cx);
            });
        })
        .detach();
    }

    fn test_java(&mut self, cx: &mut Context<Self>) {
        let path = PathBuf::from(self.text(&self.java_path, cx));
        if path.as_os_str().is_empty() {
            return;
        }
        self.java_check = JavaCheck::Running;
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { java::version(&path) }).await;
            let _ = this.update(cx, |this, cx| {
                // Only when the path was not edited meanwhile.
                if matches!(this.java_check, JavaCheck::Running) {
                    this.java_check = JavaCheck::Done(result.map_err(|e| format!("{e:#}")));
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    // ---- rendering -----------------------------------------------------------

    fn nav(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
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
            .children(SECTIONS.into_iter().map(|(section, label, icon)| {
                let active = self.section == section;
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
                    .when(active, |d| {
                        d.bg(t.accent_soft).text_color(t.accent).font_weight(FontWeight::MEDIUM)
                    })
                    .when(!active, |d| d.text_color(t.text).hover(|d| d.bg(t.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| this.show(section, cx)))
                    .child(svg().path(icon).size(px(15.)).text_color(if active { t.accent } else { t.muted }))
                    .child(label)
            }))
    }

    fn general(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let dir = self.data_dir.clone();
        let moved = self.data_dir != self.settings.data_dir;
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(field("Appearance", t).child(segments(
                [Appearance::System, Appearance::Light, Appearance::Dark].map(|a| {
                    let label = match a {
                        Appearance::System => "System",
                        Appearance::Light => "Light",
                        Appearance::Dark => "Dark",
                    };
                    segment(label, self.appearance == a, t).on_click(cx.listener(move |this, _, _, cx| {
                        this.appearance = a;
                        cx.notify();
                    }))
                }),
                t,
            )))
            .child(
                field("When the game starts", t)
                    .child(segments(
                        [(OnLaunch::KeepOpen, "Keep the launcher open"), (OnLaunch::Minimize, "Minimize")]
                            .map(|(mode, label)| {
                                segment(label, self.on_launch == mode, t).on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.on_launch = mode;
                                        cx.notify();
                                    },
                                ))
                            }),
                        t,
                    ))
                    .when(self.on_launch == OnLaunch::Minimize, |d| {
                        d.child(hint("The launcher comes back when the game closes.", t))
                    }),
            )
            .child(
                field("Launcher folder", t)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(path_box(self.data_dir.display().to_string(), t))
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
                    .child(hint(
                        if self.busy {
                            "Can not be changed while a game or a job is running."
                        } else if moved {
                            "Instances, versions and Java runtimes are not moved: \
                             the launcher starts over in the new folder."
                        } else {
                            "Holds instances, versions, libraries, assets and Java runtimes."
                        },
                        t,
                    ))
                    .when(moved, |d| {
                        d.child(link("reset-data-dir", "Use the previous folder", t).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.data_dir = this.settings.data_dir.clone();
                                cx.notify();
                            },
                        )))
                    }),
            )
    }

    fn java(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let has_path = !self.java_path.read(cx).text().trim().is_empty();
        let testing = matches!(self.java_check, JavaCheck::Running);
        let status = match &self.java_check {
            _ if !has_path => hint(
                "Each Minecraft version gets the Java it needs, downloaded from Mojang. \
                 Choose a binary only to use your own Java for every instance.",
                t,
            ),
            JavaCheck::Idle => hint("Press Test to check this Java.", t),
            JavaCheck::Running => hint("Checking…", t),
            JavaCheck::Done(Ok(version)) => div().text_xs().text_color(t.text).child(version.clone()),
            JavaCheck::Done(Err(e)) => div().text_xs().text_color(t.danger).child(e.clone()),
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                field("Java executable", t)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(self.java_path.clone()))
                            .child(
                                button("browse-java", "Browse…", true, false, t)
                                    .on_click(cx.listener(|this, _, _, cx| this.pick_java(cx))),
                            )
                            .child(
                                button("test-java", "Test", has_path && !testing, false, t)
                                    .when(has_path && !testing, |b| {
                                        b.on_click(cx.listener(|this, _, _, cx| this.test_java(cx)))
                                    }),
                            ),
                    )
                    .child(status)
                    .when(has_path, |d| {
                        d.child(link("auto-java", "Use automatic Java", t).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.java_path.update(cx, |i, cx| i.set_text("", cx));
                                this.java_check = JavaCheck::Idle;
                                cx.notify();
                            },
                        )))
                    }),
            )
            .child(
                field("Memory, MB", t)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(120.)).child(self.memory.clone()))
                            .children([2048, 4096, 6144, 8192].map(|mb| {
                                let active = self.memory.read(cx).text().trim() == mb.to_string();
                                chip(mb, active, t).on_click(cx.listener(move |this, _, _, cx| {
                                    this.memory.update(cx, |i, cx| i.set_text(mb.to_string(), cx));
                                    this.error = None;
                                    cx.notify();
                                }))
                            })),
                    )
                    .child(hint("Default for instances without their own memory setting.", t)),
            )
            .child(
                field("JVM arguments", t)
                    .child(self.jvm_args.clone())
                    .child(hint("Passed to every instance, before the instance's own arguments.", t)),
            )
    }

    fn game(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                field("Window size", t)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(90.)).child(self.width.clone()))
                            .child(div().text_sm().text_color(t.muted).child("×"))
                            .child(div().w(px(90.)).child(self.height.clone())),
                    )
                    .child(hint("Leave empty for the game's default.", t)),
            )
            .child(checkbox("fullscreen", "Start in fullscreen", self.fullscreen, t).on_click(cx.listener(
                |this, _, _, cx| {
                    this.fullscreen = !this.fullscreen;
                    cx.notify();
                },
            )))
    }

    fn services(&self, t: Theme, _: &mut Context<Self>) -> impl IntoElement {
        let env_key = std::env::var("CURSEFORGE_API_KEY").is_ok_and(|k| !k.trim().is_empty());
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                field("CurseForge API key", t)
                    .child(self.curseforge_key.clone())
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
                            .on_click(|_, _, cx| cx.open_url(CURSEFORGE_CONSOLE)),
                    ),
            )
            .child(field("Microsoft client ID", t).child(self.client_id.clone()).child(hint(
                "The Azure application used to sign in with Microsoft accounts. \
                 Mojang has to allow it to use the Minecraft API.",
                t,
            )))
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let title =
            SECTIONS.iter().find(|(s, ..)| *s == self.section).map(|(_, l, _)| *l).unwrap_or_default();
        let content = match self.section {
            Section::General => self.general(t, cx).into_any_element(),
            Section::Java => self.java(t, cx).into_any_element(),
            Section::Game => self.game(t, cx).into_any_element(),
            Section::Services => self.services(t, cx).into_any_element(),
        };
        div()
            .id("settings")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::cancel))
            .w(px(660.))
            .h(px(470.))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .rounded_xl()
            .border_1()
            .border_color(t.border)
            .bg(t.bg)
            .shadow_lg()
            .overflow_hidden()
            .child(
                div().flex_1().min_h_0().flex().child(self.nav(t, cx)).child(
                    div()
                        .id("settings-body")
                        .flex_1()
                        .min_w_0()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .px_5()
                        .py_4()
                        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title))
                        .child(content),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(t.border)
                    .child(
                        div().flex_1().min_w_0().text_xs().text_color(t.danger).children(self.error.clone()),
                    )
                    .child(
                        button("cancel", "Cancel", true, false, t)
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx))),
                    )
                    .child(
                        button("save", "Save", true, true, t)
                            .on_click(cx.listener(|this, _, window, cx| this.save(&Save, window, cx))),
                    ),
            )
    }
}

fn hint(text: &'static str, t: Theme) -> gpui::Div {
    div().text_xs().text_color(t.subtle).child(text)
}

fn link(id: &'static str, label: &'static str, t: Theme) -> gpui::Stateful<gpui::Div> {
    div().id(id).text_xs().text_color(t.accent).cursor_pointer().hover(|d| d.underline()).child(label)
}

/// A read-only path.
fn path_box(path: String, t: Theme) -> impl IntoElement {
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

fn segments(items: impl IntoIterator<Item = gpui::Stateful<gpui::Div>>, t: Theme) -> gpui::Div {
    div().flex().p_0p5().gap_0p5().rounded_md().bg(t.tile).children(items)
}

/// One option of a segmented control; matches the loader picker of "Add Instance".
fn segment(label: &'static str, active: bool, t: Theme) -> gpui::Stateful<gpui::Div> {
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
fn chip(mb: u32, active: bool, t: Theme) -> gpui::Stateful<gpui::Div> {
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

fn checkbox(id: &'static str, label: &'static str, on: bool, t: Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .text_sm()
        .text_color(t.text)
        .cursor_pointer()
        .child(
            div()
                .size(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .when(on, |d| d.bg(t.accent).border_color(t.accent))
                .when(!on, |d| d.bg(t.bg).border_color(t.subtle))
                .when(on, |d| d.child(svg().path("icons/check.svg").size(px(12.)).text_color(t.on_accent))),
        )
        .child(label)
}
