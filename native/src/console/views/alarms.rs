//! Open alarms: acknowledge, shelve for an hour, or bring back from the shelf.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, Window, div, px, uniform_list};
use voltrus_scada_native::api::Alarm;
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::components::{Button, Segment, empty_state, pill, segmented};
use vt_ui::theme::*;

use super::{
    ListKeys, age, alarm_source, focus_ring, keyboard_list, local_time, page_header,
    rows_on_screen, severity_color, tail_row, tail_rows, value_with_unit,
};
use crate::console::keys::{AcknowledgeAll, hint};
use crate::console::{AlarmFilter, Console, is_shelved, time_now_secs};

/// The alarm list's keyboard list.
pub(crate) const LIST: &str = "alarms";
const ROW_HEIGHT_PX: f32 = 54.0;

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
}

/// Enter acknowledges the row's alarm; Space shelves it for an hour (or
/// takes it off the shelf), for the roles the server lets shelve.
fn activate_row(
    console: &mut Console,
    row: usize,
    toggle: bool,
    _: &mut Window,
    _: &mut Context<Console>,
) {
    let Some(alarm) = console
        .filtered_alarms()
        .get(row)
        .and_then(|index| console.alarms.get(*index))
    else {
        return;
    };
    let (id, shelved, acknowledged) =
        (alarm.id, is_shelved(alarm), alarm.acknowledged_at.is_some());
    match (toggle, shelved) {
        (false, false) if !acknowledged => console.acknowledge(id),
        (true, false) if console.is_operator() => console.shelve(id, SHELVE_SECS),
        (true, true) if console.is_operator() => console.unshelve(id),
        _ => {}
    }
}

const SEVERITY: f32 = 96.0;
const VALUE: f32 = 104.0;
const RAISED: f32 = 88.0;
const ACTIONS: f32 = 172.0;
const SHELVE_SECS: u64 = 3600;

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let unacknowledged = console.unacknowledged();
    let theme = theme(cx).clone();
    let shelved = console
        .alarms
        .iter()
        .filter(|alarm| is_shelved(alarm))
        .count();
    let open = console.alarms.len() - shelved;
    // Filtered and sorted once per render; the list's rows index into it.
    let rows = Rc::new(console.filtered_alarms());
    let shown = rows.len();
    let filter = console.alarm_filter;

    let segments = [
        (AlarmFilter::Active, "Open", open),
        (
            AlarmFilter::Unacknowledged,
            "To acknowledge",
            unacknowledged,
        ),
        (AlarmFilter::Shelved, "Shelved", shelved),
    ]
    .into_iter()
    .map(|(choice, label, count)| {
        Segment::new(
            format!("{label}  {count}"),
            filter == choice,
            cx.listener(move |console, _, _, cx| {
                console.alarm_filter = choice;
                cx.notify();
            }),
        )
    })
    .collect();

    let controls = vec![
        segmented("alarm-filter", segments, cx),
        Button::new("ack-all", "Acknowledge all")
            .icon(Icon::Check)
            .disabled(unacknowledged == 0)
            .tooltip(hint(&AcknowledgeAll).unwrap_or_default())
            .on_click(cx.listener(|console, _, _, _| console.acknowledge_all()))
            .into_any_element(),
    ];

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
        .child(div().w(px(SEVERITY)).flex_none().child("SEVERITY"))
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
    );
    let nav = console.list(LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let scroll = nav.scroll.clone();
    let body = if shown == 0 {
        let (title, text) = match filter {
            AlarmFilter::Active => (
                "No open alarms",
                "Alarms the server raises appear here, worst first.",
            ),
            AlarmFilter::Unacknowledged => (
                "Everything is acknowledged",
                "Alarms nobody has acknowledged yet appear here.",
            ),
            AlarmFilter::Shelved => (
                "Nothing on the shelf",
                "Shelving hides a known alarm for an hour.",
            ),
        };
        empty_state(Icon::CircleCheck, title, text, Vec::new(), cx).into_any_element()
    } else {
        uniform_list(
            "alarms",
            shown + tail_rows(shown, ROW_HEIGHT_PX),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let now = time_now_secs();
                    let can_shelve = console.is_operator();
                    range
                        .map(|row| {
                            match rows.get(row).and_then(|index| console.alarms.get(*index)) {
                                Some(alarm) => {
                                    let ring = focused && cursor == Some(row);
                                    alarm_row(console, alarm, (now, can_shelve, ring), &theme, cx)
                                }
                                None => tail_row(ROW_HEIGHT_PX),
                            }
                        })
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
        .size_full()
        .child(page_header(
            "Alarms",
            format!(
                "{unacknowledged} to acknowledge · {open} open · {shelved} shelved · ↵ acknowledges, Space shelves"
            ),
            controls,
            cx,
        ))
        .child(header)
        .child(list.flex_1().min_h_0().child(body))
        .into_any_element()
}

fn alarm_row(
    console: &Console,
    alarm: &Alarm,
    (now, can_shelve, ring): (i64, bool, bool),
    theme: &Theme,
    cx: &Context<Console>,
) -> AnyElement {
    let id = alarm.id;
    let key = u64::try_from(id).unwrap_or(0);
    let color = severity_color(&alarm.severity, theme);
    let shelved = is_shelved(alarm);
    let acknowledged = alarm.acknowledged_at.is_some();
    let unit = console
        .store
        .by_key(&alarm.source_key)
        .map(|index| console.store.tags[index].info.unit.clone())
        .unwrap_or_default();
    let source = alarm_source(console, alarm);
    let key_differs = alarm.source_key != source;

    let (state_icon, state_text, state_color) = if shelved {
        (
            Icon::Clock,
            alarm.shelved_until.map_or_else(
                || "Shelved".to_owned(),
                |until| format!("Shelved until {}", local_time(until, console.utc_offset)),
            ),
            theme.fg_muted,
        )
    } else if acknowledged {
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
    };

    // The server's shelf routes refuse other roles, so their buttons stay
    // hidden like the write panel.
    let mut actions: Vec<AnyElement> = Vec::new();
    if shelved && can_shelve {
        actions.push(
            Button::new(ElementId::NamedInteger("unshelve".into(), key), "Unshelve")
                .compact()
                .on_click(cx.listener(move |console, _, _, _| console.unshelve(id)))
                .into_any_element(),
        );
    } else if !shelved {
        if !acknowledged {
            actions.push(
                Button::new(ElementId::NamedInteger("ack".into(), key), "Ack")
                    .compact()
                    .icon(Icon::Check)
                    .on_click(cx.listener(move |console, _, _, _| console.acknowledge(id)))
                    .into_any_element(),
            );
        }
        if can_shelve {
            actions.push(
                Button::new(ElementId::NamedInteger("shelve".into(), key), "Shelve 1 h")
                    .compact()
                    .ghost()
                    .icon(Icon::Clock)
                    .on_click(cx.listener(move |console, _, _, _| console.shelve(id, SHELVE_SECS)))
                    .into_any_element(),
            );
        }
    }

    div()
        .id(ElementId::NamedInteger("alarm".into(), key))
        .relative()
        .when(ring, |this| this.child(focus_ring(theme)))
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(54.0))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .when(!acknowledged && !shelved, |this| this.bg(color.alpha(0.04)))
        .child(
            div()
                .w(px(SEVERITY))
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(div().w(px(3.0)).h(px(30.0)).rounded_full().bg(color))
                .child(pill(alarm.severity.to_uppercase(), color)),
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
                        .child(local_time(alarm.triggered_at, console.utc_offset)),
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
                .justify_end()
                .gap(SPACE_3)
                .children(actions),
        )
        .into_any_element()
}
