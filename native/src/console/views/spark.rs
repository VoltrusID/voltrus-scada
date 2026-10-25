//! A sparkline drawn with quads rather than a path: the samples in view are
//! put into columns, and each column is one thin bar from its lowest to its
//! highest point (joined to its neighbours), so the line reads the same.
//!
//! Paths are costly on the GPU here: each batch of them is rasterized into
//! a full-window multisampled texture and composited back. With a sparkline
//! on every tile and table row, that was most of the time spent presenting
//! a frame. Quads go through in one instanced batch. (A vt-ui candidate.)

use gpui::prelude::*;
use gpui::{Bounds, Hsla, canvas, fill, point, px, size};
use vt_ui::chart::value_scale;

use crate::console::store::Tag;

/// How thick the line is drawn where it runs flat.
const THICKNESS: f32 = 1.25;

/// One column's extent, 0 at the bottom of the chart to 1 at its top.
pub(crate) type Column = Option<(f32, f32)>;

/// The samples between `since` and `since + span` (time order, `NaN`
/// breaks the line) as `count` columns, each covering what the line
/// crosses there; `None` where there is no line.
pub(crate) fn columns(
    samples: impl Iterator<Item = (f64, f64)> + Clone,
    since: f64,
    span: f64,
    count: usize,
) -> Vec<Column> {
    let mut out: Vec<Column> = vec![None; count];
    if count == 0 || span <= 0.0 {
        return out;
    }
    let in_view = samples.skip_while(move |(at, _)| *at < since);
    let Some((low, high)) = in_view.clone().filter(|(_, value)| value.is_finite()).fold(
        None,
        |range: Option<(f64, f64)>, (_, value)| {
            Some(range.map_or((value, value), |(low, high)| {
                (low.min(value), high.max(value))
            }))
        },
    ) else {
        return out;
    };
    let (low, high) = value_scale(low, high);
    #[allow(clippy::cast_possible_truncation)]
    let level = |value: f64| ((value - low) / (high - low)).clamp(0.0, 1.0) as f32;
    #[allow(clippy::cast_precision_loss)]
    let last = (count - 1) as f64;
    // The previous point and its column, while the line is unbroken.
    let mut previous: Option<(usize, f32)> = None;
    for (at, value) in in_view {
        if !value.is_finite() {
            previous = None;
            continue;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let column = (((at - since) / span) * last).round().clamp(0.0, last) as usize;
        let y = level(value);
        match previous {
            // Join the previous point, spreading the step over the columns
            // in between.
            Some((from, from_y)) if column > from => {
                #[allow(clippy::cast_precision_loss)]
                let steps = (column - from) as f32;
                for (step, slot) in out[from + 1..=column].iter_mut().enumerate() {
                    #[allow(clippy::cast_precision_loss)]
                    let start = from_y + (y - from_y) * (step as f32 / steps);
                    #[allow(clippy::cast_precision_loss)]
                    let end = from_y + (y - from_y) * ((step + 1) as f32 / steps);
                    *slot = Some(widen(*slot, start.min(end), start.max(end)));
                }
            }
            _ => out[column] = Some(widen(out[column], y, y)),
        }
        previous = Some((column, y));
    }
    out
}

fn widen(column: Column, low: f32, high: f32) -> (f32, f32) {
    column.map_or((low, high), |(lo, hi)| (lo.min(low), hi.max(high)))
}

/// The tag's sparkline over the `span_ms` up to `now_ms`, `count` columns
/// wide (about one per two pixels of its width).
pub(crate) fn spark(
    tag: &Tag,
    color: Hsla,
    now_ms: f64,
    span_ms: f64,
    count: usize,
) -> impl IntoElement {
    let columns = columns(
        tag.samples_since(now_ms - span_ms),
        now_ms - span_ms,
        span_ms,
        count,
    );
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            if columns.iter().all(Option::is_none) {
                return;
            }
            #[allow(clippy::cast_precision_loss)]
            let width = f32::from(bounds.size.width) / columns.len() as f32;
            let height = f32::from(bounds.size.height) - THICKNESS;
            // One layer: the bars share a draw order and a batch.
            window.paint_layer(bounds, |window| {
                for (index, column) in columns.iter().enumerate() {
                    let Some((low, high)) = *column else {
                        continue;
                    };
                    #[allow(clippy::cast_precision_loss)]
                    let left = f32::from(bounds.left()) + width * index as f32;
                    let top = f32::from(bounds.bottom()) - THICKNESS - high * height;
                    let tall = (high - low) * height + THICKNESS;
                    window.paint_quad(fill(
                        Bounds::new(point(px(left), px(top)), size(px(width + 0.25), px(tall))),
                        color,
                    ));
                }
            });
        },
    )
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: &[(f64, f64)], count: usize) -> Vec<Column> {
        columns(points.iter().copied(), 0.0, 100.0, count)
    }

    #[test]
    fn a_rising_line_fills_every_column_bottom_to_top() {
        let columns = line(&[(0.0, 0.0), (100.0, 10.0)], 5);
        assert!(columns.iter().all(Option::is_some));
        let (first, last) = (columns[0].unwrap(), columns[4].unwrap());
        assert!(first.0 < 0.1 && last.1 > 0.9, "{columns:?}");
        // Neighbours meet: each column starts where the last one ended.
        for pair in columns.windows(2) {
            let (a, b) = (pair[0].unwrap(), pair[1].unwrap());
            assert!((a.1 - b.0).abs() < 1e-5, "{columns:?}");
        }
    }

    #[test]
    fn a_break_leaves_a_gap_and_old_samples_are_left_out() {
        let columns = line(
            &[
                (-50.0, 99.0),
                (0.0, 1.0),
                (25.0, 2.0),
                (50.0, f64::NAN),
                (75.0, 3.0),
                (100.0, 4.0),
            ],
            5,
        );
        assert_eq!(columns[2], None, "the offline stretch is not drawn");
        assert!(columns[0].is_some() && columns[1].is_some() && columns[3].is_some());
        // The older sample would have stretched the scale towards 99.
        assert!(columns[4].unwrap().1 > 0.8);
    }

    #[test]
    fn nothing_in_view_draws_nothing() {
        assert!(line(&[], 4).iter().all(Option::is_none));
        assert!(line(&[(10.0, f64::NAN)], 4).iter().all(Option::is_none));
        assert!(line(&[(10.0, 1.0)], 0).is_empty());
        // A lone point is one column.
        assert_eq!(line(&[(50.0, 1.0)], 3).iter().flatten().count(), 1);
    }
}
