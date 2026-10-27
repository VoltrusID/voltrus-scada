//! The tag detail's chart, drawn by the console itself: vt-ui's trend has
//! no clock axis, hit-testing, limit lines or annotation marks. One tag and
//! its overlays over `[from, to]`; an overlay of a very different size
//! (watts beside hertz) gets its own scale. The plot's bounds are kept in a
//! cell as it paints, so a pointer position maps back to a time. Painting
//! never copies samples, and thins a dense series to a min and max per
//! pixel column.

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, Bounds, ContentMask, Hsla, PathBuilder, Pixels, Point, SharedString, TextRun, Window,
    canvas, fill, font, point, px, size,
};
use voltrus_scada_native::zone::LocalZone;
use vt_ui::chart::{format_value, needs_lanes, value_scale};
use vt_ui::theme::*;

/// A series' samples, `(Unix ms, value)` oldest first, shared with the
/// chart without copying.
pub(crate) type Samples = Rc<Vec<(f64, f64)>>;

/// One series on the chart.
#[derive(Clone)]
pub(crate) struct Line {
    pub color: Hsla,
    pub samples: Samples,
}

/// Where the plot was last painted, for mapping the pointer to a time.
pub(crate) type PlotCell = Rc<Cell<Option<Bounds<Pixels>>>>;

pub(crate) struct ChartSpec {
    /// The tag first, then its overlays.
    pub lines: Vec<Line>,
    pub from_ms: f64,
    pub to_ms: f64,
    /// Horizontal limit lines on the tag's scale (warning, critical).
    pub limits: Vec<(f64, Hsla)>,
    /// Annotation times.
    pub marks: Vec<f64>,
    /// The crosshair's time.
    pub cursor: Option<f64>,
    pub zone: LocalZone,
    pub plot: PlotCell,
}

const GUTTER: f32 = 56.0;
const TIME_STRIP: f32 = 18.0;
const TOP: f32 = 10.0;
/// Room each time label wants.
const TICK_ROOM: f32 = 96.0;

pub(crate) fn chart(spec: ChartSpec, cx: &App) -> impl IntoElement {
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

/// The time at `x` (window coordinates) on the plot last painted.
pub(crate) fn time_at(plot: &PlotCell, x: Pixels, from_ms: f64, to_ms: f64) -> Option<f64> {
    let bounds = plot.get()?;
    let width = f64::from(f32::from(bounds.size.width));
    if width <= 0.0 {
        return None;
    }
    let fraction = f64::from(f32::from(x - bounds.left())) / width;
    if (0.0..=1.0).contains(&fraction) {
        Some(from_ms + (to_ms - from_ms) * fraction)
    } else {
        None
    }
}

/// The sample of `samples` nearest to `at`.
pub(crate) fn nearest(samples: &[(f64, f64)], at: f64) -> Option<(f64, f64)> {
    let after = samples.partition_point(|(time, _)| *time < at);
    let before = after.checked_sub(1).map(|index| samples[index]);
    let after = samples.get(after).copied();
    match (before, after) {
        (Some(before), Some(after)) => Some(if at - before.0 <= after.0 - at {
            before
        } else {
            after
        }),
        (before, after) => before.or(after),
    }
}

/// Smallest and largest finite value in `[from, to]`.
fn visible_range(samples: &[(f64, f64)], from: f64, to: f64) -> Option<(f64, f64)> {
    let start = samples.partition_point(|(at, _)| *at < from);
    samples[start..]
        .iter()
        .take_while(|(at, _)| *at <= to)
        .filter(|(_, value)| value.is_finite())
        .fold(None, |range, (_, value)| match range {
            None => Some((*value, *value)),
            Some((low, high)) => Some((low.min(*value), high.max(*value))),
        })
}

/// The time labels' spacing for `span_ms` over `width` pixels: the
/// smallest round step that leaves each label its room.
pub(crate) fn tick_step(span_ms: f64, width: f32) -> f64 {
    const STEPS: [f64; 17] = [
        1_000.0,
        5_000.0,
        10_000.0,
        30_000.0,
        60_000.0,
        120_000.0,
        300_000.0,
        600_000.0,
        900_000.0,
        1_800_000.0,
        3_600_000.0,
        10_800_000.0,
        21_600_000.0,
        43_200_000.0,
        86_400_000.0,
        172_800_000.0,
        604_800_000.0,
    ];
    let room = f64::from((width / TICK_ROOM).floor().max(1.0));
    STEPS
        .into_iter()
        .find(|step| span_ms / step <= room)
        .unwrap_or(STEPS[STEPS.len() - 1])
}

/// Steps from which a tick is placed on the offset in force at the tick
/// itself, so a day's midnight stays midnight across a daylight-saving
/// change. A shorter step divides the hour, and the change moves the clock
/// by whole hours: its ticks stay round on the offset at the start.
const WALL_STEP_MS: f64 = 3.0 * 3_600_000.0;

/// The ticks in `[from, to]`, on round times of the operator's clock.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn ticks(from_ms: f64, to_ms: f64, step: f64, zone: &LocalZone) -> Vec<f64> {
    let offset_ms =
        |at_ms: f64| f64::from(zone.offset_secs((at_ms / 1000.0).floor() as i64)) * 1000.0;
    let shift = offset_ms(from_ms);
    // The wall clock's round times, as milliseconds since 1970 on the wall.
    let mut wall = ((from_ms + shift) / step).ceil() * step;
    let mut out: Vec<f64> = Vec::new();
    while out.len() < 64 {
        let at = if step < WALL_STEP_MS {
            wall - shift
        } else {
            wall - offset_ms(wall - shift)
        };
        if at > to_ms {
            break;
        }
        // A tick the change skipped or repeated may fall out of order.
        if at >= from_ms && out.last().is_none_or(|last| at > *last) {
            out.push(at);
        }
        wall += step;
    }
    out
}

/// A tick's label: seconds for short steps, the day for long spans.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn tick_label(at_ms: f64, step: f64, span_ms: f64, zone: &LocalZone) -> String {
    let Some(at) = zone.at((at_ms / 1000.0).floor() as i64) else {
        return String::new();
    };
    let month = at.month().to_string();
    let month = month.get(..3).unwrap_or(&month);
    if step >= 86_400_000.0 {
        format!("{} {month}", at.day())
    } else if span_ms > 36.0 * 3_600_000.0 {
        format!("{} {month} {:02}:{:02}", at.day(), at.hour(), at.minute())
    } else if step < 60_000.0 {
        format!("{:02}:{:02}:{:02}", at.hour(), at.minute(), at.second())
    } else {
        format!("{:02}:{:02}", at.hour(), at.minute())
    }
}

fn paint(
    spec: &ChartSpec,
    theme: &Theme,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let (from, to) = (spec.from_ms, spec.to_ms.max(spec.from_ms + 1.0));
    let plot = Bounds::new(
        point(bounds.left() + px(GUTTER), bounds.top() + px(TOP)),
        size(
            (bounds.size.width - px(GUTTER + 8.0)).max(px(10.0)),
            (bounds.size.height - px(TOP + TIME_STRIP)).max(px(10.0)),
        ),
    );
    spec.plot.set(Some(plot));

    let ranges: Vec<Option<(f64, f64)>> = spec
        .lines
        .iter()
        .map(|line| visible_range(&line.samples, from, to))
        .collect();
    let main = ranges.first().copied().flatten();
    // Overlays of the tag's size share its scale; the others get their own.
    let shared: Vec<bool> = ranges
        .iter()
        .map(|range| match (main, range) {
            (Some(main), Some(range)) => !needs_lanes(&[main, *range]),
            _ => true,
        })
        .collect();
    let shared_range = ranges
        .iter()
        .zip(&shared)
        .filter(|(_, shared)| **shared)
        .filter_map(|(range, _)| *range)
        .reduce(|(low, high), (l, h)| (low.min(l), high.max(h)));
    let scale = shared_range.map_or((0.0, 1.0), |(low, high)| value_scale(low, high));

    // Grid and value labels on the tag's scale.
    for row in 0..=4 {
        let fraction = f64::from(row) / 4.0;
        #[allow(clippy::cast_possible_truncation)]
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

    // Time labels on round times.
    let span = to - from;
    let step = tick_step(span, f32::from(plot.size.width));
    for at in ticks(from, to, step, &spec.zone) {
        let x = x_of(at, from, span, plot);
        window.paint_quad(fill(
            Bounds::new(point(x, plot.top()), size(px(1.0), plot.size.height)),
            theme.border.alpha(0.5),
        ));
        let text = tick_label(at, step, span, &spec.zone);
        let shaped = shape(&text, theme.fg_dim, window);
        let left = (x - shaped.width * 0.5)
            .max(plot.left())
            .min(plot.right() - shaped.width);
        let _ = shaped.paint(point(left, plot.bottom() + px(4.0)), px(14.0), window, cx);
    }

    window.with_content_mask(Some(ContentMask { bounds: plot }), |window| {
        for (limit, color) in &spec.limits {
            if *limit > scale.0 && *limit < scale.1 {
                let y = y_of(*limit, scale, plot);
                window.paint_quad(fill(
                    Bounds::new(point(plot.left(), y), size(plot.size.width, px(1.0))),
                    color.alpha(0.7),
                ));
            }
        }
        for at in &spec.marks {
            if (from..=to).contains(at) {
                let x = x_of(*at, from, span, plot);
                window.paint_quad(fill(
                    Bounds::new(point(x, plot.top()), size(px(1.0), plot.size.height)),
                    theme.info.alpha(0.55),
                ));
                window.paint_quad(
                    fill(
                        Bounds::new(point(x - px(3.0), plot.top()), size(px(7.0), px(7.0))),
                        theme.info,
                    )
                    .corner_radii(px(2.0)),
                );
            }
        }
        // Overlays first, so the tag's own line stays on top.
        for (index, line) in spec.lines.iter().enumerate().rev() {
            let line_scale = if shared[index] {
                scale
            } else {
                ranges[index].map_or((0.0, 1.0), |(low, high)| value_scale(low, high))
            };
            stroke(
                &line.samples,
                from,
                to,
                plot,
                line_scale,
                line.color,
                if index == 0 { px(1.75) } else { px(1.25) },
                window,
            );
        }
        if let Some(at) = spec.cursor.filter(|at| (from..=to).contains(at)) {
            let x = x_of(at, from, span, plot);
            window.paint_quad(fill(
                Bounds::new(point(x, plot.top()), size(px(1.0), plot.size.height)),
                theme.fg_dim,
            ));
            for (index, line) in spec.lines.iter().enumerate() {
                let line_scale = if shared[index] {
                    scale
                } else {
                    ranges[index].map_or((0.0, 1.0), |(low, high)| value_scale(low, high))
                };
                if let Some((_, value)) = nearest(&line.samples, at)
                    && value.is_finite()
                {
                    let y = y_of(value, line_scale, plot);
                    window.paint_quad(
                        fill(
                            Bounds::new(point(x - px(3.0), y - px(3.0)), size(px(6.0), px(6.0))),
                            line.color,
                        )
                        .corner_radii(px(3.0)),
                    );
                }
            }
        }
    });
}

#[allow(clippy::cast_possible_truncation)]
fn x_of(at: f64, from: f64, span: f64, plot: Bounds<Pixels>) -> Pixels {
    plot.left() + plot.size.width * ((at - from) / span) as f32
}

#[allow(clippy::cast_possible_truncation)]
fn y_of(value: f64, (low, high): (f64, f64), plot: Bounds<Pixels>) -> Pixels {
    let fraction = ((value - low) / (high - low)).clamp(0.0, 1.0);
    plot.bottom() - plot.size.height * fraction as f32
}

/// A polyline with breaks.
struct Pen {
    builder: PathBuilder,
    drawing: bool,
    segments: usize,
    last: Option<Point<Pixels>>,
}

impl Pen {
    fn to(&mut self, next: Point<Pixels>) {
        if self.drawing {
            self.builder.line_to(next);
            self.segments += 1;
        } else {
            self.builder.move_to(next);
            self.drawing = true;
        }
        self.last = Some(next);
    }
}

/// One pixel column's lowest and highest value while thinning, each with
/// its place in the series, so the line visits them in time order.
struct Column {
    index: i32,
    x: Pixels,
    low: (usize, f64),
    high: (usize, f64),
}

impl Column {
    fn add(&mut self, place: usize, value: f64) {
        if value < self.low.1 {
            self.low = (place, value);
        }
        if value > self.high.1 {
            self.high = (place, value);
        }
    }

    fn draw(&self, pen: &mut Pen, scale: (f64, f64), plot: Bounds<Pixels>) {
        let (first, second) = if self.low.0 <= self.high.0 {
            (self.low.1, self.high.1)
        } else {
            (self.high.1, self.low.1)
        };
        pen.to(point(self.x, y_of(first, scale, plot)));
        pen.to(point(self.x, y_of(second, scale, plot)));
    }
}

/// The line through the samples in `[from, to]` (and one either side, so
/// it runs off the edges). Breaks at non-finite values. With more than two
/// samples per pixel it keeps each column's lowest and highest.
#[allow(
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]
fn stroke(
    samples: &[(f64, f64)],
    from: f64,
    to: f64,
    plot: Bounds<Pixels>,
    scale: (f64, f64),
    color: Hsla,
    width: Pixels,
    window: &mut Window,
) {
    let start = samples
        .partition_point(|(at, _)| *at < from)
        .saturating_sub(1);
    let end = (samples.partition_point(|(at, _)| *at <= to) + 1).min(samples.len());
    let Some(visible) = samples.get(start..end) else {
        return;
    };
    let span = to - from;
    let columns = f32::from(plot.size.width).max(1.0);
    let dense = visible.len() as f32 > columns * 2.0;
    let mut pen = Pen {
        builder: PathBuilder::stroke(width),
        drawing: false,
        segments: 0,
        last: None,
    };
    let mut column: Option<Column> = None;
    for (place, (at, value)) in visible.iter().enumerate() {
        if !value.is_finite() {
            if let Some(open) = column.take() {
                open.draw(&mut pen, scale, plot);
            }
            pen.drawing = false;
            continue;
        }
        let x = x_of(*at, from, span, plot);
        if !dense {
            pen.to(point(x, y_of(*value, scale, plot)));
            continue;
        }
        let index = f32::from(x).floor() as i32;
        match column.as_mut() {
            Some(open) if open.index == index => open.add(place, *value),
            _ => {
                if let Some(open) = column.take() {
                    open.draw(&mut pen, scale, plot);
                }
                column = Some(Column {
                    index,
                    x,
                    low: (place, *value),
                    high: (place, *value),
                });
            }
        }
    }
    if let Some(open) = column.take() {
        open.draw(&mut pen, scale, plot);
    }
    if pen.segments == 0 {
        // A lone sample still shows where the value is.
        if let Some(at) = pen.last {
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(at.x - px(2.0), at.y - px(2.0)),
                        size(px(4.0), px(4.0)),
                    ),
                    color,
                )
                .corner_radii(px(2.0)),
            );
        }
        return;
    }
    if let Ok(path) = pen.builder.build() {
        window.paint_path(path, color);
    }
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
    use super::*;

    #[test]
    fn steps_leave_each_label_its_room() {
        // An hour over 600 px: six labels fit, so ten minutes apart.
        assert!((tick_step(3_600_000.0, 600.0) - 600_000.0).abs() < f64::EPSILON);
        // Five minutes over 400 px: four labels, two minutes apart.
        assert!((tick_step(300_000.0, 400.0) - 120_000.0).abs() < f64::EPSILON);
        // A week over 300 px: three labels at most.
        assert!((tick_step(7.0 * 86_400_000.0, 300.0) - 604_800_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ticks_fall_on_round_times_of_the_local_clock() {
        let zone = &LocalZone::named("Asia/Jakarta").unwrap();
        // 12:59:30 to 14:01:00 on a UTC+7 clock: ticks at 13:00 and 14:00.
        let from = 1_792_911_570_000.0 - 3_600_000.0;
        let ticks = ticks(from, from + 3_690_000.0, 3_600_000.0, zone);
        assert_eq!(ticks.len(), 2);
        assert_eq!(
            tick_label(ticks[0], 3_600_000.0, 3_690_000.0, zone),
            "13:00"
        );
        assert_eq!(
            tick_label(ticks[1], 3_600_000.0, 3_690_000.0, zone),
            "14:00"
        );
        assert_eq!(
            tick_label(0.0, 1_000.0, 60_000.0, &LocalZone::utc()),
            "00:00:00"
        );
        assert_eq!(
            tick_label(0.0, 86_400_000.0, 7.0 * 86_400_000.0, &LocalZone::utc()),
            "1 Jan"
        );
    }

    #[test]
    fn day_ticks_stay_on_midnight_across_a_daylight_saving_change() {
        let york = &LocalZone::named("America/New_York").unwrap();
        let day = 86_400_000.0;
        // 2020-10-31 12:00 EDT to 2020-11-03 11:00 EST; the clocks went back
        // early on 1 November.
        let from = 1_604_160_000_000.0;
        let ticks = ticks(from, from + 3.0 * day - 3_600_000.0, day, york);
        let labels: Vec<String> = ticks
            .iter()
            .map(|at| tick_label(*at, day, 3.0 * day, york))
            .collect();
        assert_eq!(labels, ["1 Nov", "2 Nov", "3 Nov"]);
        assert!(
            (ticks[1] - ticks[0] - 25.0 * 3_600_000.0).abs() < f64::EPSILON,
            "1 November is 25 hours long"
        );
        // Six-hour ticks stay on 00, 06, 12 and 18 o'clock either side.
        let six = super::ticks(from, from + 4.0 * day, 6.0 * 3_600_000.0, york);
        assert!(six.len() > 12);
        for at in &six {
            let wall = york.at((*at / 1000.0) as i64).unwrap();
            assert_eq!((wall.hour() % 6, wall.minute()), (0, 0));
        }
    }

    #[test]
    fn the_nearest_sample_wins_and_an_empty_series_has_none() {
        let samples = [(1000.0, 1.0), (2000.0, 2.0), (4000.0, 4.0)];
        assert_eq!(nearest(&samples, 1400.0), Some((1000.0, 1.0)));
        assert_eq!(nearest(&samples, 3100.0), Some((4000.0, 4.0)));
        assert_eq!(nearest(&samples, 9000.0), Some((4000.0, 4.0)));
        assert_eq!(nearest(&samples, 0.0), Some((1000.0, 1.0)));
        assert_eq!(nearest(&[], 0.0), None);
        assert_eq!(visible_range(&samples, 1500.0, 4000.0), Some((2.0, 4.0)));
        assert_eq!(visible_range(&samples, 5000.0, 6000.0), None);
    }
}
