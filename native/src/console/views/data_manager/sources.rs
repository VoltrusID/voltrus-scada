//! Data Manager › Sources: every data source in a virtualised table with
//! search and sort, and the actions of the web client's `SensorManager.tsx`:
//! add and edit (the form in [`super::source_form`]), duplicate, delete
//! after a confirmation, reorder (drag a row, or ⌥↑ ⌥↓) and per-tag
//! shortcuts. The rows are the console's own tag list, so after a change
//! every page shows what the server now has.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, App, Context, ElementId, Entity, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, SharedString, Subscription, Window, div, px, uniform_list,
};
use serde::de::IgnoredAny;
use serde_json::json;
use voltrus_scada_native::api::{
    Created, DataSource, Method, ModbusDevice, ModbusRegister, TagShortcuts,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, empty_state, kbd, pill};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::source_model::{self as model, Category, SortKey};
use super::{choose, sidebar_width, source_form};
use crate::console::Console;
use crate::console::keys::{
    DATA_SOURCES, DataDeleteSource, DataDuplicateSource, DataMoveSourceDown, DataMoveSourceUp,
    DataReverseSort, DataSortNext,
};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::pages::{Page, Role};
use crate::console::views::{
    ListKeys, focus_ring, keyboard_list, page_header, rows_on_screen, tail_row, tail_rows,
};

/// The table's keyboard list.
pub(crate) const LIST: &str = "data.sources";
const ROW_HEIGHT_PX: f32 = 32.0;
const DELETE: &str = "data-source-delete";

const ORDER: f32 = 40.0;
const KEY: f32 = 140.0;
const UNIT: f32 = 52.0;
const RANGE: f32 = 104.0;
const LIMIT: f32 = 64.0;
const SOURCE: f32 = 124.0;
const CHART: f32 = 78.0;
const HOTKEY: f32 = 74.0;
const ACTIONS: f32 = 120.0;
/// The name column never gets narrower than this; columns drop first.
const NAME_MIN: f32 = 120.0;

#[derive(Default)]
pub(crate) struct State {
    pub search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    pub sort: SortKey,
    pub descending: bool,
    /// The user's per-tag shortcuts by data source id, once fetched.
    pub shortcuts: Option<HashMap<i64, String>>,
    shortcuts_asked: bool,
    /// A reorder on its way to the server: the store's order when it was
    /// asked for, and the order asked for, shown until the list refreshes.
    pending_order: Option<(Vec<i64>, Vec<i64>)>,
    /// Put the cursor on this source once the list has it.
    pub focus_id: Option<i64>,
    /// The source the delete confirmation is about.
    confirm: Option<(i64, String)>,
    confirm_focus: Option<FocusHandle>,
    /// The window, for closing the form once the server has saved it.
    pub window: Option<AnyWindowHandle>,
    pub form: Option<source_form::Form>,
    pub devices: Option<Result<Vec<ModbusDevice>, String>>,
    pub devices_asked: bool,
    /// Registers by Modbus device id.
    pub registers: HashMap<String, Result<Vec<ModbusRegister>, String>>,
    pub registers_asked: Option<String>,
}

/// The section came on screen: the shortcuts are the only thing the tag
/// list does not carry.
pub fn on_open(console: &mut Console, _: &mut Context<Console>) {
    fetch_shortcuts(console, false);
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console.state_ref::<State>()?.search.clone()
}

fn fetch_shortcuts(console: &mut Console, again: bool) {
    // A stress run has no server to ask.
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    if state.shortcuts_asked && !again {
        return;
    }
    state.shortcuts_asked = true;
    console.call(
        CallKey::new("data.sources.shortcuts"),
        Call::get::<TagShortcuts>("/api/v1/shortcuts"),
    );
}

/// After a shortcut was saved: fetch the map again for the table, and
/// have the shell rebind every shortcut so the new combo opens the tag.
pub(crate) fn shortcuts_saved(console: &mut Console) {
    fetch_shortcuts(console, true);
    crate::console::shortcuts::reload(console);
}

fn ensure_search(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().search.is_some() {
        return;
    }
    let field = cx.new(|cx| TextInput::new(cx).placeholder("Search sources  /"));
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

/// The table's rows (positions in the store) and the ids of the whole list
/// in the order the operator sees it, for a reorder.
fn table(console: &mut Console, cx: &App) -> (Vec<usize>, Vec<i64>) {
    let state = console.state::<State>();
    let query = state
        .search
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    let (sort, descending) = (state.sort, state.descending);
    let pending = state.pending_order.clone();
    let stored: Vec<i64> = console.store.tags.iter().map(|tag| tag.info.id).collect();
    // The list refreshed since the reorder was asked for: it is the truth.
    let pending = pending.filter(|(before, _)| *before == stored);
    if pending.is_none() {
        console.state::<State>().pending_order = None;
    }
    let sources: Vec<&DataSource> = console.store.tags.iter().map(|tag| &tag.info).collect();
    let wanted = pending.map(|(_, wanted)| wanted);
    let rows = model::rows(&sources, &query, sort, descending, wanted.as_deref());
    (rows, wanted.unwrap_or(stored))
}

fn row_id(console: &mut Console, row: usize, cx: &App) -> Option<i64> {
    let (rows, _) = table(console, cx);
    rows.get(row)
        .map(|index| console.store.tags[*index].info.id)
}

fn cursor_id(console: &mut Console, cx: &mut Context<Console>) -> Option<i64> {
    let row = console.list(LIST, cx).cursor?;
    row_id(console, row, cx)
}

fn source(console: &Console, id: i64) -> Option<DataSource> {
    console
        .store
        .position(id)
        .map(|index| console.store.tags[index].info.clone())
}

fn reorderable(console: &mut Console) -> bool {
    let state = console.state::<State>();
    state.sort == SortKey::Order && !state.descending
}

/// The colour a source category reads in.
pub(crate) fn category_color(source_type: &str, theme: &Theme) -> Hsla {
    match model::source_type(source_type).map(|kind| kind.category) {
        Some(Category::Real) => theme.success,
        Some(Category::Simulated) => theme.warning,
        Some(Category::Api) => theme.info,
        Some(Category::Static) | None => theme.fg_dim,
    }
}

/// A source's own colour (its trend line), from its `#rrggbb`.
pub(crate) fn tag_color(color: &str, theme: &Theme) -> Hsla {
    model::parse_hex(color).map_or(theme.fg_dim, |hex| gpui::rgb(hex).into())
}

fn element_id(name: &'static str, id: i64) -> ElementId {
    ElementId::NamedInteger(name.into(), u64::try_from(id).unwrap_or(0))
}

// ── Actions ──────────────────────────────────────────────────────────────

/// "New data source" (N, the palette): the Sources section with an empty
/// form.
pub fn new_source(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !console.allows(Role::Operator) {
        console.show_toast("Adding data sources needs the operator role", true, cx);
        return;
    }
    if console.page != Page::DataManager {
        console.show(Page::DataManager, window, cx);
    }
    choose(console, "sources", cx);
    source_form::open(console, None, window, cx);
}

fn open_edit(console: &mut Console, id: i64, window: &mut Window, cx: &mut Context<Console>) {
    if let Some(source) = source(console, id) {
        source_form::open(console, Some(source), window, cx);
    }
}

fn activate_row(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(id) = row_id(console, row, cx) {
        open_edit(console, id, window, cx);
    }
}

fn duplicate(console: &mut Console, id: i64, cx: &mut Context<Console>) {
    let Some(source) = source(console, id) else {
        return;
    };
    let body = model::duplicate_body(&source);
    let delays = model::duplicate_delays(&source);
    let key = format!("{}_copy", source.key);
    console.call(
        CallKey::new("data.sources.duplicate"),
        Call::new(move |api| async move {
            let created: Created = api
                .send_json(Method::POST, "/api/v1/data-sources", &body)
                .await?;
            if let Some(delays) = delays {
                api.send_json::<_, IgnoredAny>(
                    Method::PUT,
                    &format!("/api/v1/data-sources/{}", created.id),
                    &delays,
                )
                .await?;
            }
            anyhow::Ok((created.id, key))
        }),
    );
    cx.notify();
}

pub(crate) fn ask_delete(
    console: &mut Console,
    id: i64,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(source) = source(console, id) else {
        return;
    };
    let focus = cx.focus_handle();
    let state = console.state::<State>();
    state.confirm = Some((id, source.name.clone()));
    state.confirm_focus = Some(focus.clone());
    let overlay = Overlay::modal(DELETE, px(420.0), confirm_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn delete_confirmed(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some((id, name)) = console.state::<State>().confirm.take() else {
        return;
    };
    console.close_overlay(window, cx);
    console.call(
        CallKey::new("data.sources.delete").with(id),
        Call::new(move |api| async move {
            api.send_empty::<IgnoredAny>(Method::DELETE, &format!("/api/v1/data-sources/{id}"))
                .await?;
            anyhow::Ok(name)
        }),
    );
}

/// Move source `id` to table row `to` (a drop, or ⌥↑ ⌥↓); the table shows
/// the new order at once and the server is told.
fn move_source(console: &mut Console, id: i64, to: usize, cx: &mut Context<Console>) {
    if !reorderable(console) {
        console.show_toast("Sort by order to rearrange the sources (S)", false, cx);
        return;
    }
    let (rows, order) = table(console, cx);
    let visible: Vec<i64> = rows
        .iter()
        .map(|index| console.store.tags[*index].info.id)
        .collect();
    let Some(from) = visible.iter().position(|other| *other == id) else {
        return;
    };
    let Some(wanted) = model::reordered(&order, &visible, from, to) else {
        return;
    };
    let stored: Vec<i64> = console.store.tags.iter().map(|tag| tag.info.id).collect();
    console.state::<State>().pending_order = Some((stored, wanted.clone()));
    console.list(LIST, cx).select(to);
    console.call(
        CallKey::new("data.sources.reorder"),
        Call::send::<IgnoredAny>(
            Method::PATCH,
            "/api/v1/data-sources/reorder",
            json!({ "order": wanted }),
        ),
    );
    cx.notify();
}

fn move_cursor(console: &mut Console, up: bool, cx: &mut Context<Console>) {
    let Some(row) = console.list(LIST, cx).cursor else {
        return;
    };
    let to = if up {
        row.checked_sub(1)
    } else {
        Some(row + 1).filter(|to| *to < console.list(LIST, cx).len)
    };
    if let (Some(to), Some(id)) = (to, row_id(console, row, cx)) {
        move_source(console, id, to, cx);
    }
}

fn set_sort(console: &mut Console, key: SortKey, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.sort == key {
        state.descending = !state.descending;
    } else {
        state.sort = key;
        state.descending = false;
    }
    console.list(LIST, cx).cursor = None;
    cx.notify();
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "data.sources.shortcuts" => match reply.take::<TagShortcuts>() {
            Ok(map) => console.state::<State>().shortcuts = Some(map.by_id()),
            Err(error) => console.show_toast(format!("Shortcuts: {error}"), true, cx),
        },
        "data.sources.reorder" => match reply.take::<IgnoredAny>() {
            Ok(_) => console.refresh_tags(),
            Err(error) => {
                console.state::<State>().pending_order = None;
                console.show_toast(format!("Reorder failed: {error}"), true, cx);
            }
        },
        "data.sources.duplicate" => match reply.take::<(i64, String)>() {
            Ok((id, key)) => {
                console.state::<State>().focus_id = Some(id);
                console.refresh_tags();
                console.show_toast(format!("Duplicated as {key}"), false, cx);
            }
            Err(error) => console.show_toast(format!("Duplicate failed: {error}"), true, cx),
        },
        "data.sources.delete" => match reply.take::<String>() {
            Ok(name) => {
                console.refresh_tags();
                console.show_toast(format!("Deleted {name}"), false, cx);
            }
            Err(error) => console.show_toast(format!("Delete failed: {error}"), true, cx),
        },
        _ => source_form::on_reply(console, key, reply, cx),
    }
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// Which columns fit, and how wide the name is: text only ellipsizes in a
/// known width.
#[derive(Clone, Copy)]
struct Columns {
    key: bool,
    unit: bool,
    range: bool,
    limits: bool,
    chart: bool,
    hotkey: bool,
    name: f32,
}

impl Columns {
    fn fit(width: f32) -> Self {
        let mut columns = Self {
            key: true,
            unit: true,
            range: true,
            limits: true,
            chart: true,
            hotkey: true,
            name: 0.0,
        };
        let gap = f32::from(SPACE_5);
        let padding = f32::from(SPACE_8) * 2.0 + 8.0;
        let name = |columns: &Self| {
            let mut fixed = ORDER + SOURCE + ACTIONS;
            let mut count = 4.0;
            for (shown, width) in [
                (columns.key, KEY),
                (columns.unit, UNIT),
                (columns.range, RANGE),
                (columns.limits, LIMIT * 2.0 + gap),
                (columns.chart, CHART),
                (columns.hotkey, HOTKEY),
            ] {
                if shown {
                    fixed += width;
                    count += 1.0;
                }
            }
            width - fixed - gap * (count - 1.0) - padding
        };
        // The least useful go first.
        let drops: [fn(&mut Self); 5] = [
            |c| c.chart = false,
            |c| c.range = false,
            |c| c.hotkey = false,
            |c| c.unit = false,
            |c| c.limits = false,
        ];
        for drop in drops {
            if name(&columns) >= NAME_MIN {
                break;
            }
            drop(&mut columns);
        }
        columns.name = name(&columns).max(80.0);
        columns
    }
}

/// A row being dragged to a new place, and how it looks on the way.
#[derive(Clone)]
struct Dragged {
    id: i64,
    name: SharedString,
    color: Hsla,
}

impl Render for Dragged {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        div()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .px(SPACE_6)
            .h(px(ROW_HEIGHT_PX))
            .rounded(RADIUS_MD)
            .bg(theme.elevated)
            .border_1()
            .border_color(theme.accent)
            .shadow(theme.shadow(Elevation::Popover))
            .text_size(FONT_BODY)
            .text_color(theme.fg)
            .child(div().size(px(8.0)).rounded(px(4.0)).bg(self.color))
            .child(self.name.clone())
    }
}

/// What every row of one render shares.
#[derive(Clone, Copy)]
struct RowLook {
    columns: Columns,
    cursor: Option<usize>,
    focused: bool,
    reorderable: bool,
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    ensure_search(console, window, cx);
    let theme = theme(cx).clone();
    let (rows, _) = table(console, cx);
    let rows = Rc::new(rows);
    let shown = rows.len();
    let total = console.store.tags.len();
    let can_reorder = reorderable(console);
    let state = console.state::<State>();
    let (sort, descending) = (state.sort, state.descending);
    let search_field = state.search.clone();
    let focus_id = state.focus_id;
    let filtered = search_field
        .as_ref()
        .is_some_and(|field| !field.read(cx).value().trim().is_empty());

    // A duplicate or a new source: the cursor goes to it once it is listed.
    if let Some(id) = focus_id
        && let Some(row) = rows
            .iter()
            .position(|index| console.store.tags[*index].info.id == id)
    {
        console.state::<State>().focus_id = None;
        console.list(LIST, cx).select(row);
    }

    let width = f32::from(window.viewport_size().width - sidebar_width());
    let columns = Columns::fit(width);

    let arrow = if descending { "↓" } else { "↑" };
    let header_cell = |label: &'static str,
                       key: Option<SortKey>,
                       width: Option<f32>,
                       right: bool,
                       cx: &mut Context<Console>| {
        let active = key.is_some_and(|key| key == sort);
        let cell = div()
            .flex()
            .items_center()
            .gap(SPACE_1)
            .when(right, |this| this.justify_end())
            .map(|this| match width {
                Some(width) => this.w(px(width)).flex_none(),
                None => this.flex_1().min_w_0(),
            })
            .text_color(if active { theme.fg } else { theme.fg_dim })
            .child(label)
            .when(active, |this| this.child(arrow));
        match key {
            Some(key) => cell
                .id(SharedString::from(format!("data-sort-{}", key.label())))
                .cursor_pointer()
                .tab_index(0)
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
        .child(header_cell(
            "#",
            Some(SortKey::Order),
            Some(ORDER),
            false,
            cx,
        ))
        .child(header_cell("NAME", Some(SortKey::Name), None, false, cx))
        .when(columns.key, |this| {
            this.child(header_cell("KEY", Some(SortKey::Key), Some(KEY), false, cx))
        })
        .when(columns.unit, |this| {
            this.child(header_cell(
                "UNIT",
                Some(SortKey::Unit),
                Some(UNIT),
                false,
                cx,
            ))
        })
        .when(columns.range, |this| {
            this.child(header_cell("RANGE", None, Some(RANGE), true, cx))
        })
        .when(columns.limits, |this| {
            this.child(header_cell("WARN", None, Some(LIMIT), true, cx))
                .child(header_cell("CRIT", None, Some(LIMIT), true, cx))
        })
        .child(header_cell(
            "SOURCE",
            Some(SortKey::Source),
            Some(SOURCE),
            false,
            cx,
        ))
        .when(columns.chart, |this| {
            this.child(header_cell(
                "CHART",
                Some(SortKey::Chart),
                Some(CHART),
                false,
                cx,
            ))
        })
        .when(columns.hotkey, |this| {
            this.child(header_cell("HOTKEY", None, Some(HOTKEY), false, cx))
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
        columns,
        cursor: nav.cursor,
        focused: nav.focus.is_focused(window),
        reorderable: can_reorder,
    };
    let scroll = nav.scroll.clone();

    let body = if shown == 0 {
        empty_state(
            if total == 0 {
                Icon::Database
            } else {
                Icon::Search
            },
            if total == 0 {
                "No sources yet"
            } else {
                "No source matches"
            },
            if total == 0 {
                "A source is a tag the server reads, simulates or polls. Add one with N."
            } else {
                "Nothing has this text in its name, key, unit, source type or device."
            },
            if total == 0 {
                vec![
                    Button::new("data-first-source", "New source")
                        .primary()
                        .icon(Icon::Plus)
                        .on_click(cx.listener(|console, _, window, cx| {
                            new_source(console, window, cx);
                        }))
                        .into_any_element(),
                ]
            } else {
                Vec::new()
            },
            cx,
        )
        .into_any_element()
    } else {
        uniform_list(
            "data-sources",
            shown + tail_rows(shown, ROW_HEIGHT_PX),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    range
                        .map(|row| match rows.get(row).copied() {
                            Some(index) => source_row(console, index, row, look, &theme, cx),
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
        format!("{shown} of {total} sources match")
    } else {
        format!(
            "{total} sources · sorted by {}{arrow} · N new · ↵ edit · D duplicate · ⌫ delete · ⌥↑↓ move · S sort",
            sort.label()
        )
    };
    let controls = vec![
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .w(px(240.0))
            .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
            .children(search_field.map(|field| div().flex_1().child(field)))
            .into_any_element(),
        Button::new("data-new-source", "New source")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(|console, _, window, cx| new_source(console, window, cx)))
            .into_any_element(),
    ];

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("Sources", subtitle, controls, cx))
        .child(
            div()
                .key_context(DATA_SOURCES)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &DataDuplicateSource, _, cx| {
                    if let Some(id) = cursor_id(console, cx) {
                        duplicate(console, id, cx);
                    }
                }))
                .on_action(cx.listener(|console, _: &DataDeleteSource, window, cx| {
                    if let Some(id) = cursor_id(console, cx) {
                        ask_delete(console, id, window, cx);
                    }
                }))
                .on_action(cx.listener(|console, _: &DataMoveSourceUp, _, cx| {
                    move_cursor(console, true, cx);
                }))
                .on_action(cx.listener(|console, _: &DataMoveSourceDown, _, cx| {
                    move_cursor(console, false, cx);
                }))
                .on_action(cx.listener(|console, _: &DataSortNext, _, cx| {
                    let next = console.state::<State>().sort.next();
                    set_sort(console, next, cx);
                }))
                .on_action(cx.listener(|console, _: &DataReverseSort, _, cx| {
                    let sort = console.state::<State>().sort;
                    set_sort(console, sort, cx);
                }))
                .child(header)
                .child(list.flex_1().min_h_0().child(body)),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn source_row(
    console: &Console,
    index: usize,
    row: usize,
    look: RowLook,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let source = &console.store.tags[index].info;
    let id = source.id;
    let columns = look.columns;
    let ring = look.focused && look.cursor == Some(row);
    let current = look.cursor == Some(row);
    let hover = theme.hover;
    let drop_target = theme.accent_soft;
    let color = tag_color(&source.color, theme);
    let category = model::source_type(&source.source_type).map(|kind| kind.category);
    let kind_color = category_color(&source.source_type, theme);
    let shortcut = console
        .state_ref::<State>()
        .and_then(|state| state.shortcuts.as_ref())
        .and_then(|shortcuts| shortcuts.get(&id))
        .map(|combo| model::display_combo(combo.as_str()));
    let mono = |text: String, width: f32, color: Hsla| {
        div()
            .w(px(width))
            .flex_none()
            .truncate()
            .text_right()
            .font_family(MONO)
            .text_size(FONT_SMALL)
            .text_color(color)
            .child(text)
    };
    // Configured values as typed (150000, not the chart's 1.500e5).
    let number = model::format_number;
    let limit = |value: f64| {
        if value > 0.0 {
            number(value)
        } else {
            "—".to_owned()
        }
    };
    let dragged = Dragged {
        id,
        name: source.name.clone().into(),
        color,
    };
    let position = console.store.position(id).map_or(0, |at| at + 1);

    div()
        .id(element_id("data-source", id))
        .group("data-source-row")
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
                    open_edit(console, id, window, cx);
                } else {
                    let focus = console.list(LIST, cx).focus.clone();
                    window.focus(&focus);
                }
                cx.notify();
            }),
        )
        .when(look.reorderable, |this| {
            this.on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                .drag_over::<Dragged>(move |style, _, _, _| style.bg(drop_target))
                .on_drop(cx.listener(move |console, dragged: &Dragged, _, cx| {
                    move_source(console, dragged.id, row, cx);
                }))
        })
        .when(ring, |this| this.child(focus_ring(theme)))
        .child(
            div()
                .w(px(ORDER))
                .flex_none()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(position.to_string()),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(SPACE_4)
                .child(div().flex_none().size(px(8.0)).rounded(px(4.0)).bg(color))
                .child(
                    div()
                        .w(px(columns.name - 16.0))
                        .truncate()
                        .child(source.name.clone()),
                ),
        )
        .when(columns.key, |this| {
            this.child(
                div()
                    .w(px(KEY))
                    .flex_none()
                    .truncate()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(source.key.clone()),
            )
        })
        .when(columns.unit, |this| {
            this.child(
                div()
                    .w(px(UNIT))
                    .flex_none()
                    .truncate()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(if source.unit.is_empty() {
                        "—".to_owned()
                    } else {
                        source.unit.clone()
                    }),
            )
        })
        .when(columns.range, |this| {
            this.child(mono(
                format!("{} – {}", number(source.min_val), number(source.max_val)),
                RANGE,
                theme.fg_muted,
            ))
        })
        .when(columns.limits, |this| {
            this.child(mono(limit(source.warn_val), LIMIT, theme.warning))
                .child(mono(limit(source.crit_val), LIMIT, theme.danger))
        })
        .child(
            div()
                .w(px(SOURCE))
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .min_w_0()
                .child(pill(category.map_or("?", Category::tag), kind_color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(model::type_label(&source.source_type).to_owned()),
                ),
        )
        .when(columns.chart, |this| {
            this.child(
                div()
                    .w(px(CHART))
                    .flex_none()
                    .truncate()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(source.chart_type.clone()),
            )
        })
        .when(columns.hotkey, |this| {
            this.child(div().w(px(HOTKEY)).flex_none().flex().map(|cell| {
                match shortcut {
                    Some(keys) => cell.child(kbd(keys, cx)),
                    None => cell
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child("—"),
                }
            }))
        })
        .child(
            div()
                .w(px(ACTIONS))
                .flex_none()
                .flex()
                .justify_end()
                .gap(SPACE_1)
                .opacity(if current { 1.0 } else { 0.0 })
                .group_hover("data-source-row", |style| style.opacity(1.0))
                .child(
                    IconButton::new(element_id("data-edit", id), Icon::Pencil)
                        .tooltip("Edit  ↵")
                        .on_click(cx.listener(move |console, _, window, cx| {
                            open_edit(console, id, window, cx);
                        })),
                )
                .child(
                    IconButton::new(element_id("data-duplicate", id), Icon::Copy)
                        .tooltip("Duplicate  D")
                        .on_click(cx.listener(move |console, _, _, cx| {
                            duplicate(console, id, cx);
                        })),
                )
                .child(
                    IconButton::new(element_id("data-up", id), Icon::ChevronUp)
                        .tooltip("Move up  ⌥↑")
                        .disabled(!look.reorderable || row == 0)
                        .on_click(cx.listener(move |console, _, _, cx| {
                            if let Some(to) = row.checked_sub(1) {
                                move_source(console, id, to, cx);
                            }
                        })),
                )
                .child(
                    IconButton::new(element_id("data-down", id), Icon::ChevronDown)
                        .tooltip("Move down  ⌥↓")
                        .disabled(!look.reorderable)
                        .on_click(cx.listener(move |console, _, _, cx| {
                            move_source(console, id, row + 1, cx);
                        })),
                )
                .child(
                    IconButton::new(element_id("data-delete", id), Icon::Trash)
                        .tooltip("Delete  ⌫")
                        .color(theme.danger)
                        .on_click(cx.listener(move |console, _, window, cx| {
                            ask_delete(console, id, window, cx);
                        })),
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
            if event.keystroke.key == "enter" {
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
        .child(div().text_color(theme.fg_muted).child(
            "The server stops reading it and it leaves every screen. This cannot be undone.",
        ))
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
                    Button::new("data-delete-cancel", "Keep")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.state::<State>().confirm = None;
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new("data-delete-confirm", "Delete")
                        .danger()
                        .icon(Icon::Trash)
                        .on_click(cx.listener(|console, _, window, cx| {
                            delete_confirmed(console, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_drop_the_least_useful_first_and_keep_the_name_readable() {
        let wide = Columns::fit(1600.0);
        assert!(wide.key && wide.unit && wide.range && wide.limits && wide.chart && wide.hotkey);
        assert!(wide.name >= NAME_MIN);
        // The narrowest window (1024) less the sidebar.
        let narrow = Columns::fit(1024.0 - 208.0);
        assert!(!narrow.chart && !narrow.range, "chart and range go first");
        assert!(narrow.key && narrow.limits, "the key and the limits stay");
        assert!(!narrow.hotkey && !narrow.unit);
        assert!(narrow.name >= NAME_MIN);
    }
}
