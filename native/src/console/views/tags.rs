//! Every tag in a virtualised table, and the chosen one in detail.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, div, px, uniform_list};
use vt_ui::Icon;
use vt_ui::chart::{Series, format_value, sparkline, trend_chart};
use vt_ui::components::{Button, IconButton, empty_state, field, pill, section_label, status_dot};
use vt_ui::theme::*;

use super::{age, card, now_ms, page_header, status_color, value_color, value_with_unit};
use crate::console::store::Status;
use crate::console::{Console, time_now_secs};

const KEY: f32 = 200.0;
const VALUE: f32 = 150.0;
const STATUS: f32 = 84.0;
const AGE: f32 = 64.0;
const SPARK: f32 = 120.0;
const PANEL: f32 = 360.0;
/// The table's sparklines and the panel's trend.
const SPARK_MS: f64 = 15.0 * 60_000.0;
const DETAIL_MS: f64 = 60.0 * 60_000.0;

pub fn render(console: &Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let visible = console.visible_tags(cx).len();
    let total = console.store.tags.len();
    let attention = console
        .store
        .tags
        .iter()
        .filter(|tag| tag.status.needs_attention())
        .count();
    let filtered = visible != total;

    let filter = div()
        .flex()
        .items_center()
        .gap(SPACE_3)
        .w(px(260.0))
        .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
        .child(div().flex_1().child(console.inputs.filter.clone()));

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
        .child(div().flex_1().min_w_0().child("NAME"))
        .child(div().w(px(KEY)).flex_none().child("KEY"))
        .child(div().w(px(VALUE)).flex_none().text_right().child("VALUE"))
        .child(div().w(px(STATUS)).flex_none().child("STATUS"))
        .child(div().w(px(AGE)).flex_none().text_right().child("AGE"))
        .child(div().w(px(SPARK)).flex_none().child("LAST 15 MIN"));

    let table = if visible == 0 {
        empty_state(
            Icon::Search,
            if total == 0 {
                "No tags yet"
            } else {
                "No tag matches"
            },
            if total == 0 {
                "The server's data sources appear here once it sends them."
            } else {
                "Nothing has this text in its name or key."
            },
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        uniform_list(
            "tags",
            visible,
            cx.processor(
                |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let rows = console.visible_tags(cx);
                    let now = now_ms();
                    let now_secs = time_now_secs();
                    range
                        .filter_map(|row| rows.get(row).copied())
                        .map(|index| tag_row(console, index, now, now_secs, &theme, cx))
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(console.tags_scroll.clone())
        .flex_1()
        .into_any_element()
    };

    let selected = console
        .selected
        .filter(|index| *index < console.store.tags.len());

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header(
            "Tags",
            if filtered {
                format!("{visible} of {total} tags match")
            } else {
                format!("{total} tags · {attention} need attention · ↑↓ to move")
            },
            vec![filter.into_any_element()],
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(header)
                        .child(table),
                )
                .when_some(selected, |this, index| {
                    this.child(detail(console, index, cx))
                }),
        )
        .into_any_element()
}

fn tag_row(
    console: &Console,
    index: usize,
    now: f64,
    now_secs: i64,
    theme: &Theme,
    cx: &Context<Console>,
) -> AnyElement {
    let tag = &console.store.tags[index];
    let selected = console.selected == Some(index);
    let hover = theme.hover;
    let color = status_color(tag.status, theme);
    let trended = console.trend.contains(&index);
    div()
        .id(ElementId::NamedInteger(
            "tag".into(),
            u64::try_from(index).unwrap_or(0),
        ))
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(34.0))
        .px(SPACE_9)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.selected))
        .when(!selected, |this| this.hover(move |style| style.bg(hover)))
        .on_click(cx.listener(move |console, _, _, cx| console.select(index, cx)))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(SPACE_4)
                .child(status_dot(color))
                .child(div().truncate().child(tag.info.name.clone()))
                .when(trended, |this| {
                    this.child(vt_ui::icon(Icon::ChartLine, ICON_SM, theme.fg_dim))
                }),
        )
        .child(
            div()
                .w(px(KEY))
                .flex_none()
                .truncate()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(tag.info.key.clone()),
        )
        .child(
            div()
                .w(px(VALUE))
                .flex_none()
                .flex()
                .justify_end()
                .child(value_with_unit(
                    tag.display(),
                    &tag.info.unit,
                    FONT_BODY,
                    value_color(tag.status, theme),
                    theme,
                )),
        )
        .child(
            div()
                .w(px(STATUS))
                .flex_none()
                .child(pill(tag.status.label(), color)),
        )
        .child(
            div()
                .w(px(AGE))
                .flex_none()
                .text_right()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(if tag.at_secs > 0 {
                    age(now_secs - tag.at_secs)
                } else {
                    "—".into()
                }),
        )
        .child(div().w(px(SPARK)).h(px(18.0)).flex_none().child(sparkline(
            tag.history.clone(),
            if tag.status == Status::Normal {
                theme.accent
            } else {
                color
            },
            now,
            SPARK_MS,
        )))
        .into_any_element()
}

fn detail(console: &Console, index: usize, cx: &mut Context<Console>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let tag = &console.store.tags[index];
    let color = status_color(tag.status, &theme);
    let trended = console.trend.contains(&index);
    let now = now_ms();
    let summary = tag.history.summary(now - DETAIL_MS);
    let unit = tag.info.unit.clone();
    let threshold = |value: f64| {
        if value > 0.0 {
            format!("{} {unit}", format_value(value))
                .trim_end()
                .to_owned()
        } else {
            "Off".to_owned()
        }
    };
    let stat = |value: Option<f64>| {
        value.map_or_else(
            || "—".to_owned(),
            |value| {
                format!("{} {unit}", format_value(value))
                    .trim_end()
                    .to_owned()
            },
        )
    };
    let updated = if tag.at_secs > 0 {
        format!("Updated {} ago", age(time_now_secs() - tag.at_secs))
    } else {
        "No value yet".to_owned()
    };
    let admin = console
        .user
        .as_ref()
        .is_some_and(|(_, role)| role.eq_ignore_ascii_case("admin"));
    let writing = console.writing == Some(index);
    let result = console
        .write_result
        .as_ref()
        .filter(|(at, _)| *at == index)
        .map(|(_, result)| result.clone());

    let facts = [
        ("Warning at", threshold(tag.info.warn_val)),
        ("Critical at", threshold(tag.info.crit_val)),
        ("Lowest, 1 h", stat(summary.map(|summary| summary.min))),
        ("Highest, 1 h", stat(summary.map(|summary| summary.max))),
        ("Average, 1 h", stat(summary.map(|summary| summary.mean))),
        (
            "Source",
            if tag.info.source_type.is_empty() {
                "—".to_owned()
            } else {
                tag.info.source_type.clone()
            },
        ),
    ];

    let trend_color = if tag.status == Status::Normal {
        theme.accent
    } else {
        color
    };
    let series = vec![Series {
        label: SharedString::from(tag.info.name.clone()),
        color: trend_color,
        history: Rc::clone(&tag.history),
        stepped: false,
    }];

    div()
        .id("tag-detail")
        .flex()
        .flex_col()
        .flex_none()
        .w(px(PANEL))
        .h_full()
        .overflow_y_scroll()
        .bg(theme.chrome)
        .border_l_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_start()
                .gap(SPACE_4)
                .px(SPACE_8)
                .pt(SPACE_8)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(SPACE_1)
                        .child(
                            div()
                                .text_size(FONT_TITLE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(tag.info.name.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(tag.info.key.clone()),
                        ),
                )
                .child(
                    IconButton::new("close-detail", Icon::X)
                        .tooltip("Close")
                        .on_click(cx.listener(|console, _, _, cx| {
                            console.selected = None;
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .px(SPACE_8)
                .py(SPACE_7)
                .child(value_with_unit(
                    tag.display(),
                    &tag.info.unit,
                    px(34.0),
                    value_color(tag.status, &theme),
                    &theme,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .child(pill(tag.status.label(), color))
                        .when(tag.simulated, |this| this.child(pill("SIMULATED", theme.info)))
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(updated),
                        ),
                ),
        )
        .child(
            div()
                .grid()
                .grid_cols(2)
                .gap(SPACE_5)
                .px(SPACE_8)
                .pb(SPACE_7)
                .children(facts.into_iter().map(|(label, value)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_1)
                        .min_w_0()
                        .child(
                            div()
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(label),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .child(value),
                        )
                })),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .px(SPACE_8)
                .pb(SPACE_8)
                .child(section_label("Last hour", cx))
                .child(
                    card(&theme)
                        .h(px(180.0))
                        .p(SPACE_4)
                        .child(trend_chart(series, now, DETAIL_MS, cx)),
                )
                .child(
                    Button::new(
                        "toggle-trend",
                        if trended {
                            "Remove from Trends"
                        } else {
                            "Add to Trends"
                        },
                    )
                    .icon(Icon::ChartLine)
                    .on_click(cx.listener(move |console, _, _, cx| {
                        console.toggle_trend(index, cx);
                    })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .px(SPACE_8)
                .py(SPACE_8)
                .border_t_1()
                .border_color(theme.border)
                .child(section_label("Write", cx))
                .map(|this| {
                    if admin {
                        this.child(field(
                            "New value",
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_3)
                                .child(div().flex_1().child(console.inputs.write.clone()))
                                .child(
                                    Button::new("write", if writing { "Writing…" } else { "Write" })
                                        .primary()
                                        .icon(Icon::Send)
                                        .disabled(writing)
                                        .on_click(cx.listener(|console, _, window, cx| {
                                            console.write_selected(window, cx);
                                        })),
                                ),
                            cx,
                        ))
                        .child(match result {
                            Some(Ok(detail)) => write_note(
                                Icon::CircleCheck,
                                format!("Written: {detail}"),
                                theme.success,
                            ),
                            Some(Err(error)) => write_note(Icon::CircleAlert, error, theme.danger),
                            None => write_note(
                                Icon::Info,
                                "Sent to the device through the server; the server must allow writes to this tag.".to_owned(),
                                theme.fg_dim,
                            ),
                        })
                    } else {
                        this.child(write_note(
                            Icon::Lock,
                            "Writing values needs the admin role.".to_owned(),
                            theme.fg_dim,
                        ))
                    }
                }),
        )
}

fn write_note(glyph: Icon, text: String, color: gpui::Hsla) -> impl IntoElement {
    div()
        .flex()
        .items_start()
        .gap(SPACE_3)
        .text_size(FONT_SMALL)
        .text_color(color)
        .child(div().pt(px(2.0)).child(vt_ui::icon(glyph, ICON_SM, color)))
        .child(div().flex_1().min_w_0().child(text))
}
