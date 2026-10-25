//! Views are functions over `&mut Console`; this is the window's root.

mod alarms;
mod navbar;
mod overview;
mod sign_in;
mod status_bar;
mod tags;
mod trends;

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, FontWeight, Hsla, SharedString, Window, div, px};
use time::{OffsetDateTime, UtcOffset};
use voltrus_scada_native::api::Alarm;
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::theme::*;

use super::store::Status;
use super::{
    AcknowledgeAll, CONSOLE, Console, NextTag, Page, Phase, PreviousTag, Quit, ShowAlarms,
    ShowOverview, ShowTags, ShowTrends, SignOut, ToggleAppearance, severity_rank,
};

impl Render for Console {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx).clone();
        let body = match self.phase {
            Phase::SignIn | Phase::ChangePassword => sign_in::render(self, cx),
            Phase::Console => {
                let rate = self.rate.per_second(Instant::now());
                // Counted once: the navbar, status bar and page all show it.
                let unacknowledged = self.unacknowledged();
                let page = match self.page {
                    Page::Overview => overview::render(self, rate, unacknowledged, cx),
                    Page::Tags => tags::render(self, window.viewport_size().width, cx),
                    Page::Alarms => alarms::render(self, unacknowledged, cx),
                    Page::Trends => trends::render(self, cx),
                };
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(navbar::render(self, unacknowledged, cx))
                    .child(div().flex().flex_col().flex_1().min_h_0().child(page))
                    .child(status_bar::render(self, rate, unacknowledged, cx))
                    .into_any_element()
            }
        };

        div()
            .id("console")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.fg)
            .font_family(SANS)
            .text_size(FONT_BODY)
            .track_focus(&self.focus)
            .key_context(CONSOLE)
            .on_action(cx.listener(|console, _: &Quit, _, cx| {
                console.shutdown();
                cx.quit();
            }))
            .on_action(cx.listener(|console, _: &SignOut, window, cx| {
                if console.phase != Phase::SignIn {
                    console.sign_out(window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &ShowOverview, _, cx| {
                console.show(Page::Overview, cx);
            }))
            .on_action(cx.listener(|console, _: &ShowTags, _, cx| console.show(Page::Tags, cx)))
            .on_action(cx.listener(|console, _: &ShowAlarms, _, cx| {
                console.show(Page::Alarms, cx);
            }))
            .on_action(cx.listener(|console, _: &ShowTrends, _, cx| {
                console.show(Page::Trends, cx);
            }))
            .on_action(cx.listener(|console, _: &AcknowledgeAll, _, _| {
                if console.phase == Phase::Console {
                    console.acknowledge_all();
                }
            }))
            .on_action(cx.listener(|console, _: &NextTag, _, cx| {
                if console.phase == Phase::Console && console.page == Page::Tags {
                    console.step(1, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &PreviousTag, _, cx| {
                if console.phase == Phase::Console && console.page == Page::Tags {
                    console.step(-1, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &ToggleAppearance, _, cx| {
                console.toggle_appearance(cx);
            }))
            .child(body)
            .when_some(self.toast.as_ref(), |this, toast| {
                let color = if toast.error {
                    theme.danger
                } else {
                    theme.success
                };
                // Under the navbar and the page header, and left of the tag
                // detail panel when it is open, so it never covers a control.
                let panel_open = self.phase == Phase::Console
                    && self.page == Page::Tags
                    && self.selected.is_some();
                let right = if panel_open {
                    px(tags::PANEL) + SPACE_7
                } else {
                    SPACE_7
                };
                this.child(
                    div()
                        .absolute()
                        .top(TITLE_BAR_HEIGHT + px(68.0))
                        .right(right)
                        .max_w(px(440.0))
                        .flex()
                        .items_start()
                        .gap(SPACE_4)
                        .px(SPACE_6)
                        .py(SPACE_5)
                        .rounded(RADIUS_LG)
                        .bg(theme.elevated)
                        .border_1()
                        .border_color(color.alpha(0.4))
                        .shadow(theme.shadow(Elevation::Popover))
                        .child(vt_ui::icon(
                            if toast.error {
                                Icon::CircleAlert
                            } else {
                                Icon::CircleCheck
                            },
                            ICON_LG,
                            color,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(FONT_BODY)
                                .child(toast.message.clone()),
                        ),
                )
            })
    }
}

/// Wall-clock now in milliseconds, the time base of every chart.
pub(crate) fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

/// The colour of a value of this status: plain when normal, so problems
/// stand out.
pub(crate) fn value_color(status: Status, theme: &Theme) -> Hsla {
    match status {
        Status::Normal => theme.fg,
        Status::Warning => theme.warning,
        Status::Critical => theme.danger,
        Status::Offline | Status::Unknown => theme.fg_dim,
    }
}

/// The colour of a status pill or dot.
pub(crate) fn status_color(status: Status, theme: &Theme) -> Hsla {
    match status {
        Status::Normal => theme.success,
        Status::Warning => theme.warning,
        Status::Critical => theme.danger,
        Status::Offline | Status::Unknown => theme.fg_dim,
    }
}

/// Text that reads on a `fill` background: the theme's background or
/// foreground, whichever contrasts more with it (WCAG relative luminance).
pub(crate) fn text_on(fill: Hsla, theme: &Theme) -> Hsla {
    let luminance = |color: Hsla| {
        let rgb = color.to_rgb();
        let linear = |channel: f32| {
            if channel <= 0.039_28 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
    };
    let contrast = |a: f32, b: f32| (a.max(b) + 0.05) / (a.min(b) + 0.05);
    let fill = luminance(fill);
    if contrast(fill, luminance(theme.bg)) >= contrast(fill, luminance(theme.fg)) {
        theme.bg
    } else {
        theme.fg
    }
}

pub(crate) fn severity_color(severity: &str, theme: &Theme) -> Hsla {
    match severity_rank(severity) {
        0 => theme.danger,
        1 | 2 => theme.warning,
        _ => theme.info,
    }
}

/// `12 s`, `4 min`, `3 h`, `2 d`.
pub(crate) fn age(seconds: i64) -> String {
    match seconds.max(0) {
        seconds @ 0..60 => format!("{seconds} s"),
        seconds @ 60..3600 => format!("{} min", seconds / 60),
        seconds @ 3600..86_400 => format!("{} h", seconds / 3600),
        seconds => format!("{} d", seconds / 86_400),
    }
}

/// A span of time for a caption: `45 s`, `12 min`, `6 h`.
pub(crate) fn span_label(span_ms: f64) -> String {
    #[allow(clippy::cast_possible_truncation)]
    let seconds = (span_ms / 1000.0).round() as i64;
    age(seconds)
}

/// The span a small chart covers: as far back as its data reaches, so a
/// young history still fills it, within `[shortest, longest]`.
pub(crate) fn fit_span(reach_ms: Option<f64>, shortest: f64, longest: f64) -> f64 {
    reach_ms.map_or(longest, |reach| reach.clamp(shortest, longest))
}

/// The tag an alarm is about, by the name the console shows for it.
pub(crate) fn alarm_source(console: &Console, alarm: &Alarm) -> String {
    console
        .store
        .position(&alarm.source_key)
        .map(|index| console.store.tags[index].info.name.clone())
        .filter(|name| !name.is_empty())
        .or_else(|| (!alarm.source_name.is_empty()).then(|| alarm.source_name.clone()))
        .unwrap_or_else(|| alarm.source_key.clone())
}

/// What crossed which limit, with readable numbers: `Critical limit 85,
/// reached 86.40`. The server's own message carries full float precision.
pub(crate) fn alarm_summary(alarm: &Alarm) -> String {
    let severity = capitalized(&alarm.severity);
    let verb = if alarm.alarm_type.eq_ignore_ascii_case("low") {
        "fell to"
    } else {
        "reached"
    };
    format!(
        "{severity} limit {}, {verb} {}",
        format_value(alarm.threshold),
        format_value(alarm.value)
    )
}

pub(crate) fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars.as_str().chars()).collect()
    })
}

/// A Unix time on the operator's clock: `14:05:09`, or `3 Mar 14:05` when
/// it is not today.
pub(crate) fn local_time(unix_secs: i64, offset: UtcOffset) -> String {
    let Ok(at) = OffsetDateTime::from_unix_timestamp(unix_secs) else {
        return "—".into();
    };
    let at = at.to_offset(offset);
    let today = OffsetDateTime::now_utc().to_offset(offset).date();
    if at.date() == today {
        format!("{:02}:{:02}:{:02}", at.hour(), at.minute(), at.second())
    } else {
        let month = at.month().to_string();
        format!(
            "{} {} {:02}:{:02}",
            at.day(),
            month.get(..3).unwrap_or(&month),
            at.hour(),
            at.minute()
        )
    }
}

/// `plant.local:3000` of `http://plant.local:3000`.
pub(crate) fn host(url: &str) -> &str {
    url.split_once("://").map_or(url, |(_, rest)| rest)
}

/// The page's top bar: title, a line about it, and controls on the right.
pub(crate) fn page_header(
    title: &'static str,
    subtitle: impl Into<SharedString>,
    controls: Vec<AnyElement>,
    cx: &Context<Console>,
) -> Div {
    let theme = theme(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_6)
        .h(px(56.0))
        .px(SPACE_9)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(subtitle.into()),
                ),
        )
        .children(controls)
}

/// A bordered surface for a group of content.
pub(crate) fn card(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .rounded(RADIUS_LG)
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
}

/// A card's title row, with an optional count and control.
pub(crate) fn card_title(
    title: &'static str,
    count: Option<usize>,
    control: Option<AnyElement>,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_3)
        .h(px(40.0))
        .px(SPACE_7)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(FONT_EMPHASIS)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .when_some(count, |this, count| {
            this.child(
                div()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(count.to_string()),
            )
        })
        .child(div().flex_1())
        .children(control)
}

/// A value with its unit, the unit smaller and dimmer.
pub(crate) fn value_with_unit(
    value: String,
    unit: &str,
    size: gpui::Pixels,
    color: Hsla,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(SPACE_2)
        .min_w_0()
        .child(
            div()
                .font_family(MONO)
                .text_size(size)
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .whitespace_nowrap()
                .child(value),
        )
        .when(!unit.is_empty(), |this| {
            this.child(
                div()
                    .text_size(if size > FONT_TITLE {
                        FONT_BODY
                    } else {
                        FONT_CAPTION
                    })
                    .text_color(theme.fg_dim)
                    .whitespace_nowrap()
                    .child(unit.to_owned()),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_pick_the_largest_whole_unit() {
        assert_eq!(age(-3), "0 s");
        assert_eq!(age(59), "59 s");
        assert_eq!(age(61), "1 min");
        assert_eq!(age(7200), "2 h");
        assert_eq!(age(200_000), "2 d");
    }

    #[test]
    fn times_show_on_the_given_clock() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let offset = UtcOffset::from_hms(7, 0, 0).unwrap();
        assert_eq!(
            local_time(now, offset).len(),
            8,
            "today shows the time only"
        );
        assert_eq!(local_time(0, UtcOffset::UTC), "1 Jan 00:00");
        assert_eq!(host("https://scada.plant:3000"), "scada.plant:3000");
    }

    #[test]
    fn spans_and_alarm_lines_read_well() {
        assert_eq!(span_label(15.0 * 60_000.0), "15 min");
        assert_eq!(fit_span(Some(30_000.0), 120_000.0, 900_000.0), 120_000.0);
        assert_eq!(fit_span(Some(400_000.0), 120_000.0, 900_000.0), 400_000.0);
        assert_eq!(fit_span(None, 120_000.0, 900_000.0), 900_000.0);
        let alarm = Alarm {
            severity: "critical".into(),
            alarm_type: "high".into(),
            threshold: 85.0,
            value: 86.395_149_230_957_03,
            ..Alarm::default()
        };
        assert_eq!(alarm_summary(&alarm), "Critical limit 85, reached 86.40");
        assert_eq!(capitalized(""), "");
    }
}
