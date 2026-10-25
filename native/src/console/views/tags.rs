//! Every tag in a virtualised table, and the chosen one in detail
//! (`tag_detail`, across the page when widened).

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Entity, FontWeight, Pixels, Window, div, px, uniform_list,
};
use voltrus_scada_native::runtime::{CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{empty_state, pill, status_dot};
use vt_ui::theme::*;

use super::tag_detail::{DetailState, PANEL, TagDialog};
use super::{
    ListKeys, age, fit_span, focus_ring, keyboard_list, now_ms, page_header, rows_on_screen,
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

/// The answers to the detail panel's calls (history, notes, writes,
/// policies).
pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "tags.write" | "tags.policy" => super::tag_write::on_reply(console, key, reply, cx),
        _ => super::tag_detail::on_reply(console, key, reply, cx),
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

/// Enter opens the row's tag (and the write dialog, for an operator);
/// Space puts it on Trends or takes it off.
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
        if console.is_operator() {
            super::tag_detail::open_dialog(console, TagDialog::Write, window, cx);
        }
    }
}

const KEY: f32 = 180.0;
const VALUE: f32 = 120.0;
const STATUS: f32 = 84.0;
const AGE: f32 = 56.0;
const SPARK: f32 = 110.0;
/// Columns in a row's sparkline: one per two pixels.
const SPARK_COLUMNS: usize = 55;
/// The key follows the name only when the name column has this much room.
const INLINE_KEY_MIN: f32 = 260.0;
/// The table's sparklines reach back this far at most; they start where
/// the data does when it is younger.
const SPARK_MS: f64 = 15.0 * 60_000.0;
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
    // Widened, the detail takes the page and the table steps aside.
    let expanded = selected.is_some() && console.state::<DetailState>().expanded;

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
    let detail = selected.map(|index| super::tag_detail::panel(console, index, window, cx));

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
                .when(!expanded, |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(header)
                            .child(list.flex_1().min_h_0().child(table)),
                    )
                })
                .children(detail),
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
            this.child(
                div()
                    .w(px(SPARK))
                    .h(px(18.0))
                    .flex_none()
                    .child(super::spark::spark(
                        tag,
                        if tag.status == Status::Normal {
                            theme.accent
                        } else {
                            color
                        },
                        now,
                        columns.span_ms,
                        SPARK_COLUMNS,
                    )),
            )
        })
        .into_any_element()
}
