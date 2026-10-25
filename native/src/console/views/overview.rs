//! The plant at a glance: counts, what needs attention, open alarms, and a
//! tile per tag. A tile lights up when its value changes and its level bar
//! glides to the new value; the page asks for frames only while one does.
//!
//! The page redraws at the display's rate while tiles flash, so each frame
//! is kept cheap: counts and the attention list come ready from the store,
//! and tile rows scrolled well out of view are drawn as empty rows of the
//! same height (their positions are taken from the last frame's layout).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Context, ElementId, FontWeight, Hsla, Pixels, SharedString, Window,
    div, px,
};
use voltrus_scada_native::stress::PROFILE_TREND;
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::components::{Button, pill, status_dot};
use vt_ui::theme::*;

use super::{
    LIST_TAIL, ListKeys, age, alarm_source, alarm_summary, card, card_title, fit_span, focus_ring,
    keyboard_list, local_time, now_ms, page_header, severity_color, span_label, status_color,
    value_color, value_with_unit,
};
use crate::console::anim;
use crate::console::store::{Status, Tag};
use crate::console::{Console, Page, is_shelved, time_now_secs};

/// The tiles' keyboard list.
pub(crate) const LIST: &str = "overview";

/// Tiles drawn on the overview; the Tags page has the rest.
const TILES: usize = 48;
const COLUMNS: usize = 4;
const ATTENTION_ROWS: usize = 8;
const ALARM_ROWS: usize = 6;
/// The sparklines reach back this far at most, and start where the data
/// does when it is younger.
const SPARK_MS: f64 = 15.0 * 60_000.0;
const SHORTEST_MS: f64 = 2.0 * 60_000.0;
/// Columns in a tile's sparkline: about one per two pixels.
const SPARK_COLUMNS: usize = 110;
/// Room kept around a tile the keyboard cursor scrolls into view.
const SCROLL_MARGIN: f32 = 16.0;
/// The stress profile's trend: how far back, and samples kept per line.
const PROFILE_MS: f64 = 6_000.0;
/// Columns in each profile lane: about one per two pixels.
const PROFILE_COLUMNS: usize = 300;

/// Where the tile rows were laid out in the last frame, in the scrolling
/// content's coordinates (independent of the scroll offset).
#[derive(Default)]
struct Layout {
    /// Top and bottom of each tile row.
    rows: Vec<(f32, f32)>,
    /// The height of a row of tiles, as drawn; off-screen rows take it.
    row_height: Option<f32>,
}

#[derive(Default)]
struct State {
    layout: Rc<RefCell<Layout>>,
    /// The cursor at the last render, to scroll a moved one into view.
    cursor: Option<usize>,
    /// The stress profile's trend lines, sampled every frame.
    profile: Vec<VecDeque<(f64, f64)>>,
}

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
}

fn open_tile(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(id) = console.store.tags.get(row).map(|tag| tag.info.id) {
        console.open_tag(id, window, cx);
    }
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let rate = console.rate.per_second(Instant::now());
    let unacknowledged = console.unacknowledged();
    let tile_count = console.store.tags.len().min(TILES);
    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: open_tile,
        },
        tile_count,
        COLUMNS * 3,
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let scroll = console.scroll(LIST);
    let now = now_ms();
    let profile = console
        .stress
        .as_ref()
        .is_some_and(|run| run.plan.options.profile);
    let trend_lines = profile.then(|| profile_series(console, &theme, now));
    let state = console.state::<State>();
    let moved = std::mem::replace(&mut state.cursor, cursor) != cursor;
    let layout = Rc::clone(&state.layout);
    let tile_rows = tile_count.div_ceil(COLUMNS);
    let (shown, spacer) = shown_rows(
        &layout.borrow(),
        tile_rows,
        &scroll,
        cursor.filter(|_| moved && focused).map(|at| at / COLUMNS),
    );
    let store = &console.store;
    let critical = store.count(Status::Critical);
    let warning = store.count(Status::Warning);
    let offline = store.count(Status::Offline);
    let normal = store.count(Status::Normal);
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

    // Worst first: critical, warning, then offline; kept so by the store.
    let attention_count = store.attention_count();
    let attention_rows: Vec<AnyElement> = store
        .attention()
        .take(ATTENTION_ROWS)
        .map(|index| attention_row(&store.tags[index], &theme, cx))
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
                    .on_click(cx.listener(|console, _, window, cx| {
                        console.show(Page::Tags, window, cx);
                    }))
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
                    .on_click(cx.listener(|console, _, window, cx| {
                        console.show(Page::Alarms, window, cx);
                    }))
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

    let instant = Instant::now();
    let span = fit_span(store.span_ms(now), SHORTEST_MS, SPARK_MS);
    // A profile run's trend moves on every frame.
    let mut animating = profile;
    let mut rows: Vec<AnyElement> = Vec::with_capacity(tile_rows);
    for (row, shown) in shown.iter().enumerate() {
        if let (false, Some(height)) = (*shown, spacer) {
            // Off screen: an empty row of the same height. What flashes
            // there is not drawn, so it asks for no frames either.
            rows.push(div().h(px(height)).into_any_element());
            continue;
        }
        let first = row * COLUMNS;
        let mut tiles = Vec::with_capacity(COLUMNS);
        for (index, tag) in store
            .tags
            .iter()
            .enumerate()
            .take(tile_count)
            .skip(first)
            .take(COLUMNS)
        {
            let look = TileLook::at(tag, instant);
            animating |= look.flash.is_some() || look.tweening;
            tiles.push(tile(
                index,
                tag,
                look,
                (now, span, now_secs),
                focused && cursor == Some(index),
                &theme,
                cx,
            ));
        }
        rows.push(
            div()
                .grid()
                .grid_cols(COLUMNS as u16)
                .gap(SPACE_6)
                .children(tiles)
                .into_any_element(),
        );
    }
    let record_layout = {
        let scroll = scroll.clone();
        move |bounds: Vec<Bounds<Pixels>>, _: &mut Window, _: &mut App| {
            let viewport = scroll.bounds();
            let offset = scroll.offset();
            let mut layout = layout.borrow_mut();
            layout.rows.clear();
            for (row, bounds) in bounds.iter().enumerate() {
                let top = f32::from(bounds.top() - viewport.top() - offset.y);
                let height = f32::from(bounds.size.height);
                layout.rows.push((top, top + height));
                if shown.get(row).copied().unwrap_or(false) {
                    layout.row_height = Some(height);
                }
            }
        }
    };
    let more = store.tags.len().saturating_sub(TILES);
    let tag_total = store.tags.len();

    let header_controls = if unacknowledged > 0 {
        vec![
            Button::new("overview-ack-all", "Acknowledge all")
                .icon(Icon::Check)
                // The same action as ⇧⌘A: it asks before acknowledging.
                .on_click(cx.listener(|_, _, window, cx| {
                    window.dispatch_action(Box::new(crate::console::keys::AcknowledgeAll), cx);
                }))
                .into_any_element(),
        ]
    } else {
        Vec::new()
    };

    if animating {
        anim::request_frame(window, &console.meter);
    }
    let trend_card = trend_lines.map(|lines| profile_card(console, lines, &theme));
    // The profile shows what moves, its trend and the tiles, in place of
    // the attention and alarm cards (the counts above still give them).
    let cards = (!profile).then(|| {
        div()
            .grid()
            .grid_cols(2)
            .gap(SPACE_6)
            .child(attention_card)
            .child(alarms_card)
    });

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header(
            "Overview",
            format!("{tag_total} tags · {unacknowledged} alarms to acknowledge"),
            header_controls,
            cx,
        ))
        .child(
            div()
                .id("overview")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_8)
                        .p(SPACE_9)
                        .pb(LIST_TAIL)
                        .child(kpis)
                        .children(cards)
                        .children(trend_card)
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
                                .child(div().text_size(FONT_SMALL).text_color(theme.fg_dim).child(
                                    format!("last {} · ↑↓ or j/k, ↵ opens", span_label(span)),
                                ))
                                .child(div().flex_1())
                                .when(more > 0, |this| {
                                    this.child(
                                        Button::new("tiles-more", format!("{more} more in Tags"))
                                            .ghost()
                                            .compact()
                                            .on_click(cx.listener(|console, _, window, cx| {
                                                console.show(Page::Tags, window, cx);
                                            })),
                                    )
                                }),
                        )
                        .child(
                            list.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(SPACE_6)
                                    .on_children_prepainted(record_layout)
                                    .children(rows),
                            ),
                        ),
                ),
        )
        .into_any_element()
}

/// Which tile rows to draw, scrolling the keyboard cursor's row (`reveal`)
/// into view first, and the height an undrawn row takes. Every row is drawn
/// until a frame has laid them out.
fn shown_rows(
    layout: &Layout,
    rows: usize,
    scroll: &gpui::ScrollHandle,
    reveal: Option<usize>,
) -> (Vec<bool>, Option<f32>) {
    let viewport = f32::from(scroll.bounds().size.height);
    let mut offset = scroll.offset();
    if let Some(&(top, bottom)) = reveal.and_then(|row| layout.rows.get(row))
        && let Some(y) = reveal_offset(
            (top, bottom),
            f32::from(offset.y),
            viewport,
            f32::from(scroll.max_offset().height),
        )
    {
        offset.y = px(y);
        scroll.set_offset(offset);
    }
    let shown = match layout.row_height {
        Some(_) if viewport > 0.0 => on_screen(&layout.rows, rows, f32::from(offset.y), viewport),
        _ => vec![true; rows],
    };
    (shown, layout.row_height)
}

/// Rows within a quarter screen of the viewport (`offset_y`, the scroll
/// offset, is 0 or negative); rows not laid out yet count as on screen.
/// The offset is the current one (a scroll redraws the page), so the margin
/// only covers a row growing into view between two frames.
fn on_screen(laid_out: &[(f32, f32)], rows: usize, offset_y: f32, viewport: f32) -> Vec<bool> {
    let margin = viewport * 0.25;
    (0..rows)
        .map(|row| {
            laid_out.get(row).is_none_or(|&(top, bottom)| {
                bottom + offset_y >= -margin && top + offset_y <= viewport + margin
            })
        })
        .collect()
}

/// The scroll offset that brings a row (`top`, `bottom` in content
/// coordinates) fully into view with a margin, if it is not; within the
/// scrollable range (`max`, how far it scrolls).
fn reveal_offset((top, bottom): (f32, f32), offset_y: f32, viewport: f32, max: f32) -> Option<f32> {
    if viewport <= 0.0 {
        return None;
    }
    let target = if top + offset_y < 0.0 {
        SCROLL_MARGIN - top
    } else if bottom + offset_y > viewport {
        viewport - bottom - SCROLL_MARGIN
    } else {
        return None;
    };
    Some(target.clamp(-max.max(0.0), 0.0))
}

/// The stress profile's trend lines, with this frame's values added: drawn
/// at the display's rate, each line gains a point whenever its tag moved.
/// Kept by the page, oldest first, the last [`PROFILE_MS`] of them.
fn profile_series(console: &mut Console, theme: &Theme, now: f64) -> Vec<ProfileLine> {
    let tags: Vec<(SharedString, f64)> = PROFILE_TREND
        .filter_map(|index| console.store.tags.get(index))
        .map(|tag| (tag.label.clone(), tag.value))
        .collect();
    let lines = &mut console.state::<State>().profile;
    lines.resize_with(tags.len(), VecDeque::new);
    tags.into_iter()
        .zip(lines.iter_mut())
        .enumerate()
        .map(|(lane, ((label, value), samples))| {
            if samples
                .back()
                .is_none_or(|(_, last): &(f64, f64)| last.to_bits() != value.to_bits())
            {
                samples.push_back((now, value));
            }
            while samples
                .front()
                .is_some_and(|(at, _)| *at < now - PROFILE_MS)
            {
                samples.pop_front();
            }
            ProfileLine {
                label,
                color: theme.series(lane),
                columns: super::spark::columns(
                    samples.iter().copied(),
                    now - PROFILE_MS,
                    PROFILE_MS,
                    PROFILE_COLUMNS,
                ),
            }
        })
        .collect()
}

/// One lane of the profile's trend.
struct ProfileLine {
    label: SharedString,
    color: Hsla,
    columns: Vec<super::spark::Column>,
}

/// The stress profile's card: a lane per line, drawn with quads like the
/// sparklines, and what the frames cost.
fn profile_card(console: &Console, lines: Vec<ProfileLine>, theme: &Theme) -> AnyElement {
    let figures = {
        let meter = console.meter.borrow();
        let cpu = meter.stats.cpu();
        format!(
            "{} fps · display {:.0} Hz · build p95 {:.1} ms",
            meter.fps(),
            1000.0 / meter.stats.refresh_ms(),
            cpu.p95
        )
    };
    card(theme)
        .h(px(180.0))
        .child(card_title(
            "Live trend, drawn every frame",
            None,
            Some(
                div()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(figures)
                    .into_any_element(),
            ),
            theme,
        ))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .p(SPACE_6)
                .children(lines.into_iter().map(|line| {
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .items_center()
                        .gap(SPACE_5)
                        .child(
                            div()
                                .w(px(120.0))
                                .flex_none()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(line.label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .child(super::spark::bars(line.columns, line.color)),
                        )
                })),
        )
        .into_any_element()
}

/// How a tile looks at this instant: a change flash fading out, and its
/// level bar between the old and the new value.
struct TileLook {
    /// 1 right after a change, fading to 0.
    flash: Option<f32>,
    /// The bar's fill, 0 to 1, when the tag has a range.
    level: Option<f64>,
    tweening: bool,
}

impl TileLook {
    fn at(tag: &Tag, now: Instant) -> Self {
        let level = Tag::level(tag.value, &tag.info);
        let Some(changed) = tag.changed_at else {
            return Self {
                flash: None,
                level,
                tweening: false,
            };
        };
        let flash = anim::progress(changed, now, anim::FLASH).map(|t| 1.0 - t);
        let tween = anim::progress(changed, now, anim::TWEEN);
        let level = match (tween, Tag::level(tag.previous, &tag.info), level) {
            (Some(t), Some(from), Some(to)) => Some(anim::lerp(from, to, anim::ease_out(t))),
            (_, _, level) => level,
        };
        Self {
            flash,
            level,
            tweening: tween.is_some(),
        }
    }
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

fn attention_row(tag: &Tag, theme: &Theme, cx: &Context<Console>) -> AnyElement {
    let hover = theme.hover;
    let color = status_color(tag.status, theme);
    let id = tag.info.id;
    div()
        .id(ElementId::NamedInteger(
            "attention".into(),
            id.unsigned_abs(),
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
        .on_click(cx.listener(move |console, _, window, cx| console.open_tag(id, window, cx)))
        .child(status_dot(color))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().truncate().child(tag.label.clone()))
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
    row: usize,
    tag: &Tag,
    look: TileLook,
    (now_ms, span_ms, now_secs): (f64, f64, i64),
    ring: bool,
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
    let background = match look.flash {
        Some(strength) => anim::mix(theme.surface, theme.accent_soft, strength * 0.9),
        None => theme.surface,
    };
    let updated = if tag.at_secs > 0 {
        format!("{} ago", age(now_secs - tag.at_secs))
    } else {
        "no value yet".into()
    };
    let id = tag.info.id;
    div()
        .id(ElementId::NamedInteger("tile".into(), id.unsigned_abs()))
        .relative()
        .flex()
        .flex_col()
        .gap(SPACE_3)
        .min_w_0()
        .p(SPACE_6)
        .rounded(RADIUS_LG)
        .border_1()
        .border_color(border)
        .bg(background)
        .cursor_pointer()
        .hover(move |style| style.border_color(hover))
        .on_click(cx.listener(move |console, _, window, cx| {
            console.list(LIST, cx).select(row);
            console.open_tag(id, window, cx);
        }))
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
                        .child(tag.label.clone()),
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
        // Always there, empty without a range, so every tile is as tall and
        // rows scrolled away can be stood in for by their height.
        .child({
            #[allow(clippy::cast_possible_truncation)]
            let fill = look.level.map(|level| level as f32);
            div()
                .h(px(3.0))
                .w_full()
                .rounded_full()
                .when(fill.is_some(), |this| this.bg(theme.border))
                .when_some(fill, |this, fill| {
                    this.child(
                        div()
                            .h_full()
                            .w(gpui::relative(fill))
                            .rounded_full()
                            .bg(line),
                    )
                })
        })
        .child(div().h(px(28.0)).child(super::spark::spark(
            tag,
            line,
            now_ms,
            span_ms,
            SPARK_COLUMNS,
        )))
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
                            this.child(tag.key_label.clone())
                        }),
                )
                .child(updated),
        )
        .when(ring, |this| this.child(focus_ring(theme)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_well_off_screen_are_not_drawn() {
        // Rows 100 px tall from y = 400, a 600 px viewport.
        let rows: Vec<(f32, f32)> = (0..12)
            .map(|row| {
                let top = 400.0 + 124.0 * row as f32;
                (top, top + 100.0)
            })
            .collect();
        let at_top = on_screen(&rows, 12, 0.0, 600.0);
        // Up to 600 + 150 px down: rows starting at 400, 524, 648.
        assert_eq!(at_top.iter().filter(|shown| **shown).count(), 3);
        assert!(at_top[0] && at_top[2] && !at_top[3]);
        let scrolled = on_screen(&rows, 12, -1200.0, 600.0);
        assert!(!scrolled[0] && !scrolled[4], "far above");
        assert!(scrolled[5] && scrolled[11], "near and in view");
        // A row not laid out yet (the list grew) is drawn.
        assert!(on_screen(&rows, 14, -2000.0, 600.0)[13]);
    }

    #[test]
    fn the_cursor_row_scrolls_into_view_with_a_margin() {
        // In view: nothing to do.
        assert_eq!(reveal_offset((100.0, 200.0), 0.0, 600.0, 2000.0), None);
        // Below: its bottom lands a margin above the viewport's.
        assert_eq!(
            reveal_offset((900.0, 1000.0), 0.0, 600.0, 2000.0),
            Some(600.0 - 1000.0 - SCROLL_MARGIN)
        );
        // Above: its top lands a margin below the viewport's.
        assert_eq!(
            reveal_offset((100.0, 200.0), -500.0, 600.0, 2000.0),
            Some(SCROLL_MARGIN - 100.0)
        );
        // Never past the ends.
        assert_eq!(reveal_offset((5.0, 105.0), -50.0, 600.0, 2000.0), Some(0.0));
        assert_eq!(
            reveal_offset((2500.0, 2600.0), 0.0, 600.0, 1500.0),
            Some(-1500.0)
        );
        assert_eq!(
            reveal_offset((0.0, 10.0), -50.0, 0.0, 100.0),
            None,
            "not laid out"
        );
    }
}
