//! Log console: the live output of a game, or the lines of a log file. Follows new lines while
//! scrolled to the bottom, filters by text and severity, copies and shares what is shown.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, App, ClipboardItem, Context, Entity, FontWeight, ListAlignment, ListOffset, ListState,
    PromptLevel, SharedString, Window, div, list, prelude::*, px,
};

use crate::settings_page::{segment, segments};
use crate::state::{Level, LogLine, State};
use crate::text_input::{self, TextInput};
use crate::theme::{self, Theme};
use crate::ui::{self, Style, tooltip};

pub enum Source {
    /// The last launch of an instance.
    Session { state: State, id: String },
    /// Fixed lines, e.g. from a file.
    Lines(Rc<Vec<LogLine>>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Severity {
    All,
    Warnings,
    Errors,
}

pub struct LogView {
    source: Source,
    list: ListState,
    filter: Entity<TextInput>,
    severity: Severity,
    /// Indices (into the current lines) of lines that pass the filters; `None` = all lines.
    matches: Option<Vec<usize>>,
    /// Lines looked at so far, counted from the start of the log including dropped ones.
    synced: usize,
    /// Identity of what is shown; a new launch or file starts over.
    key: Option<Instant>,
    dropped: usize,
    scrolled_up: Rc<Cell<bool>>,
    /// Shown while there are no lines.
    empty: (SharedString, SharedString),
    uploading: bool,
    /// For a session that has not run since the launcher started: the game's last log file.
    fallback: Option<(Instant, Rc<Vec<LogLine>>)>,
}

impl LogView {
    pub fn new(source: Source, empty: (&'static str, &'static str), cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| TextInput::new("Filter", cx).with_icon("icons/search.svg"));
        cx.subscribe(&filter, |this, _, _: &text_input::Changed, cx| this.refilter(cx)).detach();
        if let Source::Session { state, .. } = &source {
            cx.observe(state, |_, _, cx| cx.notify()).detach();
        }
        let list = ListState::new(0, ListAlignment::Bottom, px(400.));
        let scrolled_up = Rc::new(Cell::new(false));
        let flag = scrolled_up.clone();
        list.set_scroll_handler(move |e, _, _| flag.set(e.is_scrolled));
        let mut this = LogView {
            source,
            list,
            filter,
            severity: Severity::All,
            matches: None,
            synced: 0,
            key: None,
            dropped: 0,
            scrolled_up,
            empty: (empty.0.into(), empty.1.into()),
            uploading: false,
            fallback: None,
        };
        this.load_fallback(cx);
        this.sync(cx);
        this
    }

    /// Reads `logs/latest.log` of the instance when it has not run since the launcher started.
    fn load_fallback(&mut self, cx: &mut Context<Self>) {
        let Source::Session { state, id } = &self.source else { return };
        let state = state.read(cx);
        if state.sessions.contains_key(id) {
            return;
        }
        let Some(path) = state.instance(id).map(|i| i.game_dir.join("logs/latest.log")) else { return };
        cx.spawn(async move |this, cx| {
            let Ok(text) = cx.background_spawn(async move { gplauncher::content::read_log(&path) }).await
            else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.fallback = Some((Instant::now(), Rc::new(crate::state::log_lines(&text))));
                cx.notify();
            });
        })
        .detach();
    }

    /// Whether the lines come from the last run's log file rather than a launch.
    fn showing_fallback(&self, cx: &App) -> bool {
        match &self.source {
            Source::Session { state, id } => {
                !state.read(cx).sessions.contains_key(id) && self.fallback.is_some()
            }
            Source::Lines(_) => false,
        }
    }

    /// Shows other fixed lines (another file).
    pub fn set_lines(&mut self, lines: Vec<LogLine>, cx: &mut Context<Self>) {
        self.source = Source::Lines(Rc::new(lines));
        self.key = None;
        self.synced = usize::MAX;
        self.sync(cx);
        cx.notify();
    }

    /// Current lines, how many were dropped from the front, and the identity of the log.
    fn snapshot<'a>(&'a self, cx: &'a App) -> (&'a [LogLine], usize, Option<Instant>) {
        match &self.source {
            Source::Session { state, id } => match state.read(cx).sessions.get(id) {
                Some(s) => (&s.log, s.dropped, Some(s.started_at())),
                None => match &self.fallback {
                    Some((key, lines)) => (lines.as_slice(), 0, Some(*key)),
                    None => (&[], 0, None),
                },
            },
            Source::Lines(lines) => (lines.as_slice(), 0, Some(self.key.unwrap_or_else(Instant::now))),
        }
    }

    fn passes(&self, line: &LogLine, query: &str) -> bool {
        let level_ok = match self.severity {
            Severity::All => true,
            Severity::Warnings => matches!(line.level, Level::Warn | Level::Error),
            Severity::Errors => line.level == Level::Error,
        };
        level_ok && (query.is_empty() || line.text.to_lowercase().contains(query))
    }

    fn filtering(&self, cx: &App) -> bool {
        self.severity != Severity::All || !self.filter.read(cx).text().trim().is_empty()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        self.synced = usize::MAX;
        self.sync(cx);
        cx.notify();
    }

    /// Brings the list up to date with the source.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).text().trim().to_lowercase();
        let filtering = self.filtering(cx);
        let (lines, dropped, key) = self.snapshot(cx);
        let total = dropped + lines.len();
        let restart =
            key != self.key || dropped != self.dropped || total < self.synced || self.synced == usize::MAX;
        let from = if restart { 0 } else { self.synced - dropped };
        let new_matches: Option<Vec<usize>> =
            filtering.then(|| (from..lines.len()).filter(|&i| self.passes(&lines[i], &query)).collect());
        let len = lines.len();

        if restart {
            self.key = key;
            self.dropped = dropped;
            self.synced = total;
            let count = new_matches.as_ref().map_or(len, Vec::len);
            self.matches = new_matches;
            self.list.reset(count);
            return;
        }
        if total == self.synced {
            return;
        }
        self.synced = total;
        match (&mut self.matches, new_matches) {
            (Some(matches), Some(new)) => {
                let old = matches.len();
                matches.extend(new);
                self.list.splice(old..old, matches.len() - old);
            }
            _ => {
                let old = self.list.item_count();
                self.list.splice(old..old, len - old);
            }
        }
    }

    fn visible_text(&self, cx: &App) -> String {
        let (lines, ..) = self.snapshot(cx);
        let pick: Box<dyn Iterator<Item = &LogLine>> = match &self.matches {
            Some(m) => Box::new(m.iter().filter_map(|&i| lines.get(i))),
            None => Box::new(lines.iter()),
        };
        pick.map(|l| l.text.as_ref()).collect::<Vec<_>>().join("\n")
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.visible_text(cx)));
    }

    fn upload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.uploading {
            return;
        }
        let text = self.visible_text(cx);
        if text.trim().is_empty() {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Info,
            "Share this log on mclo.gs?",
            Some("Anyone with the link can read it. The link is copied to the clipboard and opened."),
            &["Upload", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.uploading = true;
                cx.notify();
            });
            let result = cx.background_spawn(async move { gplauncher::http::upload_log(&text) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.uploading = false;
                match result {
                    Ok(url) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                        cx.open_url(&url);
                    }
                    Err(e) => {
                        let _ = window.prompt(
                            PromptLevel::Critical,
                            "Upload failed",
                            Some(&format!("{e:#}")),
                            &["OK"],
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn jump_to_bottom(&mut self, cx: &mut Context<Self>) {
        let count = self.list.item_count();
        self.list.scroll_to(ListOffset { item_ix: count, offset_in_item: px(0.) });
        self.scrolled_up.set(false);
        cx.notify();
    }

    fn set_severity(&mut self, severity: Severity, cx: &mut Context<Self>) {
        self.severity = severity;
        self.refilter(cx);
    }

    fn render_line(&self, ix: usize, t: Theme, cx: &App) -> AnyElement {
        let (lines, ..) = self.snapshot(cx);
        let line = match &self.matches {
            Some(m) => m.get(ix).and_then(|&i| lines.get(i)),
            None => lines.get(ix),
        };
        let Some(line) = line else { return div().into_any_element() };
        let color = match line.level {
            Level::Launcher => t.accent,
            Level::Info => t.text,
            Level::Warn => t.warning,
            Level::Error => t.danger,
            Level::Debug => t.subtle,
        };
        div()
            .w_full()
            .px_3()
            .py(px(1.))
            .text_color(color)
            .when(line.level == Level::Error, |d| d.bg(t.danger.opacity(0.06)))
            .when(line.level == Level::Launcher, |d| d.font_weight(FontWeight::MEDIUM))
            .child(if line.text.is_empty() { SharedString::from(" ") } else { line.text.clone() })
            .into_any_element()
    }
}

impl Render for LogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        self.sync(cx);
        let count = self.list.item_count();
        let (lines, ..) = self.snapshot(cx);
        let has_lines = !lines.is_empty();
        let filtering = self.filtering(cx);
        let severity = self.severity;

        let toolbar = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(t.border)
            .child(div().w(px(220.)).child(self.filter.clone()))
            .child(
                segments(
                    [(Severity::All, "All"), (Severity::Warnings, "Warnings"), (Severity::Errors, "Errors")]
                        .map(|(s, label)| {
                            segment(label, severity == s, t)
                                .px_2p5()
                                .text_xs()
                                .on_click(cx.listener(move |this, _, _, cx| this.set_severity(s, cx)))
                        }),
                    t,
                )
                .flex_none(),
            )
            .child(div().flex_1())
            .when(filtering, |d| {
                d.child(
                    div().text_xs().text_color(t.subtle).child(format!("{count} of {} lines", lines.len())),
                )
            })
            .child(
                ui::icon_button("log-copy", "icons/copy.svg", has_lines, t)
                    .tooltip(tooltip("Copy shown lines"))
                    .when(has_lines, |b| b.on_click(cx.listener(|this, _, _, cx| this.copy(cx)))),
            )
            .child(
                ui::icon_button("log-upload", "icons/upload.svg", has_lines && !self.uploading, t)
                    .tooltip(tooltip(if self.uploading { "Uploading…" } else { "Share on mclo.gs" }))
                    .when(has_lines && !self.uploading, |b| {
                        b.on_click(cx.listener(|this, _, window, cx| this.upload(window, cx)))
                    }),
            );

        let body = if !has_lines {
            ui::empty_state("icons/terminal.svg", self.empty.0.clone(), self.empty.1.clone(), t)
                .into_any_element()
        } else if count == 0 {
            ui::empty_state("icons/search.svg", "No matching lines", "Try another filter.", t)
                .into_any_element()
        } else {
            list(
                self.list.clone(),
                cx.processor(move |this, ix: usize, window, cx| {
                    let t = Theme::for_appearance(window.appearance());
                    this.render_line(ix, t, cx)
                }),
            )
            .size_full()
            .font_family(theme::mono_font())
            .text_xs()
            .into_any_element()
        };

        let fallback = self.showing_fallback(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .when(fallback, |d| {
                d.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_1p5()
                        .border_b_1()
                        .border_color(t.border)
                        .bg(t.accent_soft)
                        .text_xs()
                        .text_color(t.muted)
                        .child("From the last run (logs/latest.log). Press Play to see live output."),
                )
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .py_1()
                    .bg(t.panel)
                    .font_family(theme::mono_font())
                    .text_xs()
                    .child(body)
                    .when(self.scrolled_up.get() && count > 0, |d| {
                        d.child(
                            div().absolute().bottom_3().right_4().child(
                                ui::button(
                                    "jump-bottom",
                                    Some("icons/arrow-down.svg"),
                                    "Latest",
                                    Style::Secondary,
                                    true,
                                    t,
                                )
                                .shadow_md()
                                .on_click(cx.listener(|this, _, _, cx| this.jump_to_bottom(cx))),
                            ),
                        )
                    }),
            )
    }
}
