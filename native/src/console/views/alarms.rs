//! Open alarms: acknowledge, shelve for an hour, or bring back from the shelf.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, div, px, uniform_list};
use voltrus_scada_native::api::Alarm;
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::components::{Button, Segment, empty_state, pill, segmented};
use vt_ui::theme::*;

use super::{age, local_time, page_header, severity_color, value_with_unit};
use crate::console::{AlarmFilter, Console, is_shelved, time_now_secs};

const SEVERITY: f32 = 92.0;
const VALUE: f32 = 150.0;
const RAISED: f32 = 104.0;
const STATE: f32 = 168.0;
const ACTIONS: f32 = 196.0;
const SHELVE_SECS: u64 = 3600;

pub fn render(console: &Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let shelved = console
        .alarms
        .iter()
        .filter(|alarm| is_shelved(alarm))
        .count();
    let open = console.alarms.len() - shelved;
    let unacknowledged = console.unacknowledged();
    let shown = console.filtered_alarms().len();
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
            .tooltip("⇧⌘A")
            .on_click(cx.listener(|console, _, _, _| console.acknowledge_all()))
            .into_any_element(),
    ];

    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(30.0))
        .px(SPACE_9)
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
        .child(div().w(px(STATE)).flex_none().child("STATE"))
        .child(div().w(px(ACTIONS)).flex_none());

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
            shown,
            cx.processor(
                |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let now = time_now_secs();
                    let alarms: Vec<Alarm> = console
                        .filtered_alarms()
                        .into_iter()
                        .skip(range.start)
                        .take(range.len())
                        .cloned()
                        .collect();
                    alarms
                        .iter()
                        .map(|alarm| alarm_row(console, alarm, now, &theme, cx))
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(console.alarms_scroll.clone())
        .flex_1()
        .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header(
            "Alarms",
            format!("{unacknowledged} to acknowledge · {open} open · {shelved} shelved"),
            controls,
            cx,
        ))
        .child(header)
        .child(body)
        .into_any_element()
}

fn alarm_row(
    console: &Console,
    alarm: &Alarm,
    now: i64,
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
        .position(&alarm.source_key)
        .map(|index| console.store.tags[index].info.unit.clone())
        .unwrap_or_default();
    let source = if alarm.source_name.is_empty() {
        alarm.source_key.clone()
    } else {
        format!("{} · {}", alarm.source_name, alarm.source_key)
    };

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

    let mut actions: Vec<AnyElement> = Vec::new();
    if shelved {
        actions.push(
            Button::new(ElementId::NamedInteger("unshelve".into(), key), "Unshelve")
                .compact()
                .on_click(cx.listener(move |console, _, _, _| console.unshelve(id)))
                .into_any_element(),
        );
    } else {
        if !acknowledged {
            actions.push(
                Button::new(ElementId::NamedInteger("ack".into(), key), "Ack")
                    .compact()
                    .icon(Icon::Check)
                    .on_click(cx.listener(move |console, _, _, _| console.acknowledge(id)))
                    .into_any_element(),
            );
        }
        actions.push(
            Button::new(ElementId::NamedInteger("shelve".into(), key), "Shelve 1 h")
                .compact()
                .ghost()
                .icon(Icon::Clock)
                .on_click(cx.listener(move |console, _, _, _| console.shelve(id, SHELVE_SECS)))
                .into_any_element(),
        );
    }

    div()
        .id(ElementId::NamedInteger("alarm".into(), key))
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(54.0))
        .px(SPACE_9)
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
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(alarm.message.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(source),
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
                .w(px(STATE))
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .text_size(FONT_SMALL)
                .text_color(state_color)
                .child(vt_ui::icon(state_icon, ICON_SM, state_color))
                .child(div().truncate().child(state_text)),
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
