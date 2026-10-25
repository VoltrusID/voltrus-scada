//! Chosen tags on one time axis, over a range the server keeps history for.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, div, px};
use voltrus_scada_native::runtime::Link;
use vt_ui::Icon;
use vt_ui::chart::{Series, trend_chart};
use vt_ui::components::{IconButton, Segment, empty_state, section_label, segmented};
use vt_ui::theme::*;

use super::{card, now_ms, page_header, status_color};
use crate::console::{Console, RANGES, range_ms};

const PICKER: f32 = 260.0;

pub fn render(console: &Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let span = range_ms(console.range);
    let chosen: Vec<usize> = console
        .trend
        .iter()
        .copied()
        .filter(|index| *index < console.store.tags.len())
        .collect();
    // A live link scrolls the chart with the console's once-a-second tick
    // (redrawing the whole window every frame would cost a kiosk all day);
    // otherwise it holds still at the newest sample.
    let now = if console.link == Link::Live {
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
                label: SharedString::from(tag.info.name.clone()),
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
            let index = *index;
            let tag = &console.store.tags[index];
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
                        .child(tag.info.name.clone()),
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
                        ElementId::NamedInteger(
                            "untrend".into(),
                            u64::try_from(index).unwrap_or(0),
                        ),
                        Icon::X,
                    )
                    .size(px(18.0))
                    .tooltip("Remove from the chart")
                    .on_click(cx.listener(move |console, _, _, cx| {
                        console.toggle_trend(index, cx);
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
            "Pick tags on the right, or use Add to Trends on a tag's detail.",
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        trend_chart(series, now, span, cx).into_any_element()
    };

    let picker: Vec<_> = console
        .store
        .tags
        .iter()
        .enumerate()
        .map(|(index, tag)| {
            let position = chosen.iter().position(|chosen| *chosen == index);
            let hover = theme.hover;
            div()
                .id(ElementId::NamedInteger(
                    "pick".into(),
                    u64::try_from(index).unwrap_or(0),
                ))
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(28.0))
                .px(SPACE_4)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(move |console, _, _, cx| console.toggle_trend(index, cx)))
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
                                this.bg(color).border_color(color).child(vt_ui::icon(
                                    Icon::Check,
                                    px(10.0),
                                    theme.bg,
                                ))
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
                        .child(tag.info.name.clone()),
                )
                .child(
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(status_color(tag.status, &theme)),
                )
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header(
            "Trends",
            format!(
                "{} of {} tags · history from the server, live values added as they arrive",
                chosen.len(),
                console.store.tags.len()
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
                                        .child(chosen.len().to_string()),
                                ),
                        )
                        .child(
                            div()
                                .id("trend-picker")
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .px(SPACE_3)
                                .pb(SPACE_4)
                                .children(picker),
                        ),
                ),
        )
        .into_any_element()
}
