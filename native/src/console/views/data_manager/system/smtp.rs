//! System › SMTP (operator), after the web client's
//! `SmtpSettingsPanel.tsx`: the mail server alarm emails go through
//! (`GET`/`PUT /api/v1/settings/smtp`) and a test email
//! (`POST /api/v1/settings/smtp/test`).
//!
//! The server never sends the password back: a configured server answers
//! `********`, which stands in the password field; saving with it (or
//! with the field empty, as the web panel leaves it out) keeps the stored
//! password. Typing a new one replaces it.

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, FontWeight, Subscription, Window, div, px};
use serde::de::IgnoredAny;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::eng_system::{
    Encryption, SMTP_PATH, SMTP_TEST_PATH, SmtpDraft, SmtpSettings, SmtpTestAnswer, SmtpTestBody,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, field, pill, section_label, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::{focus_body, form_note, needs};
use crate::console::Console;
use crate::console::pages::Role;
use crate::console::views::ops_kit::{notice, on_field};
use crate::console::views::{LIST_TAIL, card};

const SCROLL: &str = "data.system.smtp";

struct Fields {
    host: Entity<TextInput>,
    port: Entity<TextInput>,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    from_address: Entity<TextInput>,
    from_name: Entity<TextInput>,
    recipient: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
struct State {
    settings: Option<Result<SmtpSettings, String>>,
    asked: bool,
    get_seq: i64,
    save_seq: i64,
    test_seq: i64,
    fields: Option<Fields>,
    encryption: Encryption,
    saving: bool,
    save_note: Option<Result<String, String>>,
    testing: bool,
    test_note: Option<Result<String, String>>,
}

pub(super) fn subtitle(console: &Console) -> String {
    if console.is_operator() {
        "Outgoing mail for alarm emails · ↵ save · Tab next field · [ ] encryption".to_owned()
    } else {
        "Outgoing mail for alarm emails · operator role".to_owned()
    }
}

pub(super) fn open(console: &mut Console) {
    if console.is_operator() && !console.state::<State>().asked {
        fetch(console);
    }
}

pub(super) fn fetch(console: &mut Console) {
    if console.stress.is_some() || !console.is_operator() {
        return;
    }
    let state = console.state::<State>();
    state.asked = true;
    state.get_seq += 1;
    let seq = state.get_seq;
    console.call(
        CallKey::new("data.system.smtp.get").with(seq),
        Call::get::<SmtpSettings>(SMTP_PATH),
    );
}

pub(super) fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    let host = console
        .state_ref::<State>()
        .and_then(|state| state.fields.as_ref())
        .map(|fields| fields.host.clone());
    match host {
        Some(host) if console.is_operator() => {
            host.read(cx).focus(window);
            true
        }
        _ => false,
    }
}

/// `[` and `]`: the next or previous encryption.
pub(super) fn cycle(console: &mut Console, forward: bool, _: &mut Context<Console>) {
    let state = console.state::<State>();
    let all = Encryption::ALL;
    let at = all
        .iter()
        .position(|each| *each == state.encryption)
        .unwrap_or(0);
    state.encryption = all[if forward {
        (at + 1) % all.len()
    } else {
        (at + all.len() - 1) % all.len()
    }];
}

fn text(field: &Entity<TextInput>, cx: &gpui::App) -> String {
    field.read(cx).value().to_owned()
}

fn draft(console: &Console, cx: &gpui::App) -> Option<SmtpDraft> {
    let state = console.state_ref::<State>()?;
    let fields = state.fields.as_ref()?;
    Some(SmtpDraft {
        host: text(&fields.host, cx),
        port: text(&fields.port, cx),
        username: text(&fields.username, cx),
        password: text(&fields.password, cx),
        from_address: text(&fields.from_address, cx),
        from_name: text(&fields.from_name, cx),
        encryption: state.encryption,
    })
}

/// Put the server's settings in the form.
fn fill(console: &mut Console, settings: &SmtpSettings, cx: &mut Context<Console>) {
    let draft = settings.draft();
    let state = console.state::<State>();
    state.encryption = draft.encryption;
    let Some(fields) = &state.fields else {
        return;
    };
    for (field, value) in [
        (&fields.host, draft.host),
        (&fields.port, draft.port),
        (&fields.username, draft.username),
        (&fields.password, draft.password),
        (&fields.from_address, draft.from_address),
        (&fields.from_name, draft.from_name),
    ] {
        field.update(cx, |field, cx| field.set_text(value, cx));
    }
}

fn save(console: &mut Console, cx: &mut Context<Console>) {
    if !console.is_operator() {
        return;
    }
    let Some(draft) = draft(console, cx) else {
        return;
    };
    let state = console.state::<State>();
    if state.saving {
        return;
    }
    match draft.validate() {
        Ok(body) => {
            state.saving = true;
            state.save_note = None;
            state.save_seq += 1;
            let seq = state.save_seq;
            let body = serde_json::to_value(body).unwrap_or_default();
            console.call(
                CallKey::new("data.system.smtp.save").with(seq),
                Call::send::<IgnoredAny>(Method::PUT, SMTP_PATH, body),
            );
        }
        Err(error) => state.save_note = Some(Err(error)),
    }
    cx.notify();
}

fn send_test(console: &mut Console, cx: &mut Context<Console>) {
    if !console.is_operator() {
        return;
    }
    let Some(recipient) = console
        .state_ref::<State>()
        .and_then(|state| state.fields.as_ref())
        .map(|fields| text(&fields.recipient, cx))
    else {
        return;
    };
    let state = console.state::<State>();
    if state.testing {
        return;
    }
    match SmtpTestBody::new(&recipient) {
        Ok(body) => {
            state.testing = true;
            state.test_note = None;
            state.test_seq += 1;
            let seq = state.test_seq;
            let body = serde_json::to_value(body).unwrap_or_default();
            console.call(
                CallKey::new("data.system.smtp.test").with(seq),
                Call::send::<SmtpTestAnswer>(Method::POST, SMTP_TEST_PATH, body),
            );
        }
        Err(error) => state.test_note = Some(Err(error)),
    }
    cx.notify();
}

pub(super) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    let state = console.state::<State>();
    let current = match key.name {
        "data.system.smtp.get" => state.get_seq,
        "data.system.smtp.save" => state.save_seq,
        "data.system.smtp.test" => state.test_seq,
        _ => return,
    };
    if key.arg != current {
        return;
    }
    match key.name {
        "data.system.smtp.get" => match reply.take::<SmtpSettings>() {
            Ok(settings) => {
                fill(console, &settings, cx);
                console.state::<State>().settings = Some(Ok(settings));
            }
            Err(error) => console.state::<State>().settings = Some(Err(error)),
        },
        "data.system.smtp.save" => {
            let state = console.state::<State>();
            state.saving = false;
            match reply.take::<IgnoredAny>() {
                Ok(_) => {
                    state.save_note = Some(Ok("Configuration saved".to_owned()));
                    // The form shows what the server keeps, the password
                    // masked again.
                    fetch(console);
                }
                Err(error) => state.save_note = Some(Err(error)),
            }
        }
        "data.system.smtp.test" => {
            let state = console.state::<State>();
            state.testing = false;
            state.test_note = Some(
                reply
                    .take::<SmtpTestAnswer>()
                    .map(|answer| answer.text().to_owned()),
            );
        }
        _ => {}
    }
}

fn on_form(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Submitted => save(console, cx),
        InputEvent::Cancelled => focus_body(console, window, cx),
        InputEvent::Changed | InputEvent::Blurred => {}
    }
}

fn on_recipient(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Submitted => send_test(console, cx),
        InputEvent::Cancelled => focus_body(console, window, cx),
        InputEvent::Changed | InputEvent::Blurred => {}
    }
}

fn ensure_fields(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().fields.is_some() {
        return;
    }
    let make =
        |placeholder: &'static str, kind: InputKind, mono: bool, cx: &mut Context<Console>| {
            cx.new(|cx| {
                let input = TextInput::new(cx).placeholder(placeholder).kind(kind);
                if mono { input.mono() } else { input }
            })
        };
    let host = make("smtp.example.com", InputKind::Text, true, cx);
    let port = make("587", InputKind::Unsigned, true, cx);
    let username = make("your@email.com", InputKind::Text, false, cx);
    let password = cx.new(|cx| TextInput::new(cx).placeholder("••••••••").secure());
    let from_address = make("voltrus@example.com", InputKind::Text, false, cx);
    let from_name = make("Voltrus", InputKind::Text, false, cx);
    let recipient = make("recipient@example.com", InputKind::Text, false, cx);
    let mut subscriptions: Vec<Subscription> = [
        &host,
        &port,
        &username,
        &password,
        &from_address,
        &from_name,
    ]
    .into_iter()
    .map(|field| on_field(field, on_form, window, cx))
    .collect();
    subscriptions.push(on_field(&recipient, on_recipient, window, cx));
    console.state::<State>().fields = Some(Fields {
        host,
        port,
        username,
        password,
        from_address,
        from_name,
        recipient,
        _subscriptions: subscriptions,
    });
    // Settings that came before the form was made.
    if let Some(Ok(settings)) = console.state::<State>().settings.clone() {
        fill(console, &settings, cx);
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    if !console.is_operator() {
        return notice(
            format!("SMTP settings: {}", needs(Role::Operator).to_lowercase()),
            false,
            &theme,
        );
    }
    ensure_fields(console, window, cx);
    let state = console.state::<State>();
    let settings = match &state.settings {
        None => return notice("Loading the SMTP settings…", false, &theme),
        Some(Err(error)) => return notice(format!("SMTP settings: {error}"), true, &theme),
        Some(Ok(settings)) => settings.clone(),
    };
    let Some(fields) = &state.fields else {
        return div().into_any_element();
    };
    let (host, port, username, password, from_address, from_name, recipient) = (
        fields.host.clone(),
        fields.port.clone(),
        fields.username.clone(),
        fields.password.clone(),
        fields.from_address.clone(),
        fields.from_name.clone(),
        fields.recipient.clone(),
    );
    let (encryption, saving, save_note, testing, test_note) = (
        state.encryption,
        state.saving,
        state.save_note.clone(),
        state.testing,
        state.test_note.clone(),
    );
    let recipient_empty = recipient.read(cx).value().trim().is_empty();
    let configured = settings.configured;
    let scroll = console.scroll(SCROLL);

    let encryptions = Encryption::ALL
        .into_iter()
        .map(|each| {
            Segment::new(
                each.label(),
                each == encryption,
                cx.listener(move |console, _, _, cx| {
                    console.state::<State>().encryption = each;
                    cx.notify();
                }),
            )
        })
        .collect();
    let grid = |columns: u16, children: Vec<AnyElement>| {
        div()
            .grid()
            .grid_cols(columns)
            .gap(SPACE_6)
            .children(children)
    };

    let form = card(&theme)
        .p(SPACE_8)
        .gap(SPACE_6)
        .max_w(px(640.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.fg_dim)
                        .child("STATUS"),
                )
                .child(pill(
                    if configured {
                        "Configured"
                    } else {
                        "Not configured"
                    },
                    if configured {
                        theme.success
                    } else {
                        theme.warning
                    },
                )),
        )
        .child(section_label("Server", cx))
        .child(grid(
            3,
            vec![
                div()
                    .col_span(2)
                    .child(field("Host", host, cx))
                    .into_any_element(),
                field("Port", port, cx).into_any_element(),
            ],
        ))
        .child(section_label("Authentication", cx))
        .child(grid(
            2,
            vec![
                field("Username", username, cx).into_any_element(),
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2)
                    .child(field("Password", password, cx))
                    .when(configured, |this| {
                        this.child(
                            div()
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child("Leave it as it is to keep the current password"),
                        )
                    })
                    .into_any_element(),
            ],
        ))
        .child(section_label("Sender", cx))
        .child(grid(
            2,
            vec![
                field("From address", from_address, cx).into_any_element(),
                field("From name", from_name, cx).into_any_element(),
            ],
        ))
        .child(section_label("Encryption", cx))
        .child(segmented("system-smtp-encryption", encryptions, cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .pt(SPACE_4)
                .child(
                    Button::new(
                        "system-smtp-save",
                        if saving {
                            "Saving…"
                        } else {
                            "Save configuration"
                        },
                    )
                    .primary()
                    .icon(Icon::Check)
                    .disabled(saving)
                    .on_click(cx.listener(|console, _, _, cx| save(console, cx))),
                )
                .child(div().flex_1().min_w_0().child(form_note(
                    save_note,
                    &[("↵", "save"), ("Tab", "next field"), ("[ ]", "encryption")],
                    cx,
                ))),
        );

    let test = card(&theme)
        .p(SPACE_8)
        .gap(SPACE_5)
        .max_w(px(640.0))
        .child(section_label("Test connection", cx))
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child("Sends a test alarm email with the saved settings."),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap(SPACE_5)
                .child(div().flex_1().child(field("Recipient", recipient, cx)))
                .child(
                    Button::new(
                        "system-smtp-test",
                        if testing { "Sending…" } else { "Send test" },
                    )
                    .icon(Icon::Send)
                    .disabled(testing || recipient_empty || !configured)
                    .tooltip(if configured {
                        "↵ in the recipient field"
                    } else {
                        "Save a configuration first"
                    })
                    .on_click(cx.listener(|console, _, _, cx| send_test(console, cx))),
                ),
        )
        .child(form_note(test_note, &[("↵", "send")], cx));

    div()
        .id(SCROLL)
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .p(SPACE_9)
                .pb(LIST_TAIL)
                .child(form)
                .child(test),
        )
        .into_any_element()
}
