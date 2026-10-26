//! Data Manager › Users (strictly admin), after the web client's
//! `UserManager.tsx`: every account with its role, a new user (name,
//! password typed twice, role), a role change, a password reset and a
//! delete after a confirmation. The console refuses what would lock the
//! admins out: demoting or deleting yourself, or the last admin. Passwords
//! are typed in masked fields and never kept after the dialog closes.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, Subscription, Window, div, px, uniform_list};
use serde::de::IgnoredAny;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::admin::{
    USER_ROLES, User, check_password, delete_refused, new_user_body, role_change_refused,
    user_role, user_update_body,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, empty_state, field, pill, segmented};
use vt_ui::text_input::{InputKind, TextInput};
use vt_ui::theme::*;

use super::admin_kit::{
    self as kit, Confirm, ROW, actions, cell, dialog_body, dialog_footer, dialog_header,
    element_id, header, note, row_frame,
};
use super::sidebar_width;
use crate::console::Console;
use crate::console::keys::{DATA_ADMIN_USERS, DataAdminDelete, DataAdminPassword};
use crate::console::overlay::Overlay;
use crate::console::pages::Role;
use crate::console::views::tag_detail::{on_field, text_field};
use crate::console::views::{
    ListKeys, keyboard_list, page_header, rows_on_screen, tail_row, tail_rows,
};

pub(crate) const LIST: &str = "data.users";
const FORM: &str = "data-user-form";
const ROLE: f32 = 96.0;
const ACCESS: f32 = 320.0;
const STATUS: f32 = 150.0;
const ACTIONS: f32 = 96.0;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    New,
    Edit(User),
    Password(User),
}

struct Form {
    kind: Kind,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    again: Entity<TextInput>,
    /// `None` for a role outside `USER_ROLES` (a legacy row): kept as is
    /// unless one is picked.
    role: Option<&'static str>,
    error: Option<String>,
    busy: bool,
    /// This form's number: its save's key, so an older form's answer
    /// leaves it alone.
    seq: i64,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(crate) struct State {
    users: Option<Result<Vec<User>, String>>,
    asked: bool,
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    /// Put the cursor on this user once listed.
    focus_name: Option<String>,
    form: Option<Form>,
    /// How many forms were opened.
    form_seq: i64,
    deleting: Option<User>,
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
        CallKey::new("data.users.list"),
        Call::get::<Vec<User>>("/api/v1/users"),
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

fn me(console: &Console) -> String {
    console
        .user
        .as_ref()
        .map(|(name, _)| name.clone())
        .unwrap_or_default()
}

fn users(console: &Console) -> Vec<User> {
    match console
        .state_ref::<State>()
        .and_then(|state| state.users.as_ref())
    {
        Some(Ok(users)) => users.clone(),
        _ => Vec::new(),
    }
}

/// The rows the search leaves: positions in the list.
fn rows(console: &Console, cx: &gpui::App) -> Vec<usize> {
    let Some(state) = console.state_ref::<State>() else {
        return Vec::new();
    };
    let query = kit::query(state.search.as_ref(), cx);
    match &state.users {
        Some(Ok(users)) => users
            .iter()
            .enumerate()
            .filter(|(_, user)| {
                query.is_empty()
                    || user.username.to_lowercase().contains(&query)
                    || user.role.contains(&query)
            })
            .map(|(index, _)| index)
            .collect(),
        _ => Vec::new(),
    }
}

fn cursor_user(console: &mut Console, cx: &mut Context<Console>) -> Option<User> {
    let row = console.list(LIST, cx).cursor?;
    let index = *rows(console, cx).get(row)?;
    users(console).get(index).cloned()
}

// ── Actions ──────────────────────────────────────────────────────────────

/// "New user" (N, the palette).
pub fn new_user(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if kit::open_section(console, "users", Role::Admin, "Managing users", window, cx) {
        open_form(console, Kind::New, window, cx);
    }
}

fn open_form(console: &mut Console, kind: Kind, window: &mut Window, cx: &mut Context<Console>) {
    let username = text_field("ana.operator", InputKind::Text, false, cx);
    let masked = |placeholder: &'static str, cx: &mut Context<Console>| {
        cx.new(|cx| TextInput::new(cx).placeholder(placeholder).secure())
    };
    let password = masked("At least 8 characters", cx);
    let again = masked("The same again", cx);
    let subscriptions = [&username, &password, &again]
        .into_iter()
        .map(|input| on_field(input, submit, window, cx))
        .collect();
    let role = match &kind {
        Kind::New => Some("viewer"),
        Kind::Edit(user) | Kind::Password(user) => user_role(&user.role).map(|role| role.id),
    };
    let first = match kind {
        Kind::New => username.clone(),
        _ => password.clone(),
    };
    let state = console.state::<State>();
    state.form_seq += 1;
    let seq = state.form_seq;
    state.form = Some(Form {
        kind,
        username,
        password,
        again,
        role,
        error: None,
        busy: false,
        seq,
        _subscriptions: subscriptions,
    });
    kit::remember_window(console, window);
    let overlay = Overlay::modal(FORM, px(520.0), form_dialog, cx);
    console.open_overlay(overlay, window, cx);
    first.read(cx).focus(window);
}

fn activate(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let index = rows(console, cx).get(row).copied();
    if let Some(user) = index.and_then(|index| users(console).get(index).cloned()) {
        open_form(console, Kind::Edit(user), window, cx);
    }
}

fn reset_password(
    console: &mut Console,
    user: User,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if user.username == me(console) {
        console.show_toast(OWN_PASSWORD, true, cx);
        return;
    }
    open_form(console, Kind::Password(user), window, cx);
}

/// An admin's own password changes with the current one (the password
/// change asks for it); a reset here would skip that check.
const OWN_PASSWORD: &str =
    "Your own password is not reset here: change it with your current password";

fn ask_delete(console: &mut Console, user: User, window: &mut Window, cx: &mut Context<Console>) {
    if let Some(refused) = delete_refused(&me(console), &user, &users(console)) {
        console.show_toast(refused, true, cx);
        return;
    }
    let confirm = Confirm {
        title: format!("Delete {}?", user.username),
        body: "They can no longer sign in. Their handovers and notifications keep their name, so the server refuses while any remain. This cannot be undone.".to_owned(),
        verb: "Delete",
        run: delete_confirmed,
    };
    console.state::<State>().deleting = Some(user);
    kit::ask(console, confirm, window, cx);
}

fn delete_confirmed(console: &mut Console, _: &mut Window, _: &mut Context<Console>) {
    let Some(user) = console.state::<State>().deleting.take() else {
        return;
    };
    let (id, name) = (user.id, user.username);
    console.call(
        CallKey::new("data.users.delete").with(id),
        Call::new(move |api| async move {
            api.send_empty::<IgnoredAny>(Method::DELETE, &format!("/api/v1/users/{id}"))
                .await?;
            anyhow::Ok(name)
        }),
    );
}

/// A save's request, checked first: method, path, body; then the toast it
/// ends with and the user to put the cursor on.
struct Save {
    method: Method,
    path: String,
    body: serde_json::Value,
    done: String,
    name: String,
}

fn build(form: &Form, me: &str, users: &[User], cx: &gpui::App) -> Result<Save, String> {
    let password = form.password.read(cx).value().to_owned();
    let again = form.again.read(cx).value().to_owned();
    match &form.kind {
        Kind::New => {
            let name = form.username.read(cx).value().trim().to_owned();
            let role = form.role.unwrap_or("viewer");
            let body = new_user_body(&name, &password, &again, role, users)?;
            Ok(Save {
                method: Method::POST,
                path: "/api/v1/users".to_owned(),
                body,
                done: format!("Created {name}"),
                name,
            })
        }
        Kind::Edit(user) | Kind::Password(user) => {
            // A password reset never sends a role; an edit only a picked one.
            let role = match form.kind {
                Kind::Password(_) => None,
                _ => form.role.filter(|role| *role != user.role),
            };
            if let Some(role) = role
                && let Some(refused) = role_change_refused(me, user, role, users)
            {
                return Err(refused);
            }
            let typed = !password.is_empty() || !again.is_empty();
            if matches!(form.kind, Kind::Password(_)) || typed {
                if user.username == me {
                    return Err(OWN_PASSWORD.to_owned());
                }
                check_password(&password, &again)?;
            }
            let password = typed.then_some(password.as_str());
            let Some(body) = user_update_body(role, password) else {
                return Err("Nothing changed".to_owned());
            };
            let done = match (role, password) {
                (Some(role), Some(_)) => {
                    format!("{} is now {role}, with a new password", user.username)
                }
                (Some(role), None) => format!("{} is now {role}", user.username),
                _ => format!("New password set for {}", user.username),
            };
            Ok(Save {
                method: Method::PUT,
                path: format!("/api/v1/users/{}", user.id),
                body,
                done,
                name: user.username.clone(),
            })
        }
    }
}

fn submit(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let me = me(console);
    let list = users(console);
    let Some(form) = console.state::<State>().form.as_mut() else {
        return;
    };
    if form.busy {
        return;
    }
    match build(form, &me, &list, cx) {
        Ok(save) => {
            form.busy = true;
            form.error = None;
            let seq = form.seq;
            let Save {
                method,
                path,
                body,
                done,
                name,
            } = save;
            let call = Call::new(move |api| async move {
                api.send_json::<_, IgnoredAny>(method, &path, &body).await?;
                anyhow::Ok((done, name))
            });
            console.call(CallKey::new("data.users.save").with(seq), call);
        }
        Err(error) => form.error = Some(error),
    }
    cx.notify();
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "data.users.list" => {
            console.state::<State>().users = Some(reply.take::<Vec<User>>());
        }
        "data.users.save" => {
            let current = console
                .state::<State>()
                .form
                .as_ref()
                .is_some_and(|form| form.seq == key.arg);
            match reply.take::<(String, String)>() {
                Ok((done, name)) => {
                    let state = console.state::<State>();
                    state.focus_name = Some(name);
                    // A form opened since then keeps its edits.
                    if current {
                        state.form = None;
                        kit::close_later(console, FORM, cx);
                    }
                    fetch(console, true);
                    console.show_toast(done, false, cx);
                }
                Err(error) => match console.state::<State>().form.as_mut().filter(|_| current) {
                    Some(form) => {
                        form.busy = false;
                        form.error = Some(error);
                    }
                    None => {
                        console.show_toast(format!("Saving the user failed: {error}"), true, cx);
                    }
                },
            }
        }
        "data.users.delete" => match reply.take::<String>() {
            Ok(name) => {
                fetch(console, true);
                console.show_toast(format!("Deleted {name}"), false, cx);
            }
            Err(error) => console.show_toast(format!("Delete failed: {error}"), true, cx),
        },
        _ => {}
    }
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

pub(crate) fn role_color(role: &str, theme: &Theme) -> gpui::Hsla {
    match role {
        "admin" => theme.accent,
        "demo" => theme.info,
        "operator" => theme.success,
        _ => theme.fg_dim,
    }
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    if console.state::<State>().search.is_none() {
        let (field, subscription) = kit::search_field(LIST, "Search users  /", window, cx);
        let state = console.state::<State>();
        state.search = Some(field);
        state._search = Some(subscription);
    }
    // A form closed with Esc keeps no password.
    if !console.overlay_is(FORM) {
        console.state::<State>().form = None;
    }
    let theme = theme(cx).clone();
    let shown = rows(console, cx);
    let list = users(console);
    let me = me(console);
    let state = console.state::<State>();
    let error = match &state.users {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let loading = state.users.is_none();
    let search = state.search.clone();
    if let Some(name) = state.focus_name.clone()
        && let Some(row) = shown.iter().position(|index| list[*index].username == name)
    {
        console.state::<State>().focus_name = None;
        console.list(LIST, cx).select(row);
    }
    let wide = f32::from(window.viewport_size().width - sidebar_width()) > 980.0;
    let admins = list.iter().filter(|user| user.is_admin()).count();

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

    let mut columns = vec![("USER", None), ("ROLE", Some(ROLE))];
    if wide {
        columns.push(("ACCESS", Some(ACCESS)));
    }
    columns.extend([("STATUS", Some(STATUS)), ("", Some(ACTIONS))]);

    let body = if let Some(error) = error {
        empty_state(
            Icon::CircleAlert,
            "Users could not be listed",
            error,
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if loading {
        empty_state(
            Icon::RefreshCw,
            "Loading users",
            "Asking the server…",
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if shown.is_empty() {
        empty_state(
            Icon::Search,
            "No user matches",
            "Nothing has this text in its name or role.",
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        let count = shown.len();
        let rows_view = shown.clone();
        uniform_list(
            "data-users",
            count + tail_rows(count, ROW),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let list = users(console);
                    range
                        .map(
                            |row| match rows_view.get(row).and_then(|index| list.get(*index)) {
                                Some(user) => {
                                    user_row(user, row, cursor, focused, wide, &me, &theme, cx)
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

    let subtitle = if list.is_empty() {
        "Accounts that sign in to the console and the web client".to_owned()
    } else {
        format!(
            "{} · {} · N new · ↵ edit · P password · ⌫ delete",
            kit::count(list.len(), "user"),
            kit::count(admins, "admin")
        )
    };
    let controls = vec![
        kit::search_control(search, &theme),
        Button::new("data-new-user", "New user")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(|console, _, window, cx| new_user(console, window, cx)))
            .into_any_element(),
    ];

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("Users", subtitle, controls, cx))
        .child(kit::banner(
            "Admin manages users, roles and API keys; demo configures and operates the plant; operator, user and viewer read and acknowledge alarms. A changed role applies from the user's next sign-in.",
            &theme,
        ))
        .child(
            div()
                .key_context(DATA_ADMIN_USERS)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &DataAdminDelete, window, cx| {
                    if let Some(user) = cursor_user(console, cx) {
                        ask_delete(console, user, window, cx);
                    }
                }))
                .on_action(cx.listener(|console, _: &DataAdminPassword, window, cx| {
                    if let Some(user) = cursor_user(console, cx) {
                        reset_password(console, user, window, cx);
                    }
                }))
                .child(header(&columns, &theme))
                .child(keys.flex_1().min_h_0().child(body)),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn user_row(
    user: &User,
    row: usize,
    cursor: Option<usize>,
    focused: bool,
    wide: bool,
    me: &str,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let current = cursor == Some(row);
    let id = user.id;
    let you = user.username == me;
    let status = match (you, user.must_change_password) {
        (true, _) => "you".to_owned(),
        (false, true) => "must change password".to_owned(),
        (false, false) => "—".to_owned(),
    };
    let reach = user_role(&user.role).map_or("Not a role the server knows", |role| role.reach);
    let edit = user.clone();
    let reset = user.clone();
    let remove = user.clone();
    row_frame(
        element_id("data-user", id),
        current,
        focused && current,
        theme,
    )
    .on_click(
        cx.listener(move |console, event: &gpui::ClickEvent, window, cx| {
            console.list(LIST, cx).select(row);
            if event.click_count() >= 2 {
                open_form(console, Kind::Edit(edit.clone()), window, cx);
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
            .child(vt_ui::icon(
                Icon::Circle,
                ICON_SM,
                role_color(&user.role, theme),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(user.username.clone()),
            ),
    )
    .child(
        div()
            .w(px(ROLE))
            .flex_none()
            .flex()
            .child(pill(user.role.clone(), role_color(&user.role, theme))),
    )
    .when(wide, |this| {
        this.child(cell(reach, ACCESS, theme.fg_muted, false))
    })
    .child(cell(
        status,
        STATUS,
        if user.must_change_password && !you {
            theme.warning
        } else {
            theme.fg_dim
        },
        false,
    ))
    .child(
        actions(ACTIONS, current)
            .child(
                IconButton::new(element_id("data-user-edit", id), Icon::Pencil)
                    .tooltip("Edit  ↵")
                    .on_click(cx.listener(move |console, _, window, cx| {
                        if let Some(user) = users(console).into_iter().find(|user| user.id == id) {
                            open_form(console, Kind::Edit(user), window, cx);
                        }
                    })),
            )
            .child(
                IconButton::new(element_id("data-user-password", id), Icon::Lock)
                    .tooltip("Reset the password  P")
                    .disabled(you)
                    .on_click(cx.listener(move |console, _, window, cx| {
                        reset_password(console, reset.clone(), window, cx);
                    })),
            )
            .child(
                IconButton::new(element_id("data-user-delete", id), Icon::Trash)
                    .tooltip("Delete  ⌫")
                    .color(theme.danger)
                    .disabled(you)
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
    let me = me(console);
    let list = users(console);
    let Some(form) = console.state::<State>().form.as_ref() else {
        return div().into_any_element();
    };
    let (kind, role, error, busy) = (form.kind.clone(), form.role, form.error.clone(), form.busy);
    let (username, password, again) = (
        form.username.clone(),
        form.password.clone(),
        form.again.clone(),
    );

    let roles = USER_ROLES
        .iter()
        .map(|choice| {
            let id = choice.id;
            Segment::new(
                choice.label,
                role == Some(id),
                cx.listener(move |console, _, _, cx| {
                    if let Some(form) = console.state::<State>().form.as_mut() {
                        form.role = Some(id);
                        form.error = None;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();
    let role_picker = |cx: &mut Context<Console>| {
        let reach = role.and_then(user_role).map_or("", |role| role.reach);
        div()
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .child(field("Role", segmented("data-user-role", roles, cx), cx))
            .child(note(reach, theme.fg_muted, None))
            .into_any_element()
    };

    let (icon, title, subtitle, label) = match &kind {
        Kind::New => (
            Icon::Plus,
            "New user".to_owned(),
            "They sign in with this name and password".to_owned(),
            "Create",
        ),
        Kind::Edit(user) => (
            Icon::Pencil,
            format!("Edit {}", user.username),
            format!("User {} · role {}", user.id, user.role),
            "Save",
        ),
        Kind::Password(user) => (
            Icon::Lock,
            format!("Reset {}'s password", user.username),
            "Tell them the new password in person".to_owned(),
            "Set password",
        ),
    };

    let mut body = Vec::new();
    match &kind {
        Kind::New => {
            body.push(field("User name", username, cx).into_any_element());
            body.push(passwords(password, again, "Password", cx));
            body.push(role_picker(cx));
        }
        Kind::Edit(user) => {
            let refused = role.and_then(|role| role_change_refused(&me, user, role, &list));
            body.push(role_picker(cx));
            if let Some(refused) = refused {
                body.push(note(refused, theme.warning, Some(Icon::CircleAlert)));
            }
            if user.username == me {
                body.push(note(OWN_PASSWORD, theme.fg_dim, Some(Icon::Info)));
            } else {
                body.push(passwords(
                    password,
                    again,
                    "New password (leave empty to keep it)",
                    cx,
                ));
            }
        }
        Kind::Password(_) => body.push(passwords(password, again, "New password", cx)),
    }

    div()
        .flex()
        .flex_col()
        .child(dialog_header(icon, title, subtitle, &theme))
        .child(dialog_body(body))
        .child(dialog_footer(
            "data-user-form",
            error,
            label,
            busy,
            submit,
            cx,
        ))
        .into_any_element()
}

fn passwords(
    password: Entity<TextInput>,
    again: Entity<TextInput>,
    label: &'static str,
    cx: &mut Context<Console>,
) -> AnyElement {
    div()
        .grid()
        .grid_cols(2)
        .gap(SPACE_6)
        .child(field(label, password, cx))
        .child(field("Repeat it", again, cx))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::keys::DataNewUser;
    use crate::console::testing::Harness;
    use gpui::TestAppContext;

    fn account(id: i64, name: &str, role: &str) -> User {
        User {
            id,
            username: name.into(),
            role: role.into(),
            must_change_password: false,
        }
    }

    fn type_into(h: &mut Harness, pick: fn(&Form) -> Entity<TextInput>, text: &'static str) {
        h.update(|console, _, cx| {
            let field = console
                .state::<State>()
                .form
                .as_ref()
                .map(pick)
                .expect("the form is open");
            field.update(cx, |field, cx| field.set_text(text, cx));
        });
    }

    #[gpui::test]
    fn an_admin_adds_a_user_only_with_the_password_typed_twice(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.cx.dispatch_action(DataNewUser);
        assert!(h.update(|console, _, _| console.overlay_is(FORM)));
        assert!(
            h.calls().contains(&CallKey::new("data.users.list")),
            "the list is fetched"
        );
        h.reply(
            CallKey::new("data.users.list"),
            Reply::ok(vec![account(1, "op", "admin")]),
        );
        type_into(&mut h, |form| form.username.clone(), "ana");
        type_into(&mut h, |form| form.password.clone(), "secret-123");
        type_into(&mut h, |form| form.again.clone(), "secret-124");
        h.update(submit);
        assert!(h.calls().is_empty(), "differing passwords are not sent");
        assert!(
            h.update(|console, _, _| {
                console
                    .state::<State>()
                    .form
                    .as_ref()
                    .and_then(|form| form.error.clone())
            })
            .is_some_and(|error| error.contains("differ"))
        );

        type_into(&mut h, |form| form.again.clone(), "secret-123");
        h.update(submit);
        let calls = h.calls();
        assert_eq!(calls.len(), 1);
        let saved = calls[0];
        assert_eq!(saved.name, "data.users.save");
        assert_ne!(saved.arg, 0, "keyed with the form's number");
        h.reply(
            saved,
            Reply::ok(("Created ana".to_owned(), "ana".to_owned())),
        );
        h.draw();
        assert!(
            h.update(|console, _, _| !console.overlay_is(FORM)
                && console.state::<State>().form.is_none()),
            "saved: the dialog closes and its passwords go with it"
        );
        assert!(
            h.calls().contains(&CallKey::new("data.users.list")),
            "listed again"
        );
    }

    #[gpui::test]
    fn only_an_admin_manages_users_and_never_themselves(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("demo");
        h.cx.dispatch_action(DataNewUser);
        assert!(
            !h.update(|console, _, _| console.overlay_is(FORM)),
            "demo is not admin"
        );
        assert!(h.calls().is_empty());

        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, _, cx| on_open(console, cx));
        assert!(h.calls().contains(&CallKey::new("data.users.list")));
        h.reply(
            CallKey::new("data.users.list"),
            Reply::ok(vec![account(1, "op", "admin"), account(2, "ana", "demo")]),
        );
        h.update(|console, window, cx| ask_delete(console, account(1, "op", "admin"), window, cx));
        assert!(
            h.update(|console, _, _| console.overlay.is_none()),
            "deleting yourself is refused before asking"
        );
        h.update(|console, window, cx| ask_delete(console, account(2, "ana", "demo"), window, cx));
        assert!(
            h.update(|console, _, _| console.overlay.is_some()),
            "another user: confirm first"
        );
        assert!(h.calls().is_empty(), "nothing is deleted before the yes");
    }

    fn built(h: &mut Harness) -> Result<serde_json::Value, String> {
        h.update(|console, _, cx| {
            let me = me(console);
            let list = users(console);
            let form = console
                .state::<State>()
                .form
                .as_ref()
                .expect("the form is open");
            build(form, &me, &list, cx).map(|save| save.body)
        })
    }

    #[gpui::test]
    fn a_legacy_role_is_never_sent_back_as_viewer(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        let legacy = account(2, "old", "supervisor");
        let reset = legacy.clone();
        h.update(move |console, window, cx| reset_password(console, reset, window, cx));
        type_into(&mut h, |form| form.password.clone(), "secret-123");
        type_into(&mut h, |form| form.again.clone(), "secret-123");
        let body = built(&mut h).expect("a password reset");
        assert!(body.get("role").is_none(), "a reset sends no role: {body}");
        assert!(body.get("password").is_some());

        // An edit of the same row keeps the unknown role unless one is picked.
        h.update(move |console, window, cx| open_form(console, Kind::Edit(legacy), window, cx));
        assert_eq!(
            h.update(|console, _, _| console.state::<State>().form.as_ref().map(|f| f.role)),
            Some(None)
        );
        assert_eq!(built(&mut h), Err("Nothing changed".to_owned()));
    }

    #[gpui::test]
    fn an_older_forms_save_answer_leaves_the_new_form_alone(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.cx.dispatch_action(DataNewUser);
        h.reply(
            CallKey::new("data.users.list"),
            Reply::ok(vec![account(1, "op", "admin")]),
        );
        type_into(&mut h, |form| form.username.clone(), "ana");
        type_into(&mut h, |form| form.password.clone(), "secret-123");
        type_into(&mut h, |form| form.again.clone(), "secret-123");
        h.update(submit);
        let first = *h
            .calls()
            .iter()
            .find(|key| key.name == "data.users.save")
            .expect("saved");
        // Esc while it saves, then another form.
        h.update(|console, window, cx| {
            console.close_overlay(window, cx);
            open_form(console, Kind::New, window, cx);
        });
        h.reply(first, Reply::failed("taken"));
        h.draw();
        assert!(
            h.update(|console, _, _| {
                let form = console.state::<State>().form.as_ref().expect("still open");
                form.error.is_none() && !form.busy && form.seq != first.arg
            }),
            "the old error does not land on the new form"
        );
        h.reply(
            first,
            Reply::ok(("Created ana".to_owned(), "ana".to_owned())),
        );
        h.draw();
        assert!(
            h.update(
                |console, _, _| console.overlay_is(FORM) && console.state::<State>().form.is_some()
            ),
            "the old success does not close the new form"
        );
    }

    #[test]
    fn every_role_the_server_accepts_has_a_colour_and_a_reach() {
        assert_eq!(
            USER_ROLES.map(|role| role.id),
            ["viewer", "user", "operator", "demo", "admin"]
        );
        assert!(
            USER_ROLES
                .iter()
                .all(|role| !user_role(role.id).unwrap().reach.is_empty())
        );
    }
}
