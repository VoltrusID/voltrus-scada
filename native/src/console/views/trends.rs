//! Chosen tags on one time axis, over a range the server keeps history for.
//! While the link is live the chart scrolls continuously at the display's
//! rate: the page asks for the next frame only while it is on screen and
//! live, so an idle or hidden Trends page draws nothing.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Entity, FontWeight, Pixels, Window, div, px, uniform_list,
};
use voltrus_scada_native::runtime::Link;
use vt_ui::Icon;
use vt_ui::chart::{Series, trend_chart};
use vt_ui::components::{IconButton, Segment, empty_state, section_label, segmented};
use vt_ui::text_input::TextInput;
use vt_ui::theme::*;

use super::{
    ListKeys, card, focus_ring, keyboard_list, now_ms, page_header, rows_on_screen, status_color,
    tail_row, tail_rows,
};
use crate::console::{Console, RANGES, anim, range_ms};

pub(crate) const PICKER: f32 = 260.0;
/// The picker's keyboard list.
pub(crate) const LIST: &str = "trends";
const PICK_ROW: f32 = 28.0;

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    Some(console.inputs.trend_filter.clone())
}

pub fn side_panel(_: &Console) -> Pixels {
    px(PICKER)
}

/// The picker's tags, as store positions, matching its filter; kept by the
/// store, so the chart's frames do not filter every tag again.
fn picks(console: &Console, cx: &gpui::App) -> Rc<[usize]> {
    console
        .store
        .filter(console.inputs.trend_filter.read(cx).value())
}

/// Enter or Space puts the row's tag on the chart or takes it off.
fn toggle_row(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(id) = picks(console, cx)
        .get(row)
        .map(|index| console.store.tags[*index].info.id)
    {
        console.toggle_trend(id, cx);
    }
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let span = range_ms(console.range);
    let chosen: Vec<usize> = console
        .trend
        .iter()
        .filter_map(|id| console.store.position(*id))
        .collect();
    let live = console.link == Link::Live && !chosen.is_empty();
    // Live: the chart moves on every frame. Otherwise it holds still at the
    // newest sample.
    let now = if live {
        anim::request_frame(window, &console.meter);
        now_ms()
    } else {
        chosen
            .iter()
            .filter_map(|index| console.store.tags[*index].history.latest())
            .map(|(at, _)| at)
            .reduce(f64::max)
            .unwrap_or_else(now_ms)
    };

    let series: Vec<Series> = chosen
        .iter()
        .enumerate()
        .map(|(position, index)| {
            let tag = &console.store.tags[*index];
            Series {
                label: tag.label.clone(),
                color: theme.series(position),
                history: tag.history.clone(),
                stepped: false,
            }
        })
        .collect();

    let legend: Vec<AnyElement> = chosen
        .iter()
        .enumerate()
        .map(|(position, index)| {
            let tag = &console.store.tags[*index];
            let id = tag.info.id;
            let mut value = tag.display();
            if !tag.info.unit.is_empty() && tag.value.is_finite() {
                value = format!("{value} {}", tag.info.unit);
            }
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_3)
                .pl(SPACE_4)
                .pr(SPACE_2)
                .h(px(26.0))
                .rounded(RADIUS_MD)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(
                    div()
                        .size(px(8.0))
                        .rounded_full()
                        .bg(theme.series(position)),
                )
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(tag.label.clone()),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(super::value_color(tag.status, &theme))
                        .child(value),
                )
                .child(
                    IconButton::new(
                        ElementId::NamedInteger("untrend".into(), id.unsigned_abs()),
                        Icon::X,
                    )
                    .size(px(18.0))
                    .tooltip("Remove from the chart")
                    .on_click(cx.listener(move |console, _, _, cx| {
                        console.toggle_trend(id, cx);
                    })),
                )
                .into_any_element()
        })
        .collect();

    let ranges = RANGES
        .into_iter()
        .map(|(range, label)| {
            Segment::new(
                label,
                console.range == range,
                cx.listener(move |console, _, _, cx| console.set_range(range, cx)),
            )
        })
        .collect();

    let chart = if series.is_empty() {
        empty_state(
            Icon::ChartLine,
            "Choose tags to trend",
            "Pick tags on the right (Space toggles the row), or use Add to Trends on a tag's detail.",
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        trend_chart(series, now, span, cx).into_any_element()
    };

    // The picker: every tag matching its filter, virtualised.
    let rows = picks(console, cx);
    let shown = rows.len();
    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: toggle_row,
        },
        shown,
        rows_on_screen(window, PICK_ROW),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let scroll = nav.scroll.clone();
    let picker =
        uniform_list(
            "trend-picker",
            shown + tail_rows(shown, PICK_ROW),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    let chosen: Vec<i64> = console.trend.iter().copied().collect();
                    range
                        .map(|row| {
                            let Some(index) = rows.get(row).copied() else {
                                return tail_row(PICK_ROW);
                            };
                            let tag = &console.store.tags[index];
                            let id = tag.info.id;
                            let position = chosen.iter().position(|chosen| *chosen == id);
                            let hover = theme.hover;
                            let ring = focused && cursor == Some(row);
                            div()
                                .id(ElementId::NamedInteger("pick".into(), id.unsigned_abs()))
                                .relative()
                                .flex()
                                .items_center()
                                .gap(SPACE_4)
                                .h(px(PICK_ROW))
                                .px(SPACE_4)
                                .rounded(RADIUS_MD)
                                .cursor_pointer()
                                .hover(move |style| style.bg(hover))
                                .on_click(cx.listener(move |console, _, _, cx| {
                                    console.list(LIST, cx).select(row);
                                    console.toggle_trend(id, cx);
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .size(px(14.0))
                                        .rounded(RADIUS_SM)
                                        .border_1()
                                        .map(|this| match position {
                                            Some(position) => {
                                                let color = theme.series(position);
                                                this.bg(color).border_color(color).child(
                                                    vt_ui::icon(Icon::Check, px(10.0), theme.bg),
                                                )
                                            }
                                            None => this.border_color(theme.border_strong),
                                        }),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(if position.is_some() {
                                            theme.fg
                                        } else {
                                            theme.fg_muted
                                        })
                                        .child(tag.label.clone()),
                                )
                                .child(
                                    div()
                                        .size(px(6.0))
                                        .rounded_full()
                                        .bg(status_color(tag.status, &theme)),
                                )
                                .when(ring, |this| this.child(focus_ring(&theme)))
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .px(SPACE_3)
        .size_full();

    let chosen_count = chosen.len();
    let total = console.store.tags.len();
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header(
            "Trends",
            format!(
                "{chosen_count} of {total} tags · history from the server, live values added as they arrive"
            ),
            vec![segmented("trend-range", ranges, cx)],
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
                        .gap(SPACE_6)
                        .p(SPACE_9)
                        .when(!legend.is_empty(), |this| {
                            this.child(div().flex().flex_wrap().gap(SPACE_3).children(legend))
                        })
                        .child(card(&theme).flex_1().min_h_0().p(SPACE_6).child(chart)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .w(px(PICKER))
                        .h_full()
                        .bg(theme.chrome)
                        .border_l_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .h(px(40.0))
                                .px(SPACE_7)
                                .child(section_label("Tags on the chart", cx))
                                .child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(FONT_CAPTION)
                                        .text_color(theme.fg_dim)
                                        .child(chosen_count.to_string()),
                                ),
                        )
                        .child(
                            div()
                                .px(SPACE_5)
                                .pb(SPACE_4)
                                .child(console.inputs.trend_filter.clone()),
                        )
                        .child(list.flex_1().min_h_0().child(picker)),
                ),
        )
        .into_any_element()
}
