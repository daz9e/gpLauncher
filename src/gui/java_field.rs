//! Java executable field with Browse and a `java -version` test; used by both settings dialogs.

use std::path::PathBuf;

use gplauncher::java;
use gpui::{App, Context, Entity, PathPromptOptions, Window, div, prelude::*};

use crate::add_instance::button;
use crate::settings_page::{hint, link};
use crate::text_input::{self, TextInput};
use crate::theme::Theme;

/// Result of running `java -version` on the chosen binary.
enum JavaCheck {
    Idle,
    Running,
    Done(Result<String, String>),
}

pub struct JavaField {
    input: Entity<TextInput>,
    check: JavaCheck,
    /// Shown while the field is empty: what happens without a binary.
    empty_hint: &'static str,
    /// Link that clears the field.
    reset_label: &'static str,
}

impl JavaField {
    pub fn new(
        placeholder: String,
        path: String,
        empty_hint: &'static str,
        reset_label: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new(placeholder, cx));
        input.update(cx, |i, cx| i.set_text(path, cx));
        cx.subscribe(&input, |this: &mut Self, _, _: &text_input::Changed, cx| {
            this.check = JavaCheck::Idle;
            cx.notify();
        })
        .detach();
        JavaField { input, check: JavaCheck::Idle, empty_hint, reset_label }
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    pub fn text(&self, cx: &App) -> String {
        self.input.read(cx).text().trim().to_string()
    }

    fn pick(&mut self, cx: &mut Context<Self>) {
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
                this.input.update(cx, |i, cx| i.set_text(path.to_string_lossy().into_owned(), cx));
                this.test(cx);
            });
        })
        .detach();
    }

    fn test(&mut self, cx: &mut Context<Self>) {
        let path = PathBuf::from(self.text(cx));
        if path.as_os_str().is_empty() {
            return;
        }
        self.check = JavaCheck::Running;
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { java::version(&path) }).await;
            let _ = this.update(cx, |this, cx| {
                // Only when the path was not edited meanwhile.
                if matches!(this.check, JavaCheck::Running) {
                    this.check = JavaCheck::Done(result.map_err(|e| format!("{e:#}")));
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for JavaField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        let has_path = !self.text(cx).is_empty();
        let testing = matches!(self.check, JavaCheck::Running);
        let status = match &self.check {
            _ if !has_path => hint(self.empty_hint, t),
            JavaCheck::Idle => hint("Press Test to check this Java.", t),
            JavaCheck::Running => hint("Checking…", t),
            JavaCheck::Done(Ok(version)) => div().text_xs().text_color(t.text).child(version.clone()),
            JavaCheck::Done(Err(e)) => div().text_xs().text_color(t.danger).child(e.clone()),
        };
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().flex().flex_col().child(self.input.clone()))
                    .child(
                        button("browse-java", "Browse…", true, false, t)
                            .on_click(cx.listener(|this, _, _, cx| this.pick(cx))),
                    )
                    .child(
                        button("test-java", "Test", has_path && !testing, false, t)
                            .when(has_path && !testing, |b| {
                                b.on_click(cx.listener(|this, _, _, cx| this.test(cx)))
                            }),
                    ),
            )
            .child(status)
            .when(has_path, |d| {
                d.child(link("reset-java", self.reset_label, t).on_click(cx.listener(|this, _, _, cx| {
                    this.input.update(cx, |i, cx| i.set_text("", cx));
                    this.check = JavaCheck::Idle;
                    cx.notify();
                })))
            })
    }
}
