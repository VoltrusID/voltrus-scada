//! The alarm banner under the mimic (`AlarmBanner.tsx`, ISA-101): counts of
//! the active alarms and the first rows, unacknowledged critical alarms
//! first, acknowledged in place. It reads the alarm list the console keeps
//! (refreshed with the console's alarm refresh and on every raised alarm),
//! with no poll of its own.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, Hsla, div, px};
use voltrus_scada_native::api::Alarm;
use voltrus_scada_native::mimic::{BANNER_ROWS, banner};
use vt_ui::theme::*;

use super::paint::{Hmi, fade};
use super::scene::motion;
use crate::console::views::{alarm_source, local_time};
use crate::console::{Console, Page};

const HEADER: f32 = 30.0;
const ROW: f32 = 26.0;

/// The banner's height: its header and its rows.
pub(crate) fn height() -> gpui::Pixels {
    #[allow(clippy::cast_precision_loss)]
    let rows = BANNER_ROWS as f32;
    px(HEADER + ROW * rows)
}

/// Whether the banner draws a blinking marker (an unacknowledged critical
/// alarm among its rows).
pub(crate) fn blinking(alarms: &[Alarm]) -> bool {
    banner(alarms)
        .alarms
        .iter()
        .take(BANNER_ROWS)
        .any(|alarm| alarm.acknowledged_at.is_none() && alarm.severity == "critical")
}

/// A number as the banner writes it (`fmt` in `AlarmBanner.tsx`).
fn number(value: f64) -> String {
    if value.abs() >= 100.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

/// `High critical: 86.4 (limit 85.0)`.
fn describe(alarm: &Alarm) -> String {
    let side = if alarm.alarm_type.eq_ignore_ascii_case("low") {
        "Low"
    } else {
        "High"
    };
    let severity = if alarm.severity == "critical" {
        "critical"
    } else {
        "warning"
    };
    format!(
        "{side} {severity}: {} (limit {})",
        number(alarm.value),
        number(alarm.threshold)
    )
}

/// The severity marker: filled while unacknowledged, an outline after. The
/// web turns a square on its corner; divs do not rotate, so a small square
/// stands in.
fn marker(color: Hsla, filled: bool, size: f32) -> gpui::Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(1.5))
        .border_1()
        .border_color(color)
        .when(filled, |this| this.bg(color))
}

pub(crate) fn render(
    console: &mut Console,
    hmi: &Hmi,
    t: f32,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let blink = motion::blink(t);
    let summary = banner(&console.alarms);
    let more = summary.alarms.len().saturating_sub(BANNER_ROWS);
    let count = |color: Hsla, n: usize, label: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .text_color(if n > 0 { theme.fg } else { theme.fg_dim })
            .child(marker(color, n > 0, 8.0))
            .child(
                div()
                    .font_family(MONO)
                    .font_weight(FontWeight::BOLD)
                    .child(n.to_string()),
            )
            .child(label)
    };
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_7)
        .h(px(HEADER))
        .px(SPACE_6)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.fg_dim)
                .child("ALARMS"),
        )
        .child(count(hmi.fault, summary.critical, "critical"))
        .child(count(hmi.warn, summary.warning, "warning"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .text_color(if summary.unacknowledged > 0 {
                    theme.fg
                } else {
                    theme.fg_dim
                })
                .child(
                    div()
                        .font_family(MONO)
                        .font_weight(FontWeight::BOLD)
                        .child(summary.unacknowledged.to_string()),
                )
                .child("unacknowledged"),
        )
        .child(div().flex_1())
        .when(more > 0, |this| {
            this.child(
                div()
                    .text_color(theme.fg_dim)
                    .child(format!("+{more} more")),
            )
        })
        .child(
            div()
                .id("mimic-alarm-summary")
                .cursor_pointer()
                .text_color(theme.accent)
                .font_weight(FontWeight::SEMIBOLD)
                .child("Alarm summary →")
                .on_click(cx.listener(|console, _, window, cx| {
                    console.show(Page::Alarms, window, cx);
                })),
        );
    let rows: Vec<AnyElement> = if summary.alarms.is_empty() {
        vec![
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.fg_dim)
                .child("No active alarms")
                .into_any_element(),
        ]
    } else {
        summary
            .alarms
            .iter()
            .take(BANNER_ROWS)
            .map(|alarm| {
                let critical = alarm.severity == "critical";
                let color = if critical { hmi.fault } else { hmi.warn };
                let unacked = alarm.acknowledged_at.is_none();
                let marker_alpha = if unacked && critical { blink } else { 1.0 };
                let id = alarm.id;
                div()
                    .id(gpui::ElementId::NamedInteger(
                        "mimic-alarm".into(),
                        u64::try_from(alarm.id).unwrap_or(0),
                    ))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(SPACE_5)
                    .h(px(ROW))
                    .px(SPACE_6)
                    .border_b_1()
                    .border_color(fade(theme.border, 0.6))
                    .when(unacked, |this| this.bg(fade(color, 0.07)))
                    .child(div().w(px(18.0)).flex().justify_center().child(marker(
                        fade(color, marker_alpha),
                        unacked,
                        9.0,
                    )))
                    .child(
                        div()
                            .w(px(76.0))
                            .flex_none()
                            .font_family(MONO)
                            .text_color(theme.fg_dim)
                            .child(local_time(alarm.triggered_at, &console.zone)),
                    )
                    .child(
                        div()
                            .w(px(220.0))
                            .flex_none()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(alarm_source(console, alarm)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.fg_muted)
                            .child(describe(alarm)),
                    )
                    .child(
                        div()
                            .w(px(120.0))
                            .flex_none()
                            .truncate()
                            .text_size(FONT_CAPTION)
                            .font_weight(FontWeight::BOLD)
                            .text_color(if unacked { color } else { theme.fg_dim })
                            .child(if unacked {
                                "UNACKED".to_owned()
                            } else {
                                format!(
                                    "ACK · {}",
                                    alarm.acknowledged_by.clone().unwrap_or_default()
                                )
                            }),
                    )
                    .child(div().w(px(96.0)).flex_none().when(unacked, |this| {
                        this.child(
                            vt_ui::components::Button::new(
                                gpui::ElementId::NamedInteger(
                                    "mimic-ack".into(),
                                    u64::try_from(id).unwrap_or(0),
                                ),
                                "Acknowledge",
                            )
                            .compact()
                            .on_click(cx.listener(
                                move |console, _, _, cx| {
                                    console.acknowledge(id);
                                    cx.notify();
                                },
                            )),
                        )
                    }))
                    .into_any_element()
            })
            .collect()
    };
    div()
        .flex()
        .flex_col()
        .flex_none()
        .h(height())
        .bg(theme.surface)
        .border_t_1()
        .border_color(theme.border)
        .text_size(FONT_SMALL)
        .child(header)
        .children(rows)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alarms_read_like_the_web_banner() {
        let alarm = Alarm {
            severity: "critical".into(),
            alarm_type: "high".into(),
            value: 86.4,
            threshold: 85.0,
            ..Alarm::default()
        };
        assert_eq!(describe(&alarm), "High critical: 86.4 (limit 85.0)");
        assert_eq!(number(1234.6), "1235");
        assert_eq!(number(-12.34), "-12.3");
        let low = Alarm {
            severity: "warning".into(),
            alarm_type: "low".into(),
            value: 4.0,
            threshold: 5.0,
            ..Alarm::default()
        };
        assert_eq!(describe(&low), "Low warning: 4.00 (limit 5.00)");
        assert!(!blinking(&[low]));
        assert!(blinking(&[alarm]));
    }
}
