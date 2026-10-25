//! Commands: the writes the server's command queue carries to devices.
//! History pages through all of them by status; Active lists those still
//! on their way, fetched again every 2 s while the page is on screen.
//! Enter opens a command with its audit trail; operators cancel what has
//! not been sent yet (x, or the row's button).

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, FontWeight, Pixels, SharedString, Window, div, px, uniform_list,
};
use serde::de::IgnoredAny;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::ops::{
    self, ACTIVE_STATUSES, COMMAND_STATUSES, CommandAudit, QueuedCommand, may_cancel,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, empty_state, pill, segmented};
use vt_ui::theme::*;

use super::ops_kit::{command_status_color, notice, pager, poll_while_shown};
use super::{
    ListKeys, age, capitalized, focus_ring, keyboard_list, local_time, page_header, rows_on_screen,
    tail_row, tail_rows,
};
use crate::console::keys::{
    Dismiss, OpsCancelCommand, OpsNextFilter, OpsNextPage, OpsNextTab, OpsPreviousFilter,
    OpsPreviousPage, OpsRefresh,
};
use crate::console::{Console, Page, time_now_secs};

/// The command list's keyboard list.
pub(crate) const LIST: &str = "commands";
/// The key contexts of the page's root: its own keys (see `keys::COMMANDS`).
const KEY_CONTEXT: &str = "OpsCommands OpsPaged OpsTabbed OpsFiltered OpsRefreshable";
const ROW_HEIGHT_PX: f32 = 46.0;
/// Commands on a History page, as the web console pages them.
pub(crate) const PAGE_SIZE: usize = 20;
const ACTIVE_EVERY: Duration = Duration::from_secs(2);
const PANEL: f32 = 380.0;

const STATUS: f32 = 92.0;
const VALUE: f32 = 104.0;
const VIA: f32 = 132.0;
const BY: f32 = 96.0;
const CREATED: f32 = 92.0;
const ACTIONS: f32 = 76.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    History,
    Active,
}

/// One list as last fetched.
#[derive(Default)]
struct Listing {
    rows: Rc<Vec<QueuedCommand>>,
    loaded: bool,
    error: Option<String>,
    /// History only: the answer held more than a page (it asks for one row
    /// extra), so another page follows.
    more: bool,
    /// The newest request; older answers are dropped.
    seq: i64,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) tab: Tab,
    /// 0 for every status, else `COMMAND_STATUSES[filter - 1]`.
    pub(crate) filter: usize,
    /// History page, from 0.
    pub(crate) page: usize,
    history: Listing,
    active: Listing,
    /// The command open in the side panel and its audit trail.
    open: Option<QueuedCommand>,
    audit: Option<Result<Vec<CommandAudit>, String>>,
    audit_seq: i64,
    /// Cancels waiting for the server: call argument and command id.
    cancels: Vec<(i64, String)>,
    cancel_seq: i64,
}

impl State {
    fn listing(&self) -> &Listing {
        match self.tab {
            Tab::History => &self.history,
            Tab::Active => &self.active,
        }
    }

    /// The status the History list filters by.
    pub(crate) fn status(&self) -> Option<&'static str> {
        self.filter
            .checked_sub(1)
            .and_then(|index| COMMAND_STATUSES.get(index).copied())
    }

    /// Whether History has a page after this one: the last answer held a
    /// row beyond this page.
    fn has_next(&self) -> bool {
        self.history.more
    }
}

/// A History answer asked for one row more than a page: the page's rows,
/// and whether that extra row says another page follows.
fn page_of(mut rows: Vec<QueuedCommand>) -> (Vec<QueuedCommand>, bool) {
    let more = rows.len() > PAGE_SIZE;
    rows.truncate(PAGE_SIZE);
    (rows, more)
}

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let (history, active) = (!state.history.loaded, !state.active.loaded);
    if history {
        fetch_history(console);
    }
    if active {
        fetch_active(console);
    }
    poll_while_shown(console, Page::Commands, ACTIVE_EVERY, tick, cx);
    console.focus_list(LIST, window, cx);
}

/// Every 2 s on screen: the Active list again.
fn tick(console: &mut Console, _: &mut Context<Console>) {
    if console.state::<State>().tab == Tab::Active {
        fetch_active(console);
    }
}

fn fetch_history(console: &mut Console) {
    let state = console.state::<State>();
    state.history.seq += 1;
    let (seq, status, offset) = (state.history.seq, state.status(), state.page * PAGE_SIZE);
    console.call(
        CallKey::new("commands.history").with(seq),
        Call::get::<Vec<QueuedCommand>>(ops::commands_path(status, PAGE_SIZE + 1, offset)),
    );
}

fn fetch_active(console: &mut Console) {
    let state = console.state::<State>();
    state.active.seq += 1;
    let seq = state.active.seq;
    console.call(
        CallKey::new("commands.active").with(seq),
        Call::new(ops::active_commands),
    );
}

fn fetch_audit(console: &mut Console, id: &str) {
    let state = console.state::<State>();
    state.audit_seq += 1;
    state.audit = None;
    let seq = state.audit_seq;
    console.call(
        CallKey::new("commands.audit").with(seq),
        Call::get::<Vec<CommandAudit>>(ops::command_audit_path(id)),
    );
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    match key.name {
        "commands.history" | "commands.active" => {
            let history = key.name == "commands.history";
            let listing = if history {
                &mut state.history
            } else {
                &mut state.active
            };
            if key.arg != listing.seq {
                return;
            }
            match reply.take::<Vec<QueuedCommand>>() {
                Ok(rows) => {
                    let (rows, more) = if history {
                        page_of(rows)
                    } else {
                        (rows, false)
                    };
                    if *listing.rows != rows || !listing.loaded {
                        listing.rows = Rc::new(rows);
                    }
                    listing.loaded = true;
                    listing.error = None;
                    listing.more = more;
                }
                Err(error) => {
                    // A History page that failed shows why, not the rows of
                    // the page before it under its own number.
                    if history {
                        listing.rows = Rc::default();
                        listing.more = false;
                    }
                    listing.error = Some(error);
                }
            }
            // The open command follows its newest copy.
            if let Some(open) = &state.open
                && let Some(fresh) = state
                    .history
                    .rows
                    .iter()
                    .chain(state.active.rows.iter())
                    .find(|command| command.id == open.id)
            {
                state.open = Some(fresh.clone());
            }
        }
        "commands.audit" if key.arg == state.audit_seq => {
            state.audit = Some(reply.take::<Vec<CommandAudit>>());
        }
        "commands.cancel" => {
            let Some(position) = state.cancels.iter().position(|(seq, _)| *seq == key.arg) else {
                return;
            };
            let (_, id) = state.cancels.remove(position);
            match reply.error() {
                None => console.show_toast("Command cancelled", false, cx),
                Some(error) => console.show_toast(format!("Cancel failed: {error}"), true, cx),
            }
            refresh(console);
            if console
                .state::<State>()
                .open
                .as_ref()
                .is_some_and(|open| open.id == id)
            {
                fetch_audit(console, &id);
            }
        }
        _ => return,
    }
    cx.notify();
}

/// Both lists again, and the open command's trail.
fn refresh(console: &mut Console) {
    fetch_history(console);
    fetch_active(console);
}

/// The rows of the tab on screen.
fn shown_rows(console: &mut Console) -> Rc<Vec<QueuedCommand>> {
    console.state::<State>().listing().rows.clone()
}

/// Enter opens the row's command in the side panel; Space too, or closes
/// it when it is the one open.
fn activate_row(
    console: &mut Console,
    row: usize,
    toggle: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(command) = shown_rows(console).get(row).cloned() else {
        return;
    };
    let state = console.state::<State>();
    if toggle
        && state
            .open
            .as_ref()
            .is_some_and(|open| open.id == command.id)
    {
        state.open = None;
    } else {
        let id = command.id.clone();
        state.open = Some(command);
        fetch_audit(console, &id);
    }
    cx.notify();
}

/// Cancel `command` (operators; only pending or queued commands).
fn cancel(console: &mut Console, command: &QueuedCommand, cx: &mut Context<Console>) {
    if !may_cancel(console.is_operator(), command) {
        return;
    }
    let state = console.state::<State>();
    if state.cancels.iter().any(|(_, id)| *id == command.id) {
        return;
    }
    state.cancel_seq += 1;
    let seq = state.cancel_seq;
    state.cancels.push((seq, command.id.clone()));
    console.call(
        CallKey::new("commands.cancel").with(seq),
        Call::send_empty::<IgnoredAny>(Method::POST, ops::cancel_path(&command.id)),
    );
    cx.notify();
}

fn cancel_cursor(console: &mut Console, cx: &mut Context<Console>) {
    let row = console.list(LIST, cx).cursor;
    if let Some(command) = row.and_then(|row| shown_rows(console).get(row).cloned()) {
        cancel(console, &command, cx);
    }
}

fn set_tab(console: &mut Console, tab: Tab, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.tab == tab {
        return;
    }
    state.tab = tab;
    if tab == Tab::Active {
        fetch_active(console);
    } else {
        fetch_history(console);
    }
    console.list(LIST, cx).cursor = Some(0);
    cx.notify();
}

fn set_filter(console: &mut Console, filter: usize, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.tab = Tab::History;
    state.filter = filter % (COMMAND_STATUSES.len() + 1);
    state.page = 0;
    fetch_history(console);
    console.list(LIST, cx).cursor = Some(0);
    cx.notify();
}

fn step_filter(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    let count = COMMAND_STATUSES.len() + 1;
    let filter = console.state::<State>().filter;
    set_filter(
        console,
        if forward {
            filter + 1
        } else {
            filter + count - 1
        },
        cx,
    );
}

fn previous_page(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.tab != Tab::History || state.page == 0 {
        return;
    }
    state.page -= 1;
    fetch_history(console);
    console.list(LIST, cx).cursor = Some(0);
    cx.notify();
}

fn next_page(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.tab != Tab::History || !state.has_next() {
        return;
    }
    state.page += 1;
    fetch_history(console);
    console.list(LIST, cx).cursor = Some(0);
    cx.notify();
}

pub fn side_panel(console: &Console) -> Pixels {
    let open = console
        .state_ref::<State>()
        .is_some_and(|state| state.open.is_some());
    px(if open { PANEL } else { 0.0 })
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let state = console.state::<State>();
    let (tab, filter, page) = (state.tab, state.filter, state.page);
    let rows = state.listing().rows.clone();
    let (loaded, error) = (state.listing().loaded, state.listing().error.clone());
    let active = state.active.rows.clone();
    let has_next = state.has_next();
    let open = state.open.clone();
    let cancelling: Vec<String> = state.cancels.iter().map(|(_, id)| id.clone()).collect();
    let shown = rows.len();

    let tabs = segmented(
        "commands-tab",
        vec![
            Segment::new(
                "History",
                tab == Tab::History,
                cx.listener(|console, _, _, cx| set_tab(console, Tab::History, cx)),
            ),
            Segment::new(
                format!("Active  {}", active.len()),
                tab == Tab::Active,
                cx.listener(|console, _, _, cx| set_tab(console, Tab::Active, cx)),
            ),
        ],
        cx,
    );
    let controls = vec![
        tabs,
        IconButton::new("commands-refresh", Icon::RefreshCw)
            .tooltip("Refresh  R")
            .on_click(cx.listener(|console, _, _, cx| {
                refresh(console);
                cx.notify();
            }))
            .into_any_element(),
    ];
    let subtitle = match tab {
        Tab::History => format!(
            "Every write the command queue carried · page {} · ↵ opens, X cancels, [ ] filter, T switches tab",
            page + 1
        ),
        Tab::Active => format!(
            "{} on their way · refreshed every 2 s while shown · ↵ opens, X cancels",
            active.len()
        ),
    };

    let bar = match tab {
        Tab::History => {
            let segments = std::iter::once("All")
                .chain(COMMAND_STATUSES)
                .enumerate()
                .map(|(index, label)| {
                    Segment::new(
                        capitalized(label),
                        filter == index,
                        cx.listener(move |console, _, _, cx| set_filter(console, index, cx)),
                    )
                })
                .collect();
            sub_bar(&theme).child(segmented("commands-filter", segments, cx))
        }
        Tab::Active => sub_bar(&theme).children(ACTIVE_STATUSES.iter().map(|status| {
            let count = active
                .iter()
                .filter(|command| command.status == *status)
                .count();
            pill(
                format!("{count} {status}"),
                if count == 0 {
                    theme.fg_dim
                } else {
                    command_status_color(status, &theme)
                },
            )
        })),
    };

    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(30.0))
        .px(SPACE_8)
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(div().w(px(STATUS)).flex_none().child("STATUS"))
        .child(div().flex_1().min_w_0().child("TAG"))
        .child(div().w(px(VALUE)).flex_none().text_right().child("VALUE"))
        .child(div().w(px(VIA)).flex_none().child("VIA"))
        .child(div().w(px(BY)).flex_none().child("BY"))
        .child(div().w(px(CREATED)).flex_none().child("CREATED"))
        .child(div().w(px(ACTIONS)).flex_none());

    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate_row,
        },
        shown,
        rows_on_screen(window, ROW_HEIGHT_PX),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let scroll = nav.scroll.clone();
    let open_id = open.as_ref().map(|command| command.id.clone());
    let body = if shown == 0 {
        if let Some(error) = error {
            notice(
                format!("Could not load the commands: {error}"),
                true,
                &theme,
            )
        } else if !loaded {
            notice("Loading the commands…", false, &theme)
        } else {
            let (title, text) = match (tab, filter) {
                (Tab::Active, _) => (
                    "Nothing on its way".to_owned(),
                    "Commands still pending, queued or sent appear here.",
                ),
                (Tab::History, 0) => (
                    "No commands yet".to_owned(),
                    "Writes sent through the server's command queue appear here.",
                ),
                (Tab::History, _) => (
                    format!(
                        "No {} commands",
                        COMMAND_STATUSES
                            .get(filter - 1)
                            .copied()
                            .unwrap_or_default()
                    ),
                    "Pick another status with [ and ].",
                ),
            };
            empty_state(Icon::Send, title, text, Vec::new(), cx).into_any_element()
        }
    } else {
        uniform_list(
            "commands",
            shown + tail_rows(shown, ROW_HEIGHT_PX),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let now = time_now_secs();
                    range
                        .map(|row| match rows.get(row) {
                            Some(command) => {
                                let look = RowLook {
                                    ring: focused && cursor == Some(row),
                                    open: open_id.as_deref() == Some(command.id.as_str()),
                                    cancellable: may_cancel(operator, command),
                                    cancelling: cancelling.contains(&command.id),
                                };
                                command_row(console, command, row, look, now, &theme, cx)
                            }
                            None => tail_row(ROW_HEIGHT_PX),
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .size_full()
        .into_any_element()
    };

    let table = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(header)
        .child(list.flex_1().min_h_0().child(body))
        .when(tab == Tab::History, |this| {
            let label = if shown == 0 {
                format!("Page {}", page + 1)
            } else {
                format!(
                    "Page {} · commands {}–{}",
                    page + 1,
                    page * PAGE_SIZE + 1,
                    page * PAGE_SIZE + shown
                )
            };
            this.child(pager(
                "commands",
                label,
                (page > 0, has_next),
                (previous_page, next_page),
                cx,
            ))
        });

    div()
        .flex()
        .flex_col()
        .size_full()
        .key_context(KEY_CONTEXT)
        .on_action(cx.listener(|console, _: &OpsPreviousPage, window, cx| {
            previous_page(console, window, cx);
        }))
        .on_action(cx.listener(|console, _: &OpsNextPage, window, cx| {
            next_page(console, window, cx);
        }))
        .on_action(cx.listener(|console, _: &OpsNextTab, _, cx| {
            let next = match console.state::<State>().tab {
                Tab::History => Tab::Active,
                Tab::Active => Tab::History,
            };
            set_tab(console, next, cx);
        }))
        .on_action(cx.listener(|console, _: &OpsNextFilter, _, cx| {
            step_filter(console, true, cx);
        }))
        .on_action(cx.listener(|console, _: &OpsPreviousFilter, _, cx| {
            step_filter(console, false, cx);
        }))
        .on_action(cx.listener(|console, _: &OpsRefresh, _, cx| {
            refresh(console);
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &OpsCancelCommand, _, cx| {
            cancel_cursor(console, cx);
        }))
        // Esc closes the side panel first.
        .on_action(cx.listener(|console, _: &Dismiss, _, cx| {
            let open = console.state::<State>().open.is_some();
            if open && console.overlay.is_none() {
                console.state::<State>().open = None;
                cx.notify();
            } else {
                cx.propagate();
            }
        }))
        .child(page_header("Commands", subtitle, controls, cx))
        .child(bar)
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(table)
                .when_some(open, |this, command| {
                    this.child(detail(console, &command, operator, cx))
                }),
        )
        .into_any_element()
}

fn sub_bar(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(40.0))
        .px(SPACE_9)
        .border_b_1()
        .border_color(theme.border)
}

/// The tag a command writes, by the name the console shows for it.
fn tag_name(console: &Console, id: i64) -> (String, Option<String>) {
    console
        .store
        .position(id)
        .map(|index| &console.store.tags[index].info)
        .map_or_else(
            || (format!("Tag #{id}"), None),
            |info| {
                let name = if info.name.is_empty() {
                    info.key.clone()
                } else {
                    info.name.clone()
                };
                (name, Some(info.key.clone()))
            },
        )
}

#[derive(Clone, Copy)]
struct RowLook {
    ring: bool,
    open: bool,
    cancellable: bool,
    cancelling: bool,
}

fn command_row(
    console: &Console,
    command: &QueuedCommand,
    row: usize,
    look: RowLook,
    now: i64,
    theme: &Theme,
    cx: &Context<Console>,
) -> AnyElement {
    let color = command_status_color(&command.status, theme);
    let (name, key) = tag_name(console, command.data_source_id);
    let key = key.filter(|key| *key != name);
    let row_id = u64::try_from(row).unwrap_or(0);
    let cancel_button = look.cancellable.then(|| {
        let target = command.clone();
        Button::new(
            ElementId::NamedInteger("cancel-command".into(), row_id),
            "Cancel",
        )
        .compact()
        .ghost()
        .disabled(look.cancelling)
        .on_click(cx.listener(move |console, _, _, cx| cancel(console, &target, cx)))
        .into_any_element()
    });
    let hover = theme.hover;
    div()
        .id(ElementId::NamedInteger("command".into(), row_id))
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(ROW_HEIGHT_PX))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(look.open, |this| this.bg(theme.selected))
        .when(!look.open, |this| this.hover(move |style| style.bg(hover)))
        .on_click(cx.listener(move |console, _, window, cx| {
            let nav = console.list(LIST, cx);
            nav.select(row);
            window.focus(&nav.focus);
            activate_row(console, row, false, window, cx);
        }))
        .when(look.ring, |this| this.child(focus_ring(theme)))
        .child(
            div()
                .w(px(STATUS))
                .flex_none()
                .child(pill(command.status.to_uppercase(), color)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().truncate().font_weight(FontWeight::MEDIUM).child(name))
                .child(
                    div()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(match key {
                            Some(key) => format!("{key} · {}", short_id(&command.id)),
                            None => short_id(&command.id),
                        }),
                ),
        )
        .child(
            div()
                .w(px(VALUE))
                .flex_none()
                .text_right()
                .font_family(MONO)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.accent)
                .truncate()
                .child(command.value.clone()),
        )
        .child(
            div()
                .w(px(VIA))
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(pill(command.protocol.clone(), theme.series[3]))
                .child(
                    div()
                        .truncate()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(command.policy.clone()),
                ),
        )
        .child(
            div()
                .w(px(BY))
                .flex_none()
                .truncate()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(command.created_by.clone()),
        )
        .child(
            div()
                .w(px(CREATED))
                .flex_none()
                .flex()
                .flex_col()
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(local_time(command.created_at, console.utc_offset)),
                )
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(format!("{} ago", age(now - command.created_at))),
                ),
        )
        .child(
            div()
                .w(px(ACTIONS))
                .flex_none()
                .flex()
                .justify_end()
                .children(cancel_button),
        )
        .into_any_element()
}

/// The first characters of a command id, enough to tell rows apart.
fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// The side panel: every field of the open command, its audit trail, and
/// Cancel.
fn detail(
    console: &mut Console,
    command: &QueuedCommand,
    operator: bool,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let offset = console.utc_offset;
    let (name, key) = tag_name(console, command.data_source_id);
    let state = console.state::<State>();
    let audit = state.audit.clone();
    let cancelling = state.cancels.iter().any(|(_, id)| *id == command.id);
    let color = command_status_color(&command.status, &theme);
    let when = |at: Option<i64>| at.map_or_else(|| "—".to_owned(), |at| local_time(at, offset));

    let mut fields: Vec<(&'static str, String)> = vec![
        (
            "Tag",
            key.map_or_else(|| name.clone(), |key| format!("{name} ({key})")),
        ),
        ("Value", command.value.clone()),
        ("Policy", command.policy.clone()),
        ("Protocol", command.protocol.clone()),
        ("Device", format!("#{}", command.device_id)),
        (
            "Retries",
            format!("{} of {}", command.retry_count, command.max_retries),
        ),
        ("Created", when(Some(command.created_at))),
        ("By", command.created_by.clone()),
    ];
    if command.scheduled_at.is_some() {
        fields.push(("Scheduled", when(command.scheduled_at)));
    }
    fields.extend([
        ("Sent", when(command.sent_at)),
        ("Acknowledged", when(command.acked_at)),
        ("Resolved", when(command.resolved_at)),
    ]);
    if let Some(readback) = &command.readback_status {
        fields.push((
            "Read back",
            match command.readback_value {
                Some(value) => format!("{readback} ({})", vt_ui::chart::format_value(value)),
                None => readback.clone(),
            },
        ));
    }

    let trail: AnyElement = match audit {
        None => notice("Loading the trail…", false, &theme),
        Some(Err(error)) => notice(error, true, &theme),
        Some(Ok(entries)) if entries.is_empty() => div()
            .text_size(FONT_SMALL)
            .text_color(theme.fg_muted)
            .child("No status changes recorded.")
            .into_any_element(),
        Some(Ok(entries)) => div()
            .flex()
            .flex_col()
            .gap(SPACE_4)
            .pl(SPACE_5)
            .border_l_2()
            .border_color(theme.border)
            .children(entries.into_iter().map(|entry| {
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_1)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_2)
                            .child(pill(
                                entry.old_status.to_uppercase(),
                                command_status_color(&entry.old_status, &theme),
                            ))
                            .child(vt_ui::icon(Icon::ChevronRight, ICON_SM, theme.fg_dim))
                            .child(pill(
                                entry.new_status.to_uppercase(),
                                command_status_color(&entry.new_status, &theme),
                            )),
                    )
                    .child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(format!(
                                "{} · {}{}",
                                if entry.changed_by.is_empty() {
                                    "system"
                                } else {
                                    entry.changed_by.as_str()
                                },
                                local_time(entry.created_at, offset),
                                entry
                                    .reason
                                    .as_ref()
                                    .map(|reason| format!(" · {reason}"))
                                    .unwrap_or_default()
                            )),
                    )
            }))
            .into_any_element(),
    };

    let target = command.clone();
    div()
        .id("command-detail")
        .flex()
        .flex_col()
        .flex_none()
        .w(px(PANEL))
        .h_full()
        .overflow_y_scroll()
        .border_l_1()
        .border_color(theme.border)
        .bg(theme.chrome)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(48.0))
                .px(SPACE_7)
                .border_b_1()
                .border_color(theme.border)
                .child(pill(command.status.to_uppercase(), color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(SharedString::from(command.id.clone())),
                )
                .child(
                    IconButton::new("command-close", Icon::X)
                        .tooltip("Close  Esc")
                        .on_click(cx.listener(|console, _, _, cx| {
                            console.state::<State>().open = None;
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .p(SPACE_7)
                .children(fields.into_iter().map(|(label, value)| {
                    div()
                        .flex()
                        .gap(SPACE_4)
                        .child(
                            div()
                                .w(px(96.0))
                                .flex_none()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .child(value),
                        )
                }))
                .when_some(command.error_message.clone(), |this, error| {
                    this.child(
                        div()
                            .mt(SPACE_2)
                            .p(SPACE_4)
                            .rounded(RADIUS_MD)
                            .bg(theme.danger.alpha(0.1))
                            .text_size(FONT_SMALL)
                            .text_color(theme.danger)
                            .child(error),
                    )
                })
                .when(may_cancel(operator, command), |this| {
                    this.child(
                        div().pt(SPACE_3).child(
                            Button::new(
                                "command-cancel",
                                if cancelling {
                                    "Cancelling…"
                                } else {
                                    "Cancel the command"
                                },
                            )
                            .danger()
                            .icon(Icon::X)
                            .disabled(cancelling)
                            .tooltip("X")
                            .on_click(cx.listener(
                                move |console, _, _, cx| {
                                    cancel(console, &target, cx);
                                },
                            )),
                        ),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .px(SPACE_7)
                .pb(SPACE_9)
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.fg_dim)
                        .child("AUDIT TRAIL"),
                )
                .child(trail),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::testing::Harness;

    #[test]
    fn the_filter_names_a_status_or_none() {
        let mut state = State::default();
        assert_eq!(state.status(), None);
        state.filter = 1;
        assert_eq!(state.status(), Some("pending"));
        state.filter = COMMAND_STATUSES.len();
        assert_eq!(state.status(), Some("cancelled"));
        state.filter = COMMAND_STATUSES.len() + 1;
        assert_eq!(state.status(), None);
    }

    #[test]
    fn only_a_row_beyond_the_page_offers_a_next_one() {
        let full = || vec![QueuedCommand::default(); PAGE_SIZE];
        let (rows, more) = page_of(full());
        assert_eq!(rows.len(), PAGE_SIZE);
        assert!(!more, "exactly a page: nothing after it");
        let mut extra = full();
        extra.push(QueuedCommand::default());
        let (rows, more) = page_of(extra);
        assert_eq!(rows.len(), PAGE_SIZE, "the extra row is not shown");
        assert!(more);
        assert!(!page_of(Vec::new()).1);
    }

    #[gpui::test]
    fn a_failed_page_shows_its_error_not_the_page_before(cx: &mut gpui::TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| console.show(Page::Commands, window, cx));
        let history = |h: &mut Harness| {
            h.calls()
                .into_iter()
                .find(|key| key.name == "commands.history")
                .expect("History asked")
        };
        let first = history(&mut h);
        h.reply(
            first,
            Reply::ok(vec![QueuedCommand::default(); PAGE_SIZE + 1]),
        );
        assert!(h.update(|console, _, _| console.state::<State>().has_next()));
        h.update(|console, window, cx| next_page(console, window, cx));
        let second = history(&mut h);
        h.reply(second, Reply::failed("HTTP 502"));
        let (page, rows, error, next) = h.update(|console, _, _| {
            let state = console.state::<State>();
            (
                state.page,
                state.history.rows.len(),
                state.history.error.clone(),
                state.has_next(),
            )
        });
        assert_eq!(page, 1);
        assert_eq!(rows, 0, "page 1's rows are not there");
        assert_eq!(error.as_deref(), Some("HTTP 502"));
        assert!(!next);
    }
}
