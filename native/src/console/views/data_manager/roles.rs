//! Data Manager › Roles (strictly admin), after the web client's
//! `RoleManager.tsx` but as the server models a role: a name and a set of
//! the permissions `models::ALL_PERMISSIONS` lists (the server refuses any
//! other, so the web's free-text JSON is a checklist here). New, edit,
//! delete after a confirmation. The banner says what the server does with
//! them: it stores them; access still follows the user's role.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, Subscription, Window, div, px, uniform_list};
use serde::de::IgnoredAny;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::admin::{PERMISSIONS, RbacRole, role_body};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, empty_state, field, kbd, section_label};
use vt_ui::text_input::{InputKind, TextInput};
use vt_ui::theme::*;

use super::admin_kit::{
    self as kit, Confirm, ROW, actions, cell, dialog_body, dialog_footer, dialog_header,
    element_id, header, row_frame,
};
use super::sidebar_width;
use crate::console::Console;
use crate::console::keys::{DATA_ADMIN_ROLES, DataAdminDelete};
use crate::console::overlay::Overlay;
use crate::console::pages::Role;
use crate::console::views::tag_detail::{on_field, text_field};
use crate::console::views::{
    ListKeys, focus_ring, keyboard_list, local_time, page_header, rows_on_screen, tail_row,
    tail_rows,
};

pub(crate) const LIST: &str = "data.roles";
const PICKS: &str = "data.roles.permissions";
const FORM: &str = "data-role-form";
const PERMS: f32 = 380.0;
const CREATED: f32 = 110.0;
const ACTIONS: f32 = 64.0;
const PICK_ROW: f32 = 28.0;

struct Form {
    /// The role being edited; `None` for a new one.
    original: Option<RbacRole>,
    name: Entity<TextInput>,
    chosen: Vec<String>,
    error: Option<String>,
    busy: bool,
    /// This form's number: its save's key, so an older form's answer
    /// leaves it alone.
    seq: i64,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(crate) struct State {
    roles: Option<Result<Vec<RbacRole>, String>>,
    asked: bool,
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    focus_name: Option<String>,
    form: Option<Form>,
    /// How many forms were opened.
    form_seq: i64,
    deleting: Option<RbacRole>,
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
        CallKey::new("data.roles.list"),
        Call::get::<Vec<RbacRole>>("/api/v1/roles"),
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

fn roles(console: &Console) -> Vec<RbacRole> {
    match console
        .state_ref::<State>()
        .and_then(|state| state.roles.as_ref())
    {
        Some(Ok(roles)) => roles.clone(),
        _ => Vec::new(),
    }
}

/// Rows the search leaves: by name or by a permission the role holds.
fn rows(console: &Console, cx: &gpui::App) -> Vec<usize> {
    let Some(state) = console.state_ref::<State>() else {
        return Vec::new();
    };
    let query = kit::query(state.search.as_ref(), cx);
    match &state.roles {
        Some(Ok(roles)) => roles
            .iter()
            .enumerate()
            .filter(|(_, role)| {
                query.is_empty()
                    || role.name.to_lowercase().contains(&query)
                    || role.permissions.contains(&query)
            })
            .map(|(index, _)| index)
            .collect(),
        _ => Vec::new(),
    }
}

fn cursor_role(console: &mut Console, cx: &mut Context<Console>) -> Option<RbacRole> {
    let row = console.list(LIST, cx).cursor?;
    let index = *rows(console, cx).get(row)?;
    roles(console).get(index).cloned()
}

// ── Actions ──────────────────────────────────────────────────────────────

/// "New role" (N, the palette).
pub fn new_role(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if kit::open_section(console, "roles", Role::Admin, "Managing roles", window, cx) {
        open_form(console, None, window, cx);
    }
}

fn open_form(
    console: &mut Console,
    original: Option<RbacRole>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let name = text_field("Shift lead", InputKind::Text, false, cx);
    if let Some(role) = &original {
        let text = role.name.clone();
        name.update(cx, |field, cx| field.set_text(text, cx));
    }
    let subscriptions = vec![on_field(&name, submit, window, cx)];
    let chosen = original
        .as_ref()
        .map(RbacRole::permission_list)
        .unwrap_or_default();
    let state = console.state::<State>();
    state.form_seq += 1;
    let seq = state.form_seq;
    state.form = Some(Form {
        original,
        name: name.clone(),
        chosen,
        error: None,
        busy: false,
        seq,
        _subscriptions: subscriptions,
    });
    console.list(PICKS, cx).cursor = Some(0);
    kit::remember_window(console, window);
    let overlay = Overlay::modal(FORM, px(560.0), form_dialog, cx);
    console.open_overlay(overlay, window, cx);
    name.read(cx).focus(window);
}

fn activate(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let index = rows(console, cx).get(row).copied();
    if let Some(role) = index.and_then(|index| roles(console).get(index).cloned()) {
        open_form(console, Some(role), window, cx);
    }
}

fn toggle_permission(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(permission) = PERMISSIONS.get(row) else {
        return;
    };
    if let Some(form) = console.state::<State>().form.as_mut() {
        match form.chosen.iter().position(|chosen| chosen == permission) {
            Some(at) => {
                form.chosen.remove(at);
            }
            None => form.chosen.push((*permission).to_owned()),
        }
        form.error = None;
    }
    cx.notify();
}

fn set_all(console: &mut Console, all: bool, cx: &mut Context<Console>) {
    if let Some(form) = console.state::<State>().form.as_mut() {
        form.chosen = if all {
            PERMISSIONS
                .iter()
                .map(|permission| (*permission).to_owned())
                .collect()
        } else {
            Vec::new()
        };
    }
    cx.notify();
}

fn ask_delete(
    console: &mut Console,
    role: RbacRole,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let confirm = Confirm {
        title: format!("Delete the role {}?", role.name),
        body: "Users assigned this role lose its permissions. This cannot be undone.".to_owned(),
        verb: "Delete",
        run: delete_confirmed,
    };
    console.state::<State>().deleting = Some(role);
    kit::ask(console, confirm, window, cx);
}

fn delete_confirmed(console: &mut Console, _: &mut Window, _: &mut Context<Console>) {
    let Some(role) = console.state::<State>().deleting.take() else {
        return;
    };
    let (id, name) = (role.id, role.name);
    console.call(
        CallKey::new("data.roles.delete").with(id),
        Call::new(move |api| async move {
            api.send_empty::<IgnoredAny>(Method::DELETE, &format!("/api/v1/roles/{id}"))
                .await?;
            anyhow::Ok(name)
        }),
    );
}

fn submit(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let taken: Vec<String> = roles(console).into_iter().map(|role| role.name).collect();
    let Some(form) = console.state::<State>().form.as_mut() else {
        return;
    };
    if form.busy {
        return;
    }
    let name = form.name.read(cx).value().trim().to_owned();
    let original = form.original.clone();
    let clash = taken.iter().any(|other| {
        other.eq_ignore_ascii_case(&name)
            && original
                .as_ref()
                .is_none_or(|role| !role.name.eq_ignore_ascii_case(other))
    });
    let body = if clash {
        Err(format!("A role named {name} exists"))
    } else {
        role_body(&name, &form.chosen)
    };
    match body {
        Ok(body) => {
            form.busy = true;
            form.error = None;
            let seq = form.seq;
            let (method, path, done) = match &original {
                Some(role) => (
                    Method::PUT,
                    format!("/api/v1/roles/{}", role.id),
                    format!("Saved {name}"),
                ),
                None => (
                    Method::POST,
                    "/api/v1/roles".to_owned(),
                    format!("Created {name}"),
                ),
            };
            console.call(
                CallKey::new("data.roles.save").with(seq),
                Call::new(move |api| async move {
                    api.send_json::<_, IgnoredAny>(method, &path, &body).await?;
                    anyhow::Ok((done, name))
                }),
            );
        }
        Err(error) => form.error = Some(error),
    }
    cx.notify();
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "data.roles.list" => console.state::<State>().roles = Some(reply.take::<Vec<RbacRole>>()),
        "data.roles.save" => {
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
                        console.show_toast(format!("Saving the role failed: {error}"), true, cx);
                    }
                },
            }
        }
        "data.roles.delete" => match reply.take::<String>() {
            Ok(name) => {
                fetch(console, true);
                console.show_toast(format!("Deleted the role {name}"), false, cx);
            }
            Err(error) => console.show_toast(format!("Delete failed: {error}"), true, cx),
        },
        _ => {}
    }
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// `12 of 24 · screens:read, alarms:ack, …`
fn summary(role: &RbacRole) -> String {
    let list = role.permission_list();
    if list.is_empty() {
        return "no permissions".to_owned();
    }
    format!(
        "{} of {} · {}",
        list.len(),
        PERMISSIONS.len(),
        list.join(", ")
    )
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    if console.state::<State>().search.is_none() {
        let (field, subscription) = kit::search_field(LIST, "Search roles  /", window, cx);
        let state = console.state::<State>();
        state.search = Some(field);
        state._search = Some(subscription);
    }
    if !console.overlay_is(FORM) {
        console.state::<State>().form = None;
    }
    let theme = theme(cx).clone();
    let shown = rows(console, cx);
    let list = roles(console);
    let zone = console.zone.clone();
    let state = console.state::<State>();
    let error = match &state.roles {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let loading = state.roles.is_none();
    let search = state.search.clone();
    if let Some(name) = state.focus_name.clone()
        && let Some(row) = shown.iter().position(|index| list[*index].name == name)
    {
        console.state::<State>().focus_name = None;
        console.list(LIST, cx).select(row);
    }
    let wide = f32::from(window.viewport_size().width - sidebar_width()) > 900.0;

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
    let mut columns = vec![("ROLE", None), ("PERMISSIONS", Some(PERMS))];
    if wide {
        columns.push(("CREATED", Some(CREATED)));
    }
    columns.push(("", Some(ACTIONS)));

    let body = if let Some(error) = error {
        empty_state(
            Icon::CircleAlert,
            "Roles could not be listed",
            error,
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if loading {
        empty_state(
            Icon::RefreshCw,
            "Loading roles",
            "Asking the server…",
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if shown.is_empty() {
        empty_state(
            Icon::Layers,
            if list.is_empty() {
                "No roles yet"
            } else {
                "No role matches"
            },
            if list.is_empty() {
                "Add one with N."
            } else {
                "Nothing has this text in its name or permissions."
            },
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        let count = shown.len();
        let rows_view = shown.clone();
        uniform_list(
            "data-roles",
            count + tail_rows(count, ROW),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let list = roles(console);
                    range
                        .map(
                            |row| match rows_view.get(row).and_then(|index| list.get(*index)) {
                                Some(role) => {
                                    let current = cursor == Some(row);
                                    let id = role.id;
                                    let edit = role.clone();
                                    let remove = role.clone();
                                    row_frame(
                                        element_id("data-role", id),
                                        current,
                                        focused && current,
                                        &theme,
                                    )
                                    .on_click(cx.listener(
                                        move |console, event: &gpui::ClickEvent, window, cx| {
                                            console.list(LIST, cx).select(row);
                                            if event.click_count() >= 2 {
                                                open_form(console, Some(edit.clone()), window, cx);
                                            } else {
                                                let focus = console.list(LIST, cx).focus.clone();
                                                window.focus(&focus);
                                            }
                                            cx.notify();
                                        },
                                    ))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_1()
                                            .min_w_0()
                                            .items_center()
                                            .gap(SPACE_4)
                                            .child(vt_ui::icon(Icon::Layers, ICON_SM, theme.fg_dim))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(role.name.clone()),
                                            ),
                                    )
                                    .child(cell(summary(role), PERMS, theme.fg_muted, true))
                                    .when(wide, |this| {
                                        this.child(cell(
                                            local_time(role.created_at, &zone),
                                            CREATED,
                                            theme.fg_dim,
                                            true,
                                        ))
                                    })
                                    .child(
                                        actions(ACTIONS, current)
                                            .child(
                                                IconButton::new(
                                                    element_id("data-role-edit", id),
                                                    Icon::Pencil,
                                                )
                                                .tooltip("Edit  ↵")
                                                .on_click(cx.listener(
                                                    move |console, _, window, cx| {
                                                        if let Some(role) = roles(console)
                                                            .into_iter()
                                                            .find(|role| role.id == id)
                                                        {
                                                            open_form(
                                                                console,
                                                                Some(role),
                                                                window,
                                                                cx,
                                                            );
                                                        }
                                                    },
                                                )),
                                            )
                                            .child(
                                                IconButton::new(
                                                    element_id("data-role-delete", id),
                                                    Icon::Trash,
                                                )
                                                .tooltip("Delete  ⌫")
                                                .color(theme.danger)
                                                .on_click(cx.listener(
                                                    move |console, _, window, cx| {
                                                        ask_delete(
                                                            console,
                                                            remove.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
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

    let subtitle = format!(
        "{} · {} permissions to choose from · N new · ↵ edit · ⌫ delete",
        kit::count(list.len(), "role"),
        PERMISSIONS.len()
    );
    let controls = vec![
        kit::search_control(search, &theme),
        Button::new("data-new-role", "New role")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(|console, _, window, cx| new_role(console, window, cx)))
            .into_any_element(),
    ];

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("Roles", subtitle, controls, cx))
        .child(kit::banner(
            "The server stores these permissions with each role; it does not check them yet. What a user may do still follows their user role (Users).",
            &theme,
        ))
        .child(
            div()
                .key_context(DATA_ADMIN_ROLES)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &DataAdminDelete, window, cx| {
                    if let Some(role) = cursor_role(console, cx) {
                        ask_delete(console, role, window, cx);
                    }
                }))
                .child(header(&columns, &theme))
                .child(keys.flex_1().min_h_0().child(body)),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn form_dialog(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(form) = console.state::<State>().form.as_ref() else {
        return div().into_any_element();
    };
    let (original, name, chosen, error, busy) = (
        form.original.clone(),
        form.name.clone(),
        form.chosen.clone(),
        form.error.clone(),
        form.busy,
    );

    let picks = keyboard_list(
        ListKeys {
            id: PICKS,
            on_move: None,
            on_activate: toggle_permission,
        },
        PERMISSIONS.len(),
        8,
        console,
        cx,
    );
    let nav = console.list(PICKS, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let chosen_view = chosen.clone();
    let list = uniform_list(
        "data-role-permissions",
        PERMISSIONS.len(),
        cx.processor(
            move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = vt_ui::theme::theme(cx).clone();
                range
                    .map(|row| {
                        let permission = PERMISSIONS[row];
                        let on = chosen_view.iter().any(|chosen| chosen == permission);
                        let (area, verb) = permission.split_once(':').unwrap_or((permission, ""));
                        let hover = theme.hover;
                        div()
                            .id(("data-role-permission", row))
                            .relative()
                            .flex()
                            .items_center()
                            .gap(SPACE_4)
                            .h(px(PICK_ROW))
                            .px(SPACE_5)
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .when(focused && cursor == Some(row), |this| {
                                this.child(focus_ring(&theme))
                            })
                            .on_click(cx.listener(move |console, _, window, cx| {
                                console.list(PICKS, cx).select(row);
                                toggle_permission(console, row, true, window, cx);
                            }))
                            .child(vt_ui::icon(
                                if on { Icon::CircleCheck } else { Icon::Circle },
                                ICON_MD,
                                if on { theme.accent } else { theme.fg_dim },
                            ))
                            .child(
                                div()
                                    .w(px(110.0))
                                    .flex_none()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child(area.to_owned()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(if on { theme.fg } else { theme.fg_muted })
                                    .child(verb.to_owned()),
                            )
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .h(px(PICK_ROW * 8.0));

    let body = vec![
        field("Name", name, cx).into_any_element(),
        div()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .child(div().flex_1().child(section_label(
                format!("Permissions · {} of {}", chosen.len(), PERMISSIONS.len()),
                cx,
            )))
            .child(
                Button::new("data-role-all", "All")
                    .ghost()
                    .compact()
                    .on_click(cx.listener(|console, _, _, cx| set_all(console, true, cx))),
            )
            .child(
                Button::new("data-role-none", "None")
                    .ghost()
                    .compact()
                    .on_click(cx.listener(|console, _, _, cx| set_all(console, false, cx))),
            )
            .into_any_element(),
        picks
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(theme.border)
            .child(list)
            .into_any_element(),
        div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .text_size(FONT_CAPTION)
            .text_color(theme.fg_dim)
            .child(kbd("Tab", cx))
            .child("into the list")
            .child(kbd("Space", cx))
            .child("tick")
            .into_any_element(),
    ];

    div()
        .flex()
        .flex_col()
        .child(dialog_header(
            if original.is_some() {
                Icon::Pencil
            } else {
                Icon::Plus
            },
            match &original {
                Some(role) => format!("Edit {}", role.name),
                None => "New role".to_owned(),
            },
            "A name and the permissions the server stores with it",
            &theme,
        ))
        .child(dialog_body(body))
        .child(dialog_footer(
            "data-role-form",
            error,
            if original.is_some() { "Save" } else { "Create" },
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

    fn fill(h: &mut Harness, name: &'static str) {
        h.update(|console, _, cx| {
            let form = console
                .state::<State>()
                .form
                .as_mut()
                .expect("the form is open");
            form.chosen = vec![PERMISSIONS[0].to_owned()];
            let field = form.name.clone();
            field.update(cx, |field, cx| field.set_text(name, cx));
        });
    }

    #[gpui::test]
    fn an_older_forms_save_answer_leaves_the_new_form_alone(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| open_form(console, None, window, cx));
        fill(&mut h, "Shift lead");
        h.update(submit);
        let first = *h
            .calls()
            .iter()
            .find(|key| key.name == "data.roles.save")
            .expect("saved");
        // Esc while it saves, then another form.
        h.update(|console, window, cx| {
            console.close_overlay(window, cx);
            open_form(console, None, window, cx);
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
            Reply::ok(("Created Shift lead".to_owned(), "Shift lead".to_owned())),
        );
        h.draw();
        assert!(
            h.update(
                |console, _, _| console.overlay_is(FORM) && console.state::<State>().form.is_some()
            ),
            "the old success does not close the new form"
        );
    }
}
