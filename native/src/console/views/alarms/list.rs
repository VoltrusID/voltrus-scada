//! The alarm list: filters, search, sort, and a keyboard-driven table.
//! Enter opens an alarm's detail, `a` acknowledges it, `s` (or Space)
//! shelves it or takes it off the shelf, ⇧A acknowledges all, ← and →
//! switch the filter, `o` the order.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Div, ElementId, FontWeight, Window, div, px, uniform_list};
use voltrus_scada_native::api::Alarm;
use voltrus_scada_native::runtime::OPEN_ALARMS_LIMIT;
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::components::{Button, IconButton, Segment, empty_state, pill, segmented};
use vt_ui::theme::*;

use super::super::{
    ListKeys, age, alarm_source, focus_ring, keyboard_list, local_time, rows_on_screen,
    severity_color, tail_row, tail_rows, value_with_unit,
};
use super::{LIST, State, dialogs, fetch_history, priority_color};
use crate::console::alarms::{self, Filter, Sort, is_resolved};
use crate::console::keys::{
    ALARM_LIST, AlarmsAcknowledge, AlarmsCycleSort, AlarmsNextFilter, AlarmsPreviousFilter,
    AlarmsShelve, LIST as LIST_CONTEXT,
};
use crate::console::{Console, is_shelved, time_now_secs};

pub(super) const ROW_HEIGHT_PX: f32 = 54.0;
const SEVERITY: f32 = 112.0;
const VALUE: f32 = 104.0;
const RAISED: f32 = 88.0;
const ACTIONS: f32 = 196.0;

/// The alarms the list draws from: the open ones, or the History filter's
/// own list (`status=all`).
pub(super) fn source(console: &Console) -> &[Alarm] {
    let state = console.state_ref::<State>();
    if state.is_some_and(|state| state.filter == Filter::History) {
        state
            .and_then(|state| state.history.as_ref())
            .and_then(|history| history.as_ref().ok())
            .map(Vec::as_slice)
            .unwrap_or_default()
    } else {
        &console.alarms
    }
}

/// The list's rows, as positions in [`source`], filtered, searched and
/// sorted.
pub(super) fn rows(console: &Console, cx: &App) -> Vec<usize> {
    let (filter, sort, query) = console.state_ref::<State>().map_or(
        (Filter::default(), Sort::default(), String::new()),
        |state| {
            (
                state.filter,
                state.sort,
                state
                    .search
                    .as_ref()
                    .map(|field| field.read(cx).value().to_owned())
                    .unwrap_or_default(),
            )
        },
    );
    alarms::arrange(source(console), filter, sort, &query, |alarm| {
        alarm_source(console, alarm)
    })
}

/// The alarm under the list's cursor.
pub(super) fn cursor_alarm(console: &mut Console, cx: &mut Context<Console>) -> Option<Alarm> {
    let row = console.list(LIST, cx).cursor?;
    let index = *rows(console, cx).get(row)?;
    source(console).get(index).cloned()
}

/// Enter opens the alarm's detail; Space shelves it or takes it off the
/// shelf.
fn activate_row(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    console.list(LIST, cx).cursor = Some(row);
    let Some(alarm) = cursor_alarm(console, cx) else {
        return;
    };
    if toggle {
        shelve_or_unshelve(console, &alarm, window, cx);
    } else {
        dialogs::open_detail(console, alarm.id, window, cx);
    }
}

/// `a`: acknowledge, if it waits for it.
pub(super) fn acknowledge(console: &mut Console, alarm: &Alarm) {
    if alarm.acknowledged_at.is_none() && !is_shelved(alarm) && !is_resolved(alarm) {
        console.acknowledge(alarm.id);
    }
}

/// `s`: off the shelf if on it, else the shelve dialog; operators only,
/// like the server's routes.
pub(super) fn shelve_or_unshelve(
    console: &mut Console,
    alarm: &Alarm,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !console.is_operator() {
        console.show_toast("Shelving needs an operator role", true, cx);
    } else if is_shelved(alarm) {
        console.unshelve(alarm.id);
    } else if !is_resolved(alarm) {
        dialogs::open_shelve(console, alarm.id, window, cx);
    }
}

/// Show `filter`, its first row under the cursor.
pub(super) fn set_filter(console: &mut Console, filter: Filter, cx: &mut Context<Console>) {
    console.state::<State>().filter = filter;
    if filter == Filter::History {
        fetch_history(console);
    }
    let nav = console.list(LIST, cx);
    nav.cursor = Some(0);
    nav.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
    cx.notify();
}

fn set_sort(console: &mut Console, sort: Sort, cx: &mut Context<Console>) {
    console.state::<State>().sort = sort;
    cx.notify();
}

/// The filter, search and sort row above the table.
fn toolbar(console: &mut Console, cx: &mut Context<Console>) -> Div {
    let theme = theme(cx).clone();
    let (filter, sort, search, history) = {
        let state = console.state::<State>();
        (
            state.filter,
            state.sort,
            state.search.clone(),
            state
                .history
                .as_ref()
                .and_then(|history| history.as_ref().ok())
                .map(Vec::len),
        )
    };
    let count = |filter: Filter| {
        console
            .alarms
            .iter()
            .filter(|alarm| filter.admits(alarm))
            .count()
    };
    let segments = Filter::ALL
        .into_iter()
        .map(|choice| {
            let label = match choice {
                Filter::History => history.map_or_else(
                    || choice.label().to_owned(),
                    |len| format!("{}  {len}", choice.label()),
                ),
                _ => format!("{}  {}", choice.label(), count(choice)),
            };
            Segment::new(
                label,
                filter == choice,
                cx.listener(move |console, _, _, cx| set_filter(console, choice, cx)),
            )
        })
        .collect();
    let sorts = Sort::ALL
        .into_iter()
        .map(|choice| {
            Segment::new(
                choice.label(),
                sort == choice,
                cx.listener(move |console, _, _, cx| set_sort(console, choice, cx)),
            )
        })
        .collect();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_6)
        .h(px(44.0))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border)
        .child(segmented("alarm-filter", segments, cx))
        .child(div().flex_1())
        .children(search.map(|field| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .w(px(220.0))
                .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
                .child(div().flex_1().child(field))
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(vt_ui::icon(Icon::ArrowUpDown, ICON_SM, theme.fg_dim))
                .child(segmented("alarm-sort", sorts, cx)),
        )
}

pub(super) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let toolbar = toolbar(console, cx);
    // Filtered and sorted once per render; the rows index into the source.
    let rows = Rc::new(rows(console, cx));
    let shown = rows.len();
    let (filter, history_error, history_loading, searching) = {
        let state = console.state::<State>();
        (
            state.filter,
            state
                .history
                .as_ref()
                .and_then(|history| history.as_ref().err().cloned()),
            state.filter == Filter::History && state.history.is_none(),
            state
                .search
                .as_ref()
                .is_some_and(|field| !field.read(cx).value().trim().is_empty()),
        )
    };
    // The server lists at most so many alarms of each status; during a flood
    // the open alarms beyond them are not here, so say so.
    let capped = (console.alarms_capped && filter != Filter::History).then(|| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(SPACE_4)
            .px(SPACE_8)
            .py(SPACE_3)
            .bg(theme.warning.alpha(0.1))
            .border_b_1()
            .border_color(theme.border)
            .text_size(FONT_SMALL)
            .text_color(theme.warning)
            .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.warning))
            .child(format!(
                "More than {OPEN_ALARMS_LIMIT} alarms of one status are open; the list shows the newest {OPEN_ALARMS_LIMIT} of each."
            ))
    });

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
        .child(div().w(px(SEVERITY)).flex_none().child("PRIORITY"))
        .child(div().flex_1().min_w_0().child("ALARM"))
        .child(div().w(px(VALUE)).flex_none().text_right().child("VALUE"))
        .child(div().w(px(RAISED)).flex_none().child("RAISED"))
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
    )
    // The list's keys and the page's own: a, s, ⇧A, ←/→, o.
    .key_context(format!("{LIST_CONTEXT} {ALARM_LIST}").as_str())
    .on_action(cx.listener(|console, _: &AlarmsAcknowledge, _, cx| {
        if let Some(alarm) = cursor_alarm(console, cx) {
            acknowledge(console, &alarm);
        }
    }))
    .on_action(cx.listener(|console, _: &AlarmsShelve, window, cx| {
        if let Some(alarm) = cursor_alarm(console, cx) {
            shelve_or_unshelve(console, &alarm, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &AlarmsNextFilter, _, cx| {
        let next = console.state::<State>().filter.step(true);
        set_filter(console, next, cx);
    }))
    .on_action(cx.listener(|console, _: &AlarmsPreviousFilter, _, cx| {
        let next = console.state::<State>().filter.step(false);
        set_filter(console, next, cx);
    }))
    .on_action(cx.listener(|console, _: &AlarmsCycleSort, _, cx| {
        let next = console.state::<State>().sort.next();
        set_sort(console, next, cx);
    }));
    let nav = console.list(LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let scroll = nav.scroll.clone();
    let body = if shown == 0 {
        let (icon, title, text): (Icon, &str, String) = if let Some(error) = history_error {
            (
                Icon::CircleAlert,
                "History unavailable",
                format!("The server did not send it: {error}"),
            )
        } else if history_loading {
            (
                Icon::History,
                "Loading the history",
                "Every alarm the server kept, resolved ones too.".into(),
            )
        } else if searching {
            (
                Icon::Search,
                "No alarm matches",
                "Nothing has this text in its source, key, message or priority; #12 finds alarm 12."
                    .into(),
            )
        } else {
            let (title, text) = match filter {
                Filter::Open => (
                    "No open alarms",
                    "Alarms the server raises appear here, worst first.",
                ),
                Filter::Unacknowledged => (
                    "Everything is acknowledged",
                    "Alarms nobody has acknowledged yet appear here.",
                ),
                Filter::Acknowledged => (
                    "Nothing acknowledged is open",
                    "Acknowledged alarms stay here until their value is back inside its limit.",
                ),
                Filter::Shelved => (
                    "Nothing on the shelf",
                    "Shelving hides a known alarm for a while, with a reason.",
                ),
                Filter::History => ("No alarms yet", "The server has not raised any alarm."),
            };
            (Icon::CircleCheck, title, text.into())
        };
        empty_state(icon, title, text, Vec::new(), cx).into_any_element()
    } else {
        uniform_list(
            "alarms",
            shown + tail_rows(shown, ROW_HEIGHT_PX),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let now = time_now_secs();
                    let operator = console.is_operator();
                    let source = source(console);
                    range
                        .map(
                            |row| match rows.get(row).and_then(|index| source.get(*index)) {
                                Some(alarm) => {
                                    let ring = focused && cursor == Some(row);
                                    alarm_row(
                                        console,
                                        alarm,
                                        (row, now, operator, ring),
                                        &theme,
                                        cx,
                                    )
                                }
                                None => tail_row(ROW_HEIGHT_PX),
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

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(toolbar)
        .children(capped)
        .child(header)
        .child(list.flex_1().min_h_0().child(body))
        .into_any_element()
}

/// `HI` or `LO`: which side of its limit the value crossed.
pub(super) fn direction(alarm: &Alarm) -> &'static str {
    if alarm.alarm_type.eq_ignore_ascii_case("low") {
        "↓ LO"
    } else {
        "↑ HI"
    }
}

/// The alarm's state for a caption: icon, text and colour.
pub(super) fn state_line(
    console: &Console,
    alarm: &Alarm,
    theme: &Theme,
) -> (Icon, String, gpui::Hsla) {
    if is_resolved(alarm) {
        (
            Icon::CircleCheck,
            alarm.resolved_at.map_or_else(
                || "Resolved".to_owned(),
                |at| format!("Resolved {}", local_time(at, &console.zone)),
            ),
            theme.success,
        )
    } else if is_shelved(alarm) {
        let mut text = alarm.shelved_until.map_or_else(
            || "Shelved".to_owned(),
            |until| format!("Shelved until {}", local_time(until, &console.zone)),
        );
        if let Some(by) = &alarm.shelved_by {
            text.push_str(&format!(" by {by}"));
        }
        if let Some(reason) = alarm.shelved_reason.as_ref().filter(|r| !r.is_empty()) {
            text.push_str(&format!(" · {reason}"));
        }
        (Icon::Clock, text, theme.fg_muted)
    } else if alarm.acknowledged_at.is_some() {
        (
            Icon::Check,
            alarm.acknowledged_by.as_ref().map_or_else(
                || "Acknowledged".to_owned(),
                |by| format!("Acknowledged by {by}"),
            ),
            theme.fg_muted,
        )
    } else {
        (Icon::CircleAlert, "Unacknowledged".to_owned(), theme.danger)
    }
}

fn alarm_row(
    console: &Console,
    alarm: &Alarm,
    (row, now, operator, ring): (usize, i64, bool, bool),
    theme: &Theme,
    cx: &Context<Console>,
) -> AnyElement {
    let id = alarm.id;
    let key = u64::try_from(id).unwrap_or(0);
    let color = severity_color(&alarm.severity, theme);
    let priority = alarms::priority(alarm).to_owned();
    let priority_fill = priority_color(&priority, theme);
    let shelved = is_shelved(alarm);
    let resolved = is_resolved(alarm);
    let acknowledged = alarm.acknowledged_at.is_some();
    let unit = console
        .store
        .by_key(&alarm.source_key)
        .map(|index| console.store.tags[index].info.unit.clone())
        .unwrap_or_default();
    let source = alarm_source(console, alarm);
    let key_differs = alarm.source_key != source;
    let (state_icon, state_text, state_color) = state_line(console, alarm, theme);

    // The server's shelf routes refuse other roles, so their buttons stay
    // hidden like the write panel.
    let mut actions: Vec<AnyElement> = Vec::new();
    if !resolved && shelved && operator {
        actions.push(
            Button::new(ElementId::NamedInteger("unshelve".into(), key), "Unshelve")
                .compact()
                .on_click(cx.listener(move |console, _, _, _| console.unshelve(id)))
                .into_any_element(),
        );
    } else if !resolved && !shelved {
        if !acknowledged {
            actions.push(
                Button::new(ElementId::NamedInteger("ack".into(), key), "Ack")
                    .compact()
                    .icon(Icon::Check)
                    .on_click(cx.listener(move |console, _, _, _| console.acknowledge(id)))
                    .into_any_element(),
            );
        }
        if operator {
            actions.push(
                Button::new(ElementId::NamedInteger("shelve".into(), key), "Shelve")
                    .compact()
                    .ghost()
                    .icon(Icon::Clock)
                    .on_click(cx.listener(move |console, _, window, cx| {
                        dialogs::open_shelve(console, id, window, cx);
                    }))
                    .into_any_element(),
            );
        }
    }
    actions.push(
        IconButton::new(ElementId::NamedInteger("detail".into(), key), Icon::History)
            .tooltip("Details and lifecycle  ↵")
            .on_click(cx.listener(move |console, _, window, cx| {
                console.list(LIST, cx).select(row);
                dialogs::open_detail(console, id, window, cx);
            }))
            .into_any_element(),
    );

    div()
        .id(ElementId::NamedInteger("alarm".into(), key))
        .relative()
        .when(ring, |this| this.child(focus_ring(theme)))
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(ROW_HEIGHT_PX))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .when(!acknowledged && !shelved && !resolved, |this| {
            this.bg(color.alpha(0.04))
        })
        .when(resolved || shelved, |this| this.opacity(0.72))
        .on_click(cx.listener(move |console, _, _, cx| {
            console.list(LIST, cx).select(row);
            cx.notify();
        }))
        .child(
            div()
                .w(px(SEVERITY))
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(div().w(px(3.0)).h(px(30.0)).rounded_full().bg(color))
                .child(pill(priority.to_uppercase(), priority_fill))
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(direction(alarm)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(SPACE_3)
                        .min_w_0()
                        .child(
                            div()
                                .flex_shrink()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(source),
                        )
                        .when(key_differs, |this| {
                            this.child(
                                div()
                                    .flex_shrink()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(FONT_CAPTION)
                                    .text_color(theme.fg_dim)
                                    .child(alarm.source_key.clone()),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .min_w_0()
                        .text_size(FONT_SMALL)
                        .text_color(state_color)
                        .child(vt_ui::icon(state_icon, ICON_SM, state_color))
                        .child(div().min_w_0().truncate().child(state_text)),
                ),
        )
        .child(
            div()
                .w(px(VALUE))
                .flex_none()
                .flex()
                .flex_col()
                .items_end()
                .child(value_with_unit(
                    format_value(alarm.value),
                    &unit,
                    FONT_BODY,
                    color,
                    theme,
                ))
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(format!("limit {}", format_value(alarm.threshold))),
                ),
        )
        .child(
            div()
                .w(px(RAISED))
                .flex_none()
                .flex()
                .flex_col()
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(local_time(alarm.triggered_at, &console.zone)),
                )
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(format!("{} ago", age(now - alarm.triggered_at))),
                ),
        )
        .child(
            div()
                .w(px(ACTIONS))
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .gap(SPACE_3)
                .children(actions),
        )
        .into_any_element()
}
