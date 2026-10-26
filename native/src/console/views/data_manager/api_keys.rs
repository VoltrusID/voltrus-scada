//! Data Manager › API keys (strictly admin), after the web client's
//! `ApiKeyManager.tsx`: every key by name, its first eight characters, its
//! role, who made it, when it was last used and when it expires; a new key
//! (name, read-only or integration, a lifetime) whose secret is shown once,
//! masked until revealed, with Copy; revoke after a confirmation.
//!
//! The secret lives only in the dialog: closing it (Done, Esc, a click
//! outside) drops it, and the list never has it (the server keeps a hash).
//! A copied secret is wiped from the clipboard when the dialog closes or
//! [`COPY_LIFETIME`] after the copy, whichever comes first (only if the
//! clipboard still holds it). Nothing replaces the dialog while it shows.

use std::ops::Range;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, ClipboardItem, Context, Entity, FocusHandle, KeyDownEvent, SharedString,
    Subscription, Task, Window, div, px, uniform_list,
};
use serde::de::IgnoredAny;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::admin::{
    ApiKey, ApiKeyList, KEY_LIFETIMES, KEY_ROLES, NewApiKey, api_key_body,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, empty_state, field, kbd, pill, segmented};
use vt_ui::text_input::{InputKind, TextInput};
use vt_ui::theme::*;

use super::admin_kit::{
    self as kit, Confirm, ROW, actions, cell, dialog_body, dialog_footer, dialog_header,
    element_id, header, note, row_frame,
};
use super::sidebar_width;
use crate::console::Console;
use crate::console::keys::{DATA_ADMIN_API_KEYS, DataAdminDelete};
use crate::console::overlay::Overlay;
use crate::console::pages::Role;
use crate::console::views::tag_detail::{on_field, text_field};
use crate::console::views::{
    ListKeys, keyboard_list, local_time, page_header, rows_on_screen, tail_row, tail_rows,
};

pub(crate) const LIST: &str = "data.api-keys";
const FORM: &str = "data-api-key-form";
const KEY: f32 = 110.0;
const ROLE: f32 = 100.0;
const CREATED: f32 = 170.0;
const USED: f32 = 110.0;
const EXPIRES: f32 = 110.0;
const ACTIONS: f32 = 40.0;
/// How long a copied secret may stay on the clipboard.
const COPY_LIFETIME: Duration = Duration::from_secs(60);

struct Form {
    name: Entity<TextInput>,
    role: &'static str,
    /// Index in [`KEY_LIFETIMES`].
    lifetime: usize,
    error: Option<String>,
    busy: bool,
    /// The new key, once the server made it: shown once.
    created: Option<Created>,
    revealed: bool,
    copied: bool,
    secret_focus: FocusHandle,
    /// This form's number: its create's key. An answer for an older form
    /// never lands here (it would replace this form's secret).
    seq: i64,
    /// Wipes the copied secret off the clipboard after [`COPY_LIFETIME`].
    _wipe: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// A made key: the secret moved out of the answer once, so drawing the
/// dialog never copies it.
struct Created {
    /// The answer, its `key` emptied.
    info: NewApiKey,
    secret: SharedString,
    /// The first eight characters, then dots.
    masked: SharedString,
}

impl Created {
    fn new(mut info: NewApiKey) -> Self {
        let secret = std::mem::take(&mut info.key);
        let prefix: String = secret.chars().take(8).collect();
        let masked = format!(
            "{prefix}{}",
            "•".repeat(secret.chars().count().saturating_sub(8))
        );
        Self {
            info,
            secret: secret.into(),
            masked: masked.into(),
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    keys: Option<Result<Vec<ApiKey>, String>>,
    asked: bool,
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    form: Option<Form>,
    /// How many forms were opened.
    form_seq: i64,
    revoking: Option<ApiKey>,
}

fn fetch(console: &mut Console, again: bool) {
    if console.stress.is_some() || !console.is_admin() {
        return;
    }
    let state = console.state::<State>();
    if state.asked && !again {
        return;
    }
    state.asked = true;
    console.call(
        CallKey::new("data.api-keys.list"),
        Call::get::<ApiKeyList>("/api/v1/api-keys"),
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

fn keys(console: &Console) -> Vec<ApiKey> {
    match console
        .state_ref::<State>()
        .and_then(|state| state.keys.as_ref())
    {
        Some(Ok(keys)) => keys.clone(),
        _ => Vec::new(),
    }
}

fn rows(console: &Console, cx: &gpui::App) -> Vec<usize> {
    let Some(state) = console.state_ref::<State>() else {
        return Vec::new();
    };
    let query = kit::query(state.search.as_ref(), cx);
    match &state.keys {
        Some(Ok(keys)) => keys
            .iter()
            .enumerate()
            .filter(|(_, key)| {
                query.is_empty()
                    || key.name.to_lowercase().contains(&query)
                    || key.key_prefix.starts_with(&query)
                    || key.created_by.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect(),
        _ => Vec::new(),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

// ── Actions ──────────────────────────────────────────────────────────────

/// "New API key" (N, the palette).
pub fn new_key(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !kit::open_section(
        console,
        "api-keys",
        Role::Admin,
        "Managing API keys",
        window,
        cx,
    ) {
        return;
    }
    let name = text_field("Grafana read-only", InputKind::Text, false, cx);
    let subscriptions = vec![on_field(&name, submit, window, cx)];
    let state = console.state::<State>();
    state.form_seq += 1;
    let seq = state.form_seq;
    state.form = Some(Form {
        name: name.clone(),
        role: "api",
        lifetime: 2,
        error: None,
        busy: false,
        created: None,
        revealed: false,
        copied: false,
        secret_focus: cx.focus_handle(),
        seq,
        _wipe: None,
        _subscriptions: subscriptions,
    });
    kit::remember_window(console, window);
    let overlay = Overlay::modal(FORM, px(540.0), form_dialog, cx);
    console.open_overlay(overlay, window, cx);
    name.read(cx).focus(window);
}

fn ask_revoke(console: &mut Console, key: ApiKey, window: &mut Window, cx: &mut Context<Console>) {
    let confirm = Confirm {
        title: format!("Revoke {}?", key.name),
        body: format!(
            "Anything signing in with the key {}… loses access at once. This cannot be undone.",
            key.key_prefix
        ),
        verb: "Revoke",
        run: revoke_confirmed,
    };
    console.state::<State>().revoking = Some(key);
    kit::ask(console, confirm, window, cx);
}

fn revoke_confirmed(console: &mut Console, _: &mut Window, _: &mut Context<Console>) {
    let Some(key) = console.state::<State>().revoking.take() else {
        return;
    };
    let (id, name) = (key.id, key.name);
    console.call(
        CallKey::new("data.api-keys.revoke").with(id),
        Call::new(move |api| async move {
            api.send_empty::<IgnoredAny>(Method::DELETE, &format!("/api/v1/api-keys/{id}"))
                .await?;
            anyhow::Ok(name)
        }),
    );
}

fn submit(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(form) = console.state::<State>().form.as_mut() else {
        return;
    };
    if form.busy || form.created.is_some() {
        return;
    }
    let name = form.name.read(cx).value().to_owned();
    let days = KEY_LIFETIMES.get(form.lifetime).and_then(|(_, days)| *days);
    match api_key_body(&name, form.role, days, now_secs(), &[]) {
        Ok(body) => {
            form.busy = true;
            form.error = None;
            let seq = form.seq;
            console.call(
                CallKey::new("data.api-keys.create").with(seq),
                Call::send::<NewApiKey>(Method::POST, "/api/v1/api-keys", body),
            );
        }
        Err(error) => form.error = Some(error),
    }
    cx.notify();
}

fn copy(console: &mut Console, cx: &mut Context<Console>) {
    let Some(form) = console.state::<State>().form.as_mut() else {
        return;
    };
    if let Some(created) = &form.created {
        cx.write_to_clipboard(ClipboardItem::new_string(created.secret.to_string()));
        form.copied = true;
        form._wipe = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPY_LIFETIME).await;
            let _ = this.update(cx, |console, cx| {
                if let Some(created) = console
                    .state::<State>()
                    .form
                    .as_ref()
                    .and_then(|form| form.created.as_ref())
                {
                    wipe_clipboard(&created.secret, cx);
                }
            });
        }));
        cx.notify();
    }
}

/// Empties the clipboard if it still holds `secret` (something copied
/// since is left alone).
fn wipe_clipboard(secret: &str, cx: &mut gpui::App) {
    let holds = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .is_some_and(|text| text == secret);
    if holds {
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    }
}

/// Drops the form and its secret, wiping a copied one off the clipboard.
fn drop_form(console: &mut Console, cx: &mut gpui::App) {
    let Some(form) = console.state::<State>().form.take() else {
        return;
    };
    if let Some(created) = form.created.as_ref().filter(|_| form.copied) {
        wipe_clipboard(&created.secret, cx);
    }
}

/// The one-time secret is on screen: nothing may replace its dialog (the
/// palette, help or More would drop the key unseen).
pub(crate) fn secret_shown(console: &Console) -> bool {
    console.overlay_is(FORM)
        && console
            .state_ref::<State>()
            .and_then(|state| state.form.as_ref())
            .is_some_and(|form| form.created.is_some())
}

fn done(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    drop_form(console, cx);
    console.close_overlay(window, cx);
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "data.api-keys.list" => {
            console.state::<State>().keys = Some(reply.take::<ApiKeyList>().map(|list| list.keys));
        }
        "data.api-keys.create" => {
            let result = reply.take::<NewApiKey>();
            let open = console.overlay_is(FORM);
            // Only the form that asked, and only once: a second answer
            // never replaces a secret on screen.
            let form = console
                .state::<State>()
                .form
                .as_mut()
                .filter(|form| open && form.seq == key.arg && form.created.is_none());
            let (focus, message) = match (result, form) {
                (Ok(created), Some(form)) => {
                    let message = format!("Created the key {}", created.name);
                    form.busy = false;
                    form.created = Some(Created::new(created));
                    (Some(form.secret_focus.clone()), Some((message, false)))
                }
                // The dialog closed while the server made the key: its
                // secret is never shown, so the key is useless; say so.
                (Ok(created), None) => (
                    None,
                    Some((
                        format!(
                            "The key {} was made after its dialog closed; revoke it and make another",
                            created.name
                        ),
                        true,
                    )),
                ),
                (Err(error), Some(form)) => {
                    form.busy = false;
                    form.error = Some(error);
                    (None, None)
                }
                (Err(error), None) => (
                    None,
                    Some((format!("Creating the key failed: {error}"), true)),
                ),
            };
            if let Some(focus) = focus {
                kit::focus_later(console, focus, cx);
            }
            if let Some((message, error)) = message {
                console.show_toast(message, error, cx);
            }
            fetch(console, true);
        }
        "data.api-keys.revoke" => match reply.take::<String>() {
            Ok(name) => {
                fetch(console, true);
                console.show_toast(format!("Revoked {name}"), false, cx);
            }
            Err(error) => console.show_toast(format!("Revoke failed: {error}"), true, cx),
        },
        _ => {}
    }
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// A key has nothing to edit: Enter does nothing, ⌫ revokes.
fn no_activate(_: &mut Console, _: usize, _: bool, _: &mut Window, _: &mut Context<Console>) {}

fn key_role_color(role: &str, theme: &Theme) -> gpui::Hsla {
    if role == "integration" {
        theme.warning
    } else {
        theme.info
    }
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    if console.state::<State>().search.is_none() {
        let (field, subscription) = kit::search_field(LIST, "Search keys  /", window, cx);
        let state = console.state::<State>();
        state.search = Some(field);
        state._search = Some(subscription);
    }
    // The secret goes with its dialog.
    if !console.overlay_is(FORM) {
        drop_form(console, cx);
    }
    let theme = theme(cx).clone();
    let shown = rows(console, cx);
    let list = keys(console);
    let offset = console.utc_offset;
    let now = now_secs();
    let state = console.state::<State>();
    let error = match &state.keys {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let loading = state.keys.is_none();
    let search = state.search.clone();
    let wide = f32::from(window.viewport_size().width - sidebar_width()) > 1000.0;

    let nav_keys = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: no_activate,
        },
        shown.len(),
        rows_on_screen(window, ROW),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let mut columns = vec![("NAME", None), ("KEY", Some(KEY)), ("ROLE", Some(ROLE))];
    if wide {
        columns.push(("CREATED", Some(CREATED)));
    }
    columns.extend([
        ("LAST USED", Some(USED)),
        ("EXPIRES", Some(EXPIRES)),
        ("", Some(ACTIONS)),
    ]);

    let body = if let Some(error) = error {
        empty_state(
            Icon::CircleAlert,
            "API keys could not be listed",
            error,
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if loading {
        empty_state(
            Icon::RefreshCw,
            "Loading API keys",
            "Asking the server…",
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if shown.is_empty() {
        empty_state(
            Icon::Lock,
            if list.is_empty() { "No API keys" } else { "No key matches" },
            if list.is_empty() {
                "A key lets a script or another system read the plant (or, as integration, operate it). Make one with N."
            } else {
                "Nothing has this text in its name, prefix or maker."
            },
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        let count = shown.len();
        let rows_view = shown.clone();
        uniform_list(
            "data-api-keys",
            count + tail_rows(count, ROW),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let list = keys(console);
                    range
                        .map(
                            |row| match rows_view.get(row).and_then(|index| list.get(*index)) {
                                Some(key) => {
                                    let current = cursor == Some(row);
                                    let id = key.id;
                                    let expired = key.expired(now);
                                    let remove = key.clone();
                                    row_frame(
                                        element_id("data-api-key", id),
                                        current,
                                        focused && current,
                                        &theme,
                                    )
                                    .on_click(cx.listener(move |console, _, window, cx| {
                                        console.list(LIST, cx).select(row);
                                        let focus = console.list(LIST, cx).focus.clone();
                                        window.focus(&focus);
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_1()
                                            .min_w_0()
                                            .items_center()
                                            .gap(SPACE_4)
                                            .child(vt_ui::icon(
                                                Icon::Lock,
                                                ICON_SM,
                                                if expired { theme.danger } else { theme.fg_dim },
                                            ))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(key.name.clone()),
                                            ),
                                    )
                                    .child(cell(
                                        format!("{}…", key.key_prefix),
                                        KEY,
                                        theme.fg_muted,
                                        true,
                                    ))
                                    .child(div().w(px(ROLE)).flex_none().flex().child(pill(
                                        if key.role == "integration" {
                                            "integration"
                                        } else {
                                            "read only"
                                        },
                                        key_role_color(&key.role, &theme),
                                    )))
                                    .when(wide, |this| {
                                        this.child(cell(
                                            format!(
                                                "{} · {}",
                                                key.created_by,
                                                local_time(key.created_at, offset)
                                            ),
                                            CREATED,
                                            theme.fg_dim,
                                            false,
                                        ))
                                    })
                                    .child(cell(
                                        key.last_used_at.map_or_else(
                                            || "never".to_owned(),
                                            |at| local_time(at, offset),
                                        ),
                                        USED,
                                        theme.fg_dim,
                                        true,
                                    ))
                                    .child(cell(
                                        match key.expires_at {
                                            None => "never".to_owned(),
                                            Some(at) if expired => {
                                                format!("expired {}", local_time(at, offset))
                                            }
                                            Some(at) => local_time(at, offset),
                                        },
                                        EXPIRES,
                                        if expired { theme.danger } else { theme.fg_dim },
                                        true,
                                    ))
                                    .child(
                                        actions(ACTIONS, current).child(
                                            IconButton::new(
                                                element_id("data-api-key-revoke", id),
                                                Icon::Trash,
                                            )
                                            .tooltip("Revoke  ⌫")
                                            .color(theme.danger)
                                            .on_click(
                                                cx.listener(move |console, _, window, cx| {
                                                    ask_revoke(console, remove.clone(), window, cx);
                                                }),
                                            ),
                                        ),
                                    )
                                    .into_any_element()
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

    let expired = list.iter().filter(|key| key.expired(now)).count();
    let subtitle = format!(
        "{}{} · N new · ⌫ revoke",
        kit::count(list.len(), "key"),
        if expired > 0 {
            format!(" · {expired} expired")
        } else {
            String::new()
        }
    );
    let controls = vec![
        kit::search_control(search, &theme),
        Button::new("data-new-api-key", "New API key")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(|console, _, window, cx| new_key(console, window, cx)))
            .into_any_element(),
    ];

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("API keys", subtitle, controls, cx))
        .child(kit::banner(
            "A key signs in with the X-API-Key header. Read-only keys read; integration keys also pass the operator routes. No key can manage users, roles or keys.",
            &theme,
        ))
        .child(
            div()
                .key_context(DATA_ADMIN_API_KEYS)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &DataAdminDelete, window, cx| {
                    let row = console.list(LIST, cx).cursor;
                    let index = row.and_then(|row| rows(console, cx).get(row).copied());
                    if let Some(key) = index.and_then(|index| keys(console).get(index).cloned()) {
                        ask_revoke(console, key, window, cx);
                    }
                }))
                .child(header(&columns, &theme))
                .child(nav_keys.flex_1().min_h_0().child(body)),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn form_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(form) = console.state::<State>().form.as_ref() else {
        return div().into_any_element();
    };

    if let Some(created) = &form.created {
        let shown = if form.revealed {
            created.secret.clone()
        } else {
            created.masked.clone()
        };
        let (revealed, copied, focus) = (form.revealed, form.copied, form.secret_focus.clone());
        let (name, role, warning) = (
            created.info.name.clone(),
            created.info.role.clone(),
            created.info.warning.clone(),
        );
        return div()
            .track_focus(&focus)
            .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
                let own = console
                    .state::<State>()
                    .form
                    .as_ref()
                    .is_some_and(|form| form.secret_focus.is_focused(window));
                if !own {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "c" => copy(console, cx),
                    "enter" => done(console, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .flex()
            .flex_col()
            .child(dialog_header(Icon::CircleCheck, format!("{name} is ready"), format!("Role {role} · copy the key now"), &theme))
            .child(dialog_body(vec![
                note(
                    "This is the only time the key is shown. The server keeps only a hash: lose it and you make a new one.",
                    theme.warning,
                    Some(Icon::CircleAlert),
                ),
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .p(SPACE_5)
                    .rounded(RADIUS_MD)
                    .border_1()
                    .border_color(theme.border_strong)
                    .bg(theme.bg)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg)
                            .child(shown),
                    )
                    .child(
                        IconButton::new("data-api-key-reveal", Icon::Eye)
                            .tooltip(if revealed { "Hide" } else { "Show" })
                            .active(revealed)
                            .on_click(cx.listener(|console, _, _, cx| {
                                if let Some(form) = console.state::<State>().form.as_mut() {
                                    form.revealed = !form.revealed;
                                }
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
                note(warning, theme.fg_dim, None),
            ]))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(SPACE_4)
                    .h(px(56.0))
                    .px(SPACE_8)
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(SPACE_2)
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child(kbd("C", cx))
                            .child("copy")
                            .child(kbd("↵", cx))
                            .child("done"),
                    )
                    .child(
                        Button::new("data-api-key-copy", if copied { "Copied" } else { "Copy key" })
                            .icon(if copied { Icon::Check } else { Icon::Copy })
                            .on_click(cx.listener(|console, _, _, cx| copy(console, cx))),
                    )
                    .child(
                        Button::new("data-api-key-done", "Done")
                            .primary()
                            .on_click(cx.listener(|console, _, window, cx| done(console, window, cx))),
                    ),
            )
            .into_any_element();
    }

    let (name, role, lifetime, error, busy) = (
        form.name.clone(),
        form.role,
        form.lifetime,
        form.error.clone(),
        form.busy,
    );
    let roles = KEY_ROLES
        .iter()
        .map(|choice| {
            let id = choice.id;
            Segment::new(
                choice.label,
                role == id,
                cx.listener(move |console, _, _, cx| {
                    if let Some(form) = console.state::<State>().form.as_mut() {
                        form.role = id;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();
    let lifetimes = KEY_LIFETIMES
        .iter()
        .enumerate()
        .map(|(index, (label, _))| {
            Segment::new(
                *label,
                lifetime == index,
                cx.listener(move |console, _, _, cx| {
                    if let Some(form) = console.state::<State>().form.as_mut() {
                        form.lifetime = index;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();
    let reach = KEY_ROLES
        .iter()
        .find(|choice| choice.id == role)
        .map_or("", |choice| choice.reach);
    let body = vec![
        field("Name", name, cx).into_any_element(),
        field("Access", segmented("data-api-key-role", roles, cx), cx).into_any_element(),
        note(reach, theme.fg_muted, None),
        field(
            "Expires",
            segmented("data-api-key-lifetime", lifetimes, cx),
            cx,
        )
        .into_any_element(),
    ];

    div()
        .flex()
        .flex_col()
        .child(dialog_header(
            Icon::Plus,
            "New API key",
            "For a script or another system; the key is shown once",
            &theme,
        ))
        .child(dialog_body(body))
        .child(dialog_footer(
            "data-api-key-form",
            error,
            "Create",
            busy,
            submit,
            cx,
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::keys::{DataNewApiKey, ToggleHelp, TogglePalette};
    use crate::console::testing::Harness;
    use gpui::TestAppContext;

    fn secret(h: &mut Harness) -> Option<String> {
        h.update(|console, _, _| {
            console
                .state::<State>()
                .form
                .as_ref()
                .and_then(|form| form.created.as_ref())
                .map(|created| created.secret.to_string())
        })
    }

    fn made(name: &str, key: &str) -> Reply {
        Reply::ok(NewApiKey {
            id: 5,
            name: name.into(),
            role: "api".into(),
            key: key.into(),
            ..NewApiKey::default()
        })
    }

    /// Opens the dialog, names the key and asks for it: the create's key.
    fn ask(h: &mut Harness, name: &'static str) -> CallKey {
        // Straight to the action's handler: a dispatched action after an
        // earlier dialog closed is not what these tests are about.
        h.update(new_key);
        h.draw();
        assert!(h.update(|console, _, _| console.overlay_is(FORM)));
        h.calls();
        h.update(|console, _, cx| {
            let field = console
                .state::<State>()
                .form
                .as_ref()
                .map(|form| form.name.clone());
            field
                .expect("the form is open")
                .update(cx, |field, cx| field.set_text(name, cx));
        });
        h.update(submit);
        let calls: Vec<CallKey> = h
            .calls()
            .into_iter()
            .filter(|key| key.name == "data.api-keys.create")
            .collect();
        assert_eq!(calls.len(), 1);
        calls[0]
    }

    fn clipboard(h: &mut Harness) -> Option<String> {
        h.update(|_, _, cx| cx.read_from_clipboard().and_then(|item| item.text()))
    }

    #[gpui::test]
    fn a_new_keys_secret_lives_only_in_its_dialog(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        let key = ask(&mut h, "Grafana");
        h.reply(key, made("Grafana", &"f00d".repeat(16)));
        h.draw();
        assert_eq!(
            secret(&mut h).as_deref(),
            Some("f00d".repeat(16).as_str()),
            "shown once"
        );
        h.update(|console, _, _| {
            let form = console.state::<State>().form.as_ref().expect("open");
            let created = form.created.as_ref().expect("made");
            assert!(created.info.key.is_empty(), "the secret is held once");
            assert_eq!(
                created.masked.to_string(),
                format!("f00df00d{}", "•".repeat(56))
            );
        });
        h.update(|console, window, cx| console.close_overlay(window, cx));
        h.draw();
        assert_eq!(secret(&mut h), None, "closing the dialog drops the secret");
        assert!(
            h.update(|console, _, _| console.state::<State>().form.is_none()),
            "nothing of the dialog is kept"
        );
    }

    #[gpui::test]
    fn an_older_create_never_replaces_the_secret_on_screen(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        let first = ask(&mut h, "One");
        h.update(|console, window, cx| console.close_overlay(window, cx));
        h.draw();
        let second = ask(&mut h, "Two");
        assert_ne!(first, second);
        h.reply(second, made("Two", "secret-two"));
        h.draw();
        assert_eq!(secret(&mut h).as_deref(), Some("secret-two"));
        // The closed dialog's key arrives late: reported, never shown here.
        h.reply(first, made("One", "secret-one"));
        h.draw();
        assert_eq!(
            secret(&mut h).as_deref(),
            Some("secret-two"),
            "the second secret stays"
        );
        // Nor does a repeated answer for this very form.
        h.reply(second, made("Two", "secret-again"));
        h.draw();
        assert_eq!(secret(&mut h).as_deref(), Some("secret-two"));
    }

    #[gpui::test]
    fn nothing_replaces_the_one_time_secret(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        let key = ask(&mut h, "Grafana");
        h.reply(key, made("Grafana", "the-secret"));
        h.draw();
        h.cx.dispatch_action(TogglePalette);
        h.cx.dispatch_action(ToggleHelp);
        h.draw();
        assert!(h.update(|console, _, _| console.overlay_is(FORM)));
        assert_eq!(secret(&mut h).as_deref(), Some("the-secret"));
    }

    #[gpui::test]
    fn a_copied_secret_leaves_the_clipboard(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        let key = ask(&mut h, "Grafana");
        h.reply(key, made("Grafana", "the-secret"));
        h.draw();
        h.update(|console, _, cx| copy(console, cx));
        assert_eq!(clipboard(&mut h).as_deref(), Some("the-secret"));
        // After a while, even with the dialog still open.
        h.cx.executor().advance_clock(COPY_LIFETIME);
        h.cx.run_until_parked();
        assert_ne!(clipboard(&mut h).as_deref(), Some("the-secret"));

        // On close, at once.
        h.update(|console, _, cx| copy(console, cx));
        assert_eq!(clipboard(&mut h).as_deref(), Some("the-secret"));
        h.update(done);
        h.draw();
        assert_ne!(clipboard(&mut h).as_deref(), Some("the-secret"));

        // Something copied since is left alone.
        let key = ask(&mut h, "Other");
        h.reply(key, made("Other", "other-secret"));
        h.draw();
        h.update(|console, _, cx| {
            copy(console, cx);
            cx.write_to_clipboard(ClipboardItem::new_string("mine".to_owned()));
        });
        h.update(done);
        assert_eq!(clipboard(&mut h).as_deref(), Some("mine"));
    }

    #[gpui::test]
    fn a_viewer_or_demo_user_cannot_open_api_keys(cx: &mut TestAppContext) {
        for role in ["demo", "viewer"] {
            let mut h = Harness::new(cx);
            h.sign_in(role);
            h.cx.dispatch_action(DataNewApiKey);
            assert!(
                !h.update(|console, _, _| console.overlay_is(FORM)),
                "{role}"
            );
            assert!(h.calls().is_empty(), "{role}: nothing asked");
        }
    }
}
