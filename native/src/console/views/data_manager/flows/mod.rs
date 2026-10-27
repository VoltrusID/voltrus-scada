//! Data Manager › Flows: the web client's `FlowManager.tsx` and its flow
//! editor. The list (virtualised, `/` search, sort header) starts and
//! stops flows, opens the settings form (new flow, rename, enabled),
//! the runs history, the test run with a JSON payload and the delete
//! confirmation; Enter opens a flow on the native canvas editor
//! ([`editor`], drawn by [`canvas`], its geometry in [`model`]).
//!
//! The server's `start` and `stop` change the running engine only; the
//! stored `enabled` flag (what the engine loads at start-up) changes in
//! the settings form or the editor. The list shows both: what this console
//! last started or stopped, and the stored flag when they differ.

pub(crate) mod canvas;
pub(crate) mod editor;
pub(crate) mod forms;
pub(crate) mod model;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, Context, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent,
    SharedString, Subscription, Window, div, px, uniform_list,
};
use voltrus_scada_native::api::eng_flows::{Flow, Tone, trigger_kind};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, empty_state, kbd, pill};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use crate::console::Console;
use crate::console::keys::{
    EngFlowsDelete, EngFlowsEdit, EngFlowsNew, EngFlowsOpen, EngFlowsRefresh, EngFlowsRuns,
    EngFlowsSortNext, EngFlowsSortReverse, EngFlowsTest,
};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::views::ops_kit::notice;
use crate::console::views::{
    ListKeys, focus_ring, keyboard_list, local_time, page_header, rows_on_screen, tail_row,
    tail_rows,
};

/// The table's keyboard list.
pub(crate) const LIST: &str = "data.flows";
/// The key context around the table (`keys::ENG_FLOWS` binds in it).
const CONTEXT: &str = "EngFlows";
const ROW_HEIGHT_PX: f32 = 40.0;
const DELETE: &str = "data-flow-delete";

const STATUS: f32 = 168.0;
const ID: f32 = 200.0;
const TRIGGER: f32 = 92.0;
const STEPS: f32 = 52.0;
const UPDATED: f32 = 104.0;
const ACTIONS: f32 = 196.0;

/// The table's sort column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Sort {
    #[default]
    Name,
    Id,
    Status,
    Trigger,
    Steps,
    Updated,
}

impl Sort {
    const ALL: [Self; 6] = [
        Self::Name,
        Self::Id,
        Self::Status,
        Self::Trigger,
        Self::Steps,
        Self::Updated,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Id => "ID",
            Self::Status => "status",
            Self::Trigger => "trigger",
            Self::Steps => "steps",
            Self::Updated => "updated",
        }
    }

    fn next(self) -> Self {
        let at = Self::ALL.iter().position(|sort| *sort == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }
}

#[derive(Default)]
pub(crate) struct State {
    /// The flows as last fetched, or why they are not.
    flows: Option<Result<Vec<Flow>, String>>,
    asked: bool,
    /// Numbers list requests; an older answer is dropped.
    list_seq: i64,
    /// Numbers the other calls (start, stop, delete, runs, tests).
    seq: i64,
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    sort: Sort,
    descending: bool,
    /// What this console last started (true) or stopped, by flow id: the
    /// server reports no engine state.
    running: HashMap<String, bool>,
    /// Start, stop and delete calls on their way, by call number.
    pending: HashMap<i64, String>,
    /// Put the cursor on this flow once the list has it.
    focus_id: Option<String>,
    /// The flow the delete confirmation is about: id, name.
    confirm: Option<(String, String)>,
    confirm_focus: Option<FocusHandle>,
    /// The window, for what a reply opens or closes.
    window: Option<AnyWindowHandle>,
    form: Option<forms::MetaForm>,
    runs: Option<forms::Runs>,
    test: Option<forms::Test>,
    /// The last test payload typed, offered again.
    test_payload: Option<String>,
    editor: Option<editor::Editor>,
    /// O came before the list: open its first flow when it arrives.
    open_pending: bool,
    /// Bumped when `flows` or `running` change: the table cache's key.
    version: u64,
    /// The table last built and its rows.
    table: Option<(TableKey, Rc<[Flow]>)>,
}

/// What a built table depends on: query, sort, descending, version.
type TableKey = (String, Sort, bool, u64);

pub fn on_open(console: &mut Console, _: &mut Context<Console>) {
    if console.state::<State>().asked {
        return;
    }
    fetch(console);
}

/// After a config rollback: the flows list afresh when next opened.
pub(crate) fn stale(console: &mut Console) {
    console.state::<State>().asked = false;
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().editor.is_some() {
        editor::focus_canvas(console, window);
    } else {
        console.focus_list(LIST, window, cx);
    }
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    let state = console.state_ref::<State>()?;
    if state.editor.is_some() {
        return None;
    }
    state.search.clone()
}

/// `GET /api/v1/flows`, numbered.
fn fetch(console: &mut Console) {
    // A stress run has no server to ask.
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    state.asked = true;
    state.list_seq += 1;
    let seq = state.list_seq;
    console.call(
        CallKey::new("data.flows.list").with(seq),
        Call::new(|api| async move { api.flows().await }),
    );
}

/// Fetch the list again (after a change).
pub(super) fn refresh(console: &mut Console) {
    fetch(console);
}

fn ensure_search(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().search.is_some() {
        return;
    }
    let field = cx.new(|cx| TextInput::new(cx).placeholder("Search flows  /"));
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

/// Whether the engine runs `flow`, as far as this console knows.
fn running(state: &State, flow: &Flow) -> bool {
    state.running.get(&flow.id).copied().unwrap_or(flow.enabled)
}

/// The flows the table shows: matching the search (name, id, trigger),
/// sorted.
fn table(console: &mut Console, cx: &gpui::App) -> Rc<[Flow]> {
    let query = console
        .state_ref::<State>()
        .and_then(|state| state.search.as_ref())
        .map(|field| field.read(cx).value().trim().to_lowercase())
        .unwrap_or_default();
    let state = console.state::<State>();
    let key = (query, state.sort, state.descending, state.version);
    if let Some((cached, rows)) = &state.table
        && *cached == key
    {
        return rows.clone();
    }
    let Some(Ok(flows)) = &state.flows else {
        return Rc::from(Vec::new());
    };
    let query = &key.0;
    let mut rows: Vec<Flow> = flows
        .iter()
        .filter(|flow| {
            query.is_empty()
                || flow.name.to_lowercase().contains(query)
                || flow.id.to_lowercase().contains(query)
                || flow.trigger_label().to_lowercase().contains(query)
        })
        .cloned()
        .collect();
    let (sort, descending) = (state.sort, state.descending);
    rows.sort_by(|a, b| {
        let order = compare(state, a, b, sort);
        if descending { order.reverse() } else { order }
    });
    let rows: Rc<[Flow]> = rows.into();
    state.table = Some((key, rows.clone()));
    rows
}

fn cursor_flow(console: &mut Console, cx: &mut Context<Console>) -> Option<Flow> {
    let row = console.list(LIST, cx).cursor?;
    table(console, cx).get(row).cloned()
}

/// The colour of a trigger or step tone.
pub(super) fn tone_color(tone: Tone, theme: &Theme) -> Hsla {
    match tone {
        Tone::Info => theme.info,
        Tone::Accent => theme.accent,
        Tone::Warning => theme.warning,
        Tone::Success => theme.success,
        Tone::Danger => theme.danger,
        Tone::Series(index) => theme.series(index),
    }
}

fn trigger_color(flow: &Flow, theme: &Theme) -> Hsla {
    flow.trigger_type()
        .and_then(|kind| trigger_kind(&kind))
        .map_or(theme.fg_dim, |kind| tone_color(kind.tone, theme))
}

/// Unix seconds now.
pub(super) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0))
}

/// Unix milliseconds now, for new ids (`flow_<ms>`, `n<ms>`).
pub(super) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0))
}

/// Run `then` with the window, from a reply (which has none).
pub(super) fn with_window(
    console: &mut Console,
    cx: &mut Context<Console>,
    then: impl FnOnce(&mut Console, &mut Window, &mut Context<Console>) + 'static,
) {
    let Some(handle) = console.state::<State>().window else {
        return;
    };
    let entity = cx.entity();
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            entity.update(cx, |console, cx| then(console, window, cx));
        });
    });
}

// ── Actions ──────────────────────────────────────────────────────────────

fn activate_row(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(flow) = table(console, cx).get(row).cloned() else {
        return;
    };
    if toggle {
        toggle_run(console, &flow, cx);
    } else {
        editor::open(console, flow, window, cx);
    }
}

/// Start a stopped flow, stop a running one (`POST …/start`, `…/stop`).
fn toggle_run(console: &mut Console, flow: &Flow, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.pending.values().any(|id| *id == flow.id) {
        return;
    }
    let run = !running(state, flow);
    state.seq += 1;
    let seq = state.seq;
    state.pending.insert(seq, flow.id.clone());
    let (id, name) = (flow.id.clone(), flow.name.clone());
    console.call(
        CallKey::new("data.flows.run").with(seq),
        Call::new(move |api| async move {
            api.run_flow(&id, run).await?;
            anyhow::Ok((id, name, run))
        }),
    );
    cx.notify();
}

fn ask_delete(console: &mut Console, flow: &Flow, window: &mut Window, cx: &mut Context<Console>) {
    let focus = cx.focus_handle();
    let state = console.state::<State>();
    state.confirm = Some((flow.id.clone(), flow.name.clone()));
    state.confirm_focus = Some(focus.clone());
    let overlay = Overlay::modal(DELETE, px(420.0), confirm_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn delete_confirmed(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some((id, name)) = state.confirm.take() else {
        return;
    };
    state.seq += 1;
    let seq = state.seq;
    state.pending.insert(seq, id.clone());
    console.close_overlay(window, cx);
    console.call(
        CallKey::new("data.flows.delete").with(seq),
        Call::new(move |api| async move {
            api.delete_flow(&id).await?;
            anyhow::Ok(name)
        }),
    );
}

fn set_sort(console: &mut Console, sort: Sort, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.sort == sort {
        state.descending = !state.descending;
    } else {
        state.sort = sort;
        state.descending = false;
    }
    console.list(LIST, cx).cursor = None;
    cx.notify();
}

fn open_new(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    forms::open_meta(console, None, window, cx);
}

/// O: open the cursor's flow (or the first) on the canvas; before the list
/// has come, once it comes.
fn open_cursor(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.editor.is_some() {
        return;
    }
    if !matches!(state.flows, Some(Ok(_))) {
        state.open_pending = true;
        state.window = Some(window.window_handle());
        return;
    }
    let flow = match cursor_flow(console, cx) {
        Some(flow) => Some(flow),
        None => table(console, cx).first().cloned(),
    };
    match flow {
        Some(flow) => editor::open(console, flow, window, cx),
        None => console.show_toast("No flow to open yet: N makes one", false, cx),
    }
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "data.flows.list" => {
            let state = console.state::<State>();
            let stale = key.arg != state.list_seq && state.flows.is_some();
            let shown = matches!(state.flows, Some(Ok(_)));
            if !stale {
                match reply.take::<Vec<Flow>>() {
                    Ok(flows) => {
                        let state = console.state::<State>();
                        state.flows = Some(Ok(flows));
                        state.version += 1;
                        if std::mem::take(&mut state.open_pending) {
                            with_window(console, cx, open_cursor);
                        }
                    }
                    // Keep a list already shown; say why it is not fresh.
                    Err(error) if shown => {
                        console.show_toast(format!("Flows: {error}"), true, cx);
                    }
                    Err(error) => {
                        let state = console.state::<State>();
                        state.flows = Some(Err(error));
                        state.version += 1;
                    }
                }
            }
        }
        "data.flows.run" => {
            let id = console.state::<State>().pending.remove(&key.arg);
            match reply.take::<(String, String, bool)>() {
                Ok((id, name, run)) => {
                    let state = console.state::<State>();
                    state.running.insert(id, run);
                    state.version += 1;
                    let verb = if run { "Started" } else { "Stopped" };
                    console.show_toast(format!("{verb} {name}"), false, cx);
                }
                Err(error) => console.show_toast(
                    format!("{}: {error}", id.unwrap_or_else(|| "Flow".into())),
                    true,
                    cx,
                ),
            }
        }
        "data.flows.delete" => {
            let id = console.state::<State>().pending.remove(&key.arg);
            match reply.take::<String>() {
                Ok(name) => {
                    if let Some(id) = id {
                        let state = console.state::<State>();
                        state.running.remove(&id);
                        state.version += 1;
                    }
                    console.show_toast(format!("Deleted {name}"), false, cx);
                    refresh(console);
                }
                Err(error) => console.show_toast(format!("Delete failed: {error}"), true, cx),
            }
        }
        "data.flows.save" => editor::on_saved(console, key, reply, cx),
        _ => forms::on_reply(console, key, reply, cx),
    }
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// What every row of one render shares.
#[derive(Clone, Copy)]
struct RowLook {
    cursor: Option<usize>,
    focused: bool,
    id_column: bool,
    updated_column: bool,
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    if console.state::<State>().editor.is_some() {
        return editor::render(console, window, cx);
    }
    console.state::<State>().window = Some(window.window_handle());
    ensure_search(console, window, cx);
    let theme = theme(cx).clone();
    let rows = table(console, cx);
    let shown = rows.len();
    let state = console.state::<State>();
    let (sort, descending) = (state.sort, state.descending);
    let search_field = state.search.clone();
    let focus_id = state.focus_id.clone();
    let (total, active, failure, loaded) = match &state.flows {
        Some(Ok(flows)) => (
            flows.len(),
            flows.iter().filter(|flow| running(state, flow)).count(),
            None,
            true,
        ),
        Some(Err(error)) => (0, 0, Some(error.clone()), true),
        None => (0, 0, None, false),
    };
    let filtered = search_field
        .as_ref()
        .is_some_and(|field| !field.read(cx).value().trim().is_empty());

    // A new or edited flow: the cursor goes to it once it is listed.
    if let Some(id) = focus_id
        && let Some(row) = rows.iter().position(|flow| flow.id == id)
    {
        console.state::<State>().focus_id = None;
        console.list(LIST, cx).select(row);
    }

    let width = f32::from(window.viewport_size().width - super::sidebar_width());
    let id_column = width >= 980.0;
    let updated_column = width >= 860.0;
    let arrow = if descending { "↓" } else { "↑" };
    let header_cell =
        |label: &'static str, key: Sort, width: Option<f32>, cx: &mut Context<Console>| {
            let active = key == sort;
            div()
                .id(SharedString::from(format!(
                    "data-flows-sort-{}",
                    key.label()
                )))
                .flex()
                .items_center()
                .gap(SPACE_1)
                .map(|this| match width {
                    Some(width) => this.w(px(width)).flex_none(),
                    None => this.flex_1().min_w_0(),
                })
                .text_color(if active { theme.fg } else { theme.fg_dim })
                .cursor_pointer()
                .tab_index(0)
                .hover(|style| style.text_color(theme.fg))
                .on_click(cx.listener(move |console, _, _, cx| set_sort(console, key, cx)))
                .child(label)
                .when(active, |this| this.child(arrow))
                .into_any_element()
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
        .child(header_cell("STATUS", Sort::Status, Some(STATUS), cx))
        .child(header_cell("NAME", Sort::Name, None, cx))
        .when(id_column, |this| {
            this.child(header_cell("ID", Sort::Id, Some(ID), cx))
        })
        .child(header_cell("TRIGGER", Sort::Trigger, Some(TRIGGER), cx))
        .child(header_cell("STEPS", Sort::Steps, Some(STEPS), cx))
        .when(updated_column, |this| {
            this.child(header_cell("UPDATED", Sort::Updated, Some(UPDATED), cx))
        })
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
    let look = RowLook {
        cursor: nav.cursor,
        focused: nav.focus.is_focused(window),
        id_column,
        updated_column,
    };
    let scroll = nav.scroll.clone();

    let new_button = |id: &'static str, cx: &mut Context<Console>| {
        Button::new(id, "New flow")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(|console, _, window, cx| open_new(console, window, cx)))
            .into_any_element()
    };
    let body = if let Some(error) = failure {
        notice(
            format!("Flows could not be loaded: {error} · R to retry"),
            true,
            &theme,
        )
    } else if !loaded {
        notice("Loading flows…", false, &theme)
    } else if shown == 0 {
        empty_state(
            if total == 0 {
                Icon::Layers
            } else {
                Icon::Search
            },
            if total == 0 {
                "No flows configured"
            } else {
                "No flow matches"
            },
            if total == 0 {
                "Create a flow to transform data, compute metrics, or send alerts."
            } else {
                "Nothing has this text in its name, ID or trigger."
            },
            if total == 0 {
                vec![new_button("data-flows-first", cx)]
            } else {
                Vec::new()
            },
            cx,
        )
        .into_any_element()
    } else {
        let rows = Rc::clone(&rows);
        uniform_list(
            "data-flows",
            shown + tail_rows(shown, ROW_HEIGHT_PX),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    range
                        .map(|row| match rows.get(row) {
                            Some(flow) => flow_row(console, flow, row, look, &theme, cx),
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

    let subtitle = if filtered {
        format!("{shown} of {total} flows match")
    } else {
        format!(
            "{total} flows · {active} running · sorted by {}{arrow} · ↵/O edit · Space start/stop · N new · E settings · T test · H runs · ⌫ delete · S sort",
            sort.label()
        )
    };
    let controls = vec![
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .w(px(220.0))
            .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
            .children(search_field.map(|field| div().flex_1().child(field)))
            .into_any_element(),
        IconButton::new("data-flows-refresh", Icon::RefreshCw)
            .tooltip("Refresh  R")
            .on_click(cx.listener(|console, _, _, cx| {
                refresh(console);
                cx.notify();
            }))
            .into_any_element(),
        new_button("data-flows-new", cx),
    ];

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("Flows", subtitle, controls, cx))
        .child(
            div()
                .key_context(CONTEXT)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &EngFlowsOpen, window, cx| {
                    open_cursor(console, window, cx);
                }))
                .on_action(cx.listener(|console, _: &EngFlowsNew, window, cx| {
                    open_new(console, window, cx);
                }))
                .on_action(cx.listener(|console, _: &EngFlowsEdit, window, cx| {
                    if let Some(flow) = cursor_flow(console, cx) {
                        forms::open_meta(console, Some(flow), window, cx);
                    }
                }))
                .on_action(cx.listener(|console, _: &EngFlowsDelete, window, cx| {
                    if let Some(flow) = cursor_flow(console, cx) {
                        ask_delete(console, &flow, window, cx);
                    }
                }))
                .on_action(cx.listener(|console, _: &EngFlowsRuns, window, cx| {
                    if let Some(flow) = cursor_flow(console, cx) {
                        forms::open_runs(console, &flow, window, cx);
                    }
                }))
                .on_action(cx.listener(|console, _: &EngFlowsTest, window, cx| {
                    if let Some(flow) = cursor_flow(console, cx) {
                        forms::open_test(console, &flow, false, window, cx);
                    }
                }))
                .on_action(cx.listener(|console, _: &EngFlowsRefresh, _, cx| {
                    refresh(console);
                    cx.notify();
                }))
                .on_action(cx.listener(|console, _: &EngFlowsSortNext, _, cx| {
                    let next = console.state::<State>().sort.next();
                    set_sort(console, next, cx);
                }))
                .on_action(cx.listener(|console, _: &EngFlowsSortReverse, _, cx| {
                    let sort = console.state::<State>().sort;
                    set_sort(console, sort, cx);
                }))
                .child(header)
                .child(list.flex_1().min_h_0().child(body)),
        )
        .into_any_element()
}

fn flow_row(
    console: &Console,
    flow: &Flow,
    row: usize,
    look: RowLook,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let Some(state) = console.state_ref::<State>() else {
        return tail_row(ROW_HEIGHT_PX);
    };
    let current = look.cursor == Some(row);
    let ring = look.focused && current;
    let is_running = running(state, flow);
    let busy = state.pending.values().any(|id| *id == flow.id);
    let drift = is_running != flow.enabled;
    let hover = theme.hover;
    let steps = flow
        .node_count()
        .map_or_else(|| "?".to_owned(), |count| count.to_string());
    let trigger_tint = trigger_color(flow, theme);
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
    let (open, run, edit, settings, runs, test, delete) = (
        flow.clone(),
        flow.clone(),
        flow.clone(),
        flow.clone(),
        flow.clone(),
        flow.clone(),
        flow.clone(),
    );

    div()
        .id(("data-flow", row))
        .group("data-flow-row")
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
        .when(current, |this| this.bg(theme.selected))
        .when(!current, |this| this.hover(move |style| style.bg(hover)))
        .on_click(
            cx.listener(move |console, event: &gpui::ClickEvent, window, cx| {
                console.list(LIST, cx).select(row);
                if event.click_count() >= 2 {
                    editor::open(console, open.clone(), window, cx);
                } else {
                    let focus = console.list(LIST, cx).focus.clone();
                    window.focus(&focus);
                }
                cx.notify();
            }),
        )
        .when(ring, |this| this.child(focus_ring(theme)))
        .child(
            div()
                .w(px(STATUS))
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .min_w_0()
                .child(
                    div()
                        .flex_none()
                        .size(px(8.0))
                        .rounded(px(4.0))
                        .bg(if is_running {
                            theme.success
                        } else {
                            theme.fg_dim
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(FONT_SMALL)
                        .text_color(if is_running { theme.fg } else { theme.fg_muted })
                        .child(if is_running { "Running" } else { "Stopped" }),
                )
                .when(drift, |this| {
                    this.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child(if flow.enabled {
                                "· on at restart"
                            } else {
                                "· off at restart"
                            }),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .child(flow.name.clone()),
        )
        .when(look.id_column, |this| {
            this.child(mono(flow.id.clone(), ID, theme.fg_muted))
        })
        .child(
            div()
                .w(px(TRIGGER))
                .flex_none()
                .flex()
                .child(pill(flow.trigger_label(), trigger_tint)),
        )
        .child(mono(steps, STEPS, theme.fg_muted).text_right())
        .when(look.updated_column, |this| {
            this.child(mono(
                if flow.updated_at > 0 {
                    local_time(flow.updated_at, &console.zone)
                } else {
                    "—".to_owned()
                },
                UPDATED,
                theme.fg_dim,
            ))
        })
        .child(
            div()
                .w(px(ACTIONS))
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .gap(SPACE_1)
                .child(
                    Button::new(
                        ("data-flow-run", row),
                        if is_running { "Stop" } else { "Start" },
                    )
                    .compact()
                    .ghost()
                    .icon(if is_running { Icon::Square } else { Icon::Play })
                    .disabled(busy)
                    .tooltip(if is_running {
                        "Stop the flow in the engine  Space"
                    } else {
                        "Start the flow in the engine  Space"
                    })
                    .on_click(cx.listener(move |console, _, _, cx| {
                        toggle_run(console, &run, cx);
                    })),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_1)
                        .opacity(if current { 1.0 } else { 0.0 })
                        .group_hover("data-flow-row", |style| style.opacity(1.0))
                        .child(
                            IconButton::new(("data-flow-edit", row), Icon::Waypoints)
                                .tooltip("Open the editor  ↵")
                                .on_click(cx.listener(move |console, _, window, cx| {
                                    editor::open(console, edit.clone(), window, cx);
                                })),
                        )
                        .child(
                            IconButton::new(("data-flow-settings", row), Icon::Pencil)
                                .tooltip("Rename, enable  E")
                                .on_click(cx.listener(move |console, _, window, cx| {
                                    forms::open_meta(console, Some(settings.clone()), window, cx);
                                })),
                        )
                        .child(
                            IconButton::new(("data-flow-runs", row), Icon::History)
                                .tooltip("Runs  H")
                                .on_click(cx.listener(move |console, _, window, cx| {
                                    forms::open_runs(console, &runs, window, cx);
                                })),
                        )
                        .child(
                            IconButton::new(("data-flow-test", row), Icon::Send)
                                .tooltip("Test with a payload  T")
                                .on_click(cx.listener(move |console, _, window, cx| {
                                    forms::open_test(console, &test, false, window, cx);
                                })),
                        )
                        .child(
                            IconButton::new(("data-flow-delete", row), Icon::Trash)
                                .tooltip("Delete  ⌫")
                                .color(theme.danger)
                                .on_click(cx.listener(move |console, _, window, cx| {
                                    ask_delete(console, &delete, window, cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
}

/// The delete confirmation: Enter or the button deletes, Esc keeps.
fn confirm_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let Some((_, name)) = state.confirm.clone() else {
        return div().into_any_element();
    };
    let focus = state.confirm_focus.clone();
    div()
        .when_some(focus, |this, focus| this.track_focus(&focus))
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
            // Enter only while the dialog itself has focus (not a button).
            let own = console
                .state::<State>()
                .confirm_focus
                .as_ref()
                .is_some_and(|focus| focus.is_focused(window));
            if own && event.keystroke.key == "enter" {
                delete_confirmed(console, window, cx);
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
                .child(vt_ui::icon(Icon::Trash, ICON_LG, theme.danger))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Delete {name}?")),
                ),
        )
        .child(
            div().text_color(theme.fg_muted).child(
                "The engine stops running it and its steps are gone. This cannot be undone.",
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
                        .child("delete")
                        .child(kbd("Esc", cx))
                        .child("keep"),
                )
                .child(
                    Button::new("data-flow-delete-cancel", "Keep")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.state::<State>().confirm = None;
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new("data-flow-delete-confirm", "Delete")
                        .danger()
                        .icon(Icon::Trash)
                        .on_click(cx.listener(|console, _, window, cx| {
                            delete_confirmed(console, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

/// How two flows order under `sort` (ties by name, then id).
fn compare(state: &State, a: &Flow, b: &Flow, sort: Sort) -> Ordering {
    match sort {
        Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        Sort::Id => a.id.cmp(&b.id),
        Sort::Status => running(state, b).cmp(&running(state, a)),
        Sort::Trigger => a.trigger_label().cmp(&b.trigger_label()),
        Sort::Steps => a.node_count().cmp(&b.node_count()),
        Sort::Updated => b.updated_at.cmp(&a.updated_at),
    }
    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    .then_with(|| a.id.cmp(&b.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_cycle_through_every_column() {
        let mut sort = Sort::default();
        let mut seen = Vec::new();
        for _ in 0..Sort::ALL.len() {
            seen.push(sort);
            sort = sort.next();
        }
        assert_eq!(sort, Sort::Name);
        assert_eq!(seen, Sort::ALL);
    }

    #[test]
    fn running_follows_this_consoles_start_and_stop() {
        let flow = Flow::blank("f", "F", true, trigger_kind("timer").unwrap(), 1);
        let mut state = State::default();
        assert!(
            running(&state, &flow),
            "the stored flag until started or stopped"
        );
        state.running.insert("f".into(), false);
        assert!(!running(&state, &flow));
        let newer = Flow {
            updated_at: 5,
            ..flow.clone()
        };
        assert_eq!(
            compare(&state, &newer, &flow, Sort::Updated),
            Ordering::Less,
            "newest first"
        );
        // Stopped here: a running flow sorts first.
        let other = Flow::blank("g", "G", true, trigger_kind("timer").unwrap(), 1);
        assert_eq!(compare(&state, &other, &flow, Sort::Status), Ordering::Less);
    }
}

#[cfg(test)]
mod table_tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::console::testing::Harness;

    #[gpui::test]
    fn the_table_is_sorted_once_per_change_not_per_render(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, _, cx| {
            let timer = trigger_kind("timer").unwrap();
            let state = console.state::<State>();
            state.sort = Sort::Status;
            state.flows = Some(Ok(vec![
                Flow::blank("a", "A", true, timer, 1),
                Flow::blank("b", "B", false, timer, 1),
            ]));
            state.version += 1;
            let first = table(console, cx);
            let again = table(console, cx);
            assert!(
                Rc::ptr_eq(&first, &again),
                "an unchanged list is not rebuilt"
            );
            assert_eq!(first[0].id, "a", "running first");
            on_reply(
                console,
                CallKey::new("data.flows.run").with(1),
                Reply::ok(("b".to_owned(), "B".to_owned(), true)),
                cx,
            );
            on_reply(
                console,
                CallKey::new("data.flows.run").with(2),
                Reply::ok(("a".to_owned(), "A".to_owned(), false)),
                cx,
            );
            let after = table(console, cx);
            assert!(!Rc::ptr_eq(&first, &after), "a start or stop rebuilds it");
            assert_eq!(after[0].id, "b");
        });
    }
}
