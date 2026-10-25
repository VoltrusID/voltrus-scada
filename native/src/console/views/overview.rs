//! The plant at a glance: counts, what needs attention, open alarms, and a
//! tile per tag.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, Hsla, SharedString, div, px};
use vt_ui::Icon;
use vt_ui::chart::{format_value, sparkline};
use vt_ui::components::{Button, pill, status_dot};
use vt_ui::theme::*;

use super::{
    age, alarm_source, alarm_summary, card, card_title, fit_span, local_time, now_ms, page_header,
    severity_color, span_label, status_color, value_color, value_with_unit,
};
use crate::console::store::{Status, Tag};
use crate::console::{Console, Page, is_shelved, time_now_secs};

/// Tiles drawn on the overview; the Tags page has the rest.
const TILES: usize = 48;
const ATTENTION_ROWS: usize = 8;
const ALARM_ROWS: usize = 6;
/// The sparklines reach back this far at most, and start where the data
/// does when it is younger.
const SPARK_MS: f64 = 15.0 * 60_000.0;
const SHORTEST_MS: f64 = 2.0 * 60_000.0;

pub fn render(console: &Console, rate: usize, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let store = &console.store;
    let critical = store.count(Status::Critical);
    let warning = store.count(Status::Warning);
    let offline = store.count(Status::Offline);
    let normal = store.count(Status::Normal);
    let unacknowledged = console.unacknowledged();
    let shelved = console
        .alarms
        .iter()
        .filter(|alarm| is_shelved(alarm))
        .count();
    let open = console.alarms.len() - shelved;
    let now_secs = time_now_secs();

    let kpis = div()
        .grid()
        .grid_cols(4)
        .gap(SPACE_6)
        .child(kpi(
            Icon::Database,
            "Tags",
            store.tags.len().to_string(),
            format!("{normal} normal · {offline} offline"),
            theme.fg,
            &theme,
        ))
        .child(kpi(
            Icon::CircleAlert,
            "Need attention",
            (critical + warning + offline).to_string(),
            format!("{critical} critical · {warning} warning"),
            if critical > 0 {
                theme.danger
            } else if warning + offline > 0 {
                theme.warning
            } else {
                theme.fg
            },
            &theme,
        ))
        .child(kpi(
            Icon::Zap,
            "Alarms to acknowledge",
            unacknowledged.to_string(),
            format!("{open} open · {shelved} shelved"),
            if unacknowledged > 0 {
                theme.danger
            } else {
                theme.fg
            },
            &theme,
        ))
        .child(kpi(
            Icon::Activity,
            "Updates per second",
            rate.to_string(),
            format!("{} link to the server", console.link.label()),
            theme.fg,
            &theme,
        ));

    // Worst first: critical, warning, then offline.
    let mut attention: Vec<usize> = (0..store.tags.len())
        .filter(|index| store.tags[*index].status.needs_attention())
        .collect();
    attention.sort_by_key(|index| match store.tags[*index].status {
        Status::Critical => 0,
        Status::Warning => 1,
        _ => 2,
    });
    let attention_count = attention.len();
    let attention_rows: Vec<AnyElement> = attention
        .into_iter()
        .take(ATTENTION_ROWS)
        .map(|index| attention_row(index, &store.tags[index], &theme, cx))
        .collect();

    let mut alarms: Vec<_> = console
        .alarms
        .iter()
        .filter(|alarm| !is_shelved(alarm))
        .collect();
    alarms.sort_by_key(|alarm| {
        (
            alarm.acknowledged_at.is_some(),
            crate::console::severity_rank(&alarm.severity),
            std::cmp::Reverse(alarm.triggered_at),
        )
    });
    let alarm_count = alarms.len();
    let alarm_rows: Vec<AnyElement> = alarms
        .into_iter()
        .take(ALARM_ROWS)
        .map(|alarm| {
            let id = alarm.id;
            let color = severity_color(&alarm.severity, &theme);
            let source = alarm_source(console, alarm);
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .h(px(48.0))
                .px(SPACE_7)
                .border_b_1()
                .border_color(theme.border.alpha(0.6))
                .child(div().w(px(3.0)).h(px(28.0)).rounded_full().bg(color))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(source),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(format!(
                                    "{} · {}",
                                    alarm_summary(alarm),
                                    local_time(alarm.triggered_at, console.utc_offset)
                                )),
                        ),
                )
                .map(|this| {
                    if alarm.acknowledged_at.is_none() {
                        this.child(
                            Button::new(
                                ElementId::NamedInteger("ack".into(), id.unsigned_abs()),
                                "Ack",
                            )
                            .compact()
                            .icon(Icon::Check)
                            .on_click(cx.listener(
                                move |console, _, _, _| {
                                    console.acknowledge(id);
                                },
                            )),
                        )
                    } else {
                        this.child(
                            div()
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child("Acknowledged"),
                        )
                    }
                })
                .into_any_element()
        })
        .collect();

    let attention_card = card(&theme)
        .child(card_title(
            "Needs attention",
            Some(attention_count),
            (attention_count > ATTENTION_ROWS).then(|| {
                Button::new("attention-all", "All tags")
                    .ghost()
                    .compact()
                    .on_click(cx.listener(|console, _, _, cx| console.show(Page::Tags, cx)))
                    .into_any_element()
            }),
            &theme,
        ))
        .map(|this| {
            if attention_rows.is_empty() {
                this.child(all_clear(
                    "Every tag is within its limits",
                    if store.tags.is_empty() {
                        "Waiting for the server's tag list."
                    } else {
                        "Warnings, critical values and offline devices show here."
                    },
                    &theme,
                ))
            } else {
                this.children(attention_rows)
            }
        });

    let alarms_card = card(&theme)
        .child(card_title(
            "Open alarms",
            Some(alarm_count),
            Some(
                Button::new("alarms-all", "View all")
                    .ghost()
                    .compact()
                    .on_click(cx.listener(|console, _, _, cx| console.show(Page::Alarms, cx)))
                    .into_any_element(),
            ),
            &theme,
        ))
        .map(|this| {
            if alarm_rows.is_empty() {
                this.child(all_clear(
                    "No open alarms",
                    "New alarms appear here as the server raises them.",
                    &theme,
                ))
            } else {
                this.children(alarm_rows)
            }
        });

    let now = now_ms();
    let span = fit_span(
        store
            .tags
            .iter()
            .filter_map(|tag| tag.span_ms(now))
            .reduce(f64::max),
        SHORTEST_MS,
        SPARK_MS,
    );
    let tiles: Vec<AnyElement> = store
        .tags
        .iter()
        .enumerate()
        .take(TILES)
        .map(|(index, tag)| tile(index, tag, now, span, now_secs, &theme, cx))
        .collect();
    let more = store.tags.len().saturating_sub(TILES);

    let header_controls = if unacknowledged > 0 {
        vec![
            Button::new("overview-ack-all", "Acknowledge all")
                .icon(Icon::Check)
                .on_click(cx.listener(|console, _, _, _| console.acknowledge_all()))
                .into_any_element(),
        ]
    } else {
        Vec::new()
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header(
            "Overview",
            format!(
                "{} tags · {} alarms to acknowledge",
                store.tags.len(),
                unacknowledged
            ),
            header_controls,
            cx,
        ))
        .child(
            div()
                .id("overview")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_8)
                        .p(SPACE_9)
                        .child(kpis)
                        .child(
                            div()
                                .grid()
                                .grid_cols(2)
                                .gap(SPACE_6)
                                .child(attention_card)
                                .child(alarms_card),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_3)
                                .child(
                                    div()
                                        .text_size(FONT_EMPHASIS)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Live values"),
                                )
                                .child(
                                    div()
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_dim)
                                        .child(format!("last {}", span_label(span))),
                                )
                                .child(div().flex_1())
                                .when(more > 0, |this| {
                                    this.child(
                                        Button::new("tiles-more", format!("{more} more in Tags"))
                                            .ghost()
                                            .compact()
                                            .on_click(cx.listener(|console, _, _, cx| {
                                                console.show(Page::Tags, cx);
                                            })),
                                    )
                                }),
                        )
                        .child(div().grid().grid_cols(4).gap(SPACE_6).children(tiles)),
                ),
        )
        .into_any_element()
}

fn kpi(
    glyph: Icon,
    label: &'static str,
    value: String,
    detail: String,
    color: Hsla,
    theme: &Theme,
) -> impl IntoElement {
    card(theme)
        .gap(SPACE_3)
        .p(SPACE_7)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(vt_ui::icon(glyph, ICON_MD, theme.fg_dim))
                .child(label),
        )
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_VALUE)
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(value),
        )
        .child(
            div()
                .truncate()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(detail),
        )
}

fn all_clear(title: &'static str, body: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_6)
        .px(SPACE_7)
        .py(SPACE_8)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(32.0))
                .rounded(RADIUS_LG)
                .bg(theme.success.alpha(0.12))
                .child(vt_ui::icon(Icon::CircleCheck, ICON_LG, theme.success)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(body),
                ),
        )
}

fn attention_row(index: usize, tag: &Tag, theme: &Theme, cx: &Context<Console>) -> AnyElement {
    let hover = theme.hover;
    let color = status_color(tag.status, theme);
    div()
        .id(ElementId::NamedInteger(
            "attention".into(),
            u64::try_from(index).unwrap_or(0),
        ))
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(40.0))
        .px(SPACE_7)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(move |console, _, _, cx| console.open_tag(index, cx)))
        .child(status_dot(color))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().truncate().child(tag.info.name.clone()))
                .child(
                    div()
                        .truncate()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(attention_detail(tag)),
                ),
        )
        .child(value_with_unit(
            tag.display(),
            &tag.info.unit,
            FONT_BODY,
            value_color(tag.status, theme),
            theme,
        ))
        .child(
            div()
                .w(px(64.0))
                .flex()
                .justify_end()
                .child(pill(tag.status.label(), color)),
        )
        .into_any_element()
}

fn attention_detail(tag: &Tag) -> String {
    let limit = |label: &str, value: f64| {
        format!("{label} limit {} {}", format_value(value), tag.info.unit)
            .trim_end()
            .to_owned()
    };
    let what = match tag.status {
        Status::Critical => limit("Critical", tag.info.crit_val),
        Status::Warning => limit("Warning", tag.info.warn_val),
        _ => "No values from the device".to_owned(),
    };
    if tag.info.key == tag.info.name {
        what
    } else {
        format!("{} · {what}", tag.info.key)
    }
}

fn tile(
    index: usize,
    tag: &Tag,
    now_ms: f64,
    span_ms: f64,
    now_secs: i64,
    theme: &Theme,
    cx: &Context<Console>,
) -> AnyElement {
    let border = if tag.status.needs_attention() {
        status_color(tag.status, theme).alpha(0.45)
    } else {
        theme.border
    };
    let hover = theme.border_strong;
    let line = if tag.status == Status::Normal {
        theme.accent
    } else {
        status_color(tag.status, theme)
    };
    let updated = if tag.at_secs > 0 {
        format!("{} ago", age(now_secs - tag.at_secs))
    } else {
        "no value yet".into()
    };
    div()
        .id(ElementId::NamedInteger(
            "tile".into(),
            u64::try_from(index).unwrap_or(0),
        ))
        .flex()
        .flex_col()
        .gap(SPACE_3)
        .min_w_0()
        .p(SPACE_6)
        .rounded(RADIUS_LG)
        .border_1()
        .border_color(border)
        .bg(theme.surface)
        .cursor_pointer()
        .hover(move |style| style.border_color(hover))
        .on_click(cx.listener(move |console, _, _, cx| console.open_tag(index, cx)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(tag.info.name.clone()),
                )
                .child(status_dot(status_color(tag.status, theme))),
        )
        .child(value_with_unit(
            tag.display(),
            &tag.info.unit,
            FONT_DISPLAY,
            value_color(tag.status, theme),
            theme,
        ))
        .child(
            div()
                .h(px(28.0))
                .child(sparkline(tag.history.clone(), line, now_ms, span_ms)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO)
                        .when(tag.info.key != tag.info.name, |this| {
                            this.child(SharedString::from(tag.info.key.clone()))
                        }),
                )
                .child(updated),
        )
        .into_any_element()
}
