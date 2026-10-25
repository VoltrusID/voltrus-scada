//! Every tag in a virtualised table, and the chosen one in detail.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Entity, FontWeight, Pixels, SharedString, Window, div, px,
    uniform_list,
};
use vt_ui::Icon;
use vt_ui::chart::{Series, format_value, sparkline, trend_chart};
use vt_ui::components::{Button, IconButton, empty_state, field, pill, section_label, status_dot};
use vt_ui::theme::*;

use super::{
    ListKeys, age, card, fit_span, focus_ring, keyboard_list, now_ms, page_header, rows_on_screen,
    span_label, status_color, tail_row, tail_rows, value_color, value_with_unit,
};
use crate::console::store::Status;
use crate::console::{Console, time_now_secs};
use vt_ui::text_input::TextInput;

/// The table's keyboard list.
pub(crate) const LIST: &str = "tags";
const ROW_HEIGHT_PX: f32 = 34.0;

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    Some(console.inputs.filter.clone())
}

pub fn side_panel(console: &Console) -> Pixels {
    if console.selected.is_some() {
        px(PANEL)
    } else {
        px(0.0)
    }
}

/// The tag on table row `row`, with the filter as it is.
fn row_tag(console: &Console, row: usize, cx: &gpui::App) -> Option<i64> {
    let index = *console.visible_tags(cx).get(row)?;
    Some(console.store.tags[index].info.id)
}

/// Moving the cursor opens its tag in the detail panel.
fn follow_cursor(console: &mut Console, row: usize, _: &mut Window, cx: &mut Context<Console>) {
    if let Some(id) = row_tag(console, row, cx) {
        console.select(id, cx);
    }
}

/// Enter opens the row's tag (and its write field, for an admin); Space
/// puts it on Trends or takes it off.
fn activate_row(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(id) = row_tag(console, row, cx) else {
        return;
    };
    if toggle {
        console.toggle_trend(id, cx);
    } else {
        console.select(id, cx);
        if console.is_admin() {
            console
                .inputs
                .write
                .update(cx, |field, _| field.focus(window));
        }
    }
}

const KEY: f32 = 180.0;
const VALUE: f32 = 120.0;
const STATUS: f32 = 84.0;
const AGE: f32 = 56.0;
const SPARK: f32 = 110.0;
pub(crate) const PANEL: f32 = 340.0;
/// The key follows the name only when the name column has this much room.
const INLINE_KEY_MIN: f32 = 260.0;
/// The table's sparklines reach back this far at most, the panel's trend
/// an hour; both start where the data does when it is younger.
const SPARK_MS: f64 = 15.0 * 60_000.0;
const DETAIL_MS: f64 = 60.0 * 60_000.0;
const SHORTEST_MS: f64 = 2.0 * 60_000.0;

/// Which optional columns fit the table's width, and how wide the name is:
/// text only ellipsizes inside a known width.
#[derive(Clone, Copy)]
struct Columns {
    key: bool,
    trend: bool,
    /// The name text's width, after its status dot.
    name: f32,
    /// The sparklines' span.
    span_ms: f64,
}

impl Columns {
    fn fit(table_width: f32, span_ms: f64) -> Self {
        let key = table_width >= 860.0;
        let trend = table_width >= 600.0;
        let mut fixed = VALUE + STATUS + AGE;
        let mut count = 4.0;
        if key {
            fixed += KEY;
            count += 1.0;
        }
        if trend {
            fixed += SPARK;
            count += 1.0;
        }
        let gaps = f32::from(SPACE_5) * (count - 1.0);
        let padding = f32::from(SPACE_8) * 2.0;
        // The status dot and its gap, and room for the list's scrollbar.
        let dot = 8.0 + f32::from(SPACE_4) + 8.0;
        Self {
            key,
            trend,
            name: (table_width - fixed - gaps - padding - dot).max(40.0),
            span_ms,
        }
    }
}

/// `width` is the window's width: the page spans it.
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let width = window.viewport_size().width;
    let theme = theme(cx).clone();
    // Filtered once per tag list and query (the store keeps it); the
    // list's rows index into it.
    let rows = console.visible_tags(cx);
    let visible = rows.len();
    let total = console.store.tags.len();
    let attention = console.store.attention_count();
    let filtered = visible != total;
    let selected = console.selected.and_then(|id| console.store.position(id));

    let table_width = if selected.is_some() {
        width - px(PANEL)
    } else {
        width
    };
    let now = now_ms();
    let columns = Columns::fit(
        f32::from(table_width),
        fit_span(console.store.span_ms(now), SHORTEST_MS, SPARK_MS),
    );

    let filter = div()
        .flex()
        .items_center()
        .gap(SPACE_3)
        .w(px(240.0))
        .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
        .child(div().flex_1().child(console.inputs.filter.clone()));

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
        .child(div().flex_1().min_w_0().child("NAME"))
        .when(columns.key, |this| {
            this.child(div().w(px(KEY)).flex_none().child("KEY"))
        })
        .child(div().w(px(VALUE)).flex_none().text_right().child("VALUE"))
        .child(div().w(px(STATUS)).flex_none().child("STATUS"))
        .child(div().w(px(AGE)).flex_none().text_right().child("AGE"))
        .when(columns.trend, |this| {
            this.child(div().w(px(SPARK)).flex_none().child(format!(
                "LAST {}",
                span_label(columns.span_ms).to_uppercase()
            )))
        });

    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: Some(follow_cursor),
            on_activate: activate_row,
        },
        visible,
        rows_on_screen(window, ROW_HEIGHT_PX),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let scroll = nav.scroll.clone();
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
            visible + tail_rows(visible, ROW_HEIGHT_PX),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let clock = (now_ms(), time_now_secs());
                    range
                        .map(|row| match rows.get(row).copied() {
                            Some(index) => {
                                let ring = focused && cursor == Some(row);
                                tag_row(console, index, row, columns, clock, ring, &theme, cx)
                            }
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

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header(
            "Tags",
            if filtered {
                format!("{visible} of {total} tags match")
            } else {
                format!(
                    "{total} tags · {attention} need attention · j/k or ↑↓ to move, / to filter"
                )
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
                        .child(list.flex_1().min_h_0().child(table)),
                )
                .when_some(selected, |this, index| {
                    this.child(detail(console, index, cx))
                }),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn tag_row(
    console: &Console,
    index: usize,
    row: usize,
    columns: Columns,
    (now, now_secs): (f64, i64),
    ring: bool,
    theme: &Theme,
    cx: &Context<Console>,
) -> AnyElement {
    let tag = &console.store.tags[index];
    let id = tag.info.id;
    let selected = console.selected == Some(id);
    let hover = theme.hover;
    let color = status_color(tag.status, theme);
    let trended = console.trend.contains(&id);
    // Without its own column the key follows the name, unless they match.
    let inline_key = !columns.key && tag.info.key != tag.info.name;
    div()
        .id(ElementId::NamedInteger(
            "tag".into(),
            u64::try_from(index).unwrap_or(0),
        ))
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(34.0))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.selected))
        .when(!selected, |this| this.hover(move |style| style.bg(hover)))
        .relative()
        .on_click(cx.listener(move |console, _, _, cx| {
            console.list(LIST, cx).select(row);
            console.select(id, cx);
        }))
        .when(ring, |this| this.child(focus_ring(theme)))
        .child({
            let gap = f32::from(SPACE_4);
            let room = columns.name
                - if trended {
                    f32::from(ICON_SM) + gap
                } else {
                    0.0
                };
            let (name_width, key_width) = if inline_key && room >= INLINE_KEY_MIN {
                let name_width = (room - gap) * 0.6;
                (name_width, Some(room - gap - name_width))
            } else {
                (room, None)
            };
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .items_center()
                .gap(SPACE_4)
                .child(status_dot(color))
                .child(
                    div()
                        .flex_none()
                        .w(px(name_width))
                        .truncate()
                        .child(tag.info.name.clone()),
                )
                .when_some(key_width, |this, key_width| {
                    this.child(
                        div()
                            .flex_none()
                            .w(px(key_width))
                            .truncate()
                            .font_family(MONO)
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child(tag.info.key.clone()),
                    )
                })
                .when(trended, |this| {
                    this.child(vt_ui::icon(Icon::ChartLine, ICON_SM, theme.fg_dim))
                })
        })
        .when(columns.key, |this| {
            this.child(
                div()
                    .w(px(KEY))
                    .flex_none()
                    .truncate()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(tag.info.key.clone()),
            )
        })
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
                .flex()
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
        .when(columns.trend, |this| {
            this.child(div().w(px(SPARK)).h(px(18.0)).flex_none().child(sparkline(
                tag.history.clone(),
                if tag.status == Status::Normal {
                    theme.accent
                } else {
                    color
                },
                now,
                columns.span_ms,
            )))
        })
        .into_any_element()
}

fn detail(console: &Console, index: usize, cx: &mut Context<Console>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let tag = &console.store.tags[index];
    let id = tag.info.id;
    let color = status_color(tag.status, &theme);
    let trended = console.trend.contains(&id);
    let now = now_ms();
    let span = fit_span(tag.span_ms(now), 5.0 * 60_000.0, DETAIL_MS);
    let span_text = span_label(span);
    let summary = tag.history.summary(now - span);
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
    let admin = console.is_admin();
    let writing = console.writing == Some(id);
    let result = console
        .write_result
        .as_ref()
        .filter(|(at, _)| *at == id)
        .map(|(_, result)| result.clone());
    let device = if tag.info.device_id.is_empty() {
        "—".to_owned()
    } else {
        match console.devices.get(&tag.info.device_id) {
            Some(link) if link.online => match link.latency_ms {
                Some(ms) => format!("{} · online, {ms} ms", tag.info.device_id),
                None => format!("{} · online", tag.info.device_id),
            },
            Some(_) => format!("{} · offline", tag.info.device_id),
            None => tag.info.device_id.clone(),
        }
    };

    let facts = [
        ("Warning at".to_owned(), threshold(tag.info.warn_val)),
        ("Critical at".to_owned(), threshold(tag.info.crit_val)),
        (
            format!("Lowest, {span_text}"),
            stat(summary.map(|summary| summary.min)),
        ),
        (
            format!("Highest, {span_text}"),
            stat(summary.map(|summary| summary.max)),
        ),
        (
            format!("Average, {span_text}"),
            stat(summary.map(|summary| summary.mean)),
        ),
        (
            "Source".to_owned(),
            if tag.info.source_type.is_empty() {
                "—".to_owned()
            } else {
                tag.info.source_type.clone()
            },
        ),
        ("Device".to_owned(), device),
        (
            "Alarm delays".to_owned(),
            format!(
                "on {} s · off {} s",
                tag.info.on_delay_secs, tag.info.off_delay_secs
            ),
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
                .child(section_label(format!("Last {span_text}"), cx))
                .child(
                    card(&theme)
                        .h(px(180.0))
                        .p(SPACE_4)
                        .child(trend_chart(series, now, span, cx)),
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
                        console.toggle_trend(id, cx);
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
