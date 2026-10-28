//! Data Manager › Notifications (operator), after the web client's
//! `NotificationSettingsPanel.tsx`: who the server tells when a tag (or any
//! tag) alarms, by email, webhook or push. Rules list with their targets
//! and state; new and edit (the tag key, the channel, the targets one per
//! line, on or off); Space turns a rule on or off; T sends a test through
//! the server and shows each target's outcome; delete after a
//! confirmation. Telegram, WhatsApp and other edition channels are listed, tested and
//! deleted here but edited in the web client. Browser push subscriptions
//! are a web-only feature.

use std::collections::HashMap;
use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, EntityInputHandler, Subscription, Window, div, px, uniform_list,
};
use serde::de::IgnoredAny;
use serde_json::json;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::admin::{
    CHANNELS, NotificationRule, TestOutcome, channel, parse_targets, rule_body, test_body,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, empty_state, field, pill, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::admin_kit::{
    self as kit, Confirm, ROW, actions, cell, dialog_body, dialog_footer, dialog_header,
    element_id, header, note, row_frame,
};
use super::sidebar_width;
use crate::console::Console;
use crate::console::keys::{DATA_ADMIN_RULES, DataAdminDelete, DataAdminTest};
use crate::console::overlay::Overlay;
use crate::console::pages::Role;
use crate::console::text_area::TextArea;
use crate::console::views::tag_detail::{on_field, text_field};
use crate::console::views::{
    ListKeys, keyboard_list, page_header, rows_on_screen, tail_row, tail_rows,
};

pub(crate) const LIST: &str = "data.notifications";
const FORM: &str = "data-rule-form";
const CHANNEL: f32 = 84.0;
const TARGETS: f32 = 250.0;
const STATE: f32 = 56.0;
const TESTED: f32 = 190.0;
const ACTIONS: f32 = 96.0;

struct Form {
    /// The rule being edited; `None` for a new one.
    original: Option<NotificationRule>,
    source_key: Entity<TextInput>,
    targets: Entity<TextArea>,
    channel: &'static str,
    enabled: bool,
    error: Option<String>,
    busy: bool,
    testing: bool,
    test: Option<Result<TestOutcome, String>>,
    /// This form's number: its save's key, so an older form's answer
    /// leaves it alone.
    seq: i64,
    /// The test the form waits for (`data.notifications.form-test`); a
    /// new channel or new targets drop it.
    test_seq: Option<i64>,
    _subscriptions: Vec<Subscription>,
}

impl Form {
    /// Drops the last form test and any on its way: it went to another
    /// channel or other targets.
    fn forget_test(&mut self) {
        self.test = None;
        self.testing = false;
        self.test_seq = None;
    }
}

#[derive(Default)]
pub(crate) struct State {
    rules: Option<Result<Vec<NotificationRule>, String>>,
    asked: bool,
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    form: Option<Form>,
    /// Numbers forms and form tests.
    seq: i64,
    deleting: Option<NotificationRule>,
    /// The last test of each rule, by id.
    tests: HashMap<i64, Result<TestOutcome, String>>,
    /// Rules with a test or a switch on the way.
    busy: Vec<i64>,
}

fn fetch(console: &mut Console, again: bool) {
    if console.stress.is_some() || !console.is_operator() {
        return;
    }
    let state = console.state::<State>();
    if state.asked && !again {
        return;
    }
    state.asked = true;
    console.call(
        CallKey::new("data.notifications.list"),
        Call::get::<Vec<NotificationRule>>("/api/v1/notifications/settings"),
    );
}

pub fn on_open(console: &mut Console, _: &mut Context<Console>) {
    fetch(console, false);
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console.state_ref::<State>()?.search.clone()
}

fn rules(console: &Console) -> Vec<NotificationRule> {
    match console
        .state_ref::<State>()
        .and_then(|state| state.rules.as_ref())
    {
        Some(Ok(rules)) => rules.clone(),
        _ => Vec::new(),
    }
}

fn rows(console: &Console, cx: &gpui::App) -> Vec<usize> {
    let Some(state) = console.state_ref::<State>() else {
        return Vec::new();
    };
    let query = kit::query(state.search.as_ref(), cx);
    match &state.rules {
        Some(Ok(rules)) => rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| {
                query.is_empty()
                    || rule.source_key.to_lowercase().contains(&query)
                    || rule.channel_type.contains(&query)
                    || rule.config.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect(),
        _ => Vec::new(),
    }
}

fn cursor_rule(console: &mut Console, cx: &mut Context<Console>) -> Option<NotificationRule> {
    let row = console.list(LIST, cx).cursor?;
    let index = *rows(console, cx).get(row)?;
    rules(console).get(index).cloned()
}

/// The tag a rule's key names, as the store has it.
fn tag_name(console: &Console, key: &str) -> Option<String> {
    console
        .store
        .by_key(key)
        .map(|index| console.store.tags[index].info.name.clone())
}

/// `3 of 3 sent`, or the first failure.
fn outcome_line(outcome: &Result<TestOutcome, String>) -> (String, bool) {
    match outcome {
        Ok(outcome) if outcome.results.is_empty() => ("no targets".to_owned(), false),
        Ok(outcome) => {
            let sent = outcome.sent();
            let total = outcome.results.len();
            match outcome.results.iter().find(|result| !result.sent()) {
                None => (format!("{sent} of {total} sent"), true),
                Some(failed) => (
                    format!(
                        "{sent} of {total} sent · {}: {}",
                        failed.target(),
                        failed.error.as_deref().unwrap_or(&failed.status)
                    ),
                    false,
                ),
            }
        }
        Err(error) => (error.clone(), false),
    }
}

// ── Actions ──────────────────────────────────────────────────────────────

/// "New notification rule" (N, the palette).
pub fn new_rule(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if kit::open_section(
        console,
        "notifications",
        Role::Operator,
        "Managing notifications",
        window,
        cx,
    ) {
        open_form(console, None, window, cx);
    }
}

fn open_form(
    console: &mut Console,
    original: Option<NotificationRule>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(rule) = &original
        && !rule.editable()
    {
        console.show_toast(
            format!("{} rules are edited in the web client", rule.channel_type),
            true,
            cx,
        );
        return;
    }
    let source_key = text_field("Every tag (or a key: ac_power)", InputKind::Text, true, cx);
    let targets = cx.new(|cx| TextArea::new(4, cx).placeholder(CHANNELS[0].placeholder));
    let (channel_id, enabled) = match &original {
        Some(rule) => {
            let key = rule.source_key.clone();
            source_key.update(cx, |field, cx| field.set_text(key, cx));
            let text = rule.targets().join("\n");
            targets.update(cx, |area, cx| {
                area.replace_text_in_range(None, &text, window, cx)
            });
            (
                channel(&rule.channel_type).map_or("email", |channel| channel.id),
                rule.enabled,
            )
        }
        None => ("email", true),
    };
    let mut subscriptions = vec![on_field(&source_key, submit, window, cx)];
    subscriptions.push(cx.subscribe_in(
        &targets,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted => submit(console, window, cx),
            InputEvent::Cancelled => console.close_overlay(window, cx),
            InputEvent::Changed => {
                if let Some(form) = console.state::<State>().form.as_mut() {
                    form.error = None;
                    // The last test went to the old targets.
                    form.forget_test();
                }
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    ));
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    state.form = Some(Form {
        original,
        source_key: source_key.clone(),
        targets,
        channel: channel_id,
        enabled,
        error: None,
        busy: false,
        testing: false,
        test: None,
        seq,
        test_seq: None,
        _subscriptions: subscriptions,
    });
    kit::remember_window(console, window);
    let overlay = Overlay::modal(FORM, px(560.0), form_dialog, cx);
    console.open_overlay(overlay, window, cx);
    source_key.read(cx).focus(window);
}

fn activate(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let index = rows(console, cx).get(row).copied();
    let Some(rule) = index.and_then(|index| rules(console).get(index).cloned()) else {
        return;
    };
    if toggle {
        switch(console, &rule, cx);
    } else {
        open_form(console, Some(rule), window, cx);
    }
}

/// Turn a rule on or off: `PUT …/settings/{id} {enabled}`.
fn switch(console: &mut Console, rule: &NotificationRule, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.busy.contains(&rule.id) {
        return;
    }
    state.busy.push(rule.id);
    let (id, on) = (rule.id, !rule.enabled);
    console.call(
        CallKey::new("data.notifications.switch").with(id),
        Call::new(move |api| async move {
            api.send_json::<_, IgnoredAny>(
                Method::PUT,
                &format!("/api/v1/notifications/settings/{id}"),
                &json!({ "enabled": on }),
            )
            .await?;
            anyhow::Ok(on)
        }),
    );
    cx.notify();
}

fn test_rule(console: &mut Console, rule: &NotificationRule, cx: &mut Context<Console>) {
    if !channel(&rule.channel_type).is_none_or(|channel| channel.testable) {
        console.show_toast("Push is tested from a browser that subscribed", true, cx);
        return;
    }
    let state = console.state::<State>();
    if state.busy.contains(&rule.id) {
        return;
    }
    state.busy.push(rule.id);
    console.call(
        CallKey::new("data.notifications.test").with(rule.id),
        Call::send::<TestOutcome>(Method::POST, "/api/v1/notifications/test", test_body(rule)),
    );
    cx.notify();
}

/// The form's Test: the targets as typed, before saving.
fn test_form(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    let Some(form) = state.form.as_mut() else {
        return;
    };
    if form.testing {
        return;
    }
    let key = form.source_key.read(cx).value().to_owned();
    let text = form.targets.read(cx).value().to_owned();
    match rule_body(&key, form.channel, &text, true) {
        Ok(body) => {
            form.testing = true;
            form.test = None;
            form.error = None;
            form.test_seq = Some(seq);
            let probe = NotificationRule {
                source_key: key.trim().to_owned(),
                channel_type: form.channel.to_owned(),
                config: body["config"].as_str().unwrap_or("[]").to_owned(),
                ..NotificationRule::default()
            };
            console.call(
                CallKey::new("data.notifications.form-test").with(seq),
                Call::send::<TestOutcome>(
                    Method::POST,
                    "/api/v1/notifications/test",
                    test_body(&probe),
                ),
            );
        }
        Err(error) => form.error = Some(error),
    }
    cx.notify();
}

fn ask_delete(
    console: &mut Console,
    rule: NotificationRule,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let what = if rule.source_key.is_empty() {
        "every tag".to_owned()
    } else {
        rule.source_key.clone()
    };
    let confirm = Confirm {
        title: format!("Delete the {} rule for {what}?", rule.channel_type),
        body: format!(
            "{} stop hearing of its alarms. This cannot be undone.",
            rule.targets_summary()
        ),
        verb: "Delete",
        run: delete_confirmed,
    };
    console.state::<State>().deleting = Some(rule);
    kit::ask(console, confirm, window, cx);
}

fn delete_confirmed(console: &mut Console, _: &mut Window, _: &mut Context<Console>) {
    let Some(rule) = console.state::<State>().deleting.take() else {
        return;
    };
    let id = rule.id;
    console.call(
        CallKey::new("data.notifications.delete").with(id),
        Call::send_empty::<IgnoredAny>(
            Method::DELETE,
            format!("/api/v1/notifications/settings/{id}"),
        ),
    );
}

fn submit(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(form) = console.state::<State>().form.as_mut() else {
        return;
    };
    if form.busy {
        return;
    }
    let key = form.source_key.read(cx).value().to_owned();
    let text = form.targets.read(cx).value().to_owned();
    match rule_body(&key, form.channel, &text, form.enabled) {
        Ok(body) => {
            form.busy = true;
            form.error = None;
            let seq = form.seq;
            let call = match &form.original {
                Some(rule) => Call::send::<IgnoredAny>(
                    Method::PUT,
                    format!("/api/v1/notifications/settings/{}", rule.id),
                    body,
                ),
                None => {
                    Call::send::<IgnoredAny>(Method::POST, "/api/v1/notifications/settings", body)
                }
            };
            console.call(CallKey::new("data.notifications.save").with(seq), call);
        }
        Err(error) => form.error = Some(error),
    }
    cx.notify();
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "data.notifications.list" => {
            console.state::<State>().rules = Some(reply.take::<Vec<NotificationRule>>());
        }
        "data.notifications.save" => {
            let state = console.state::<State>();
            let current = state.form.as_ref().is_some_and(|form| form.seq == key.arg);
            match reply.take::<IgnoredAny>() {
                Ok(_) => {
                    // A form opened since then keeps its edits.
                    let saved = if current {
                        let form = state.form.take();
                        kit::close_later(console, FORM, cx);
                        form.map(|form| form.original.is_some())
                    } else {
                        None
                    };
                    fetch(console, true);
                    let verb = if saved == Some(false) {
                        "Added"
                    } else {
                        "Saved"
                    };
                    console.show_toast(format!("{verb} the notification rule"), false, cx);
                }
                Err(error) => match state.form.as_mut().filter(|_| current) {
                    Some(form) => {
                        form.busy = false;
                        form.error = Some(error);
                    }
                    None => {
                        console.show_toast(format!("Saving the rule failed: {error}"), true, cx);
                    }
                },
            }
        }
        "data.notifications.switch" => {
            console.state::<State>().busy.retain(|id| *id != key.arg);
            match reply.take::<bool>() {
                Ok(on) => {
                    fetch(console, true);
                    console.show_toast(if on { "Rule on" } else { "Rule off" }, false, cx);
                }
                Err(error) => {
                    console.show_toast(format!("Switching the rule failed: {error}"), true, cx)
                }
            }
        }
        "data.notifications.form-test" => {
            let outcome = reply.take::<TestOutcome>();
            // Only the test the form still waits for: not one sent before
            // the channel or targets changed, nor an older form's.
            if let Some(form) = console
                .state::<State>()
                .form
                .as_mut()
                .filter(|form| form.test_seq == Some(key.arg))
            {
                form.testing = false;
                form.test_seq = None;
                form.test = Some(outcome);
            }
        }
        "data.notifications.test" => {
            let outcome = reply.take::<TestOutcome>();
            let (line, ok) = outcome_line(&outcome);
            let state = console.state::<State>();
            state.busy.retain(|id| *id != key.arg);
            state.tests.insert(key.arg, outcome);
            console.show_toast(format!("Test: {line}"), !ok, cx);
        }
        "data.notifications.delete" => match reply.take::<IgnoredAny>() {
            Ok(_) => {
                console.state::<State>().tests.remove(&key.arg);
                fetch(console, true);
                console.show_toast("Deleted the notification rule", false, cx);
            }
            Err(error) => console.show_toast(format!("Delete failed: {error}"), true, cx),
        },
        _ => {}
    }
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn channel_color(id: &str, theme: &Theme) -> gpui::Hsla {
    match id {
        "email" => theme.accent,
        "webhook" => theme.info,
        "push" => theme.success,
        _ => theme.warning,
    }
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    if console.state::<State>().search.is_none() {
        let (field, subscription) = kit::search_field(LIST, "Search rules  /", window, cx);
        let state = console.state::<State>();
        state.search = Some(field);
        state._search = Some(subscription);
    }
    if !console.overlay_is(FORM) {
        console.state::<State>().form = None;
    }
    let theme = theme(cx).clone();
    let shown = rows(console, cx);
    let list = rules(console);
    let state = console.state::<State>();
    let error = match &state.rules {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let loading = state.rules.is_none();
    let search = state.search.clone();
    let wide = f32::from(window.viewport_size().width - sidebar_width()) > 1000.0;

    let keys = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate,
        },
        shown.len(),
        rows_on_screen(window, ROW),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let mut columns = vec![
        ("TAG", None),
        ("CHANNEL", Some(CHANNEL)),
        ("TARGETS", Some(TARGETS)),
        ("STATE", Some(STATE)),
    ];
    if wide {
        columns.push(("LAST TEST", Some(TESTED)));
    }
    columns.push(("", Some(ACTIONS)));

    let body = if let Some(error) = error {
        empty_state(
            Icon::CircleAlert,
            "Rules could not be listed",
            error,
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if loading {
        empty_state(
            Icon::RefreshCw,
            "Loading rules",
            "Asking the server…",
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if shown.is_empty() {
        empty_state(
            Icon::Send,
            if list.is_empty() {
                "No notification rules"
            } else {
                "No rule matches"
            },
            if list.is_empty() {
                "A rule tells people by email, webhook or push when a tag alarms. Add one with N."
            } else {
                "Nothing has this text in its tag, channel or targets."
            },
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        let count = shown.len();
        let rows_view = shown.clone();
        uniform_list(
            "data-rules",
            count + tail_rows(count, ROW),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let list = rules(console);
                    range
                        .map(
                            |row| match rows_view.get(row).and_then(|index| list.get(*index)) {
                                Some(rule) => {
                                    rule_row(console, rule, row, cursor, focused, wide, &theme, cx)
                                }
                                None => tail_row(ROW),
                            },
                        )
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .size_full()
        .into_any_element()
    };

    let on = list.iter().filter(|rule| rule.enabled).count();
    let subtitle = format!(
        "{} · {on} on · N new · ↵ edit · Space on/off · T test · ⌫ delete",
        kit::count(list.len(), "rule")
    );
    let controls = vec![
        kit::search_control(search, &theme),
        Button::new("data-new-rule", "New rule")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(|console, _, window, cx| new_rule(console, window, cx)))
            .into_any_element(),
    ];

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("Notifications", subtitle, controls, cx))
        .child(kit::banner(
            "The server sends email through its SMTP settings and posts webhooks itself; a test sends a sample alarm to each target now.",
            &theme,
        ))
        .child(
            div()
                .key_context(DATA_ADMIN_RULES)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &DataAdminDelete, window, cx| {
                    if let Some(rule) = cursor_rule(console, cx) {
                        ask_delete(console, rule, window, cx);
                    }
                }))
                .on_action(cx.listener(|console, _: &DataAdminTest, _, cx| {
                    if let Some(rule) = cursor_rule(console, cx) {
                        test_rule(console, &rule, cx);
                    }
                }))
                .child(header(&columns, &theme))
                .child(keys.flex_1().min_h_0().child(body)),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn rule_row(
    console: &Console,
    rule: &NotificationRule,
    row: usize,
    cursor: Option<usize>,
    focused: bool,
    wide: bool,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let current = cursor == Some(row);
    let id = rule.id;
    let state = console.state_ref::<State>();
    let busy = state.is_some_and(|state| state.busy.contains(&id));
    let tested = state
        .and_then(|state| state.tests.get(&id))
        .map(outcome_line);
    let (key, tag) = if rule.source_key.is_empty() {
        ("*".to_owned(), "every tag".to_owned())
    } else {
        (
            rule.source_key.clone(),
            tag_name(console, &rule.source_key).unwrap_or_else(|| "no such tag".to_owned()),
        )
    };
    let testable = channel(&rule.channel_type).is_none_or(|channel| channel.testable);
    let (edit, switch_rule, test, remove) =
        (rule.clone(), rule.clone(), rule.clone(), rule.clone());
    row_frame(
        element_id("data-rule", id),
        current,
        focused && current,
        theme,
    )
    .on_click(
        cx.listener(move |console, event: &gpui::ClickEvent, window, cx| {
            console.list(LIST, cx).select(row);
            if event.click_count() >= 2 {
                open_form(console, Some(edit.clone()), window, cx);
            } else {
                let focus = console.list(LIST, cx).focus.clone();
                window.focus(&focus);
            }
            cx.notify();
        }),
    )
    .child(
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(SPACE_4)
            .child(
                div()
                    .flex_none()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .child(key),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(tag),
            ),
    )
    .child(div().w(px(CHANNEL)).flex_none().flex().child(pill(
        rule.channel_type.clone(),
        channel_color(&rule.channel_type, theme),
    )))
    .child(cell(rule.targets_summary(), TARGETS, theme.fg_muted, true))
    .child(div().w(px(STATE)).flex_none().flex().child(pill(
        if rule.enabled { "on" } else { "off" },
        if rule.enabled {
            theme.success
        } else {
            theme.fg_dim
        },
    )))
    .when(wide, |this| {
        this.child(match (busy, tested) {
            (true, _) => cell("working…", TESTED, theme.fg_dim, false),
            (false, Some((line, ok))) => cell(
                line,
                TESTED,
                if ok { theme.success } else { theme.danger },
                false,
            ),
            (false, None) => cell("—", TESTED, theme.fg_dim, false),
        })
    })
    .child(
        actions(ACTIONS, current)
            .child(
                IconButton::new(element_id("data-rule-test", id), Icon::Send)
                    .tooltip(if testable {
                        "Send a test  T"
                    } else {
                        "Push is tested from a browser"
                    })
                    .disabled(!testable || busy)
                    .on_click(cx.listener(move |console, _, _, cx| test_rule(console, &test, cx))),
            )
            .child(
                IconButton::new(
                    element_id("data-rule-switch", id),
                    if rule.enabled {
                        Icon::Pause
                    } else {
                        Icon::Play
                    },
                )
                .tooltip(if rule.enabled {
                    "Turn off  Space"
                } else {
                    "Turn on  Space"
                })
                .disabled(busy)
                .on_click(cx.listener(move |console, _, _, cx| switch(console, &switch_rule, cx))),
            )
            .child(
                IconButton::new(element_id("data-rule-delete", id), Icon::Trash)
                    .tooltip("Delete  ⌫")
                    .color(theme.danger)
                    .on_click(cx.listener(move |console, _, window, cx| {
                        ask_delete(console, remove.clone(), window, cx);
                    })),
            ),
    )
    .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn form_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(form) = console
        .state_ref::<State>()
        .and_then(|state| state.form.as_ref())
    else {
        return div().into_any_element();
    };
    let (original, source_key, targets, chosen, enabled, error, busy, testing, test) = (
        form.original.clone(),
        form.source_key.clone(),
        form.targets.clone(),
        form.channel,
        form.enabled,
        form.error.clone(),
        form.busy,
        form.testing,
        form.test.clone(),
    );
    let typed_key = source_key.read(cx).value().trim().to_owned();
    let key_note = if typed_key.is_empty() {
        ("Every tag's alarms".to_owned(), theme.fg_dim)
    } else {
        match tag_name(console, &typed_key) {
            Some(name) => (format!("{name}'s alarms"), theme.fg_dim),
            None => (
                "No tag has this key; the rule waits until one does".to_owned(),
                theme.warning,
            ),
        }
    };
    let spec = channel(chosen).unwrap_or(CHANNELS[0]);
    let count = parse_targets(targets.read(cx).value()).len();

    let channels = CHANNELS
        .iter()
        .map(|choice| {
            let id = choice.id;
            Segment::new(
                choice.label,
                chosen == id,
                cx.listener(move |console, _, _, cx| {
                    if let Some(form) = console.state::<State>().form.as_mut() {
                        form.channel = id;
                        form.forget_test();
                        form.error = None;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();
    let states = [("On", true), ("Off", false)]
        .into_iter()
        .map(|(label, on)| {
            Segment::new(
                label,
                enabled == on,
                cx.listener(move |console, _, _, cx| {
                    if let Some(form) = console.state::<State>().form.as_mut() {
                        form.enabled = on;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();

    let mut body = vec![
        div()
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .child(field("Tag key (empty: every tag)", source_key, cx))
            .child(note(key_note.0, key_note.1, None))
            .into_any_element(),
        div()
            .flex()
            .items_end()
            .gap(SPACE_7)
            .child(field(
                "Channel",
                segmented("data-rule-channel", channels, cx),
                cx,
            ))
            .child(field("State", segmented("data-rule-state", states, cx), cx))
            .into_any_element(),
        field(
            format!("Targets · {} · {count} now", spec.target),
            targets,
            cx,
        )
        .into_any_element(),
    ];
    if spec.testable {
        let result = test.as_ref().map(outcome_line);
        body.push(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    Button::new(
                        "data-rule-test",
                        if testing { "Sending…" } else { "Send a test" },
                    )
                    .compact()
                    .icon(Icon::Send)
                    .disabled(testing)
                    .on_click(cx.listener(|console, _, window, cx| test_form(console, window, cx))),
                )
                .child(div().flex_1().min_w_0().child(match result {
                    Some((line, ok)) => note(
                        line,
                        if ok { theme.success } else { theme.danger },
                        Some(if ok {
                            Icon::CircleCheck
                        } else {
                            Icon::CircleAlert
                        }),
                    ),
                    None => note("A sample alarm to each target, now", theme.fg_dim, None),
                }))
                .into_any_element(),
        );
    } else {
        body.push(note(
            "Push reaches the browsers these users subscribed from the web client.",
            theme.fg_dim,
            Some(Icon::Info),
        ));
    }

    div()
        .flex()
        .flex_col()
        .child(dialog_header(
            if original.is_some() {
                Icon::Pencil
            } else {
                Icon::Plus
            },
            if original.is_some() {
                "Edit notification rule"
            } else {
                "New notification rule"
            },
            "⌘↵ in the targets saves",
            &theme,
        ))
        .child(dialog_body(body))
        .child(dialog_footer(
            "data-rule-form",
            error,
            if original.is_some() { "Save" } else { "Add" },
            busy,
            submit,
            cx,
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::testing::Harness;
    use gpui::TestAppContext;
    use voltrus_scada_native::api::admin::TestResult;

    fn set_targets(h: &mut Harness, text: &'static str) {
        h.update(|console, window, cx| {
            let targets = console
                .state::<State>()
                .form
                .as_ref()
                .map(|form| form.targets.clone())
                .expect("the form is open");
            targets.update(cx, |area, cx| {
                area.replace_text_in_range(None, text, window, cx);
            });
        });
    }

    fn sent_once() -> Reply {
        Reply::ok(TestOutcome {
            channel: "email".into(),
            results: vec![TestResult {
                to: Some("ops@example.com".into()),
                status: "sent".into(),
                ..TestResult::default()
            }],
        })
    }

    #[gpui::test]
    fn a_form_test_for_old_targets_does_not_land_on_new_ones(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| open_form(console, None, window, cx));
        set_targets(&mut h, "ops@example.com");
        h.update(test_form);
        let sent = *h
            .calls()
            .iter()
            .find(|key| key.name == "data.notifications.form-test")
            .expect("tested");
        // New targets while the test is on its way.
        set_targets(&mut h, "other@example.com");
        h.reply(sent, sent_once());
        h.draw();
        assert!(
            h.update(|console, _, _| {
                let form = console.state::<State>().form.as_ref().expect("open");
                form.test.is_none() && !form.testing
            }),
            "the old answer is dropped"
        );
        // The form's own test still lands.
        h.update(test_form);
        let again = *h
            .calls()
            .iter()
            .find(|key| key.name == "data.notifications.form-test")
            .expect("tested again");
        assert_ne!(again, sent);
        h.reply(again, sent_once());
        assert!(h.update(|console, _, _| {
            console
                .state::<State>()
                .form
                .as_ref()
                .is_some_and(|form| form.test.as_ref().is_some_and(Result::is_ok))
        }));
    }

    #[gpui::test]
    fn an_older_forms_save_answer_leaves_the_new_form_alone(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| open_form(console, None, window, cx));
        set_targets(&mut h, "ops@example.com");
        h.update(submit);
        let first = *h
            .calls()
            .iter()
            .find(|key| key.name == "data.notifications.save")
            .expect("saved");
        h.update(|console, window, cx| {
            console.close_overlay(window, cx);
            open_form(console, None, window, cx);
        });
        h.reply(first, Reply::failed("refused"));
        h.draw();
        assert!(h.update(|console, _, _| {
            let form = console.state::<State>().form.as_ref().expect("open");
            form.error.is_none() && form.seq != first.arg
        }));
        h.reply(first, Reply::ok(IgnoredAny));
        h.draw();
        assert!(
            h.update(
                |console, _, _| console.overlay_is(FORM) && console.state::<State>().form.is_some()
            ),
            "the old success does not close the new form"
        );
    }

    #[test]
    fn a_test_reads_as_sent_counts_or_the_first_failure() {
        let sent = TestResult {
            to: Some("ops@example.com".into()),
            status: "sent".into(),
            ..TestResult::default()
        };
        let failed = TestResult {
            url: Some("https://hooks.example.com".into()),
            status: "error".into(),
            error: Some("connection refused".into()),
            ..TestResult::default()
        };
        let all = Ok(TestOutcome {
            channel: "email".into(),
            results: vec![sent.clone(), sent.clone()],
        });
        assert_eq!(outcome_line(&all), ("2 of 2 sent".to_owned(), true));
        let some = Ok(TestOutcome {
            channel: "webhook".into(),
            results: vec![sent, failed],
        });
        assert_eq!(
            outcome_line(&some),
            (
                "1 of 2 sent · https://hooks.example.com: connection refused".to_owned(),
                false
            )
        );
        assert_eq!(
            outcome_line(&Err("SMTP is not configured".into())),
            ("SMTP is not configured".to_owned(), false)
        );
    }
}
