//! Data Manager › Config snapshots: the web client's
//! `ConfigSnapshotPanel.tsx` — every snapshot (the server's newest 50) in a
//! virtualised table with search and sort, a new snapshot with an optional
//! label, one snapshot opened (its tables and rows), the diff of two
//! (added, removed and changed rows per table, with the changed fields),
//! and a rollback after a confirmation. Operators only, as the server's
//! routes.
//!
//! Keys in the table: ↵ open, N new, F/T pick the diff's from/to, C
//! compare, R roll back, S sort, `/` search.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, App, Context, ElementId, Entity, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, SharedString, Subscription, Window, div, px, uniform_list,
};
use voltrus_scada_native::api::eng_snapshots::{
    self as model, Change, DiffSummary, SnapshotDetail, SnapshotSummary, SortKey,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, empty_state, kbd, pill};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::sidebar_width;
use crate::console::Console;
use crate::console::keys::{
    EngSnapshotsCompare, EngSnapshotsDiffFrom, EngSnapshotsDiffTo, EngSnapshotsNew,
    EngSnapshotsRollback, EngSnapshotsSortNext,
};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::views::{
    ListKeys, focus_ring, keyboard_list, local_time, ops_kit, page_header, rows_on_screen,
    tail_row, tail_rows,
};

const LIST: &str = "data.snapshots";
const DETAIL_LIST: &str = "data.snapshots.detail";
const DIFF_LIST: &str = "data.snapshots.diff";
const CREATE: &str = "data-snapshot-create";
const ROLLBACK: &str = "data-snapshot-rollback";
const DETAIL: &str = "data-snapshot-detail";
const DIFF: &str = "data-snapshot-diff";
const ROW: f32 = 40.0;
const LINE: f32 = 28.0;

const ID: f32 = 56.0;
const CREATED: f32 = 116.0;
const TRIGGER: f32 = 84.0;
const ROWS: f32 = 72.0;
const SIZE: f32 = 76.0;
const ACTIONS: f32 = 120.0;

#[derive(Default)]
struct State {
    list: Option<Result<Vec<SnapshotSummary>, String>>,
    asked: bool,
    /// The newest list request; older answers are dropped.
    seq: i64,
    sort: SortKey,
    reversed: bool,
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    /// Put the cursor on this snapshot once listed (a new one).
    focus_id: Option<i64>,
    window: Option<AnyWindowHandle>,
    // The diff.
    from: Option<i64>,
    to: Option<i64>,
    diff: Option<Result<DiffSummary, String>>,
    comparing: bool,
    // One snapshot opened.
    detail_id: Option<i64>,
    detail: Option<Result<SnapshotDetail, String>>,
    // New snapshot.
    label: Option<Entity<TextInput>>,
    _label: Option<Subscription>,
    create_error: Option<String>,
    creating: bool,
    // Rollback.
    confirm: Option<SnapshotSummary>,
    confirm_focus: Option<FocusHandle>,
    rolling: bool,
}

pub fn on_open(console: &mut Console, _: &mut Context<Console>) {
    if console.stress.is_some() || console.state::<State>().asked {
        return;
    }
    console.state::<State>().asked = true;
    fetch(console);
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console.state_ref::<State>()?.search.clone()
}

fn fetch(console: &mut Console) {
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    console.call(
        CallKey::new("data.snapshots.list").with(seq),
        Call::new(|api| async move { api.config_snapshots().await }),
    );
}

fn snapshots(console: &Console) -> &[SnapshotSummary] {
    match console
        .state_ref::<State>()
        .and_then(|state| state.list.as_ref())
    {
        Some(Ok(list)) => list,
        _ => &[],
    }
}

fn query(console: &Console, cx: &App) -> String {
    console
        .state_ref::<State>()
        .and_then(|state| state.search.as_ref())
        .map(|field| ops_kit::value(field, cx))
        .unwrap_or_default()
}

/// The table's rows: positions in the list.
fn table(console: &Console, cx: &App) -> Vec<usize> {
    let Some(state) = console.state_ref::<State>() else {
        return Vec::new();
    };
    model::visible_rows(
        snapshots(console),
        &query(console, cx),
        state.sort,
        state.reversed,
    )
}

fn snapshot_at(
    console: &mut Console,
    row: usize,
    cx: &mut Context<Console>,
) -> Option<SnapshotSummary> {
    let rows = table(console, cx);
    rows.get(row)
        .and_then(|index| snapshots(console).get(*index))
        .cloned()
}

fn cursor_snapshot(console: &mut Console, cx: &mut Context<Console>) -> Option<SnapshotSummary> {
    let row = console.list(LIST, cx).cursor?;
    snapshot_at(console, row, cx)
}

fn by_id(console: &Console, id: i64) -> Option<SnapshotSummary> {
    snapshots(console)
        .iter()
        .find(|snapshot| snapshot.id == id)
        .cloned()
}

fn ensure_search(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().search.is_some() {
        return;
    }
    let field = ops_kit::text_field("Search snapshots  /", false, cx);
    let subscription = cx.subscribe_in(
        &field,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                console.list(LIST, cx).cursor = None;
                cx.notify();
            }
            InputEvent::Submitted | InputEvent::Cancelled => {
                let nav = console.list(LIST, cx);
                nav.step(Move::First);
                window.focus(&nav.focus);
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    );
    let state = console.state::<State>();
    state.search = Some(field);
    state._search = Some(subscription);
}

fn set_sort(console: &mut Console, key: SortKey, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.sort == key {
        state.reversed = !state.reversed;
    } else {
        state.sort = key;
        state.reversed = false;
    }
    console.list(LIST, cx).cursor = None;
    cx.notify();
}

// ── Open one ─────────────────────────────────────────────────────────────

fn open_detail(console: &mut Console, id: i64, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.detail_id = Some(id);
    state.detail = None;
    console.call(
        CallKey::new("data.snapshots.detail").with(id),
        Call::new(move |api| async move { api.config_snapshot(id).await }),
    );
    console.list(DETAIL_LIST, cx).cursor = None;
    let overlay = Overlay::sheet(DETAIL, px(520.0), detail_sheet, cx);
    console.open_overlay(overlay, window, cx);
    console.focus_list(DETAIL_LIST, window, cx);
}

fn activate_row(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(snapshot) = snapshot_at(console, row, cx) {
        open_detail(console, snapshot.id, window, cx);
    }
}

// ── Diff ─────────────────────────────────────────────────────────────────

fn pick(console: &mut Console, id: i64, from: bool, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if from {
        state.from = Some(id);
    } else {
        state.to = Some(id);
    }
    state.diff = None;
    state.comparing = false;
    cx.notify();
}

fn pick_cursor(console: &mut Console, from: bool, cx: &mut Context<Console>) {
    if let Some(snapshot) = cursor_snapshot(console, cx) {
        pick(console, snapshot.id, from, cx);
    }
}

fn compare(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some((from, to)) = model::diff_ready(state.from, state.to) else {
        console.show_toast("Pick the snapshots to compare: F from, T to", false, cx);
        return;
    };
    state.diff = None;
    state.comparing = true;
    console.call(
        CallKey::new("data.snapshots.diff").with(from),
        Call::new(move |api| async move { api.diff_config_snapshots(from, to).await }),
    );
    console.list(DIFF_LIST, cx).cursor = None;
    let overlay = Overlay::sheet(DIFF, px(560.0), diff_sheet, cx);
    console.open_overlay(overlay, window, cx);
    console.focus_list(DIFF_LIST, window, cx);
}

fn clear_diff(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.from = None;
    state.to = None;
    state.diff = None;
    cx.notify();
}

// ── New ──────────────────────────────────────────────────────────────────

fn open_create(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !console.is_operator() {
        return;
    }
    let field = ops_kit::text_field("e.g. Before flow refactor", false, cx);
    let subscription = ops_kit::on_field(
        &field,
        |console, event, window, cx| match event {
            InputEvent::Submitted => create(console, cx),
            InputEvent::Cancelled => console.close_overlay(window, cx),
            InputEvent::Changed | InputEvent::Blurred => {}
        },
        window,
        cx,
    );
    let state = console.state::<State>();
    state.label = Some(field.clone());
    state._label = Some(subscription);
    state.create_error = None;
    state.creating = false;
    state.window = Some(window.window_handle());
    let overlay = Overlay::modal(CREATE, px(440.0), create_dialog, cx);
    console.open_overlay(overlay, window, cx);
    field.read(cx).focus(window);
}

fn create(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.creating {
        return;
    }
    let label = state
        .label
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    state.creating = true;
    state.create_error = None;
    console.call(
        CallKey::new("data.snapshots.create"),
        Call::new(move |api| async move { api.create_config_snapshot(&label).await }),
    );
    cx.notify();
}

// ── Rollback ─────────────────────────────────────────────────────────────

fn ask_rollback(console: &mut Console, id: i64, window: &mut Window, cx: &mut Context<Console>) {
    if !console.is_operator() {
        return;
    }
    let Some(snapshot) = by_id(console, id) else {
        return;
    };
    let focus = cx.focus_handle();
    let state = console.state::<State>();
    state.confirm = Some(snapshot);
    state.confirm_focus = Some(focus.clone());
    state.rolling = false;
    state.window = Some(window.window_handle());
    let overlay = Overlay::modal(ROLLBACK, px(460.0), rollback_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn rollback_confirmed(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.rolling {
        return;
    }
    let Some(snapshot) = state.confirm.clone() else {
        return;
    };
    state.rolling = true;
    let id = snapshot.id;
    console.call(
        CallKey::new("data.snapshots.rollback").with(id),
        Call::new(move |api| async move { api.rollback_config_snapshot(id).await }),
    );
    cx.notify();
}

fn close_later(console: &mut Console, id: &'static str, cx: &mut Context<Console>) {
    if let Some(window) = console.state::<State>().window {
        ops_kit::close_overlay_in(window, id, cx);
    }
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "data.snapshots.list" => {
            let state = console.state::<State>();
            if key.arg == state.seq {
                state.list = Some(reply.take::<Vec<SnapshotSummary>>());
            }
        }
        "data.snapshots.detail" => {
            let state = console.state::<State>();
            if state.detail_id == Some(key.arg) {
                state.detail = Some(reply.take::<SnapshotDetail>());
            }
        }
        "data.snapshots.diff" => {
            let state = console.state::<State>();
            if state.from == Some(key.arg) {
                state.comparing = false;
                state.diff = Some(reply.take::<DiffSummary>());
            }
        }
        "data.snapshots.create" => {
            let state = console.state::<State>();
            state.creating = false;
            match reply.take::<SnapshotSummary>() {
                Ok(created) => {
                    state.focus_id = Some(created.id);
                    close_later(console, CREATE, cx);
                    console.show_toast(
                        format!("Saved snapshot #{} ({})", created.id, created.counts_line()),
                        false,
                        cx,
                    );
                    fetch(console);
                }
                Err(error) => state.create_error = Some(error),
            }
        }
        "data.snapshots.rollback" => {
            let state = console.state::<State>();
            state.rolling = false;
            match reply.take::<()>() {
                Ok(()) => {
                    state.confirm = None;
                    close_later(console, ROLLBACK, cx);
                    console.show_toast(
                        format!(
                            "Rolled back to snapshot #{}; the previous configuration was saved as a restore snapshot",
                            key.arg
                        ),
                        false,
                        cx,
                    );
                    // Sources and screens may have changed under every page.
                    console.refresh_tags();
                    fetch(console);
                }
                Err(error) => {
                    console.show_toast(format!("Rollback failed: {error}"), true, cx);
                }
            }
        }
        _ => {}
    }
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn trigger_color(trigger: &str, theme: &Theme) -> Hsla {
    match trigger {
        "manual" => theme.success,
        "restore" => theme.warning,
        _ => theme.info,
    }
}

fn element_id(name: &'static str, id: i64) -> ElementId {
    ElementId::NamedInteger(name.into(), u64::try_from(id).unwrap_or(0))
}

#[derive(Clone, Copy)]
struct RowLook {
    cursor: Option<usize>,
    focused: bool,
    from: Option<i64>,
    to: Option<i64>,
    operator: bool,
    offset: time::UtcOffset,
    /// Width of the label column.
    label: f32,
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    ensure_search(console, window, cx);
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let rows: Rc<[usize]> = table(console, cx).into();
    let shown = rows.len();
    // Loaded or not, and the failure, without copying the list.
    let list_state: Option<Result<(), String>> = console
        .state_ref::<State>()
        .and_then(|state| state.list.as_ref())
        .map(|list| list.as_ref().map(|_| ()).map_err(Clone::clone));
    let total = snapshots(console).len();
    let state = console.state::<State>();
    let (sort, reversed, from, to, comparing) = (
        state.sort,
        state.reversed,
        state.from,
        state.to,
        state.comparing,
    );
    let search_field = state.search.clone();
    let focus_id = state.focus_id;

    if let Some(id) = focus_id
        && let Some(row) = rows
            .iter()
            .position(|index| snapshots(console).get(*index).is_some_and(|s| s.id == id))
    {
        console.state::<State>().focus_id = None;
        console.list(LIST, cx).select(row);
    }

    let width = f32::from(window.viewport_size().width - sidebar_width());
    let label_width = (width - ID - CREATED - TRIGGER - ROWS - SIZE - ACTIONS - 120.0).max(120.0);

    let arrow = if reversed { "↑" } else { "↓" };
    let header_cell = |label: &'static str,
                       key: Option<SortKey>,
                       width: Option<f32>,
                       cx: &mut Context<Console>| {
        let active = key.is_some_and(|key| key == sort);
        let cell = div()
            .flex()
            .items_center()
            .gap(SPACE_1)
            .map(|this| match width {
                Some(width) => this.w(px(width)).flex_none(),
                None => this.flex_1().min_w_0(),
            })
            .text_color(if active { theme.fg } else { theme.fg_dim })
            .child(label)
            .when(active, |this| this.child(arrow));
        match key {
            Some(key) => cell
                .id(SharedString::from(format!("snapshot-sort-{}", key.label())))
                .cursor_pointer()
                .hover(|style| style.text_color(theme.fg))
                .on_click(cx.listener(move |console, _, _, cx| set_sort(console, key, cx)))
                .into_any_element(),
            None => cell.into_any_element(),
        }
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
        .child(header_cell("ID", None, Some(ID), cx))
        .child(header_cell(
            "CREATED",
            Some(SortKey::Created),
            Some(CREATED),
            cx,
        ))
        .child(header_cell("LABEL", Some(SortKey::Label), None, cx))
        .child(header_cell(
            "TRIGGER",
            Some(SortKey::Trigger),
            Some(TRIGGER),
            cx,
        ))
        .child(header_cell("ROWS", None, Some(ROWS), cx))
        .child(header_cell("SIZE", Some(SortKey::Size), Some(SIZE), cx))
        .child(div().w(px(ACTIONS)).flex_none());

    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate_row,
        },
        shown,
        rows_on_screen(window, ROW),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let look = RowLook {
        cursor: nav.cursor,
        focused: nav.focus.is_focused(window),
        from,
        to,
        operator,
        offset: console.utc_offset,
        label: label_width,
    };
    let scroll = console.list(LIST, cx).scroll.clone();

    let body = match &list_state {
        None => ops_kit::notice("Loading snapshots…", false, &theme),
        Some(Err(error)) => ops_kit::notice(format!("Snapshots: {error}"), true, &theme),
        Some(Ok(_)) if shown == 0 => empty_state(
            if total == 0 { Icon::History } else { Icon::Search },
            if total == 0 {
                "No snapshots yet"
            } else {
                "No snapshot matches"
            },
            if total == 0 {
                "A snapshot saves the devices, sources, flows, alarms, screens and recipes. Make one with N."
            } else {
                "Nothing has this text in its label, trigger, author or #id."
            },
            if total == 0 && operator {
                vec![
                    Button::new("snapshot-first", "New snapshot")
                        .primary()
                        .icon(Icon::Plus)
                        .on_click(cx.listener(|console, _, window, cx| {
                            open_create(console, window, cx);
                        }))
                        .into_any_element(),
                ]
            } else {
                Vec::new()
            },
            cx,
        )
        .into_any_element(),
        Some(Ok(_)) => uniform_list(
            "data-snapshots",
            shown + tail_rows(shown, ROW),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    range
                        .map(|row| match rows.get(row).copied() {
                            Some(index) => snapshot_row(console, index, row, look, &theme, cx),
                            None => tail_row(ROW),
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .size_full()
        .into_any_element(),
    };

    let subtitle = format!(
        "{total} snapshots (the newest 50) · sorted by {}{arrow} · ↵ open · N new · F/T diff from/to · C compare · R roll back · S sort",
        sort.label()
    );
    let controls = vec![
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .w(px(220.0))
            .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
            .children(search_field.map(|field| div().flex_1().child(field)))
            .into_any_element(),
        IconButton::new("snapshots-refresh", Icon::RefreshCw)
            .tooltip("Refresh")
            .on_click(cx.listener(|console, _, _, cx| {
                fetch(console);
                cx.notify();
            }))
            .into_any_element(),
        Button::new("snapshots-new", "New snapshot")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .disabled(!operator)
            .on_click(cx.listener(|console, _, window, cx| open_create(console, window, cx)))
            .into_any_element(),
    ];

    let end = |id: Option<i64>, word: &'static str| {
        let text = id.map_or_else(
            || format!("{word}: pick a row"),
            |id| {
                by_id(console, id).map_or_else(
                    || format!("{word}: #{id}"),
                    |snapshot| format!("{word}: #{id} {}", snapshot.title()),
                )
            },
        );
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .font_family(MONO)
            .text_size(FONT_SMALL)
            .text_color(if id.is_some() { theme.fg } else { theme.fg_dim })
            .child(text)
    };
    let diff_bar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .px(SPACE_9)
        .py(SPACE_3)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child("DIFF"),
        )
        .child(kbd("F", cx))
        .child(end(from, "From"))
        .child(vt_ui::icon(Icon::ChevronRight, ICON_SM, theme.fg_dim))
        .child(kbd("T", cx))
        .child(end(to, "To"))
        .child(
            Button::new(
                "snapshots-compare",
                if comparing { "Comparing…" } else { "Compare" },
            )
            .compact()
            .icon(Icon::Columns)
            .tooltip("C")
            .disabled(from.is_none() || to.is_none())
            .on_click(cx.listener(|console, _, window, cx| compare(console, window, cx))),
        )
        .when(from.is_some() || to.is_some(), |this| {
            this.child(
                Button::new("snapshots-clear", "Clear")
                    .compact()
                    .ghost()
                    .on_click(cx.listener(|console, _, _, cx| clear_diff(console, cx))),
            )
        });

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("Config snapshots", subtitle, controls, cx))
        .child(diff_bar)
        .child(
            div()
                .key_context("EngSnapshots")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &EngSnapshotsNew, window, cx| {
                    open_create(console, window, cx);
                }))
                .on_action(cx.listener(|console, _: &EngSnapshotsDiffFrom, _, cx| {
                    pick_cursor(console, true, cx);
                }))
                .on_action(cx.listener(|console, _: &EngSnapshotsDiffTo, _, cx| {
                    pick_cursor(console, false, cx);
                }))
                .on_action(cx.listener(|console, _: &EngSnapshotsCompare, window, cx| {
                    compare(console, window, cx);
                }))
                .on_action(
                    cx.listener(|console, _: &EngSnapshotsRollback, window, cx| {
                        if let Some(snapshot) = cursor_snapshot(console, cx) {
                            ask_rollback(console, snapshot.id, window, cx);
                        }
                    }),
                )
                .on_action(cx.listener(|console, _: &EngSnapshotsSortNext, _, cx| {
                    let next = console.state::<State>().sort.next();
                    set_sort(console, next, cx);
                }))
                .child(header)
                .child(list.flex_1().min_h_0().child(body)),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn snapshot_row(
    console: &Console,
    index: usize,
    row: usize,
    look: RowLook,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let Some(snapshot) = snapshots(console).get(index).cloned() else {
        return tail_row(ROW);
    };
    let id = snapshot.id;
    let current = look.cursor == Some(row);
    let hover = theme.hover;
    let mono = |text: String, width: f32, color: Hsla| {
        div()
            .w(px(width))
            .flex_none()
            .truncate()
            .font_family(MONO)
            .text_size(FONT_SMALL)
            .text_color(color)
            .child(text)
    };
    let byline = match (&snapshot.created_by, &snapshot.label, &snapshot.description) {
        (Some(by), _, _) => Some(format!("by {by}")),
        (None, Some(_), Some(description)) => Some(description.clone()),
        _ => None,
    };
    div()
        .id(element_id("snapshot", id))
        .group("snapshot-row")
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(ROW))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(current, |this| this.bg(theme.selected))
        .when(!current, |this| this.hover(move |style| style.bg(hover)))
        .on_click(
            cx.listener(move |console, event: &gpui::ClickEvent, window, cx| {
                console.list(LIST, cx).select(row);
                if event.click_count() >= 2 {
                    open_detail(console, id, window, cx);
                } else {
                    let focus = console.list(LIST, cx).focus.clone();
                    window.focus(&focus);
                }
                cx.notify();
            }),
        )
        .when(look.focused && current, |this| {
            this.child(focus_ring(theme))
        })
        .child(mono(format!("#{id}"), ID, theme.fg_dim))
        .child(mono(
            local_time(snapshot.created_at, look.offset),
            CREATED,
            theme.fg_muted,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .child(
                            div()
                                .w(px(look.label - 80.0))
                                .truncate()
                                .child(snapshot.title()),
                        )
                        .when(look.from == Some(id), |this| {
                            this.child(pill("FROM", theme.accent))
                        })
                        .when(look.to == Some(id), |this| {
                            this.child(pill("TO", theme.accent))
                        }),
                )
                .when_some(byline, |this, line| {
                    this.child(
                        div()
                            .truncate()
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child(line),
                    )
                }),
        )
        .child(div().w(px(TRIGGER)).flex_none().flex().child(pill(
            snapshot.trigger.to_uppercase(),
            trigger_color(&snapshot.trigger, theme),
        )))
        .child(mono(
            if snapshot.unreadable {
                "?".to_owned()
            } else {
                snapshot.total_rows().to_string()
            },
            ROWS,
            theme.fg_muted,
        ))
        .child(mono(
            model::size_label(snapshot.bytes),
            SIZE,
            theme.fg_muted,
        ))
        .child(
            div()
                .w(px(ACTIONS))
                .flex_none()
                .flex()
                .justify_end()
                .gap(SPACE_1)
                .opacity(if current { 1.0 } else { 0.0 })
                .group_hover("snapshot-row", |style| style.opacity(1.0))
                .child(
                    IconButton::new(element_id("snapshot-open", id), Icon::Eye)
                        .tooltip("Open  ↵")
                        .on_click(cx.listener(move |console, _, window, cx| {
                            open_detail(console, id, window, cx);
                        })),
                )
                .child(
                    IconButton::new(element_id("snapshot-from", id), Icon::ChevronLeft)
                        .tooltip("Diff from  F")
                        .active(look.from == Some(id))
                        .on_click(
                            cx.listener(move |console, _, _, cx| pick(console, id, true, cx)),
                        ),
                )
                .child(
                    IconButton::new(element_id("snapshot-to", id), Icon::ChevronRight)
                        .tooltip("Diff to  T")
                        .active(look.to == Some(id))
                        .on_click(
                            cx.listener(move |console, _, _, cx| pick(console, id, false, cx)),
                        ),
                )
                .child(
                    IconButton::new(element_id("snapshot-rollback", id), Icon::History)
                        .tooltip(if look.operator {
                            "Roll back to this  R"
                        } else {
                            "Operators only"
                        })
                        .color(theme.warning)
                        .disabled(!look.operator)
                        .on_click(cx.listener(move |console, _, window, cx| {
                            ask_rollback(console, id, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

// ── Sheets ───────────────────────────────────────────────────────────────

/// One line of the detail or diff sheet.
#[derive(Clone)]
enum Line {
    Heading {
        name: String,
        info: String,
    },
    Item {
        sign: Option<Change>,
        text: String,
        detail: String,
    },
    Note(String, bool),
}

fn detail_lines(detail: &SnapshotDetail) -> Vec<Line> {
    let mut lines = Vec::new();
    if detail.summary.unreadable {
        lines.push(Line::Note(
            "The snapshot's data is not readable JSON.".to_owned(),
            true,
        ));
    }
    for table in &detail.tables {
        lines.push(Line::Heading {
            name: table.name.clone(),
            info: format!("{} rows", table.rows.len()),
        });
        if table.rows.is_empty() {
            lines.push(Line::Note("empty".to_owned(), false));
        }
        lines.extend(table.rows.iter().map(|row| Line::Item {
            sign: None,
            text: row.clone(),
            detail: String::new(),
        }));
    }
    lines
}

fn diff_lines(diff: &DiffSummary) -> Vec<Line> {
    if diff.is_empty() {
        return vec![Line::Note(
            "No differences between these snapshots.".to_owned(),
            false,
        )];
    }
    let mut lines = Vec::new();
    for table in &diff.tables {
        lines.push(Line::Heading {
            name: table.name.clone(),
            info: format!("+{} −{} ~{}", table.added, table.removed, table.changed),
        });
        lines.extend(table.items.iter().map(|item| Line::Item {
            sign: Some(item.change),
            text: item.label.clone(),
            detail: item.fields.join(", "),
        }));
    }
    lines
}

fn change_color(change: Option<Change>, theme: &Theme) -> Hsla {
    match change {
        Some(Change::Added) => theme.success,
        Some(Change::Removed) => theme.danger,
        Some(Change::Changed) => theme.warning,
        None => theme.fg_muted,
    }
}

fn draw_line(line: &Line, index: usize, ring: bool, theme: &Theme) -> AnyElement {
    let base = div()
        .id(("snapshot-line", index))
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .h(px(LINE))
        .px(SPACE_8)
        .when(ring, |this| this.child(focus_ring(theme)));
    let element = match line {
        Line::Heading { name, info } => base
            .bg(theme.surface)
            .font_weight(FontWeight::SEMIBOLD)
            .child(div().flex_1().min_w_0().truncate().child(name.clone()))
            .child(
                div()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(info.clone()),
            ),
        Line::Item { sign, text, detail } => {
            let color = change_color(*sign, theme);
            base.child(
                div()
                    .w(px(14.0))
                    .flex_none()
                    .font_family(MONO)
                    .text_color(color)
                    .child(sign.map_or("·", Change::sign)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(if sign.is_some() { color } else { theme.fg })
                    .child(text.clone()),
            )
            .when(!detail.is_empty(), |this| {
                this.child(
                    div()
                        .max_w(px(240.0))
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(detail.clone()),
                )
            })
        }
        Line::Note(text, error) => base.child(
            div()
                .text_size(FONT_SMALL)
                .text_color(if *error { theme.danger } else { theme.fg_dim })
                .child(text.clone()),
        ),
    };
    element.into_any_element()
}

/// A sheet: a title block and a keyboard list of lines.
#[allow(clippy::too_many_arguments)]
fn sheet(
    list_id: &'static str,
    title: String,
    subtitle: String,
    lines: Option<Result<Vec<Line>, String>>,
    footer: Option<AnyElement>,
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let lines: Rc<[Line]> = match lines {
        None => vec![Line::Note("Loading…".to_owned(), false)].into(),
        Some(Err(error)) => vec![Line::Note(error, true)].into(),
        Some(Ok(lines)) => lines.into(),
    };
    let count = lines.len();
    let list = keyboard_list(
        ListKeys {
            id: list_id,
            on_move: None,
            on_activate: |_, _, _, _, _| {},
        },
        count,
        rows_on_screen(window, LINE),
        console,
        cx,
    );
    let nav = console.list(list_id, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(SPACE_1)
                .px(SPACE_8)
                .py(SPACE_6)
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(subtitle),
                ),
        )
        .child(
            list.flex_1().min_h_0().child(
                uniform_list(
                    list_id,
                    count + tail_rows(count, LINE),
                    cx.processor(
                        move |_: &mut Console,
                              range: Range<usize>,
                              _,
                              cx: &mut Context<Console>| {
                            let theme = vt_ui::theme::theme(cx).clone();
                            range
                                .map(|index| match lines.get(index) {
                                    Some(line) => draw_line(
                                        line,
                                        index,
                                        focused && cursor == Some(index),
                                        &theme,
                                    ),
                                    None => tail_row(LINE),
                                })
                                .collect::<Vec<_>>()
                        },
                    ),
                )
                .track_scroll(scroll)
                .size_full(),
            ),
        )
        .children(footer)
        .into_any_element()
}

fn detail_sheet(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let state = console.state::<State>();
    let id = state.detail_id.unwrap_or(0);
    let detail = state.detail.clone();
    let operator = console.is_operator();
    let summary = detail
        .as_ref()
        .and_then(|detail| detail.as_ref().ok())
        .map(|detail| detail.summary.clone())
        .or_else(|| by_id(console, id));
    let (title, subtitle) = summary.as_ref().map_or_else(
        || (format!("Snapshot #{id}"), String::new()),
        |summary| {
            (
                format!("#{id} · {}", summary.title()),
                format!(
                    "{} · {} · {} · {} rows · {}",
                    local_time(summary.created_at, console.utc_offset),
                    summary.trigger,
                    summary.created_by.as_deref().unwrap_or("server"),
                    summary.total_rows(),
                    model::size_label(summary.bytes)
                ),
            )
        },
    );
    let theme = theme(cx).clone();
    let footer = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(52.0))
        .px(SPACE_8)
        .border_t_1()
        .border_color(theme.border)
        .child(
            Button::new("snapshot-detail-from", "Diff from")
                .compact()
                .on_click(cx.listener(move |console, _, _, cx| pick(console, id, true, cx))),
        )
        .child(
            Button::new("snapshot-detail-to", "Diff to")
                .compact()
                .on_click(cx.listener(move |console, _, _, cx| pick(console, id, false, cx))),
        )
        .child(div().flex_1())
        .child(
            Button::new("snapshot-detail-rollback", "Roll back…")
                .compact()
                .danger()
                .icon(Icon::History)
                .disabled(!operator)
                .on_click(cx.listener(move |console, _, window, cx| {
                    console.close_overlay(window, cx);
                    ask_rollback(console, id, window, cx);
                })),
        )
        .into_any_element();
    let lines = detail.map(|detail| detail.map(|detail| detail_lines(&detail)));
    sheet(
        DETAIL_LIST,
        title,
        subtitle,
        lines,
        Some(footer),
        console,
        window,
        cx,
    )
}

fn diff_sheet(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let state = console.state::<State>();
    let (from, to) = (state.from.unwrap_or(0), state.to.unwrap_or(0));
    let diff = state.diff.clone();
    let subtitle = match &diff {
        Some(Ok(diff)) => {
            let (added, removed, changed) = diff.totals();
            format!(
                "{} tables differ · {added} added · {removed} removed · {changed} changed",
                diff.tables.len()
            )
        }
        Some(Err(_)) => "The comparison failed".to_owned(),
        None => "Comparing…".to_owned(),
    };
    let lines = diff.map(|diff| diff.map(|diff| diff_lines(&diff)));
    sheet(
        DIFF_LIST,
        format!("Diff #{from} → #{to}"),
        subtitle,
        lines,
        None,
        console,
        window,
        cx,
    )
}

// ── Dialogs ──────────────────────────────────────────────────────────────

fn create_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let field = state.label.clone();
    let (error, creating) = (state.create_error.clone(), state.creating);
    div()
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(vt_ui::icon(Icon::History, ICON_LG, theme.accent))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("New config snapshot"),
                ),
        )
        .child(div().text_color(theme.fg_muted).child(
            "Saves the devices, data sources, flows, alarms, screens and recipes as they are now.",
        ))
        .children(field.map(|field| ops_kit::labelled("Label (optional)", field, &theme)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .map(|this| match &error {
                            Some(error) => this
                                .text_size(FONT_SMALL)
                                .text_color(theme.danger)
                                .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.danger))
                                .child(div().min_w_0().truncate().child(error.clone())),
                            None => this
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(kbd("↵", cx))
                                .child("save")
                                .child(kbd("Esc", cx))
                                .child("close"),
                        }),
                )
                .child(
                    Button::new("snapshot-create-cancel", "Cancel")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new(
                        "snapshot-create-save",
                        if creating { "Saving…" } else { "Save" },
                    )
                    .primary()
                    .icon(Icon::Check)
                    .disabled(creating)
                    .on_click(cx.listener(|console, _, _, cx| create(console, cx))),
                ),
        )
        .into_any_element()
}

/// The rollback confirmation: Enter or the button rolls back, Esc keeps.
fn rollback_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let offset = console.utc_offset;
    let state = console.state::<State>();
    let Some(snapshot) = state.confirm.clone() else {
        return div().into_any_element();
    };
    let focus = state.confirm_focus.clone();
    let rolling = state.rolling;
    div()
        .when_some(focus, |this, focus| this.track_focus(&focus))
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "enter" {
                rollback_confirmed(console, cx);
                cx.stop_propagation();
            }
        }))
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(vt_ui::icon(Icon::History, ICON_LG, theme.danger))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Roll back to snapshot #{}?", snapshot.id)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .text_color(theme.fg_muted)
                .child(format!(
                    "Restores the configuration saved {} — {} ({}).",
                    local_time(snapshot.created_at, offset),
                    snapshot.title(),
                    snapshot.counts_line()
                ))
                .child(
                    div()
                        .text_color(theme.danger)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("This overwrites the current configuration. The server saves it first as a restore snapshot."),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(kbd("↵", cx))
                        .child("roll back")
                        .child(kbd("Esc", cx))
                        .child("keep"),
                )
                .child(
                    Button::new("snapshot-rollback-cancel", "Keep")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.state::<State>().confirm = None;
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new(
                        "snapshot-rollback-confirm",
                        if rolling { "Rolling back…" } else { "Roll back" },
                    )
                    .danger()
                    .icon(Icon::History)
                    .disabled(rolling)
                    .on_click(cx.listener(|console, _, _, cx| rollback_confirmed(console, cx))),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use voltrus_scada_native::api::eng_snapshots::{
        ConfigDiff, ConfigSnapshot, DiffItem, DiffTable, TableDiff,
    };

    #[test]
    fn the_detail_sheet_lists_each_table_and_its_rows() {
        let snapshot = ConfigSnapshot {
            id: 3,
            snapshot_data: r#"{"devices":[{"id":1,"name":"A"}],"flows":[]}"#.into(),
            ..ConfigSnapshot::default()
        };
        let lines = detail_lines(&SnapshotDetail::from_snapshot(&snapshot));
        assert_eq!(
            lines.len(),
            4,
            "devices heading + row, flows heading + empty"
        );
        assert!(
            matches!(&lines[0], Line::Heading { name, info } if name == "devices" && info == "1 rows")
        );
        assert!(matches!(&lines[1], Line::Item { text, .. } if text == "A (#1)"));
        assert!(matches!(&lines[3], Line::Note(text, false) if text == "empty"));
    }

    #[test]
    fn the_diff_sheet_counts_per_table_and_signs_each_row() {
        let mut diff = ConfigDiff::new();
        diff.insert(
            "flows".into(),
            TableDiff {
                added: vec![serde_json::json!({"id": 2, "name": "New"})],
                ..TableDiff::default()
            },
        );
        let summary = DiffSummary::new(1, 2, &diff);
        let lines = diff_lines(&summary);
        assert!(matches!(&lines[0], Line::Heading { info, .. } if info == "+1 −0 ~0"));
        assert!(matches!(
            &lines[1],
            Line::Item {
                sign: Some(Change::Added),
                ..
            }
        ));
        let none = diff_lines(&DiffSummary::default());
        assert!(matches!(&none[0], Line::Note(..)));
        // Changed rows carry their fields.
        let changed = DiffSummary {
            from: 1,
            to: 2,
            tables: vec![DiffTable {
                name: "screens".into(),
                added: 0,
                removed: 0,
                changed: 1,
                items: vec![DiffItem {
                    change: Change::Changed,
                    label: "PLTS".into(),
                    fields: vec!["title".into(), "layout".into()],
                }],
            }],
        };
        assert!(
            matches!(&diff_lines(&changed)[1], Line::Item { detail, .. } if detail == "title, layout")
        );
    }
}
