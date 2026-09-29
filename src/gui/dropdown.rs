//! A button that opens a list of options below it. The caller keeps the state:
//! which dropdown is open and what is selected.

use gpui::{
    AnyElement, App, ClickEvent, Corner, FontWeight, MouseButton, MouseDownEvent, SharedString, Window,
    anchored, deferred, div, prelude::*, px, svg,
};

use crate::theme::Theme;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type SelectHandler = Box<dyn Fn(usize, &mut Window, &mut App)>;
type DismissHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

pub struct Dropdown {
    id: SharedString,
    /// Shown before the value, e.g. `Version`.
    label: &'static str,
    value: SharedString,
    options: Vec<(SharedString, bool)>,
    open: bool,
    theme: Theme,
    on_toggle: Option<ClickHandler>,
    on_select: Option<SelectHandler>,
    on_dismiss: Option<DismissHandler>,
}

impl Dropdown {
    pub fn new(
        id: impl Into<SharedString>,
        label: &'static str,
        value: impl Into<SharedString>,
        t: Theme,
    ) -> Self {
        Dropdown {
            id: id.into(),
            label,
            value: value.into(),
            options: Vec::new(),
            open: false,
            theme: t,
            on_toggle: None,
            on_select: None,
            on_dismiss: None,
        }
    }

    /// Options as `(label, selected)`.
    pub fn options(mut self, options: impl IntoIterator<Item = (SharedString, bool)>) -> Self {
        self.options = options.into_iter().collect();
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn on_toggle(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(f));
        self
    }

    pub fn on_select(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }

    /// Called on a click outside the open list.
    pub fn on_dismiss(mut self, f: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Box::new(f));
        self
    }
}

impl IntoElement for Dropdown {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let on_select = self.on_select.map(std::rc::Rc::new);
        let button = div()
            .id(self.id.clone())
            .flex()
            .items_center()
            .gap_1()
            .h(px(28.))
            .px_2p5()
            .rounded_md()
            .border_1()
            .border_color(if self.open { t.accent } else { t.border })
            .bg(t.bg)
            .text_xs()
            .cursor_pointer()
            .hover(|d| d.bg(t.hover))
            .when_some(self.on_toggle, |d, f| d.on_click(f))
            .child(div().text_color(t.muted).child(format!("{}:", self.label)))
            .child(div().text_color(t.text).font_weight(FontWeight::MEDIUM).child(self.value))
            .child(svg().path("icons/chevron-down.svg").size(px(12.)).text_color(t.subtle));

        let menu = self.open.then(|| {
            let list = div()
                .id(SharedString::from(format!("{}-menu", self.id)))
                .occlude()
                .mt_1()
                .min_w(px(160.))
                .max_h(px(280.))
                .overflow_y_scroll()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(t.border)
                .bg(t.bg)
                .shadow_md()
                .when_some(self.on_dismiss, |d, f| d.on_mouse_down_out(f))
                .children(self.options.into_iter().enumerate().map(|(ix, (label, selected))| {
                    let on_select = on_select.clone();
                    div()
                        .id(ix)
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(if selected { t.accent } else { t.text })
                        .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                        .cursor_pointer()
                        .hover(|d| d.bg(t.hover))
                        // Handle the press itself so the dismiss handler of the list does not race it.
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(f) = &on_select {
                                f(ix, window, cx);
                            }
                        })
                        .child(label)
                }));
            deferred(anchored().anchor(Corner::TopLeft).snap_to_window_with_margin(px(8.)).child(list))
                .with_priority(1)
        });

        div().flex().flex_col().child(button).children(menu).into_any_element()
    }
}
