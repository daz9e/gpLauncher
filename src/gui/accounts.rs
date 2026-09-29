//! "Accounts" dialog: pick the account to play with, add offline profiles, sign in with Microsoft.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use futures::channel::mpsc;
use gplauncher::auth::{self, Account, AccountKind, DeviceCode};
use gpui::{
    App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    SharedString, Window, actions, div, prelude::*, px, svg,
};

use crate::add_instance::{button, field};
use crate::text_input::{self, TextInput};
use crate::theme::Theme;

actions!(accounts, [Close, Confirm]);

const CONTEXT: &str = "Accounts";
const AZURE_APPS: &str = "https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Close, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm, Some(CONTEXT)),
    ]);
}

pub enum AccountsEvent {
    /// The account list or the selection changed; the dialog stays open.
    Changed {
        accounts: Vec<Account>,
        selected: usize,
    },
    /// A new Azure application client ID was entered; the dialog stays open.
    SaveClientId(String),
    Dismissed,
}

/// Progress of a Microsoft sign-in.
enum Login {
    Idle,
    Requesting,
    Waiting(DeviceCode),
    Failed(String),
}

enum LoginMsg {
    Code(DeviceCode),
    Done(Result<Account, String>),
}

pub struct Accounts {
    focus_handle: FocusHandle,
    accounts: Vec<Account>,
    selected: usize,
    client_id: String,
    editing_client_id: bool,
    offline_name: Entity<TextInput>,
    client_id_input: Entity<TextInput>,
    offline_error: Option<String>,
    login: Login,
    /// Stops the sign-in running on a worker thread.
    cancel: Arc<AtomicBool>,
}

impl EventEmitter<AccountsEvent> for Accounts {}

impl Focusable for Accounts {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Drop for Accounts {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Accounts {
    pub fn new(
        accounts: Vec<Account>,
        selected: usize,
        client_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let offline_name = cx.new(|cx| TextInput::new("Player name", cx));
        cx.subscribe(&offline_name, |this: &mut Self, _, _: &text_input::Changed, cx| {
            this.offline_error = None;
            cx.notify();
        })
        .detach();
        let client_id_input = cx.new(|cx| TextInput::new("Application (client) ID", cx));
        client_id_input.update(cx, |i, cx| i.set_text(client_id.clone(), cx));
        cx.subscribe(&client_id_input, |_, _, _: &text_input::Changed, cx| cx.notify()).detach();
        window.focus(&offline_name.focus_handle(cx));
        Accounts {
            focus_handle: cx.focus_handle(),
            accounts,
            selected,
            client_id,
            editing_client_id: false,
            offline_name,
            client_id_input,
            offline_error: None,
            login: Login::Idle,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(AccountsEvent::Changed { accounts: self.accounts.clone(), selected: self.selected });
        cx.notify();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.changed(cx);
    }

    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.accounts.len() {
            return;
        }
        self.accounts.remove(index);
        if self.selected > index {
            self.selected -= 1;
        }
        self.selected = self.selected.min(self.accounts.len().saturating_sub(1));
        self.changed(cx);
    }

    /// Adds `account` and selects it; an account with the same UUID is replaced.
    fn add(&mut self, account: Account, cx: &mut Context<Self>) {
        match self.accounts.iter().position(|a| a.kind == account.kind && a.uuid == account.uuid) {
            Some(i) => {
                self.accounts[i] = account;
                self.selected = i;
            }
            None => {
                self.accounts.push(account);
                self.selected = self.accounts.len() - 1;
            }
        }
        self.changed(cx);
    }

    fn add_offline(&mut self, cx: &mut Context<Self>) {
        let name = self.offline_name.read(cx).text().trim().to_string();
        if !auth::valid_offline_name(&name) {
            self.offline_error = Some("Use 3–16 characters: letters, digits and _".into());
            cx.notify();
            return;
        }
        self.offline_name.update(cx, |i, cx| i.set_text("", cx));
        self.add(Account::offline(&name), cx);
    }

    fn save_client_id(&mut self, cx: &mut Context<Self>) {
        let id = self.client_id_input.read(cx).text().trim().to_string();
        if id.is_empty() {
            return;
        }
        self.client_id = id.clone();
        self.editing_client_id = false;
        cx.emit(AccountsEvent::SaveClientId(id));
        cx.notify();
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        self.cancel_sign_in();
        let cancel = self.cancel.clone();
        let client_id = self.client_id.clone();
        self.login = Login::Requesting;
        let (tx, mut rx) = mpsc::unbounded();
        std::thread::spawn(move || {
            let result = auth::request_device_code(&client_id).and_then(|code| {
                let _ = tx.unbounded_send(LoginMsg::Code(code.clone()));
                auth::complete_device_code(&client_id, &code, || cancel.load(Ordering::Relaxed))
            });
            let _ = tx.unbounded_send(LoginMsg::Done(result.map_err(|e| format!("{e:#}"))));
        });

        let cancel = self.cancel.clone();
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let alive = this.update(cx, |this, cx| {
                    match msg {
                        LoginMsg::Code(code) => this.login = Login::Waiting(code),
                        LoginMsg::Done(Ok(account)) => {
                            this.login = Login::Idle;
                            this.add(account, cx);
                        }
                        LoginMsg::Done(Err(e)) => this.login = Login::Failed(e),
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// Stops a running sign-in; a new one gets a fresh flag.
    fn cancel_sign_in(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.login = Login::Idle;
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.client_id_input.focus_handle(cx).is_focused(window) {
            self.save_client_id(cx);
        } else if !self.offline_name.read(cx).text().trim().is_empty() {
            self.add_offline(cx);
        }
    }

    fn close(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        self.cancel_sign_in();
        cx.emit(AccountsEvent::Dismissed);
    }

    // ---- rendering -----------------------------------------------------------

    fn list(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        if self.accounts.is_empty() {
            return div()
                .py_3()
                .text_sm()
                .text_color(t.muted)
                .child("No accounts yet: the game starts offline as Player.")
                .into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(self.accounts.iter().enumerate().map(|(i, account)| {
                let selected = i == self.selected;
                let kind = match account.kind {
                    AccountKind::Microsoft => "Microsoft",
                    AccountKind::Offline => "Offline",
                };
                div()
                    .id(SharedString::from(format!("account-{i}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_2p5()
                    .py_2()
                    .rounded_lg()
                    .border_1()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.accent_soft).border_color(t.accent.opacity(0.35)))
                    .when(!selected, |d| d.border_color(gpui::transparent_black()).hover(|d| d.bg(t.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(i, cx)))
                    .child(avatar(&account.name, 30., t))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(t.text)
                                    .truncate()
                                    .child(account.name.clone()),
                            )
                            .child(div().text_xs().text_color(t.muted).child(kind)),
                    )
                    .when(selected, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.accent)
                                .child("Active"),
                        )
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("remove-{i}")))
                            .flex_none()
                            .p_1()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|d| d.bg(t.hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove(i, cx);
                            }))
                            .child(svg().path("icons/trash.svg").size(px(14.)).text_color(t.muted)),
                    )
            }))
            .into_any_element()
    }

    fn offline_section(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let can_add = !self.offline_name.read(cx).text().trim().is_empty();
        field("Offline account", t)
            .child(
                div().flex().gap_2().child(div().flex_1().child(self.offline_name.clone())).child(
                    button("add-offline", "Add", can_add, false, t)
                        .when(can_add, |b| b.on_click(cx.listener(|this, _, _, cx| this.add_offline(cx)))),
                ),
            )
            .child(match &self.offline_error {
                Some(e) => div().text_xs().text_color(t.danger).child(e.clone()),
                None => div()
                    .text_xs()
                    .text_color(t.subtle)
                    .child("For singleplayer and servers in offline mode."),
            })
    }

    fn microsoft_section(&self, t: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let section = field("Microsoft account", t);
        if self.client_id.is_empty() || self.editing_client_id {
            let can_save = !self.client_id_input.read(cx).text().trim().is_empty();
            return section
                .child(div().text_xs().text_color(t.muted).child(
                    "Signing in needs the client ID of an Azure application \
                     that Mojang allowed to use the Minecraft API.",
                ))
                .child(div().flex().gap_2().child(div().flex_1().child(self.client_id_input.clone())).child(
                    button("save-client-id", "Save", can_save, false, t).when(can_save, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.save_client_id(cx)))
                    }),
                ))
                .child(
                    link("azure", "Open Azure app registrations", t)
                        .on_click(|_, _, cx| cx.open_url(AZURE_APPS)),
                );
        }
        match &self.login {
            Login::Idle => section.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        button("sign-in", "Sign in with Microsoft", true, true, t)
                            .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                    )
                    .child(link("change-client-id", "Change client ID", t).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.editing_client_id = true;
                            window.focus(&this.client_id_input.focus_handle(cx));
                            cx.notify();
                        },
                    ))),
            ),
            Login::Requesting => {
                section.child(div().text_sm().text_color(t.muted).child("Requesting a code…"))
            }
            Login::Waiting(code) => {
                let (user_code, uri) = (code.user_code.clone(), code.verification_uri.clone());
                section
                    .child(
                        div().text_sm().text_color(t.muted).child(format!("Open {uri} and enter the code")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child(user_code.clone()),
                            )
                            .child(div().flex_1())
                            .child(button("open-login", "Copy code & open", true, true, t).on_click(
                                move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(user_code.clone()));
                                    cx.open_url(&uri);
                                },
                            ))
                            .child(button("cancel-login", "Cancel", true, false, t).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.cancel_sign_in();
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(div().text_xs().text_color(t.subtle).child("Waiting for you to sign in…"))
            }
            Login::Failed(e) => section.child(div().text_xs().text_color(t.danger).child(e.clone())).child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        button("retry", "Try again", true, false, t)
                            .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                    )
                    .child(link("change-client-id", "Change client ID", t).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.login = Login::Idle;
                            this.editing_client_id = true;
                            window.focus(&this.client_id_input.focus_handle(cx));
                            cx.notify();
                        },
                    ))),
            ),
        }
    }
}

impl Render for Accounts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::for_appearance(window.appearance());
        div()
            .id("accounts")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::close))
            .w(px(460.))
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
                div()
                    .px_5()
                    .pt_4()
                    .pb_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child("Accounts"),
            )
            .child(
                div()
                    .id("accounts-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .px_5()
                    .py_3()
                    .child(self.list(t, cx))
                    .child(div().h(px(1.)).bg(t.border))
                    .child(self.offline_section(t, cx))
                    .child(self.microsoft_section(t, cx)),
            )
            .child(
                div().flex().justify_end().px_5().py_3().border_t_1().border_color(t.border).child(
                    button("done", "Done", true, false, t)
                        .on_click(cx.listener(|this, _, window, cx| this.close(&Close, window, cx))),
                ),
            )
    }
}

pub fn avatar(name: &str, size: f32, t: Theme) -> gpui::Div {
    let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(t.tile)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.muted)
        .child(initial)
}

fn link(id: &'static str, label: &'static str, t: Theme) -> gpui::Stateful<gpui::Div> {
    div().id(id).text_xs().text_color(t.accent).cursor_pointer().hover(|d| d.underline()).child(label)
}
