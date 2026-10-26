//! The operator views of the Alarms page: analytics (the sources that
//! alarmed most in the last 24 hours, a flood warning, the time to
//! acknowledge) and the audit trail (every acknowledge, shelve and
//! unshelve). Both are keyboard lists: Enter on a source lists its alarms,
//! Enter on an audit entry opens the alarm.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, FontWeight, Window, div, px, relative, uniform_list};
use voltrus_scada_native::api::{AlarmAnalyticsEntry, AlarmAuditEntry};
use vt_ui::Icon;
use vt_ui::components::{Button, empty_state, pill, section_label};
use vt_ui::theme::*;

use super::super::{
    LIST_TAIL, ListKeys, age, card, focus_ring, keyboard_list, local_time, rows_on_screen,
    tail_row, tail_rows,
};
use super::{ANALYTICS_LIST, AUDIT_LIST, LIST, State, dialogs, fetch_analytics, fetch_audit, list};
use crate::console::alarms::{
    Counts, Filter, TOP, View, duration, flood, readable_details, shelve_details,
};
use crate::console::{Console, time_now_secs};

const AUDIT_ROW_PX: f32 = 52.0;
const RANK_ROW_PX: f32 = 44.0;

/// A loading, failed or empty view.
fn placeholder(
    icon: Icon,
    title: &'static str,
    text: String,
    retry: Option<fn(&mut Console)>,
    cx: &mut Context<Console>,
) -> AnyElement {
    let actions = retry
        .map(|retry| {
            vec![
                Button::new("alarms-retry", "Try again")
                    .icon(Icon::RefreshCw)
                    .on_click(cx.listener(move |console, _, _, cx| {
                        retry(console);
                        cx.notify();
                    }))
                    .into_any_element(),
            ]
        })
        .unwrap_or_default();
    div()
        .flex_1()
        .min_h_0()
        .child(empty_state(icon, title, text, actions, cx))
        .into_any_element()
}

/// One figure of the analytics header.
fn figure(label: &'static str, value: String, note: String, theme: &Theme) -> Div {
    card(theme)
        .flex_1()
        .gap(SPACE_2)
        .px(SPACE_7)
        .py(SPACE_6)
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(label),
        )
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_DISPLAY)
                .font_weight(FontWeight::MEDIUM)
                .child(value),
        )
        .child(
            div()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(note),
        )
}

/// Enter on a ranked source: the alarm history, searched for it.
fn open_source(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let key = console
        .state_ref::<State>()
        .and_then(|state| state.analytics.as_ref())
        .and_then(|analytics| analytics.as_ref().ok())
        .and_then(|entries| entries.get(row))
        .map(|entry| entry.source_key.clone());
    let Some(key) = key else {
        return;
    };
    let field = console.state::<State>().search.clone();
    if let Some(field) = field {
        field.update(cx, |field, cx| field.set_text(key, cx));
    }
    console.state::<State>().view = View::List;
    list::set_filter(console, Filter::History, cx);
    console.focus_list(LIST, window, cx);
}

pub(super) fn analytics(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let now = time_now_secs();
    // Figures from the whole answer; only the ranked rows are copied out.
    let (top, total, sources, flooding, counts) = {
        let state = console.state::<State>();
        match state.analytics.as_ref() {
            None => {
                return placeholder(
                    Icon::ChartLine,
                    "Loading analytics",
                    "Counting the last 24 hours of alarms.".into(),
                    None,
                    cx,
                );
            }
            Some(Err(error)) => {
                let text = format!("The server did not send them: {error}");
                return placeholder(
                    Icon::CircleAlert,
                    "Analytics unavailable",
                    text,
                    Some(fetch_analytics),
                    cx,
                );
            }
            Some(Ok(entries)) => (
                entries
                    .iter()
                    .take(TOP)
                    .cloned()
                    .collect::<Vec<AlarmAnalyticsEntry>>(),
                entries.iter().map(|entry| entry.trigger_count).sum::<i64>(),
                entries.len(),
                state
                    .history
                    .as_ref()
                    .and_then(|history| history.as_ref().ok())
                    .and_then(|history| flood(history, now)),
                state
                    .summary
                    .as_ref()
                    .and_then(|summary| summary.as_ref().ok())
                    .map(Counts::from_server),
            ),
        }
    };
    // The console's name for each source; the server's row may carry none.
    let names: Vec<String> = top
        .iter()
        .map(|entry| {
            console
                .store
                .by_key(&entry.source_key)
                .map(|index| console.store.tags[index].info.name.clone())
                .filter(|name| !name.is_empty())
                .or_else(|| (!entry.source_name.is_empty()).then(|| entry.source_name.clone()))
                .unwrap_or_else(|| entry.source_key.clone())
        })
        .collect();
    let most = top
        .iter()
        .map(|entry| entry.trigger_count)
        .max()
        .unwrap_or(1)
        .max(1);

    let list_keys = keyboard_list(
        ListKeys {
            id: ANALYTICS_LIST,
            on_move: None,
            on_activate: open_source,
        },
        top.len(),
        rows_on_screen(window, RANK_ROW_PX),
        console,
        cx,
    );
    let nav = console.list(ANALYTICS_LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let scroll = console.scroll(ANALYTICS_LIST);

    let figures = div()
        .flex()
        .gap(SPACE_6)
        .child(figure(
            "Alarms raised",
            total.to_string(),
            format!("by {sources} sources in the last 24 h"),
            &theme,
        ))
        .child(figure(
            "Time to acknowledge",
            counts
                .and_then(|counts| counts.avg_ack_secs)
                .map_or_else(|| "—".to_owned(), duration),
            "on average, last 24 h".into(),
            &theme,
        ))
        .child(figure(
            "Waiting now",
            counts.map_or_else(
                || "—".to_owned(),
                |counts| (counts.critical + counts.warning + counts.info).to_string(),
            ),
            "open and unacknowledged".into(),
            &theme,
        ));

    let flood_banner = flooding.map(|count| {
        div()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .px(SPACE_7)
            .py(SPACE_5)
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(theme.danger.alpha(0.5))
            .bg(theme.danger.alpha(0.1))
            .text_color(theme.danger)
            .font_weight(FontWeight::MEDIUM)
            .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.danger))
            .child(format!(
                "Alarm flood: {count} alarms raised in the last 5 minutes"
            ))
    });

    let ranking = if top.is_empty() {
        div()
            .py(SPACE_9)
            .text_color(theme.fg_muted)
            .child("No alarm in the last 24 hours.")
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .children(top.iter().enumerate().map(|(row, entry)| {
                let ring = focused && cursor == Some(row);
                #[allow(clippy::cast_precision_loss)]
                let share = entry.trigger_count as f32 / most as f32;
                let recent = now - entry.last_triggered_at;
                div()
                    .id(("alarm-rank", row))
                    .relative()
                    .when(ring, |this| this.child(focus_ring(&theme)))
                    .flex()
                    .items_center()
                    .gap(SPACE_6)
                    .h(px(RANK_ROW_PX))
                    .px(SPACE_5)
                    .border_b_1()
                    .border_color(theme.border.alpha(0.6))
                    .cursor_pointer()
                    .on_click(cx.listener(move |console, _, window, cx| {
                        console.list(ANALYTICS_LIST, cx).select(row);
                        open_source(console, row, false, window, cx);
                    }))
                    .child(
                        div()
                            .w(px(22.0))
                            .flex_none()
                            .text_right()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child(format!("{}.", row + 1)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(SPACE_2)
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(SPACE_3)
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(names[row].clone()),
                                    )
                                    .when(names[row] != entry.source_key, |this| {
                                        this.child(
                                            div()
                                                .truncate()
                                                .font_family(MONO)
                                                .text_size(FONT_CAPTION)
                                                .text_color(theme.fg_dim)
                                                .child(entry.source_key.clone()),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .h(px(4.0))
                                    .w_full()
                                    .rounded_full()
                                    .bg(theme.border)
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative(share))
                                            .rounded_full()
                                            .bg(theme.accent),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .w(px(56.0))
                            .flex_none()
                            .text_right()
                            .font_family(MONO)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.accent)
                            .child(entry.trigger_count.to_string()),
                    )
                    .child(
                        div()
                            .w(px(120.0))
                            .flex_none()
                            .text_right()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(if entry.avg_time_to_ack_secs > 0.0 {
                                format!("ack in {}", duration(entry.avg_time_to_ack_secs))
                            } else {
                                "not acknowledged".to_owned()
                            }),
                    )
                    .child(
                        div()
                            .w(px(84.0))
                            .flex_none()
                            .text_right()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child(format!("{} ago", age(recent))),
                    )
            }))
            .into_any_element()
    };

    list_keys
        .flex_1()
        .min_h_0()
        .child(
            div()
                .id("alarm-analytics")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .flex()
                .flex_col()
                .gap(SPACE_8)
                .px(SPACE_9)
                .pt(SPACE_8)
                .pb(LIST_TAIL)
                .child(figures)
                .children(flood_banner)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_4)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .child(section_label("Most frequent · last 24 h", cx))
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_size(FONT_CAPTION)
                                        .text_color(theme.fg_dim)
                                        .child("↵ lists the source's alarms"),
                                ),
                        )
                        .child(ranking),
                ),
        )
        .into_any_element()
}

/// The audit entry's alarm, if the console has it: its source's name.
fn audit_source(console: &Console, alarm: i64) -> Option<String> {
    console
        .alarms
        .iter()
        .chain(
            console
                .state_ref::<State>()
                .and_then(|state| state.history.as_ref())
                .and_then(|history| history.as_ref().ok())
                .into_iter()
                .flatten(),
        )
        .find(|candidate| candidate.id == alarm)
        .map(|candidate| super::super::alarm_source(console, candidate))
}

/// Enter on an audit entry: open its alarm, if the console has it.
fn open_audit_alarm(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let alarm = console
        .state_ref::<State>()
        .and_then(|state| state.audit.as_ref())
        .and_then(|audit| audit.as_ref().ok())
        .and_then(|entries| entries.get(row))
        .map(|entry| entry.alarm_id);
    let Some(alarm) = alarm else {
        return;
    };
    if audit_source(console, alarm).is_some() {
        dialogs::open_detail(console, alarm, window, cx);
    } else {
        console.show_toast(
            format!("Alarm {alarm} is not among the open alarms or the loaded history"),
            true,
            cx,
        );
    }
}

fn audit_color(action: &str, theme: &Theme) -> gpui::Hsla {
    match action {
        "acknowledged" | "acknowledge" | "ack" => theme.success,
        "created" | "triggered" | "raised" => theme.warning,
        "shelved" | "shelve" => theme.info,
        "unshelved" | "unshelve" => theme.accent,
        _ => theme.fg_muted,
    }
}

pub(super) fn audit(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let shown = match console.state::<State>().audit.as_ref() {
        None => {
            return placeholder(
                Icon::ScrollText,
                "Loading the audit trail",
                "Who acknowledged, shelved and unshelved what.".into(),
                None,
                cx,
            );
        }
        Some(Err(error)) => {
            let error = error.clone();
            return placeholder(
                Icon::CircleAlert,
                "Audit trail unavailable",
                format!("The server did not send it: {error}"),
                Some(fetch_audit),
                cx,
            );
        }
        Some(Ok(entries)) if entries.is_empty() => {
            return placeholder(
                Icon::ScrollText,
                "Nothing audited yet",
                "Acknowledging, shelving and unshelving alarms leaves a trail here.".into(),
                None,
                cx,
            );
        }
        Some(Ok(entries)) => entries.len(),
    };
    let theme = theme(cx).clone();
    let list_keys = keyboard_list(
        ListKeys {
            id: AUDIT_LIST,
            on_move: None,
            on_activate: open_audit_alarm,
        },
        shown,
        rows_on_screen(window, AUDIT_ROW_PX),
        console,
        cx,
    );
    let nav = console.list(AUDIT_LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let scroll = nav.scroll.clone();

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
        .child(div().w(px(96.0)).flex_none().child("WHEN"))
        .child(div().w(px(112.0)).flex_none().child("ACTION"))
        .child(div().flex_1().min_w_0().child("ALARM"))
        .child(div().w(px(140.0)).flex_none().child("BY"));

    let rows = uniform_list(
        "alarm-audit",
        shown + tail_rows(shown, AUDIT_ROW_PX),
        cx.processor(
            move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = vt_ui::theme::theme(cx).clone();
                let now = time_now_secs();
                let entries: &[AlarmAuditEntry] = console
                    .state_ref::<State>()
                    .and_then(|state| state.audit.as_ref())
                    .and_then(|audit| audit.as_ref().ok())
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                range
                    .map(|row| match entries.get(row) {
                        Some(entry) => {
                            let ring = focused && cursor == Some(row);
                            audit_row(console, entry, (row, now, ring), &theme, cx)
                        }
                        None => tail_row(AUDIT_ROW_PX),
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .size_full();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(header)
        .child(list_keys.flex_1().min_h_0().child(rows))
        .into_any_element()
}

fn audit_row(
    console: &Console,
    entry: &AlarmAuditEntry,
    (row, now, ring): (usize, i64, bool),
    theme: &Theme,
    cx: &Context<Console>,
) -> AnyElement {
    let color = audit_color(&entry.action, theme);
    let source = audit_source(console, entry.alarm_id);
    let details = match shelve_details(&entry.details) {
        Some(shelve) => {
            let mut parts = Vec::new();
            if let Some(until) = shelve.until {
                parts.push(format!("until {}", local_time(until, console.utc_offset)));
            }
            if let Some(reason) = shelve.reason {
                parts.push(format!("“{reason}”"));
            }
            parts.join(" · ")
        }
        None => readable_details(&entry.details).unwrap_or_else(|| entry.details.clone()),
    };
    let by = if entry.performed_by.is_empty() {
        "system".to_owned()
    } else {
        entry.performed_by.clone()
    };
    div()
        .id(("alarm-audit-row", row))
        .relative()
        .when(ring, |this| this.child(focus_ring(theme)))
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(AUDIT_ROW_PX))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .on_click(cx.listener(move |console, _, _, cx| {
            console.list(AUDIT_LIST, cx).select(row);
            cx.notify();
        }))
        .child(
            div()
                .w(px(96.0))
                .flex_none()
                .flex()
                .flex_col()
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(local_time(entry.created_at, console.utc_offset)),
                )
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(format!("{} ago", age(now - entry.created_at))),
                ),
        )
        .child(
            div()
                .w(px(112.0))
                .flex_none()
                .child(pill(entry.action.to_uppercase(), color)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(SPACE_3)
                        .min_w_0()
                        .child(
                            div()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(format!("#{}", entry.alarm_id)),
                        )
                        .when_some(source, |this, source| {
                            this.child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(source),
                            )
                        }),
                )
                .when(!details.is_empty(), |this| {
                    this.child(
                        div()
                            .truncate()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(details),
                    )
                }),
        )
        .child(
            div()
                .w(px(140.0))
                .flex_none()
                .truncate()
                .text_size(FONT_SMALL)
                .child(by),
        )
        .into_any_element()
}
