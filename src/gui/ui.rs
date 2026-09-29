//! Small building blocks shared by the windows: buttons, switches, badges, tooltips, empty states.

use gpui::{
    AnyView, App, Context, FontWeight, Hsla, IntoElement, Render, SharedString, Window, div, prelude::*, px,
    relative, svg,
};

use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Filled with the accent.
    Primary,
    /// Filled green: starts a game.
    Play,
    /// Filled red: stops a game.
    Stop,
    /// Outlined.
    Secondary,
    /// No border until hovered.
    Ghost,
}

/// A button with an optional icon and label.
pub fn button(
    id: impl Into<gpui::ElementId>,
    icon: Option<&'static str>,
    label: impl Into<SharedString>,
    style: Style,
    enabled: bool,
    t: Theme,
) -> gpui::Stateful<gpui::Div> {
    let (bg, fg, border) = match (enabled, style) {
        (false, Style::Ghost) => (transparent(), t.subtle, transparent()),
        (false, _) => (transparent(), t.subtle, t.border),
        (true, Style::Primary) => (t.accent, t.on_accent, t.accent),
        (true, Style::Play) => (t.success, t.on_accent, t.success),
        (true, Style::Stop) => (t.danger, t.on_accent, t.danger),
        (true, Style::Secondary) => (t.bg, t.text, t.border),
        (true, Style::Ghost) => (transparent(), t.text, transparent()),
    };
    let filled = matches!(style, Style::Primary | Style::Play | Style::Stop);
    let label: SharedString = label.into();
    div()
        .id(id)
        .flex_none()
        .h(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .px_3()
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .when(enabled, |d| {
            d.cursor_pointer()
                .when(filled, |d| d.hover(|d| d.opacity(0.9)))
                .when(!filled, |d| d.hover(|d| d.bg(t.hover)))
        })
        .when_some(icon, |d, icon| d.child(svg().path(icon).size(px(14.)).flex_none().text_color(fg)))
        .when(!label.is_empty(), |d| d.child(label))
}

/// A square button showing only an icon; give it a [`tooltip`].
pub fn icon_button(
    id: impl Into<gpui::ElementId>,
    icon: &'static str,
    enabled: bool,
    t: Theme,
) -> gpui::Stateful<gpui::Div> {
    let color = if enabled { t.muted } else { t.subtle.opacity(0.6) };
    div()
        .id(id)
        .flex_none()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(t.hover)))
        .child(svg().path(icon).size(px(15.)).text_color(color))
}

/// An on/off switch.
pub fn switch(id: impl Into<gpui::ElementId>, on: bool, t: Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(32.))
        .h(px(18.))
        .flex()
        .items_center()
        .when(on, |d| d.justify_end())
        .p(px(2.))
        .rounded_full()
        .bg(if on { t.accent } else { t.subtle.opacity(0.45) })
        .cursor_pointer()
        .child(div().size(px(14.)).rounded_full().bg(gpui::white()).shadow_sm())
}

/// A small rounded label, e.g. `Update` or `Disabled`.
pub fn badge(text: impl Into<SharedString>, color: Hsla) -> gpui::Div {
    div()
        .flex_none()
        .px_1p5()
        .py(px(1.))
        .rounded(px(4.))
        .bg(color.opacity(0.14))
        .text_color(color)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

/// Centered icon, title and explanation for empty lists.
pub fn empty_state(
    icon: &'static str,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    t: Theme,
) -> gpui::Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .p_6()
        .child(
            div()
                .size(px(48.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(t.tile)
                .child(svg().path(icon).size(px(22.)).text_color(t.subtle)),
        )
        .child(div().mt_1().font_weight(FontWeight::MEDIUM).text_color(t.text).child(title.into()))
        .child(div().max_w(px(360.)).text_center().text_sm().text_color(t.muted).child(detail.into()))
}

pub fn progress_bar(ratio: f32, t: Theme) -> gpui::Div {
    div()
        .h(px(4.))
        .rounded_full()
        .bg(t.border)
        .overflow_hidden()
        .child(div().h_full().w(relative(ratio.clamp(0., 1.))).rounded_full().bg(t.accent))
}

pub fn transparent() -> Hsla {
    gpui::transparent_black()
}

/// Hover text for icon buttons: `.tooltip(tooltip("Open folder"))`.
pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tooltip(text.clone())).into()
}

pub struct Tooltip(SharedString);

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(t.border)
            .bg(t.bg)
            .shadow_md()
            .text_xs()
            .text_color(t.text)
            .child(self.0.clone())
    }
}

/// "3 minutes ago", "Yesterday", "12 days ago".
pub fn ago(unix: u64) -> String {
    if unix == 0 {
        return "Never".into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(unix);
    let secs = now.saturating_sub(unix);
    match secs {
        0..60 => "Just now".into(),
        60..3600 => plural(secs / 60, "minute"),
        3600..86400 => plural(secs / 3600, "hour"),
        86400..172800 => "Yesterday".into(),
        _ if secs < 86400 * 60 => plural(secs / 86400, "day"),
        _ if secs < 86400 * 730 => plural(secs / (86400 * 30), "month"),
        _ => plural(secs / (86400 * 365), "year"),
    }
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") }
}

/// `1h 05m`, `12m`, `40s`.
pub fn duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h {:02}m", s / 3600, s / 60 % 60),
    }
}
