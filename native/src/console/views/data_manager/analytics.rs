//! Data Manager › Analytics, the web client's `AnalyticsPanel.tsx`: one
//! device metric over a range, as buckets (each bucket's low-to-high span
//! as a bar, its average marked and joined), the range's figures, and a
//! comparison of the range's recent half with the half before it. The
//! metrics are the chosen device's tags. Open to every user.
//!
//! The chart follows the pointer with a crosshair at the display's rate
//! and reads out the bucket under it. Keys on the chart: ←/→ move between
//! the bars, Enter drills into the bar (its span with finer buckets), ⌫ or
//! Esc comes back out, ↑/↓ change the metric. Anywhere in the section:
//! `,`/`.` step through the devices, `[`/`]` the ranges, `m` switches
//! Aggregate and Compare, `r` fetches again, `e` saves the buckets as CSV.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, ContentMask, Context, FocusHandle, FontWeight, Hsla, MouseMoveEvent,
    PathBuilder, Pixels, Point, SharedString, TextRun, Window, actions, canvas, div, fill, font,
    point, px, size, uniform_list,
};
use time::UtcOffset;
use voltrus_scada_native::api::insights::{
    self, ANALYTICS_RANGES, Bucket, ComparisonResult, Overall, buckets_csv, halves, overall,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::chart::{format_value, value_scale};
use vt_ui::components::{Button, IconButton, Segment, empty_state, kbd, pill, segmented};
use vt_ui::theme::*;

use super::insights_kit::{self, bar, chips, device_tags, devices, figure};
use crate::console::keys::Dismiss;
use crate::console::views::ops_kit::notice;
use crate::console::views::tag_chart::{tick_label, tick_step, ticks};
use crate::console::views::{
    ListKeys, card, focus_ring, keyboard_list, local_time, page_header, rows_on_screen, tail_row,
    tail_rows,
};
use crate::console::{Console, time_now_secs};

/// The section's id: its calls are `data.analytics.*`.
pub(crate) const ID: &str = "analytics";
/// The section root's key context; its keys never fire while typing.
pub const ANALYTICS: &str = "Analytics && !TextInput";
/// The chart's key context.
pub const ANALYTICS_CHART: &str = "AnalyticsChart";
const METRICS: &str = "data.analytics.metrics";
const METRIC_ROW: f32 = 40.0;
const PICKER: f32 = 240.0;
const GUTTER: f32 = 56.0;
const TIME_STRIP: f32 = 18.0;
const TOP: f32 = 10.0;

/// A device, the tag list it was read from, and its tags' positions.
type DeviceTags = (SharedString, Rc<[usize]>, Rc<Vec<usize>>);

actions!(
    voltrus_console,
    [
        AnalyticsPreviousBar,
        AnalyticsNextBar,
        AnalyticsFirstBar,
        AnalyticsLastBar,
        AnalyticsDrill,
        AnalyticsBack,
        AnalyticsPreviousMetric,
        AnalyticsNextMetric,
        AnalyticsPreviousDevice,
        AnalyticsNextDevice,
        AnalyticsPreviousRange,
        AnalyticsNextRange,
        AnalyticsSwitchMode,
        AnalyticsRefresh,
        AnalyticsExport,
    ]
);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Aggregate,
    Compare,
}

/// A span of time and its bucket size, in Unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub from: i64,
    pub to: i64,
    pub interval: i64,
}

/// A level drilled out of: what it showed, to come back to at once.
struct Level {
    span: Span,
    buckets: Rc<Vec<Bucket>>,
    cursor: Option<usize>,
}

#[derive(Default)]
pub(crate) struct State {
    device: Option<SharedString>,
    device_chosen: bool,
    /// The metric (the tag's key) on the chart.
    metric: Option<String>,
    /// Index into [`ANALYTICS_RANGES`]; the finest (1 h of minute bars)
    /// first, where the web opens at 24 h of hourly ones.
    range: usize,
    pub(crate) mode: Mode,
    /// The span on the chart, and the levels drilled out of.
    span: Option<Span>,
    levels: Vec<Level>,
    buckets: Option<Result<Rc<Vec<Bucket>>, String>>,
    buckets_seq: i64,
    compare: Option<Result<ComparisonResult, String>>,
    compare_seq: i64,
    /// The bar under the keyboard cursor, and the one under the pointer.
    pub(crate) cursor: Option<usize>,
    hover: Option<usize>,
    chart: Option<FocusHandle>,
    plot: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The chosen device's tags, with the tag list they came from.
    tags: Option<DeviceTags>,
}

fn chart_focus(console: &mut Console, cx: &mut Context<Console>) -> FocusHandle {
    console
        .state::<State>()
        .chart
        .get_or_insert_with(|| cx.focus_handle())
        .clone()
}

/// The chosen device's tags (positions in the store), made again only
/// when the device or the tag list changes.
fn metric_rows(console: &mut Console) -> Rc<Vec<usize>> {
    let Some(device) = console.state::<State>().device.clone() else {
        return Rc::default();
    };
    let all = console.store.filter("");
    if let Some((kept, from, rows)) = &console.state::<State>().tags
        && *kept == device
        && Rc::ptr_eq(from, &all)
    {
        return Rc::clone(rows);
    }
    let rows = Rc::new(device_tags(console, &device));
    console.state::<State>().tags = Some((device, all, Rc::clone(&rows)));
    rows
}

/// The chosen device's tag with key `key`.
fn tag_of<'a>(console: &'a Console, key: &str) -> Option<&'a crate::console::store::Tag> {
    let device = console.state_ref::<State>()?.device.clone()?;
    console
        .store
        .tags
        .iter()
        .find(|tag| tag.info.device_id == device.as_ref() && tag.info.key == key)
}

pub fn on_open(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        return;
    }
    if !console.state::<State>().device_chosen {
        let first = devices(console).first().cloned();
        console.state::<State>().device_chosen = true;
        pick_device(console, first, cx);
    }
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(METRICS, window, cx);
}

/// The span the range asks for now.
fn range_span(range: usize) -> Span {
    let now = time_now_secs();
    let (_, length, interval) = ANALYTICS_RANGES[range % ANALYTICS_RANGES.len()];
    Span {
        from: now - length,
        to: now,
        interval,
    }
}

/// The figures for the metric over the span on the chart.
fn fetch(console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    let (Some(device), Some(metric)) = (state.device.clone(), state.metric.clone()) else {
        return;
    };
    let span = match state.span {
        Some(span) => span,
        None => {
            let span = range_span(state.range);
            state.span = Some(span);
            span
        }
    };
    state.buckets_seq += 1;
    state.compare_seq += 1;
    state.buckets = None;
    state.compare = None;
    let buckets_seq = state.buckets_seq;
    // The comparison follows the buckets (`fetch_compare`).
    console.call(
        CallKey::new("data.analytics.aggregate").with(buckets_seq),
        Call::get::<Vec<Bucket>>(insights::aggregate_path(
            &device,
            &metric,
            span.from,
            span.to,
            span.interval,
        )),
    );
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    match key.name {
        "data.analytics.aggregate" if key.arg == state.buckets_seq => {
            let buckets = reply.take::<Vec<Bucket>>().map(Rc::new);
            let len = buckets.as_ref().map_or(0, |rows| rows.len());
            state.cursor = state
                .cursor
                .map(|at| at.min(len.saturating_sub(1)))
                .or_else(|| len.checked_sub(1))
                .filter(|_| len > 0);
            state.hover = None;
            state.buckets = Some(buckets);
            fetch_compare(console);
        }
        "data.analytics.compare" if key.arg == state.compare_seq => {
            state.compare = Some(reply.take::<ComparisonResult>());
        }
        _ => return,
    }
    cx.notify();
}

// ── Choices ──────────────────────────────────────────────────────────────

/// Back to the range's own span: drilling ends.
fn reset_span(state: &mut State) {
    state.levels.clear();
    state.span = None;
    state.cursor = None;
}

fn pick_device(console: &mut Console, device: Option<SharedString>, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.device = device;
    state.device_chosen = true;
    reset_span(state);
    let first = metric_rows(console)
        .first()
        .map(|index| console.store.tags[*index].info.key.clone());
    console.state::<State>().metric = first;
    console.list(METRICS, cx).cursor = Some(0);
    fetch(console);
    cx.notify();
}

fn step_device(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    let options = devices(console);
    let current = console.state::<State>().device.clone();
    if let Some(next) = insights_kit::step(&options, insights_kit::text(&current), forward).cloned()
    {
        pick_device(console, Some(next), cx);
    }
}

fn pick_metric(console: &mut Console, row: usize, cx: &mut Context<Console>) {
    let rows = metric_rows(console);
    let Some(key) = rows
        .get(row)
        .map(|index| console.store.tags[*index].info.key.clone())
    else {
        return;
    };
    let state = console.state::<State>();
    if state.metric.as_deref() == Some(key.as_str()) {
        return;
    }
    state.metric = Some(key);
    reset_span(state);
    fetch(console);
    cx.notify();
}

fn step_metric(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    let len = metric_rows(console).len();
    if len == 0 {
        return;
    }
    let nav = console.list(METRICS, cx);
    let row = match nav.cursor {
        Some(row) if forward => (row + 1).min(len - 1),
        Some(row) => row.saturating_sub(1),
        None => 0,
    };
    nav.select(row);
    pick_metric(console, row, cx);
}

fn on_metric_move(console: &mut Console, row: usize, _: &mut Window, cx: &mut Context<Console>) {
    pick_metric(console, row, cx);
}

fn on_metric_enter(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    pick_metric(console, row, cx);
    let focus = chart_focus(console, cx);
    window.focus(&focus);
    cx.notify();
}

fn set_range(console: &mut Console, range: usize, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.range = range % ANALYTICS_RANGES.len();
    reset_span(state);
    fetch(console);
    cx.notify();
}

fn set_mode(console: &mut Console, mode: Mode, cx: &mut Context<Console>) {
    console.state::<State>().mode = mode;
    cx.notify();
}

fn buckets_of(console: &Console) -> Rc<Vec<Bucket>> {
    console
        .state_ref::<State>()
        .and_then(|state| state.buckets.as_ref())
        .and_then(|buckets| buckets.as_ref().ok())
        .cloned()
        .unwrap_or_default()
}

/// Move the bar cursor: `step` bars on, or to an end.
fn move_bar(console: &mut Console, to: BarMove, cx: &mut Context<Console>) {
    let len = buckets_of(console).len();
    if len == 0 {
        return;
    }
    let state = console.state::<State>();
    let at = state.cursor.unwrap_or(len - 1);
    state.cursor = Some(match to {
        BarMove::Previous => at.saturating_sub(1),
        BarMove::Next => (at + 1).min(len - 1),
        BarMove::First => 0,
        BarMove::Last => len - 1,
    });
    cx.notify();
}

#[derive(Clone, Copy)]
enum BarMove {
    Previous,
    Next,
    First,
    Last,
}

/// Enter on a bar: its span with finer buckets.
fn drill_in(console: &mut Console, cx: &mut Context<Console>) {
    let buckets = buckets_of(console);
    let state = console.state::<State>();
    let (Some(span), Some(at)) = (state.span, state.cursor) else {
        return;
    };
    let Some((start, _)) = buckets.get(at) else {
        return;
    };
    let Some((from, to, interval)) = insights::drill(*start, span.interval) else {
        console.show_toast("A minute is the finest bucket the server makes", false, cx);
        return;
    };
    state.levels.push(Level {
        span,
        buckets,
        cursor: Some(at),
    });
    state.span = Some(Span { from, to, interval });
    state.cursor = None;
    fetch(console);
    cx.notify();
}

/// ⌫ or Esc on a drilled chart: the level above, as it was.
fn drill_out(console: &mut Console, cx: &mut Context<Console>) -> bool {
    let state = console.state::<State>();
    let Some(level) = state.levels.pop() else {
        return false;
    };
    state.span = Some(level.span);
    state.cursor = level.cursor;
    state.hover = None;
    state.buckets_seq += 1;
    state.buckets = Some(Ok(level.buckets));
    fetch_compare(console);
    cx.notify();
    true
}

/// The comparison of the span's halves, once its buckets are in: the span
/// starts at the first stored value, since the server cannot compare a
/// half that has none (it answers 500).
fn fetch_compare(console: &mut Console) {
    let buckets = buckets_of(console);
    let state = console.state::<State>();
    state.compare_seq += 1;
    if let Some(Err(error)) = &state.buckets {
        state.compare = Some(Err(error.clone()));
        return;
    }
    let (Some(device), Some(metric), Some(span)) =
        (state.device.clone(), state.metric.clone(), state.span)
    else {
        return;
    };
    let Some((from, to)) = insights::compare_span(span.from, span.to, &buckets) else {
        // Nothing stored: both halves are empty.
        state.compare = Some(Ok(ComparisonResult {
            metric,
            ..ComparisonResult::default()
        }));
        return;
    };
    state.compare = None;
    let seq = state.compare_seq;
    let (current, previous) = halves(from, to);
    console.call(
        CallKey::new("data.analytics.compare").with(seq),
        Call::get::<ComparisonResult>(insights::compare_path(&device, &metric, current, previous)),
    );
}

fn export(console: &mut Console, cx: &mut Context<Console>) {
    let buckets = buckets_of(console);
    let Some(metric) = console.state::<State>().metric.clone() else {
        return;
    };
    if buckets.is_empty() {
        console.show_toast("No buckets to save", false, cx);
        return;
    }
    let name = format!("analytics_{metric}_{}.csv", time_now_secs());
    insights_kit::save_text(name, buckets_csv(&buckets, &metric), cx);
}

/// The bucket whose middle is nearest `at` (Unix seconds).
#[allow(clippy::cast_precision_loss)]
pub(crate) fn bucket_at(buckets: &[Bucket], interval: i64, at: f64) -> Option<usize> {
    let half = interval as f64 / 2.0;
    let after = buckets.partition_point(|(start, _)| *start as f64 + half < at);
    let before = after.checked_sub(1);
    match (before, (after < buckets.len()).then_some(after)) {
        (Some(before), Some(after)) => {
            let middle = |index: usize| buckets[index].0 as f64 + half;
            Some(if at - middle(before) <= middle(after) - at {
                before
            } else {
                after
            })
        }
        (before, after) => before.or(after),
    }
}

/// `1 min`, `5 min`, `1 h`, `6 h`, `1 d`: a bucket size.
pub(crate) fn bucket_label(interval: i64) -> String {
    match interval {
        secs if secs % 86_400 == 0 => format!("{} d", secs / 86_400),
        secs if secs % 3600 == 0 => format!("{} h", secs / 3600),
        secs if secs % 60 == 0 => format!("{} min", secs / 60),
        secs => format!("{secs} s"),
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let options = devices(console);
    let rows = metric_rows(console);
    let chart = chart_focus(console, cx);
    let state = console.state::<State>();
    let (device, metric, range, mode) = (
        state.device.clone(),
        state.metric.clone(),
        state.range,
        state.mode,
    );
    let depth = state.levels.len();
    let span = state.span;
    let buckets = state.buckets.clone();
    let compare = state.compare.clone();
    let (cursor, hover) = (state.cursor, state.hover);
    let plot = Rc::clone(&state.plot);

    let ranges = segmented(
        "analytics-range",
        ANALYTICS_RANGES
            .iter()
            .enumerate()
            .map(|(index, (label, _, _))| {
                Segment::new(
                    *label,
                    index == range,
                    cx.listener(move |console, _, _, cx| set_range(console, index, cx)),
                )
            })
            .collect(),
        cx,
    );
    let modes = segmented(
        "analytics-mode",
        vec![
            Segment::new(
                "Aggregate",
                mode == Mode::Aggregate,
                cx.listener(|console, _, _, cx| set_mode(console, Mode::Aggregate, cx)),
            )
            .icon(Icon::ChartLine),
            Segment::new(
                "Compare",
                mode == Mode::Compare,
                cx.listener(|console, _, _, cx| set_mode(console, Mode::Compare, cx)),
            )
            .icon(Icon::Columns),
        ],
        cx,
    );
    let has_buckets = buckets
        .as_ref()
        .is_some_and(|buckets| buckets.as_ref().is_ok_and(|rows| !rows.is_empty()));
    let controls = vec![
        modes,
        ranges,
        Button::new("analytics-export", "CSV")
            .compact()
            .icon(Icon::Download)
            .tooltip("Save the buckets as CSV  E")
            .disabled(!has_buckets)
            .on_click(cx.listener(|console, _, _, cx| export(console, cx)))
            .into_any_element(),
        IconButton::new("analytics-refresh", Icon::RefreshCw)
            .tooltip("Fetch again  R")
            .on_click(cx.listener(|console, _, _, cx| {
                fetch(console);
                cx.notify();
            }))
            .into_any_element(),
    ];
    let subtitle = match (&device, &metric) {
        (Some(device), Some(metric)) => {
            format!("{device} · {metric} · ←→ bar · ↵ drill in · ⌫ back · ↑↓ metric · M mode")
        }
        (Some(device), None) => format!("{device} has no tags to analyse"),
        _ => "Choose a device".to_owned(),
    };

    let device_bar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .px(SPACE_9)
        .py(SPACE_4)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_none()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child("DEVICE"),
        )
        .child(chips(
            "analytics-device",
            &options,
            insights_kit::text(&device),
            None,
            pick_device,
            cx,
        ))
        .child(div().flex_1())
        .child(kbd(",  .", cx));

    // The metric picker: the device's tags.
    let picker_list = keyboard_list(
        ListKeys {
            id: METRICS,
            on_move: Some(on_metric_move),
            on_activate: on_metric_enter,
        },
        rows.len(),
        rows_on_screen(window, METRIC_ROW),
        console,
        cx,
    );
    let wanted = metric.as_ref().and_then(|metric| {
        rows.iter()
            .position(|index| console.store.tags[*index].info.key == *metric)
    });
    let nav = console.list(METRICS, cx);
    if nav.cursor.is_none() {
        nav.cursor = wanted;
    }
    let nav = console.list(METRICS, cx);
    let (metric_cursor, picker_focused, picker_scroll) =
        (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let shown = rows.len();
    let picker_rows = Rc::clone(&rows);
    let picker = picker_list
        .flex()
        .flex_col()
        .flex_none()
        .w(px(PICKER))
        .h_full()
        .border_r_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_none()
                .px(SPACE_7)
                .pt(SPACE_6)
                .pb(SPACE_3)
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child(format!("METRICS · {shown}")),
        )
        .child(if shown == 0 {
            notice("No tags on this device.", false, &theme)
        } else {
            let chosen = metric.clone();
            uniform_list(
                "analytics-metrics",
                shown + tail_rows(shown, METRIC_ROW),
                cx.processor(
                    move |console: &mut Console,
                          range: Range<usize>,
                          _,
                          cx: &mut Context<Console>| {
                        let theme = vt_ui::theme::theme(cx).clone();
                        range
                            .map(|row| match picker_rows.get(row) {
                                None => tail_row(METRIC_ROW),
                                Some(index) => {
                                    let tag = &console.store.tags[*index];
                                    let active = chosen.as_deref() == Some(tag.info.key.as_str());
                                    let hover = theme.hover;
                                    div()
                                        .id(("analytics-metric", row))
                                        .relative()
                                        .flex()
                                        .flex_col()
                                        .justify_center()
                                        .h(px(METRIC_ROW))
                                        .mx(SPACE_3)
                                        .px(SPACE_4)
                                        .rounded(RADIUS_MD)
                                        .cursor_pointer()
                                        .when(active, |this| this.bg(theme.selected))
                                        .when(!active, |this| {
                                            this.hover(move |style| style.bg(hover))
                                        })
                                        .on_click(cx.listener(move |console, _, window, cx| {
                                            console.list(METRICS, cx).select(row);
                                            pick_metric(console, row, cx);
                                            let focus = console.list(METRICS, cx).focus.clone();
                                            window.focus(&focus);
                                        }))
                                        .when(
                                            picker_focused && metric_cursor == Some(row),
                                            |this| this.child(focus_ring(&theme)),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(FONT_BODY)
                                                .text_color(if active {
                                                    theme.fg
                                                } else {
                                                    theme.fg_muted
                                                })
                                                .child(tag.label.clone()),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .font_family(MONO)
                                                .text_size(FONT_CAPTION)
                                                .text_color(theme.fg_dim)
                                                .child(tag.key_label.clone()),
                                        )
                                        .into_any_element()
                                }
                            })
                            .collect::<Vec<_>>()
                    },
                ),
            )
            .track_scroll(picker_scroll)
            .flex_1()
            .into_any_element()
        });

    let (unit, limits) = metric
        .as_deref()
        .and_then(|key| {
            let tag = tag_of(console, key)?;
            let limits: Vec<(f64, Hsla)> = [
                (tag.info.warn_val, theme.warning),
                (tag.info.crit_val, theme.danger),
            ]
            .into_iter()
            .filter(|(value, _)| *value > 0.0)
            .collect();
            Some((tag.info.unit.clone(), limits))
        })
        .unwrap_or_default();

    let content = match (&metric, mode) {
        (None, _) => empty_state(
            Icon::ChartLine,
            "Nothing to analyse",
            "Choose a device with tags (, .) and one of its metrics.",
            Vec::new(),
            cx,
        )
        .into_any_element(),
        (Some(_), Mode::Aggregate) => aggregate_view(
            AggregateView {
                buckets,
                span,
                depth,
                range,
                cursor,
                hover,
                unit,
                limits,
                plot,
                focus: chart.clone(),
                focused: chart.is_focused(window),
                offset: console.utc_offset,
            },
            cx,
        ),
        (Some(_), Mode::Compare) => compare_view(compare, &unit, console.utc_offset, &theme),
    };

    div()
        .key_context("Analytics")
        .flex()
        .flex_col()
        .size_full()
        .on_action(cx.listener(|console, _: &AnalyticsPreviousDevice, _, cx| {
            step_device(console, false, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsNextDevice, _, cx| {
            step_device(console, true, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsPreviousRange, _, cx| {
            let range = console.state::<State>().range;
            set_range(console, range + ANALYTICS_RANGES.len() - 1, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsNextRange, _, cx| {
            let range = console.state::<State>().range;
            set_range(console, range + 1, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsSwitchMode, _, cx| {
            let mode = match console.state::<State>().mode {
                Mode::Aggregate => Mode::Compare,
                Mode::Compare => Mode::Aggregate,
            };
            set_mode(console, mode, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsRefresh, _, cx| {
            fetch(console);
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &AnalyticsExport, _, cx| export(console, cx)))
        .child(page_header("Analytics", subtitle, controls, cx))
        .child(device_bar)
        .child(
            div().flex().flex_1().min_h_0().child(picker).child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(SPACE_6)
                    .px(SPACE_9)
                    .py(SPACE_7)
                    .child(content),
            ),
        )
        .into_any_element()
}

struct AggregateView {
    buckets: Option<Result<Rc<Vec<Bucket>>, String>>,
    span: Option<Span>,
    depth: usize,
    range: usize,
    cursor: Option<usize>,
    hover: Option<usize>,
    unit: String,
    limits: Vec<(f64, Hsla)>,
    plot: Rc<Cell<Option<Bounds<Pixels>>>>,
    focus: FocusHandle,
    focused: bool,
    offset: UtcOffset,
}

#[allow(clippy::too_many_lines)]
fn aggregate_view(view: AggregateView, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let (Some(span), Some(buckets)) = (view.span, view.buckets) else {
        return notice("Fetching the buckets…", false, &theme);
    };
    let buckets = match buckets {
        Ok(buckets) => buckets,
        Err(error) => return notice(format!("Analytics: {error}"), true, &theme),
    };
    let offset = view.offset;
    let shown_at = view.hover.or(view.cursor);
    let unit = view.unit;

    // Where the chart is: the range, or the bar drilled into.
    let place = if view.depth == 0 {
        format!(
            "Last {} · {} buckets",
            ANALYTICS_RANGES[view.range].0,
            bucket_label(span.interval)
        )
    } else {
        format!(
            "{} – {} · {} buckets · drilled {} level{}",
            local_time(span.from, offset),
            local_time(span.to, offset),
            bucket_label(span.interval),
            view.depth,
            if view.depth == 1 { "" } else { "s" }
        )
    };
    let figures = match overall(&buckets) {
        Some(Overall {
            min,
            max,
            avg,
            samples,
            buckets: count,
        }) => div()
            .grid()
            .grid_cols(5)
            .gap(SPACE_6)
            .child(figure("Min", with_unit(min, &unit), theme.fg, &theme))
            .child(figure("Max", with_unit(max, &unit), theme.fg, &theme))
            .child(figure(
                "Average",
                with_unit(avg, &unit),
                theme.accent,
                &theme,
            ))
            .child(figure("Samples", samples.to_string(), theme.fg, &theme))
            .child(figure("Buckets", count.to_string(), theme.fg, &theme))
            .into_any_element(),
        None => div().into_any_element(),
    };
    if buckets.is_empty() {
        return div()
            .flex()
            .flex_col()
            .gap(SPACE_6)
            .size_full()
            .child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(place),
            )
            .child(notice(
                "The server stored no values of this metric in the span.",
                false,
                &theme,
            ))
            .into_any_element();
    }

    // The bar the keyboard or the pointer is on, read out.
    let readout = match shown_at.and_then(|at| buckets.get(at)) {
        Some((start, bucket)) => div()
            .flex()
            .items_center()
            .gap(SPACE_6)
            .font_family(MONO)
            .text_size(FONT_SMALL)
            .child(div().text_color(theme.fg).child(format!(
                "{} – {}",
                local_time(*start, offset),
                local_time(start + span.interval, offset)
            )))
            .child(
                div()
                    .text_color(theme.fg_muted)
                    .child(format!("min {}", format_value(bucket.min))),
            )
            .child(
                div()
                    .text_color(theme.accent)
                    .child(format!("avg {}", format_value(bucket.avg))),
            )
            .child(
                div()
                    .text_color(theme.fg_muted)
                    .child(format!("max {}", format_value(bucket.max))),
            )
            .child(
                div()
                    .text_color(theme.fg_dim)
                    .child(format!("{} samples", bucket.count)),
            )
            .into_any_element(),
        None => div()
            .text_size(FONT_SMALL)
            .text_color(theme.fg_dim)
            .child("Point at a bar, or ←→ on the chart")
            .into_any_element(),
    };

    let spec = BarChart {
        buckets: Rc::clone(&buckets),
        interval: span.interval,
        from: span.from,
        to: span.to,
        cursor: view.cursor,
        hover: view.hover,
        focused: view.focused,
        limits: view.limits,
        offset,
        plot: Rc::clone(&view.plot),
    };
    let plot = view.plot;
    let hover_buckets = Rc::clone(&buckets);
    let click_buckets = Rc::clone(&buckets);
    let interval = span.interval;
    let chart = div()
        .id("analytics-chart")
        .key_context(ANALYTICS_CHART)
        .track_focus(&view.focus)
        .relative()
        .flex_1()
        .min_h(px(180.0))
        .rounded(RADIUS_LG)
        .border_1()
        .border_color(if view.focused {
            theme.accent.alpha(0.6)
        } else {
            theme.border
        })
        .bg(theme.surface)
        .p(SPACE_4)
        .cursor_crosshair()
        .on_mouse_move(cx.listener({
            let plot = Rc::clone(&plot);
            move |console, event: &MouseMoveEvent, _, cx| {
                let at = time_at(&plot, event.position, span.from, span.to)
                    .and_then(|at| bucket_at(&hover_buckets, interval, at));
                let state = console.state::<State>();
                if state.hover != at {
                    state.hover = at;
                    cx.notify();
                }
            }
        }))
        .on_hover(cx.listener(|console, hovered: &bool, _, cx| {
            if !*hovered && console.state::<State>().hover.take().is_some() {
                cx.notify();
            }
        }))
        .on_click(
            cx.listener(move |console, event: &gpui::ClickEvent, window, cx| {
                let at = time_at(&plot, event.position(), span.from, span.to)
                    .and_then(|at| bucket_at(&click_buckets, interval, at));
                let focus = chart_focus(console, cx);
                window.focus(&focus);
                if let Some(at) = at {
                    console.state::<State>().cursor = Some(at);
                    if event.click_count() >= 2 {
                        drill_in(console, cx);
                    }
                }
                cx.notify();
            }),
        )
        .on_action(cx.listener(|console, _: &AnalyticsPreviousBar, _, cx| {
            move_bar(console, BarMove::Previous, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsNextBar, _, cx| {
            move_bar(console, BarMove::Next, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsFirstBar, _, cx| {
            move_bar(console, BarMove::First, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsLastBar, _, cx| {
            move_bar(console, BarMove::Last, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsDrill, _, cx| drill_in(console, cx)))
        .on_action(cx.listener(|console, _: &AnalyticsBack, _, cx| {
            drill_out(console, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsPreviousMetric, _, cx| {
            step_metric(console, false, cx);
        }))
        .on_action(cx.listener(|console, _: &AnalyticsNextMetric, _, cx| {
            step_metric(console, true, cx);
        }))
        // Esc comes out of a drill first; at the top it goes back to the
        // sidebar as everywhere in the Data Manager.
        .on_action(cx.listener(|console, _: &Dismiss, _, cx| {
            if !drill_out(console, cx) {
                cx.propagate();
            }
        }))
        .child(bar_chart(spec, cx));

    div()
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .size_full()
        .child(figures)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(place)
                .when(view.depth > 0, |this| {
                    this.child(kbd("⌫", cx)).child("back out")
                })
                .child(div().flex_1())
                .child(readout),
        )
        .child(chart)
        .into_any_element()
}

fn with_unit(value: f64, unit: &str) -> String {
    if unit.is_empty() {
        format_value(value)
    } else {
        format!("{} {unit}", format_value(value))
    }
}

/// The recent half of the range against the half before it.
fn compare_view(
    compare: Option<Result<ComparisonResult, String>>,
    unit: &str,
    offset: UtcOffset,
    theme: &Theme,
) -> AnyElement {
    let compared = match compare {
        None => return notice("Comparing the halves…", false, theme),
        Some(Err(error)) => return notice(format!("Compare: {error}"), true, theme),
        Some(Ok(compared)) => compared,
    };
    if compared.current.count == 0 && compared.previous.count == 0 {
        return notice(
            "The server stored no values of this metric in either half.",
            false,
            theme,
        );
    }
    let change = compared.change_percent;
    let change_color = if change > 0.0 {
        theme.success
    } else if change < 0.0 {
        theme.danger
    } else {
        theme.fg_muted
    };
    let top = compared.current.max.max(compared.previous.max);
    let bottom = compared.current.min.min(compared.previous.min).min(0.0);
    let scale = |value: f64| {
        if top > bottom {
            (value - bottom) / (top - bottom)
        } else {
            0.0
        }
    };
    let period = |title: &'static str, metric: &insights::AggregatedMetric, color: Hsla| {
        div()
            .flex()
            .flex_col()
            .gap(SPACE_4)
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_3)
                    .child(div().size(px(8.0)).rounded(px(4.0)).bg(color))
                    .child(
                        div()
                            .text_size(FONT_EMPHASIS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    ),
            )
            .child(
                div()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(if metric.count == 0 {
                        "no values".to_owned()
                    } else {
                        format!(
                            "{} – {}",
                            local_time(metric.first_ts, offset),
                            local_time(metric.last_ts, offset)
                        )
                    }),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(SPACE_5)
                    .child(figure("Average", with_unit(metric.avg, unit), color, theme))
                    .child(figure("Samples", metric.count.to_string(), theme.fg, theme))
                    .child(figure("Min", with_unit(metric.min, unit), theme.fg, theme))
                    .child(figure("Max", with_unit(metric.max, unit), theme.fg, theme)),
            )
    };
    let (previous_color, current_color) = (theme.fg_dim, theme.accent);
    let pair = |label: &'static str, previous: f64, current: f64| {
        div()
            .flex()
            .flex_col()
            .gap(SPACE_2)
            .child(
                div()
                    .text_size(FONT_CAPTION)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg_dim)
                    .child(label),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .child(bar(scale(previous), previous_color, px(8.0), theme))
                    .child(
                        div()
                            .w(px(96.0))
                            .text_right()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(format_value(previous)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .child(bar(scale(current), current_color, px(8.0), theme))
                    .child(
                        div()
                            .w(px(96.0))
                            .text_right()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .child(format_value(current)),
                    ),
            )
    };
    card(theme)
        .gap(SPACE_7)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .flex_1()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Period comparison"),
                )
                .child(pill(
                    format!(
                        "{}{:.1}% average",
                        if change > 0.0 { "+" } else { "" },
                        change
                    ),
                    change_color,
                )),
        )
        .child(
            div()
                .flex()
                .gap(SPACE_9)
                .child(period("Earlier half", &compared.previous, previous_color))
                .child(period("Recent half", &compared.current, current_color)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_5)
                .child(pair("MIN", compared.previous.min, compared.current.min))
                .child(pair("AVERAGE", compared.previous.avg, compared.current.avg))
                .child(pair("MAX", compared.previous.max, compared.current.max)),
        )
        .into_any_element()
}

// ── The bucket chart ─────────────────────────────────────────────────────

struct BarChart {
    buckets: Rc<Vec<Bucket>>,
    interval: i64,
    from: i64,
    to: i64,
    cursor: Option<usize>,
    hover: Option<usize>,
    focused: bool,
    limits: Vec<(f64, Hsla)>,
    offset: UtcOffset,
    plot: Rc<Cell<Option<Bounds<Pixels>>>>,
}

fn bar_chart(spec: BarChart, cx: &App) -> impl IntoElement {
    let theme = theme(cx).clone();
    canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                paint(&spec, &theme, bounds, window, cx);
            });
        },
    )
    .size_full()
}

/// The time (Unix seconds) at `position` on the plot last painted.
#[allow(clippy::cast_precision_loss)]
fn time_at(
    plot: &Rc<Cell<Option<Bounds<Pixels>>>>,
    position: Point<Pixels>,
    from: i64,
    to: i64,
) -> Option<f64> {
    let bounds = plot.get()?;
    let width = f64::from(f32::from(bounds.size.width));
    if width <= 0.0 {
        return None;
    }
    let fraction = f64::from(f32::from(position.x - bounds.left())) / width;
    (-0.02..=1.02)
        .contains(&fraction)
        .then(|| from as f64 + (to - from) as f64 * fraction.clamp(0.0, 1.0))
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn paint(
    spec: &BarChart,
    theme: &Theme,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let plot = Bounds::new(
        point(bounds.left() + px(GUTTER), bounds.top() + px(TOP)),
        size(
            (bounds.size.width - px(GUTTER + 8.0)).max(px(10.0)),
            (bounds.size.height - px(TOP + TIME_STRIP)).max(px(10.0)),
        ),
    );
    spec.plot.set(Some(plot));
    let (from, to) = (spec.from as f64, (spec.to.max(spec.from + 1)) as f64);
    let span = to - from;
    let x_of = |at: f64| plot.left() + plot.size.width * ((at - from) / span) as f32;

    let (low, high) = spec
        .buckets
        .iter()
        .map(|(_, bucket)| (bucket.min, bucket.max))
        .chain(spec.limits.iter().map(|(limit, _)| (*limit, *limit)))
        .filter(|(low, high)| low.is_finite() && high.is_finite())
        .fold(None, |range: Option<(f64, f64)>, (low, high)| match range {
            None => Some((low, high)),
            Some((l, h)) => Some((l.min(low), h.max(high))),
        })
        .unwrap_or((0.0, 1.0));
    let scale = value_scale(low, high);
    let y_of = |value: f64| {
        let fraction = ((value - scale.0) / (scale.1 - scale.0).max(f64::EPSILON)) as f32;
        plot.bottom() - plot.size.height * fraction
    };

    for row in 0..=4 {
        let fraction = f64::from(row) / 4.0;
        let y = plot.bottom() - plot.size.height * fraction as f32;
        window.paint_quad(fill(
            Bounds::new(point(plot.left(), y), size(plot.size.width, px(1.0))),
            if row == 0 {
                theme.border_strong
            } else {
                theme.border
            },
        ));
        let value = scale.0 + (scale.1 - scale.0) * fraction;
        label(
            &format_value(value),
            point(bounds.left() + px(4.0), y - px(7.0)),
            theme.fg_dim,
            window,
            cx,
        );
    }
    let (from_ms, to_ms) = (from * 1000.0, to * 1000.0);
    let step = tick_step(to_ms - from_ms, f32::from(plot.size.width));
    for at in ticks(from_ms, to_ms, step, spec.offset) {
        let x = x_of(at / 1000.0);
        let text = tick_label(at, step, to_ms - from_ms, spec.offset);
        let shaped = shape(&text, theme.fg_dim, window);
        let left = (x - shaped.width * 0.5)
            .max(plot.left())
            .min(plot.right() - shaped.width);
        let _ = shaped.paint(point(left, plot.bottom() + px(4.0)), px(14.0), window, cx);
    }

    window.with_content_mask(Some(ContentMask { bounds: plot }), |window| {
        for (limit, color) in &spec.limits {
            if *limit > scale.0 && *limit < scale.1 {
                window.paint_quad(fill(
                    Bounds::new(
                        point(plot.left(), y_of(*limit)),
                        size(plot.size.width, px(1.0)),
                    ),
                    color.alpha(0.7),
                ));
            }
        }
        let color = theme.series[0];
        let slot = plot.size.width * (spec.interval as f64 / span) as f32;
        let width = (slot * 0.68).max(px(1.0));
        let mut pen = PathBuilder::stroke(px(1.5));
        let mut drawn = 0;
        for (index, (start, bucket)) in spec.buckets.iter().enumerate() {
            let middle = x_of(*start as f64 + spec.interval as f64 / 2.0);
            let marked = spec.cursor == Some(index) || spec.hover == Some(index);
            let fill_color = if spec.cursor == Some(index) && spec.focused {
                theme.accent.alpha(0.45)
            } else if marked {
                color.alpha(0.42)
            } else {
                color.alpha(0.22)
            };
            let (top, bottom) = (y_of(bucket.max), y_of(bucket.min));
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(middle - width * 0.5, top),
                        size(width, (bottom - top).max(px(2.0))),
                    ),
                    fill_color,
                )
                .corner_radii(px(2.0)),
            );
            let average = y_of(bucket.avg);
            window.paint_quad(fill(
                Bounds::new(
                    point(middle - width * 0.5, average - px(1.0)),
                    size(width, px(2.0)),
                ),
                if marked { theme.fg } else { color },
            ));
            if drawn == 0 {
                pen.move_to(point(middle, average));
            } else {
                pen.line_to(point(middle, average));
            }
            drawn += 1;
        }
        if drawn > 1
            && let Ok(path) = pen.build()
        {
            window.paint_path(path, color.alpha(0.8));
        }
        // The crosshair on the bar under the pointer.
        if let Some((start, _)) = spec.hover.and_then(|at| spec.buckets.get(at)) {
            let x = x_of(*start as f64 + spec.interval as f64 / 2.0);
            window.paint_quad(fill(
                Bounds::new(point(x, plot.top()), size(px(1.0), plot.size.height)),
                theme.fg_dim,
            ));
        }
    });
}

fn shape(text: &str, color: Hsla, window: &mut Window) -> gpui::ShapedLine {
    let text: SharedString = text.to_owned().into();
    let run = TextRun {
        len: text.len(),
        font: font(MONO),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, FONT_CAPTION, &[run], None)
}

fn label(text: &str, origin: Point<Pixels>, color: Hsla, window: &mut Window, cx: &mut App) {
    let shaped = shape(text, color, window);
    let _ = shaped.paint(origin, px(14.0), window, cx);
}

#[cfg(test)]
mod tests {
    use super::super::insights_kit::testing::{open, sent, signed_in};
    use super::*;
    use voltrus_scada_native::api::insights::AggregatedMetric;

    fn hourly(starts: &[i64]) -> Vec<Bucket> {
        starts
            .iter()
            .map(|start| {
                (
                    *start,
                    AggregatedMetric {
                        metric: "p".into(),
                        min: 1.0,
                        max: 3.0,
                        avg: 2.0,
                        count: 60,
                        first_ts: *start,
                        last_ts: start + 3599,
                        ..AggregatedMetric::default()
                    },
                )
            })
            .collect()
    }

    #[gpui::test]
    fn a_bar_drills_in_and_back_by_keyboard_for_any_user(cx: &mut gpui::TestAppContext) {
        // Analytics only reads, so a viewer has all of it.
        let mut h = signed_in(cx, "viewer");
        h.update(|console, _, _| console.state::<State>().range = 2); // 24 h of hours
        open(&mut h, ID);
        let aggregate = sent(&h.calls(), "data.analytics.aggregate").expect("the buckets");
        assert_eq!(
            h.update(|console, _, _| console.state::<State>().metric.clone()),
            Some("p".to_owned()),
            "the device's first tag"
        );
        let now = time_now_secs() / 3600 * 3600;
        h.reply(aggregate, Reply::ok(hourly(&[now - 7200, now - 3600, now])));
        assert!(
            sent(&h.calls(), "data.analytics.compare").is_some(),
            "the halves are compared once the buckets are in"
        );
        h.draw();
        // Enter on the metric goes to the chart; ← moves a bar back.
        h.keys("enter");
        h.draw();
        h.keys("left");
        assert_eq!(
            h.update(|console, _, _| console.state::<State>().cursor),
            Some(1)
        );
        h.keys("enter");
        let state = |h: &mut crate::console::testing::Harness| {
            h.update(|console, _, _| {
                let state = console.state::<State>();
                (state.levels.len(), state.span.map(|span| span.interval))
            })
        };
        assert_eq!(
            state(&mut h),
            (1, Some(300)),
            "the hour in five-minute bars"
        );
        let finer = sent(&h.calls(), "data.analytics.aggregate").expect("its buckets");
        assert_eq!(finer.arg, aggregate.arg + 1);
        h.keys("backspace");
        assert_eq!(state(&mut h), (0, Some(3600)), "back out, as it was");
        assert_eq!(h.update(|console, _, _| buckets_of(console).len()), 3);
    }

    fn buckets(starts: &[i64]) -> Vec<Bucket> {
        starts
            .iter()
            .map(|start| (*start, AggregatedMetric::default()))
            .collect()
    }

    #[test]
    fn the_pointer_lands_on_the_nearest_bar() {
        let rows = buckets(&[0, 60, 120, 300]);
        assert_eq!(bucket_at(&rows, 60, 10.0), Some(0));
        assert_eq!(bucket_at(&rows, 60, 89.0), Some(1));
        assert_eq!(bucket_at(&rows, 60, 150.0), Some(2));
        assert_eq!(
            bucket_at(&rows, 60, 260.0),
            Some(3),
            "a gap goes to the nearer"
        );
        assert_eq!(bucket_at(&rows, 60, 9999.0), Some(3));
        assert_eq!(bucket_at(&rows, 60, -50.0), Some(0));
        assert_eq!(bucket_at(&[], 60, 0.0), None);
    }

    #[test]
    fn bucket_sizes_read_short() {
        assert_eq!(bucket_label(60), "1 min");
        assert_eq!(bucket_label(300), "5 min");
        assert_eq!(bucket_label(3600), "1 h");
        assert_eq!(bucket_label(21_600), "6 h");
        assert_eq!(bucket_label(86_400), "1 d");
        assert_eq!(bucket_label(45), "45 s");
    }
}
