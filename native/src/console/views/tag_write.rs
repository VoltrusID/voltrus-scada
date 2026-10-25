//! Writing to a tag, and its write policy. Operators (admin, demo,
//! integration) write through `POST /api/v1/tags/{id}/write`, immediate or
//! queued, with a reason, checked against the policy and answered with the
//! command's id; an admin can also write directly over the WebSocket. The
//! policy editor sends `PATCH /api/v1/data-sources/{id}/write-policy`. The
//! server has no route that reads a policy back, so the console checks
//! limits only for a policy it saved itself this session.

use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{AnyElement, ClipboardItem, Context, Entity, Subscription, Window, div, px};
use reqwest::Method;
use serde::de::IgnoredAny;
use voltrus_scada_native::runtime::{Call, CallKey, Command, Reply};
use voltrus_scada_native::tag_api::{self, PolicyPatch, WriteAccepted, WriteMode, WritePolicy};
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::components::{Button, IconButton, Segment, field, kbd, section_label, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::tag_detail::{dialog, note_line, on_field, open_index, text_field, write_keys};
use crate::console::Console;
use crate::console::overlay::Overlay;

#[derive(Default)]
pub(crate) struct WriteState {
    mode: Option<WriteMode>,
    reason: Option<Entity<TextInput>>,
    /// Policies saved this session, by tag id.
    policies: HashMap<i64, WritePolicy>,
    /// The tag whose REST write awaits the server.
    sending: Option<i64>,
    /// The last REST write's answer, by tag.
    result: Option<(i64, Result<WriteAccepted, String>)>,
    /// Why the dialog's value cannot go, shown once the operator tried.
    error: Option<String>,
    // The policy editor.
    min: Option<Entity<TextInput>>,
    max: Option<Entity<TextInput>>,
    rate: Option<Entity<TextInput>>,
    deadband: Option<Entity<TextInput>>,
    /// The editor's choices; `None` while unmade, for a policy the console
    /// does not know.
    writable: Option<bool>,
    requires_reason: Option<bool>,
    policy_error: Option<String>,
    /// The change on its way to the server, and for which tag.
    saving: Option<(i64, PolicyPatch)>,
    saved: Option<i64>,
    subscriptions: Vec<Subscription>,
}

/// The tag's policy when this session saved one.
fn known_policy(console: &mut Console, id: i64) -> Option<WritePolicy> {
    console.state::<WriteState>().policies.get(&id).cloned()
}

/// ⇧⌘W: the write dialog for the open tag.
pub(crate) fn open_write(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let modes = WriteMode::allowed(console.is_admin(), console.is_operator());
    if modes.is_empty() {
        console.show_toast(
            "Writing needs the operator role (admin, demo or integration)",
            true,
            cx,
        );
        return;
    }
    if console.state::<WriteState>().reason.is_none() {
        let reason = text_field("Why (sent with the command)", InputKind::Text, false, cx);
        let subscriptions = [
            on_field(&reason, submit, window, cx),
            // The value field is the console's own; its Esc closes the
            // dialog too (its other subscription refocuses the page).
            cx.subscribe_in(
                &console.inputs.write,
                window,
                |console, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Cancelled if console.overlay_is("tag-write") => {
                        console.close_overlay(window, cx);
                    }
                    InputEvent::Changed => cx.notify(),
                    _ => {}
                },
            ),
        ];
        let state = console.state::<WriteState>();
        state.subscriptions.extend(subscriptions);
        state.reason = Some(reason);
    }
    let state = console.state::<WriteState>();
    // Each opening starts afresh: the policy-checked, recorded write and an
    // empty value, never a mode or value an earlier write left behind.
    state.mode = Some(WriteMode::Immediate);
    state.error = None;
    // A REST answer about another tag no longer applies.
    let id = console.selected;
    let state = console.state::<WriteState>();
    if state.result.as_ref().is_some_and(|(at, _)| Some(*at) != id) {
        state.result = None;
    }
    console
        .inputs
        .write
        .update(cx, |field, cx| field.set_text("", cx));
    let overlay = Overlay::modal("tag-write", px(460.0), write_dialog, cx);
    console.open_overlay(overlay, window, cx);
    console
        .inputs
        .write
        .update(cx, |field, _| field.focus(window));
}

/// Enter in the dialog, or its Write button (also `Console::write_selected`).
pub(crate) fn submit(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(index) = open_index(console) else {
        return;
    };
    let id = console.store.tags[index].info.id;
    // One write at a time.
    if console.writing.is_some() || console.state::<WriteState>().sending.is_some() {
        return;
    }
    let text = console.inputs.write.read(cx).value().to_owned();
    let policy = known_policy(console, id);
    let state = console.state::<WriteState>();
    let mode = state.mode.unwrap_or(WriteMode::Immediate);
    let reason = state
        .reason
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    let value = match tag_api::check_write(&text, &reason, mode, policy.as_ref()) {
        Ok(value) => value,
        Err(error) => {
            state.error = Some(error);
            cx.notify();
            return;
        }
    };
    state.error = None;
    match tag_api::write_request(value, &reason, mode) {
        Some(body) => {
            state.sending = Some(id);
            state.result = None;
            let path = format!("/api/v1/tags/{id}/write");
            console.call(
                CallKey::new("tags.write").with(id),
                Call::new(move |api| async move {
                    api.send_json::<_, WriteAccepted>(Method::POST, &path, &body)
                        .await
                }),
            );
        }
        None => {
            console.write_result = None;
            console.writing = Some(id);
            console.send(Command::Write { id, value });
        }
    }
    cx.notify();
}

/// The policy editor for the open tag (operators).
pub(crate) fn open_policy(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.show_toast(
            "Write policies need the operator role (admin, demo or integration)",
            true,
            cx,
        );
        return;
    }
    let Some(index) = open_index(console) else {
        return;
    };
    let id = console.store.tags[index].info.id;
    if console.state::<WriteState>().min.is_none() {
        let min = text_field("No minimum", InputKind::Number, true, cx);
        let max = text_field("No maximum", InputKind::Number, true, cx);
        let rate = text_field("No limit", InputKind::Unsigned, true, cx);
        let deadband = text_field("None", InputKind::Number, true, cx);
        let subscriptions = [
            on_field(&min, save_policy, window, cx),
            on_field(&max, save_policy, window, cx),
            on_field(&rate, save_policy, window, cx),
            on_field(&deadband, save_policy, window, cx),
        ];
        let state = console.state::<WriteState>();
        state.subscriptions.extend(subscriptions);
        state.min = Some(min);
        state.max = Some(max);
        state.rate = Some(rate);
        state.deadband = Some(deadband);
    }
    let known = known_policy(console, id);
    let policy = known.clone().unwrap_or_default();
    let state = console.state::<WriteState>();
    state.writable = known.as_ref().map(|policy| policy.writable);
    state.requires_reason = known.as_ref().map(|policy| policy.write_requires_reason);
    state.policy_error = None;
    state.saved = None;
    let (Some(min), Some(max), Some(rate), Some(deadband)) = (
        state.min.clone(),
        state.max.clone(),
        state.rate.clone(),
        state.deadband.clone(),
    ) else {
        return;
    };
    let number = |value: Option<f64>| value.map(|value| value.to_string()).unwrap_or_default();
    let fields = [
        (&min, number(policy.write_min)),
        (&max, number(policy.write_max)),
        (
            &rate,
            policy
                .write_max_rate
                .map(|rate| rate.to_string())
                .unwrap_or_default(),
        ),
        (&deadband, number(policy.write_deadband)),
    ];
    for (field, text) in fields {
        field.update(cx, |field, cx| field.set_text(text, cx));
    }
    let overlay = Overlay::modal("tag-policy", px(480.0), policy_dialog, cx);
    console.open_overlay(overlay, window, cx);
    min.update(cx, |field, _| field.focus(window));
}

fn save_policy(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(index) = open_index(console) else {
        return;
    };
    let id = console.store.tags[index].info.id;
    let known = known_policy(console, id);
    let state = console.state::<WriteState>();
    if state.saving.is_some() {
        return;
    }
    let text = |field: &Option<Entity<TextInput>>| {
        field
            .as_ref()
            .map(|field| field.read(cx).value().to_owned())
            .unwrap_or_default()
    };
    let parsed = tag_api::policy_patch(
        known.as_ref(),
        state.writable,
        state.requires_reason,
        &text(&state.min),
        &text(&state.max),
        &text(&state.rate),
        &text(&state.deadband),
    );
    match parsed {
        Ok(patch) if patch.is_empty() => {
            state.policy_error = Some("Nothing to change".to_owned());
        }
        Ok(patch) => {
            state.policy_error = None;
            state.saved = None;
            state.saving = Some((id, patch.clone()));
            let path = format!("/api/v1/data-sources/{id}/write-policy");
            console.call(
                CallKey::new("tags.policy").with(id),
                Call::new(move |api| async move {
                    api.send_json::<_, IgnoredAny>(Method::PATCH, &path, &patch)
                        .await
                }),
            );
        }
        Err(error) => state.policy_error = Some(error),
    }
    cx.notify();
}

/// A direct write of tag `tag` landed: its value leaves the dialog, so a
/// repeated Enter cannot send it again.
pub(crate) fn written(console: &mut Console, tag: i64, cx: &mut Context<Console>) {
    if console.selected == Some(tag) {
        console
            .inputs
            .write
            .update(cx, |field, cx| field.set_text("", cx));
    }
}

pub(crate) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    match key.name {
        "tags.write" => {
            let result = reply.take::<WriteAccepted>();
            let open = console.overlay_is("tag-write");
            let state = console.state::<WriteState>();
            state.sending = None;
            state.result = Some((key.arg, result.clone()));
            // The value went: Enter again must not send it a second time.
            if result.is_ok() {
                written(console, key.arg, cx);
            }
            // With the dialog closed, the answer still shows.
            if !open {
                match result {
                    Ok(answer) => console.show_toast(
                        format!("Write submitted: command {}", answer.command_id),
                        false,
                        cx,
                    ),
                    Err(error) => console.show_toast(format!("Write failed: {error}"), true, cx),
                }
            }
        }
        "tags.policy" => {
            let saving = console.state::<WriteState>().saving.take();
            match (reply.error(), saving) {
                (None, Some((id, patch))) => {
                    let known = known_policy(console, id);
                    let state = console.state::<WriteState>();
                    // Known from here on only when the patch and what was
                    // known before make up the whole policy.
                    if let Some(policy) = patch.applied(known.as_ref()) {
                        state.policies.insert(id, policy);
                    }
                    state.saved = Some(id);
                    console.show_toast("Write policy saved", false, cx);
                }
                (Some(error), _) => {
                    console.state::<WriteState>().policy_error = Some(error.to_owned());
                }
                (None, None) => {}
            }
        }
        _ => {}
    }
    cx.notify();
}

/// A policy in a line: `writable · 0 to 100 · reason · 6 per minute`.
fn describe(policy: &WritePolicy) -> String {
    if !policy.writable {
        return "Not writable".to_owned();
    }
    let mut parts = vec!["Writable".to_owned()];
    match (policy.write_min, policy.write_max) {
        (Some(min), Some(max)) => parts.push(format!("{min} to {max}")),
        (Some(min), None) => parts.push(format!("at least {min}")),
        (None, Some(max)) => parts.push(format!("at most {max}")),
        (None, None) => {}
    }
    if policy.write_requires_reason {
        parts.push("reason required".to_owned());
    }
    if let Some(rate) = policy.write_max_rate {
        parts.push(format!("{rate} per minute"));
    }
    if let Some(deadband) = policy.write_deadband {
        parts.push(format!("deadband {deadband}"));
    }
    parts.join(" · ")
}

fn mode_note(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::Direct => {
            "Over the WebSocket, at once. The server checks only that the tag is writable; no command record, no reason."
        }
        WriteMode::Immediate => {
            "Through the command queue now: checked against the write policy and recorded with its id."
        }
        WriteMode::Queued => {
            "Queued: the command queue sends it in turn, checked and recorded like an immediate write."
        }
    }
}

/// The detail panel's write section.
pub(crate) fn section(console: &mut Console, id: i64, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let policy = known_policy(console, id);
    let last = console
        .state::<WriteState>()
        .result
        .clone()
        .filter(|(at, _)| *at == id)
        .map(|(_, result)| result);
    let direct = console
        .write_result
        .as_ref()
        .filter(|(at, _)| *at == id)
        .map(|(_, result)| result.clone());
    div()
        .flex()
        .flex_col()
        .gap(SPACE_4)
        .px(SPACE_8)
        .py(SPACE_7)
        .border_t_1()
        .border_color(theme.border)
        .child(section_label("Write", cx))
        .map(|this| {
            if !operator {
                return this.child(note_line(
                    Icon::Lock,
                    "Writing values needs the operator role (admin, demo or integration)."
                        .to_owned(),
                    theme.fg_dim,
                ));
            }
            this.child(note_line(
                Icon::Info,
                policy.as_ref().map_or_else(
                    || {
                        "Policy unknown: the server cannot be asked for it, and checks each write."
                            .to_owned()
                    },
                    |policy| format!("Policy: {}", describe(policy)),
                ),
                theme.fg_dim,
            ))
            .when_some(last, |this, result| {
                this.child(match result {
                    Ok(answer) => note_line(
                        Icon::CircleCheck,
                        format!("Command {} {}", answer.command_id, answer.status),
                        theme.success,
                    ),
                    Err(error) => note_line(Icon::CircleAlert, error, theme.danger),
                })
            })
            .when_some(direct, |this, result| {
                this.child(match result {
                    Ok(detail) => note_line(
                        Icon::CircleCheck,
                        format!("Written: {detail}"),
                        theme.success,
                    ),
                    Err(error) => note_line(Icon::CircleAlert, error, theme.danger),
                })
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_3)
                    .child(
                        Button::new("open-write", "Write…")
                            .primary()
                            .icon(Icon::Send)
                            .tooltip(write_keys())
                            .on_click(cx.listener(|console, _, window, cx| {
                                open_write(console, window, cx);
                            })),
                    )
                    .child(
                        Button::new("open-policy", "Write policy…")
                            .icon(Icon::Settings)
                            .on_click(cx.listener(|console, _, window, cx| {
                                open_policy(console, window, cx);
                            })),
                    ),
            )
        })
        .into_any_element()
}

fn write_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(index) = open_index(console) else {
        return div().into_any_element();
    };
    let tag = &console.store.tags[index];
    let (id, name) = (tag.info.id, tag.info.name.clone());
    let current = if tag.value.is_finite() {
        format!("{} {}", tag.display(), tag.info.unit)
            .trim_end()
            .to_owned()
    } else {
        "no value yet".to_owned()
    };
    let modes = WriteMode::allowed(console.is_admin(), console.is_operator());
    let policy = known_policy(console, id);
    let text = console.inputs.write.read(cx).value().to_owned();
    let direct_busy = console.writing == Some(id);
    let direct = console
        .write_result
        .as_ref()
        .filter(|(at, _)| *at == id)
        .map(|(_, result)| result.clone());
    let value_field = console.inputs.write.clone();
    let state = console.state::<WriteState>();
    let mode = state.mode.unwrap_or(WriteMode::Immediate);
    let Some(reason_field) = state.reason.clone() else {
        return div().into_any_element();
    };
    let reason = reason_field.read(cx).value().to_owned();
    let sending = state.sending == Some(id);
    let result = state
        .result
        .clone()
        .filter(|(at, _)| *at == id)
        .map(|(_, result)| result);
    // Checked as the operator types, once there is something to check.
    let problem = state.error.clone().or_else(|| {
        (!text.trim().is_empty())
            .then(|| tag_api::check_write(&text, &reason, mode, policy.as_ref()).err())
            .flatten()
    });
    let needs_reason = policy
        .as_ref()
        .is_some_and(|policy| policy.write_requires_reason);
    let busy = sending || direct_busy;

    let segments = modes
        .iter()
        .map(|choice| {
            let choice = *choice;
            Segment::new(
                choice.label(),
                choice == mode,
                cx.listener(move |console, _, _, cx| {
                    let state = console.state::<WriteState>();
                    state.mode = Some(choice);
                    state.error = None;
                    cx.notify();
                }),
            )
        })
        .collect();

    let outcome: Option<AnyElement> = if mode == WriteMode::Direct {
        if direct_busy {
            Some(note_line(Icon::Clock, "Writing…".to_owned(), theme.fg_dim).into_any_element())
        } else {
            direct.map(|result| match result {
                Ok(detail) => note_line(
                    Icon::CircleCheck,
                    format!("Written: {detail}"),
                    theme.success,
                )
                .into_any_element(),
                Err(error) => note_line(Icon::CircleAlert, error, theme.danger).into_any_element(),
            })
        }
    } else if sending {
        Some(note_line(Icon::Clock, "Submitting…".to_owned(), theme.fg_dim).into_any_element())
    } else {
        result.map(|result| match result {
            Ok(answer) => {
                let command = answer.command_id.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2)
                    .p(SPACE_5)
                    .rounded(RADIUS_MD)
                    .border_1()
                    .border_color(theme.success.alpha(0.5))
                    .bg(theme.success.alpha(0.08))
                    .child(
                        div()
                            .text_color(theme.success)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!(
                                "Command submitted, {} at {}",
                                answer.status,
                                format_value(answer.value)
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_3)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .child(answer.command_id),
                            )
                            .child(
                                IconButton::new("copy-command", Icon::Copy)
                                    .tooltip("Copy the command id")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            command.clone(),
                                        ));
                                    })),
                            ),
                    )
                    .into_any_element()
            }
            Err(error) => note_line(Icon::CircleAlert, error, theme.danger).into_any_element(),
        })
    };

    let mut body = vec![
        segmented("write-mode", segments, cx),
        note_line(Icon::Info, mode_note(mode).to_owned(), theme.fg_dim).into_any_element(),
        field(format!("New value (now {current})"), value_field, cx).into_any_element(),
    ];
    if mode != WriteMode::Direct {
        body.push(
            field(
                if needs_reason {
                    "Reason (required)"
                } else {
                    "Reason (optional)"
                },
                reason_field,
                cx,
            )
            .into_any_element(),
        );
    }
    body.push(
        note_line(
            Icon::Settings,
            policy.as_ref().map_or_else(
                || "Policy unknown here; the server checks the write and answers.".to_owned(),
                |policy| format!("Policy: {}", describe(policy)),
            ),
            theme.fg_dim,
        )
        .into_any_element(),
    );
    if let Some(problem) = problem {
        body.push(note_line(Icon::CircleAlert, problem, theme.danger).into_any_element());
    }
    body.extend(outcome);

    dialog(
        format!("Write to {name}"),
        "Enter writes; Esc closes.",
        body,
        vec![
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(kbd(write_keys(), cx))
                .into_any_element(),
            div().flex_1().into_any_element(),
            Button::new("write-close", "Close")
                .ghost()
                .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx)))
                .into_any_element(),
            Button::new("write-send", if busy { "Writing…" } else { "Write" })
                .primary()
                .icon(Icon::Send)
                .disabled(busy)
                .on_click(cx.listener(|console, _, window, cx| submit(console, window, cx)))
                .into_any_element(),
        ],
        cx,
    )
}

fn policy_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(index) = open_index(console) else {
        return div().into_any_element();
    };
    let (id, name) = {
        let tag = &console.store.tags[index];
        (tag.info.id, tag.info.name.clone())
    };
    let known = known_policy(console, id).is_some();
    let state = console.state::<WriteState>();
    let (Some(min), Some(max), Some(rate), Some(deadband)) = (
        state.min.clone(),
        state.max.clone(),
        state.rate.clone(),
        state.deadband.clone(),
    ) else {
        return div().into_any_element();
    };
    let (writable, requires_reason) = (state.writable, state.requires_reason);
    let error = state.policy_error.clone();
    let saving = state.saving.as_ref().is_some_and(|(at, _)| *at == id);
    let saved = state.saved == Some(id);
    // Each segment sets its own choice.
    let choice = |label: &'static str, selected: bool, apply: fn(&mut WriteState)| {
        Segment::new(
            label,
            selected,
            cx.listener(move |console, _, _, cx| {
                apply(console.state::<WriteState>());
                cx.notify();
            }),
        )
    };
    let writable_segments = vec![
        choice("Locked", writable == Some(false), |state| {
            state.writable = Some(false);
        }),
        choice("Writable", writable == Some(true), |state| {
            state.writable = Some(true);
        }),
    ];
    let reason_segments = vec![
        choice("Optional", requires_reason == Some(false), |state| {
            state.requires_reason = Some(false);
        }),
        choice("Required", requires_reason == Some(true), |state| {
            state.requires_reason = Some(true);
        }),
    ];
    let pair = |left: AnyElement, right: AnyElement| {
        div()
            .flex()
            .gap(SPACE_5)
            .child(div().flex_1().min_w_0().child(left))
            .child(div().flex_1().min_w_0().child(right))
            .into_any_element()
    };
    let mut body = vec![
        pair(
            field("Writes", segmented("policy-writable", writable_segments, cx), cx)
                .into_any_element(),
            field("Reason", segmented("policy-reason", reason_segments, cx), cx)
                .into_any_element(),
        ),
        pair(
            field("Minimum", min, cx).into_any_element(),
            field("Maximum", max, cx).into_any_element(),
        ),
        pair(
            field("Writes per minute", rate, cx).into_any_element(),
            field("Deadband", deadband, cx).into_any_element(),
        ),
        note_line(
            Icon::Info,
            if known {
                "Saving sends the settings changed here; an emptied field clears its limit."
            } else {
                "The server cannot be asked for the current policy: saving sends only the choices made and the limits typed, and leaves the rest as stored."
            }
            .to_owned(),
            theme.fg_dim,
        )
        .into_any_element(),
    ];
    if let Some(error) = error {
        body.push(note_line(Icon::CircleAlert, error, theme.danger).into_any_element());
    } else if saved {
        body.push(
            note_line(Icon::CircleCheck, "Saved".to_owned(), theme.success).into_any_element(),
        );
    }
    dialog(
        format!("Write policy of {name}"),
        "Who may write, and within what limits. Enter saves; Esc closes.",
        body,
        vec![
            Button::new("policy-close", "Close")
                .ghost()
                .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx)))
                .into_any_element(),
            Button::new(
                "policy-save",
                if saving { "Saving…" } else { "Save policy" },
            )
            .primary()
            .icon(Icon::Check)
            .disabled(saving)
            .on_click(cx.listener(|console, _, window, cx| save_policy(console, window, cx)))
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

    /// Signed in as `role` with tag 7 open on the Tags page.
    fn writing(cx: &mut TestAppContext, role: &str) -> Harness {
        let mut h = Harness::new(cx);
        h.sign_in(role);
        h.event(Event::Tags(vec![Harness::tag(7, "plts_1.p")]));
        h.update(|console, window, cx| console.open_tag(7, window, cx));
        h.commands();
        h
    }

    fn type_value(h: &mut Harness, text: &str) {
        let text = text.to_owned();
        h.update(|console, _, cx| {
            console
                .inputs
                .write
                .update(cx, |field, cx| field.set_text(text, cx));
        });
    }

    #[gpui::test]
    fn enter_after_a_successful_write_sends_nothing(cx: &mut TestAppContext) {
        let mut h = writing(cx, "demo");
        h.update(|console, window, cx| open_write(console, window, cx));
        type_value(&mut h, "42");
        h.draw();
        h.keys("enter");
        assert_eq!(h.calls(), vec![CallKey::new("tags.write").with(7)]);
        let accepted = WriteAccepted {
            command_id: "c1".into(),
            status: "pending".into(),
            data_source_id: 7,
            value: 42.0,
            created_at: 0,
        };
        h.reply(CallKey::new("tags.write").with(7), Reply::ok(accepted));
        h.draw();
        h.keys("enter");
        assert!(h.commands().is_empty());
    }

    #[gpui::test]
    fn enter_after_a_direct_write_landed_sends_nothing(cx: &mut TestAppContext) {
        let mut h = writing(cx, "admin");
        h.update(|console, window, cx| {
            open_write(console, window, cx);
            console.state::<WriteState>().mode = Some(WriteMode::Direct);
        });
        type_value(&mut h, "5");
        h.draw();
        h.keys("enter");
        let sent = h.commands();
        assert!(
            matches!(sent.as_slice(), [Command::Write { id: 7, .. }]),
            "{sent:?}"
        );
        h.event(Event::Written {
            tag: 7,
            result: Ok("5".into()),
            late: false,
        });
        h.draw();
        h.keys("enter");
        assert!(h.commands().is_empty());
    }

    /// An earlier write's mode and value never carry into the next opening.
    #[gpui::test]
    fn the_write_dialog_opens_empty_and_immediate(cx: &mut TestAppContext) {
        let mut h = writing(cx, "admin");
        h.update(|console, window, cx| {
            open_write(console, window, cx);
            console.state::<WriteState>().mode = Some(WriteMode::Direct);
        });
        type_value(&mut h, "5");
        h.update(|console, window, cx| {
            console.close_overlay(window, cx);
            open_write(console, window, cx);
        });
        let (text, mode) = h.update(|console, _, cx| {
            let text = console.inputs.write.read(cx).value().to_owned();
            (text, console.state::<WriteState>().mode)
        });
        assert_eq!(text, "");
        assert_eq!(mode, Some(WriteMode::Immediate));
    }

    #[test]
    fn a_policy_reads_as_one_line() {
        assert_eq!(describe(&WritePolicy::default()), "Not writable");
        let policy = WritePolicy {
            writable: true,
            write_min: Some(0.0),
            write_max: Some(100.0),
            write_max_rate: Some(6),
            write_requires_reason: true,
            write_deadband: Some(0.5),
        };
        assert_eq!(
            describe(&policy),
            "Writable · 0 to 100 · reason required · 6 per minute · deadband 0.5"
        );
        let floor = WritePolicy {
            writable: true,
            write_min: Some(-5.0),
            ..WritePolicy::default()
        };
        assert_eq!(describe(&floor), "Writable · at least -5");
    }
}
