//! The electronic signature prompt (21 CFR Part 11). An Enterprise server
//! may refuse a tag write, an alarm acknowledge or a shelve until the
//! signed-in user signs it: the prompt shows the meaning the server stated
//! (read-only), takes the user's password (masked) and a reason, and sends
//! the action again with the signature ([`Command::Sign`]). A wrong
//! password asks again with the server's error; any other failure (a locked
//! account, a signature the server cannot take) closes the prompt with a
//! toast. The password field is emptied as soon as the signature is built
//! and whenever the prompt opens or closes; it is never logged.
//!
//! The pure rules (what the server's answer means, the retry's body) are
//! `voltrus_scada_native::signature`.

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, Subscription, Window, div, px};
use voltrus_scada_native::runtime::Command;
use voltrus_scada_native::signature::{self, Ask, Signable, SignatureRequired};
use voltrus_scada_native::tag_api::WriteMode;
use vt_ui::Icon;
use vt_ui::components::{Button, field};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::tag_detail::{dialog, note_line};
use crate::console::Console;
use crate::console::overlay::Overlay;

/// The prompt's overlay id.
pub(crate) const ID: &str = "sign";

#[derive(Default)]
pub(crate) struct SignState {
    /// The action to sign.
    action: Option<Signable>,
    request: SignatureRequired,
    /// Why the last try failed (a wrong password, an empty field).
    error: Option<String>,
    /// The signed action is on its way.
    busy: bool,
    password: Option<Entity<TextInput>>,
    reason: Option<Entity<TextInput>>,
    /// Asked from a REST reply, which has no window: opened by
    /// [`after_reply`].
    queued: Option<(Signable, Ask)>,
    subscriptions: Vec<Subscription>,
}

/// Ask the user to sign `action`, or to sign it again (`ask.error` says
/// why the last signature was refused).
pub(crate) fn ask(
    console: &mut Console,
    action: Signable,
    ask: Ask,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    // A direct write the server wants signed no longer waits.
    if let Signable::TagWrite { tag, .. } = &action
        && console.writing == Some(*tag)
    {
        console.writing = None;
    }
    ensure_fields(console, window, cx);
    let state = console.state::<SignState>();
    let again = state.action.as_ref() == Some(&action) && ask.error.is_some();
    state.action = Some(action);
    state.request = ask.request;
    state.error = ask.error;
    state.busy = false;
    let (password, reason) = (state.password.clone(), state.reason.clone());
    clear(password.as_ref(), cx);
    // A first prompt starts empty; asking again keeps the reason typed.
    if !again {
        clear(reason.as_ref(), cx);
    }
    if !console.overlay_is(ID) {
        let overlay = Overlay::modal(ID, px(480.0), render, cx);
        console.open_overlay(overlay, window, cx);
    }
    if let Some(password) = password {
        password.update(cx, |field, _| field.focus(window));
    }
    cx.notify();
}

/// [`ask`] from a REST reply handler, which has no window: the prompt opens
/// once the reply is handled.
pub(crate) fn queue(console: &mut Console, action: Signable, ask: Ask) {
    console.state::<SignState>().queued = Some((action, ask));
}

/// After a REST reply: open the prompt a reply asked for.
pub(crate) fn after_reply(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let queued = console
        .state_ref::<SignState>()
        .and_then(|state| state.queued.as_ref())
        .is_some();
    if queued && let Some((action, request)) = console.state::<SignState>().queued.take() {
        ask(console, action, request, window, cx);
    }
}

/// The answer to a signed action sent through REST ([`Command::Sign`]).
pub(crate) fn settled(
    console: &mut Console,
    action: &Signable,
    result: Result<Option<String>, String>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    finish(console, action, window, cx);
    let what = action.describe();
    match result {
        Ok(Some(detail)) => console.show_toast(format!("{what}: signed, {detail}"), false, cx),
        Ok(None) => console.show_toast(format!("{what}: signed"), false, cx),
        Err(error) => console.show_toast(format!("{what} failed: {error}"), true, cx),
    }
}

/// A direct write landed: if it was the signed one, the prompt closes.
pub(crate) fn written(
    console: &mut Console,
    tag: i64,
    result: &Result<String, String>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(action) = console.state_ref::<SignState>().and_then(|state| {
        state.action.clone().filter(|action| {
            state.busy
                && matches!(action, Signable::TagWrite { tag: at, mode: WriteMode::Direct, .. } if *at == tag)
        })
    }) else {
        return;
    };
    settled(
        console,
        &action,
        result
            .clone()
            .map(|detail| Some(format!("written: {detail}"))),
        window,
        cx,
    );
}

/// Close the prompt if it waited for `action`.
fn finish(
    console: &mut Console,
    action: &Signable,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let state = console.state::<SignState>();
    if state.action.as_ref() != Some(action) {
        return;
    }
    state.action = None;
    state.busy = false;
    state.error = None;
    let password = state.password.clone();
    clear(password.as_ref(), cx);
    if console.overlay_is(ID) {
        console.close_overlay(window, cx);
    }
}

fn clear(field: Option<&Entity<TextInput>>, cx: &mut Context<Console>) {
    if let Some(field) = field {
        field.update(cx, |field, cx| field.set_text("", cx));
    }
}

fn ensure_fields(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<SignState>().password.is_some() {
        return;
    }
    let password = cx.new(|cx| TextInput::new(cx).placeholder("Your password").secure());
    let reason = cx.new(|cx| TextInput::new(cx).placeholder("Why you sign"));
    let subscriptions = [&password, &reason].map(|field| {
        cx.subscribe_in(
            field,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => submit(console, window, cx),
                InputEvent::Cancelled => cancel(console, window, cx),
                InputEvent::Changed | InputEvent::Blurred => cx.notify(),
            },
        )
    });
    let state = console.state::<SignState>();
    state.subscriptions.extend(subscriptions);
    state.password = Some(password);
    state.reason = Some(reason);
}

/// Esc or Cancel: the action is not sent.
fn cancel(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<SignState>();
    if state.busy {
        return;
    }
    let action = state.action.take();
    state.error = None;
    let password = state.password.clone();
    clear(password.as_ref(), cx);
    console.close_overlay(window, cx);
    if let Some(action) = action {
        console.show_toast(
            format!("{}: not signed, not sent", action.describe()),
            false,
            cx,
        );
    }
}

/// Enter or Sign: build the signature and send the action again with it.
fn submit(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<SignState>();
    if state.busy {
        return;
    }
    let (Some(action), Some(password_field), Some(reason_field)) = (
        state.action.clone(),
        state.password.clone(),
        state.reason.clone(),
    ) else {
        return;
    };
    let built = signature::signature(
        &state.request,
        password_field.read(cx).value(),
        reason_field.read(cx).value(),
    );
    let signature = match built {
        Ok(signature) => signature,
        Err(error) => {
            console.state::<SignState>().error = Some(error.to_owned());
            cx.notify();
            return;
        }
    };
    // The password lives on only in the signature on its way.
    clear(Some(&password_field), cx);
    let state = console.state::<SignState>();
    state.busy = true;
    state.error = None;
    if let Signable::TagWrite {
        tag,
        mode: WriteMode::Direct,
        ..
    } = &action
    {
        console.write_result = None;
        console.writing = Some(*tag);
    }
    console.send(Command::Sign { action, signature });
    cx.notify();
}

/// Type `password` and `reason` into the open prompt and sign.
#[cfg(test)]
pub(crate) fn sign_with(
    console: &mut Console,
    password: &str,
    reason: &str,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let state = console.state::<SignState>();
    let (Some(password_field), Some(reason_field)) = (state.password.clone(), state.reason.clone())
    else {
        return;
    };
    let (password, reason) = (password.to_owned(), reason.to_owned());
    password_field.update(cx, |field, cx| field.set_text(password, cx));
    reason_field.update(cx, |field, cx| field.set_text(reason, cx));
    submit(console, window, cx);
}

fn render(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<SignState>();
    let (Some(action), Some(password), Some(reason)) = (
        state.action.clone(),
        state.password.clone(),
        state.reason.clone(),
    ) else {
        return div().into_any_element();
    };
    let (meaning, error, busy) = (
        state.request.meaning.clone(),
        state.error.clone(),
        state.busy,
    );
    let mut body = vec![
        div()
            .flex()
            .flex_col()
            .gap(SPACE_2)
            .child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child("Meaning of this signature"),
            )
            .child(
                div()
                    .p(SPACE_5)
                    .rounded(RADIUS_MD)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.hover)
                    .child(meaning),
            )
            .into_any_element(),
        field("Password", password, cx).into_any_element(),
        field("Reason (required)", reason, cx).into_any_element(),
    ];
    if let Some(error) = error {
        body.push(note_line(Icon::CircleAlert, error, theme.danger).into_any_element());
    } else if busy {
        body.push(
            note_line(
                Icon::Clock,
                "Sending the signed action…".to_owned(),
                theme.fg_dim,
            )
            .into_any_element(),
        );
    }
    dialog(
        format!("Sign: {}", action.describe()),
        "This action needs your electronic signature. Enter signs; Esc cancels.",
        body,
        vec![
            Button::new("sign-cancel", "Cancel")
                .ghost()
                .disabled(busy)
                .on_click(cx.listener(|console, _, window, cx| cancel(console, window, cx)))
                .into_any_element(),
            Button::new("sign-send", if busy { "Signing…" } else { "Sign" })
                .primary()
                .icon(Icon::Check)
                .disabled(busy)
                .on_click(cx.listener(|console, _, window, cx| submit(console, window, cx)))
                .into_any_element(),
        ],
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::testing::Harness;
    use gpui::TestAppContext;
    use voltrus_scada_native::runtime::Event;

    fn asking() -> Ask {
        Ask {
            request: SignatureRequired {
                action: "alarm_ack".into(),
                meaning: "I reviewed this alarm".into(),
            },
            error: None,
        }
    }

    fn type_into(h: &mut Harness, password: &str, reason: &str) {
        let (password, reason) = (password.to_owned(), reason.to_owned());
        h.update(|console, _, cx| {
            let state = console.state::<SignState>();
            let (Some(p), Some(r)) = (state.password.clone(), state.reason.clone()) else {
                panic!("the prompt has its fields");
            };
            p.update(cx, |field, cx| field.set_text(password, cx));
            r.update(cx, |field, cx| field.set_text(reason, cx));
        });
    }

    fn password_text(h: &mut Harness) -> String {
        h.update(|console, _, cx| {
            console
                .state::<SignState>()
                .password
                .as_ref()
                .map(|field| field.read(cx).value().to_owned())
                .unwrap_or_default()
        })
    }

    #[gpui::test]
    fn an_acknowledge_is_signed_and_sent_again(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("operator");
        h.commands();
        h.event(Event::SignatureNeeded {
            action: Signable::Acknowledge(3),
            ask: asking(),
        });
        assert!(h.update(|console, _, _| console.overlay_is(ID)));
        // Nothing goes without a reason.
        type_into(&mut h, "secret", "  ");
        h.update(submit);
        assert!(h.commands().is_empty());
        type_into(&mut h, "secret", "checked the pump");
        h.update(submit);
        let sent = h.commands();
        let [Command::Sign { action, signature }] = sent.as_slice() else {
            panic!("expected a signed command: {sent:?}");
        };
        assert_eq!(*action, Signable::Acknowledge(3));
        assert_eq!(signature.meaning, "I reviewed this alarm");
        assert_eq!(signature.reason, "checked the pump");
        assert_eq!(password_text(&mut h), "", "the password is cleared");
        // A wrong password asks again, the reason kept.
        h.event(Event::SignatureNeeded {
            action: Signable::Acknowledge(3),
            ask: Ask {
                error: Some("Invalid password".into()),
                ..asking()
            },
        });
        let (error, reason) = h.update(|console, _, cx| {
            let state = console.state::<SignState>();
            let reason = state
                .reason
                .as_ref()
                .map(|field| field.read(cx).value().to_owned());
            (state.error.clone(), reason)
        });
        assert_eq!(error.as_deref(), Some("Invalid password"));
        assert_eq!(reason.as_deref(), Some("checked the pump"));
        type_into(&mut h, "right", "checked the pump");
        h.update(submit);
        assert_eq!(h.commands().len(), 1);
        // The server took it: the prompt closes.
        h.event(Event::Signed {
            action: Signable::Acknowledge(3),
            result: Ok(None),
        });
        assert!(!h.update(|console, _, _| console.overlay_is(ID)));
    }

    #[gpui::test]
    fn a_locked_account_closes_the_prompt(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("operator");
        h.event(Event::SignatureNeeded {
            action: Signable::AcknowledgeAll,
            ask: asking(),
        });
        type_into(&mut h, "secret", "end of shift");
        h.update(submit);
        h.event(Event::Signed {
            action: Signable::AcknowledgeAll,
            result: Err("Account locked".into()),
        });
        assert!(!h.update(|console, _, _| console.overlay_is(ID)));
    }

    #[gpui::test]
    fn esc_sends_nothing_and_forgets_the_password(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("operator");
        h.commands();
        h.event(Event::SignatureNeeded {
            action: Signable::Acknowledge(1),
            ask: asking(),
        });
        type_into(&mut h, "secret", "why");
        h.update(cancel);
        assert!(h.commands().is_empty());
        assert!(!h.update(|console, _, _| console.overlay_is(ID)));
        assert_eq!(password_text(&mut h), "");
    }
}
