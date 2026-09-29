//! "Edit Instance" dialog: name and group, plus Java and game window settings that override the
//! launcher's for this instance.

use std::path::Path;

use gplauncher::instance::Instance;
use gplauncher::settings::Settings;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, Window, actions, div,
    prelude::*, px,
};

use crate::add_instance::{button, field};
use crate::java_field::JavaField;
use crate::settings_dialog::{
    MEMORY_PRESETS, MIN_MEMORY_MB, chip, hint, nav_item, nav_panel, path_box, segment, segments,
};
use crate::text_input::{self, TextInput};
use crate::theme::Theme;

actions!(edit_instance, [Cancel, Save]);

const CONTEXT: &str = "EditInstance";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("enter", Save, Some(CONTEXT)),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Name,
    Group,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    Java,
    Game,
}

const SECTIONS: [(Section, &str, &str); 3] = [
    (Section::General, "General", "icons/settings.svg"),
    (Section::Java, "Java", "icons/coffee.svg"),
    (Section::Game, "Game", "icons/monitor.svg"),
];

pub enum EditInstanceEvent {
    Saved(Instance),
    Dismissed,
}

pub struct EditInstance {
    focus_handle: FocusHandle,
    instance: Instance,
    /// Launcher settings, shown as the defaults of empty fields.
    settings: Settings,
    section: Section,
    name: Entity<TextInput>,
    group: Entity<TextInput>,
    java: Entity<JavaField>,
    memory: Entity<TextInput>,
    jvm_args: Entity<TextInput>,
    width: Entity<TextInput>,
    height: Entity<TextInput>,
    fullscreen: Option<bool>,
    error: Option<String>,
}

impl EventEmitter<EditInstanceEvent> for EditInstance {}

impl Focusable for EditInstance {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EditInstance {
    pub fn new(
        instance: Instance,
        settings: Settings,
        focus: Focus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |placeholder: String, text: String, cx: &mut Context<Self>| {
            let input = cx.new(|cx| TextInput::new(placeholder, cx));
            input.update(cx, |i, cx| i.set_text(text, cx));
            clear_error_on_edit(&input, cx);
            input
        };
        let size = |v: Option<u32>| v.map(|v| v.to_string()).unwrap_or_default();
        let (s, inst) = (&settings, &instance);
        let name = input(inst.minecraft.clone(), inst.name.clone(), cx);
        let group = input("Ungrouped".into(), inst.group.clone(), cx);
        let java = cx.new(|cx| {
            let global = match s.java_path.trim() {
                "" => "Automatic".to_string(),
                path => path.to_string(),
            };
            JavaField::new(
                format!("Default ({global})"),
                inst.java_path.clone(),
                "Uses the Java from Settings → Java. \
                 Choose a binary only if this instance needs a different one.",
                "Use the default Java",
                cx,
            )
        });
        let java_input = java.read(cx).input().clone();
        clear_error_on_edit(&java_input, cx);
        let memory = input(format!("Default ({})", s.memory_mb), size(inst.memory_mb), cx);
        let jvm_args = input("None".into(), inst.jvm_args.clone(), cx);
        let (default_width, default_height) = s.resolution().unwrap_or((854, 480));
        let width = input(default_width.to_string(), size(inst.window_width), cx);
        let height = input(default_height.to_string(), size(inst.window_height), cx);
        let target = if focus == Focus::Group { &group } else { &name };
        window.focus(&target.focus_handle(cx));
        EditInstance {
            focus_handle: cx.focus_handle(),
            section: Section::General,
            name,
            group,
            java,
            memory,
            jvm_args,
            width,
            height,
            fullscreen: inst.fullscreen,
            error: None,
            instance,
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
        let mut inst = self.instance.clone();
        let name = self.text(&self.name, cx);
        inst.name = if name.is_empty() { inst.minecraft.clone() } else { name };
        inst.group = self.text(&self.group, cx);

        inst.memory_mb = match self.text(&self.memory, cx).as_str() {
            "" => None,
            m => match m.parse::<u32>() {
                Ok(mb) if mb >= MIN_MEMORY_MB => Some(mb),
                _ => {
                    let error = format!("Memory must be a number of megabytes, at least {MIN_MEMORY_MB}");
                    return self.fail(Section::Java, error, cx);
                }
            },
        };
        inst.java_path = self.java.read(cx).text(cx);
        if !inst.java_path.is_empty() && !Path::new(&inst.java_path).is_file() {
            return self.fail(Section::Java, format!("No Java at {}", inst.java_path), cx);
        }
        inst.jvm_args = self.text(&self.jvm_args, cx);

        let size = |text: String| match text.as_str() {
            "" => Ok(None),
            t => t.parse::<u32>().ok().filter(|&v| v > 0).map(Some).ok_or(()),
        };
        match (size(self.text(&self.width, cx)), size(self.text(&self.height, cx))) {
            (Ok(w), Ok(h)) if w.is_some() == h.is_some() => (inst.window_width, inst.window_height) = (w, h),
            (Ok(_), Ok(_)) => return self.fail(Section::Game, "Set both width and height, or neither", cx),
            _ => return self.fail(Section::Game, "Window size must be a number of pixels", cx),
        }
        inst.fullscreen = self.fullscreen;

        match inst.save() {
            Ok(()) => cx.emit(EditInstanceEvent::Saved(inst)),
            Err(e) => {
                self.error = Some(format!("Could not save: {e:#}"));
                cx.notify();
            }
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(EditInstanceEvent::Dismissed);
    }

    fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    // ---- rendering -----------------------------------------------------------

    fn nav(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        nav_panel(t)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px_2()
                    .pt_1()
                    .pb_2()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(self.instance.name.clone()),
                    )
                    .child(div().truncate().text_xs().text_color(t.muted).child(self.instance.description())),
            )
            .children(SECTIONS.into_iter().map(|(section, label, icon)| {
                nav_item(label, icon, self.section == section, t)
                    .on_click(cx.listener(move |this, _, _, cx| this.show(section, cx)))
            }))
    }

    fn general(&self, t: Theme) -> impl IntoElement {
        let dir = self.instance.game_dir.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(field("Name", t).child(self.name.clone()))
            .child(field("Group", t).child(self.group.clone()))
            .child(field("Version", t).child(path_box(self.instance.description(), t)))
            .child(field("Game folder", t).child(
                div().flex().items_center().gap_2().child(path_box(dir.display().to_string(), t)).child(
                    button("open-game-dir", "Open", true, false, t).on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&dir);
                        cx.open_with_system(&dir);
                    }),
                ),
            ))
    }

    fn java(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let global_args = self.settings.jvm_args.trim();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(field("Java executable", t).child(self.java.clone()))
            .child(
                field("Memory, MB", t)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(120.)).child(self.memory.clone()))
                            .children(MEMORY_PRESETS.map(|mb| {
                                let active = self.memory.read(cx).text().trim() == mb.to_string();
                                chip(mb, active, t).on_click(cx.listener(move |this, _, _, cx| {
                                    this.memory.update(cx, |i, cx| i.set_text(mb.to_string(), cx));
                                    this.error = None;
                                    cx.notify();
                                }))
                            })),
                    )
                    .child(hint("Leave empty for the default from Settings → Java.", t)),
            )
            .child(field("JVM arguments", t).child(self.jvm_args.clone()).child(hint(
                match global_args {
                    "" => "Added to the JVM arguments from Settings → Java.".to_string(),
                    args => format!("Added after the arguments from Settings → Java: {args}"),
                },
                t,
            )))
    }

    fn game(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let global_size = match self.settings.resolution() {
            Some((w, h)) => format!("{w} × {h}"),
            None => "the game's default".into(),
        };
        let global_mode = if self.settings.fullscreen { "fullscreen" } else { "windowed" };
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
                    .child(hint(
                        format!("Leave empty for the size from Settings → Game ({global_size})."),
                        t,
                    )),
            )
            .child(
                field("Display", t)
                    .child(segments(
                        [(None, "Default"), (Some(false), "Windowed"), (Some(true), "Fullscreen")].map(
                            |(mode, label)| {
                                segment(label, self.fullscreen == mode, t).on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.fullscreen = mode;
                                        cx.notify();
                                    },
                                ))
                            },
                        ),
                        t,
                    ))
                    .when(self.fullscreen.is_none(), |d| {
                        d.child(hint(format!("Follows Settings → Game: {global_mode}."), t))
                    }),
            )
    }
}

/// Clears the dialog's error whenever `input` is edited.
fn clear_error_on_edit(input: &Entity<TextInput>, cx: &mut Context<EditInstance>) {
    cx.subscribe(input, |this: &mut EditInstance, _, _: &text_input::Changed, cx| {
        this.error = None;
        cx.notify();
    })
    .detach();
}

impl Render for EditInstance {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let title =
            SECTIONS.iter().find(|(s, ..)| *s == self.section).map(|(_, l, _)| *l).unwrap_or_default();
        let content = match self.section {
            Section::General => self.general(t).into_any_element(),
            Section::Java => self.java(t, cx).into_any_element(),
            Section::Game => self.game(t, cx).into_any_element(),
        };
        div()
            .id("edit-instance")
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
                        .id("edit-instance-body")
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
