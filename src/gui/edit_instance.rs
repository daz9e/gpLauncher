//! "Edit Instance" dialog: name, group, memory and JVM arguments of one instance.

use gplauncher::instance::Instance;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, Window, actions, div,
    prelude::*, px,
};

use crate::add_instance::{button, field};
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

pub enum EditInstanceEvent {
    Saved(Instance),
    Dismissed,
}

pub struct EditInstance {
    focus_handle: FocusHandle,
    instance: Instance,
    name: Entity<TextInput>,
    group: Entity<TextInput>,
    memory: Entity<TextInput>,
    jvm_args: Entity<TextInput>,
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
        default_memory: u32,
        focus: Focus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |placeholder: String, text: String, cx: &mut Context<Self>| {
            let input = cx.new(|cx| TextInput::new(placeholder, cx));
            input.update(cx, |i, cx| i.set_text(text, cx));
            cx.subscribe(&input, |this: &mut Self, _, _: &text_input::Changed, cx| {
                this.error = None;
                cx.notify();
            })
            .detach();
            input
        };
        let name = input(instance.minecraft.clone(), instance.name.clone(), cx);
        let group = input("Ungrouped".into(), instance.group.clone(), cx);
        let memory = input(
            format!("Default ({default_memory})"),
            instance.memory_mb.map(|m| m.to_string()).unwrap_or_default(),
            cx,
        );
        let jvm_args = input("None".into(), instance.jvm_args.clone(), cx);
        let target = if focus == Focus::Group { &group } else { &name };
        window.focus(&target.focus_handle(cx));
        EditInstance { focus_handle: cx.focus_handle(), instance, name, group, memory, jvm_args, error: None }
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        let memory = self.memory.read(cx).text().trim();
        let memory_mb = match memory {
            "" => None,
            m => match m.parse::<u32>() {
                Ok(mb) if mb >= 512 => Some(mb),
                _ => {
                    self.error = Some("Memory must be a number of megabytes, at least 512".into());
                    cx.notify();
                    return;
                }
            },
        };
        let mut inst = self.instance.clone();
        let name = self.name.read(cx).text().trim();
        inst.name = if name.is_empty() { inst.minecraft.clone() } else { name.to_string() };
        inst.group = self.group.read(cx).text().trim().to_string();
        inst.memory_mb = memory_mb;
        inst.jvm_args = self.jvm_args.read(cx).text().trim().to_string();
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
}

impl Render for EditInstance {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        div()
            .id("edit-instance")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::cancel))
            .w(px(420.))
            .max_w_full()
            .flex()
            .flex_col()
            .rounded_xl()
            .border_1()
            .border_color(t.border)
            .bg(t.bg)
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .px_5()
                    .pt_4()
                    .pb_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child("Edit Instance"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .child(field("Name", t).child(self.name.clone()))
                    .child(field("Group", t).child(self.group.clone()))
                    .child(field("Memory, MB", t).child(self.memory.clone()))
                    .child(field("JVM arguments", t).child(self.jvm_args.clone())),
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
