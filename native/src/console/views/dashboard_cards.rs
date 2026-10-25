//! What a dashboard card draws, by its tag's `chart_type` (the web client's
//! `Chart.tsx`): a sparkline with the warning and critical limits, a gauge,
//! a big number with its level bar, a picture or a text note. Gauge needles
//! and level bars glide to a new value; the dashboard asks for frames only
//! while one does.

use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, Div, FontWeight, Hsla, Image, ObjectFit, PathBuilder, Pixels, Point,
    SharedString, Window, canvas, div, fill, img, point, px, size,
};
use vt_ui::Icon;
use vt_ui::chart::{History, format_value, sparkline, value_scale};
use vt_ui::components::{pill, status_dot};
use vt_ui::theme::*;

use super::{fit_span, span_label, status_color, value_color, value_with_unit};
use crate::console::anim;
use crate::console::store::{Status, Tag};
use voltrus_scada_native::screens::CardKind;

/// Sparklines reach back this far at most, less while the history is young.
const SPARK_MS: f64 = 15.0 * 60_000.0;
const SHORTEST_MS: f64 = 2.0 * 60_000.0;
/// The gauge's arc.
const ARC_WIDTH: f32 = 10.0;

/// An image card's picture, fetched once per URL.
#[derive(Clone)]
pub(super) enum Picture {
    Loading,
    Ready(Arc<Image>),
    Failed(SharedString),
}

/// A tag's colour (`#rrggbb` or `#rgb`, as the web client stores it), or
/// `fallback`.
pub(super) fn tag_color(text: &str, fallback: Hsla) -> Hsla {
    let hex = text.trim().trim_matches('"').trim_start_matches('#');
    let full = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => hex.to_owned(),
        _ => return fallback,
    };
    u32::from_str_radix(&full, 16).map_or(fallback, |value| gpui::rgb(value).into())
}

/// The web client's source badge: live device, simulation, API or none.
fn source_badge(source_type: &str, theme: &Theme) -> (SharedString, Hsla) {
    match source_type {
        "modbus" => ("REAL".into(), theme.success),
        "random_walk" | "sine" | "step" | "sawtooth" | "constant" | "noise" => {
            ("SIM".into(), theme.warning)
        }
        "api" => ("API".into(), theme.series(3)),
        "none" | "" => ("NONE".into(), theme.fg_dim),
        other => (other.to_uppercase().into(), theme.info),
    }
}

/// The card's top row: status, name, source badge, simulation mark and
/// status pill.
pub(super) fn header(tag: &Tag, theme: &Theme) -> Div {
    let color = status_color(tag.status, theme);
    let (source, source_color) = source_badge(&tag.info.source_type, theme);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_3)
        .min_w_0()
        .child(status_dot(color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(FONT_SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_muted)
                .child(tag.info.name.clone()),
        )
        .child(pill(source, source_color))
        .when(tag.simulated, |this| this.child(pill("SIM", theme.warning)))
        .child(pill(tag.status.label(), color))
}

/// The value a moving needle or bar shows now: from the previous value to
/// the new one over [`anim::TWEEN`]; whether it is still moving.
fn tweened(tag: &Tag, now: Instant) -> (f64, bool) {
    match tag
        .changed_at
        .and_then(|changed| anim::progress(changed, now, anim::TWEEN))
    {
        Some(t) if tag.previous.is_finite() && tag.value.is_finite() => {
            (anim::lerp(tag.previous, tag.value, anim::ease_out(t)), true)
        }
        _ => (tag.value, false),
    }
}

fn decimals(tag: &Tag) -> usize {
    usize::try_from(tag.info.decimals.clamp(0, 6)).unwrap_or(2)
}

/// The colour a value reads in: the tag's own while normal, the status's
/// otherwise (the web client's rule).
fn reading_color(tag: &Tag, own: Hsla, theme: &Theme) -> Hsla {
    if tag.status == Status::Normal {
        own
    } else {
        value_color(tag.status, theme)
    }
}

/// What the card shows under its header: `height` is the card's in pixels.
/// Returns the element and whether it is animating.
pub(super) fn body(
    tag: &Tag,
    kind: CardKind,
    picture: Option<&Picture>,
    (now, now_ms): (Instant, f64),
    height: f32,
    theme: &Theme,
) -> (AnyElement, bool) {
    let own = tag_color(&tag.info.color, theme.accent);
    match kind {
        CardKind::Sparkline => (spark(tag, own, now_ms, theme), false),
        CardKind::Gauge => gauge(tag, own, now, theme),
        CardKind::BigNumber => big_number(tag, own, now, height, theme),
        CardKind::Image => (picture_card(picture, theme), false),
        CardKind::Text => (
            text_card(
                &voltrus_scada_native::screens::card_config(
                    &tag.info.source_config,
                    "text_content",
                ),
                theme,
            ),
            false,
        ),
    }
}

fn spark(tag: &Tag, own: Hsla, now_ms: f64, theme: &Theme) -> AnyElement {
    let span = fit_span(tag.span_ms(now_ms), SHORTEST_MS, SPARK_MS);
    let history = tag.history.clone();
    let guides = Guides {
        history: history.clone(),
        now_ms,
        span,
        limits: [
            (tag.info.warn_val, theme.warning),
            (tag.info.crit_val, theme.danger),
        ],
        color: own,
    };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap(SPACE_3)
        .child(value_with_unit(
            tag.display(),
            &tag.info.unit,
            FONT_DISPLAY,
            value_color(tag.status, theme),
            theme,
        ))
        .child(
            div()
                .relative()
                .flex_1()
                .min_h(px(24.0))
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, (), window, _| guides.paint(bounds, window),
                    )
                    .absolute()
                    .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .size_full()
                        .child(sparkline(history, own, now_ms, span)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap(SPACE_4)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(format!("last {}", span_label(span)))
                .child(div().flex_1())
                .when(tag.info.warn_val != 0.0, |this| {
                    this.child(format!("warn {}", format_value(tag.info.warn_val)))
                })
                .when(tag.info.crit_val != 0.0, |this| {
                    this.child(format!("crit {}", format_value(tag.info.crit_val)))
                }),
        )
        .into_any_element()
}

/// The sparkline's limit lines and latest-value dot, on the sparkline's
/// own scale.
struct Guides {
    history: std::rc::Rc<History>,
    now_ms: f64,
    span: f64,
    limits: [(f64, Hsla); 2],
    color: Hsla,
}

impl Guides {
    #[allow(clippy::cast_possible_truncation)]
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let since = self.now_ms - self.span;
        let Some((low, high)) = self.history.range(since) else {
            return;
        };
        let (low, high) = value_scale(low, high);
        let y_of = |value: f64| {
            let share = ((value - low) / (high - low)).clamp(0.0, 1.0) as f32;
            bounds.bottom() - px(1.0) - (bounds.size.height - px(2.0)) * share
        };
        for (limit, color) in self.limits {
            if limit == 0.0 || limit <= low || limit >= high {
                continue;
            }
            let y = y_of(limit);
            let mut line = PathBuilder::stroke(px(1.0)).dash_array(&[px(4.0), px(4.0)]);
            line.move_to(point(bounds.left(), y));
            line.line_to(point(bounds.right(), y));
            if let Ok(path) = line.build() {
                window.paint_path(path, color.opacity(0.55));
            }
        }
        if let Some((at, value)) = self.history.latest()
            && value.is_finite()
        {
            let share = ((at - since) / self.span).clamp(0.0, 1.0) as f32;
            let x = bounds.left() + bounds.size.width * share;
            let y = y_of(value);
            window.paint_quad(
                fill(
                    Bounds::new(point(x - px(6.0), y - px(6.0)), size(px(12.0), px(12.0))),
                    self.color.opacity(0.22),
                )
                .corner_radii(px(6.0)),
            );
            window.paint_quad(
                fill(
                    Bounds::new(point(x - px(3.0), y - px(3.0)), size(px(6.0), px(6.0))),
                    self.color,
                )
                .corner_radii(px(3.0)),
            );
        }
    }
}

/// A point on the gauge's half circle: `share` 0 at the left end, 1 at the
/// right.
#[allow(clippy::cast_possible_truncation)]
fn on_arc(center: Point<Pixels>, radius: Pixels, share: f64) -> Point<Pixels> {
    let angle = PI * (1.0 - share.clamp(0.0, 1.0));
    point(
        center.x + radius * (angle.cos() as f32),
        center.y - radius * (angle.sin() as f32),
    )
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn paint_arc(
    window: &mut Window,
    center: Point<Pixels>,
    radius: Pixels,
    (from, to): (f64, f64),
    width: Pixels,
    color: Hsla,
) {
    if to <= from {
        return;
    }
    let steps = ((to - from) * 64.0).ceil().max(1.0) as usize;
    let mut path = PathBuilder::stroke(width);
    for step in 0..=steps {
        #[allow(clippy::cast_precision_loss)]
        let share = from + (to - from) * step as f64 / steps as f64;
        let at = on_arc(center, radius, share);
        if step == 0 {
            path.move_to(at);
        } else {
            path.line_to(at);
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// The gauge's bands, needle and hub.
struct Dial {
    /// The needle's place, 0 to 1; `None` before a value.
    needle: Option<f64>,
    /// Where warning and critical start, 0 to 1; `None` without limits.
    limits: Option<(f64, f64)>,
    track: Hsla,
    bands: [Hsla; 3],
    color: Hsla,
}

impl Dial {
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let width = px(ARC_WIDTH);
        let radius = (bounds.size.width * 0.5)
            .min(bounds.size.height)
            .max(px(ARC_WIDTH * 2.0))
            - width * 0.5
            - px(2.0);
        let center = point(
            bounds.left() + bounds.size.width * 0.5,
            bounds.bottom() - px(2.0),
        );
        paint_arc(window, center, radius, (0.0, 1.0), width, self.track);
        if let Some((warn, crit)) = self.limits {
            paint_arc(window, center, radius, (0.0, warn), width, self.bands[0]);
            paint_arc(window, center, radius, (warn, crit), width, self.bands[1]);
            paint_arc(window, center, radius, (crit, 1.0), width, self.bands[2]);
        }
        let Some(needle) = self.needle else {
            return;
        };
        let tip = on_arc(center, radius * 0.72, needle);
        let mut line = PathBuilder::stroke(px(2.5));
        line.move_to(center);
        line.line_to(tip);
        if let Ok(path) = line.build() {
            window.paint_path(path, self.color);
        }
        window.paint_quad(
            fill(
                Bounds::new(
                    point(center.x - px(4.0), center.y - px(4.0)),
                    size(px(8.0), px(8.0)),
                ),
                self.color,
            )
            .corner_radii(px(4.0)),
        );
    }
}

fn gauge(tag: &Tag, own: Hsla, now: Instant, theme: &Theme) -> (AnyElement, bool) {
    let info = &tag.info;
    let (min, max) = (info.min_val, info.max_val);
    let range = if max > min { max - min } else { 1.0 };
    let share = |value: f64| ((value - min) / range).clamp(0.0, 1.0);
    let (value, moving) = tweened(tag, now);
    let limits = (info.warn_val != 0.0 || info.crit_val != 0.0).then(|| {
        let warn = share(info.warn_val);
        (warn, share(info.crit_val).max(warn))
    });
    let dial = Dial {
        needle: value.is_finite().then(|| share(value)),
        limits,
        track: theme.border,
        bands: [
            theme.success.opacity(0.75),
            theme.warning.opacity(0.85),
            theme.danger.opacity(0.85),
        ],
        color: own,
    };
    let reading = if value.is_finite() {
        format!("{:.*}", decimals(tag), value)
    } else {
        "—".to_owned()
    };
    let element = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap(SPACE_3)
        .child(
            div().relative().flex_1().min_h(px(32.0)).child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, _| dial.paint(bounds, window),
                )
                .absolute()
                .size_full(),
            ),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_end()
                .gap(SPACE_4)
                .child(limit_caption(min, theme))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .justify_center()
                        .child(value_with_unit(
                            reading,
                            &info.unit,
                            FONT_DISPLAY,
                            reading_color(tag, own, theme),
                            theme,
                        )),
                )
                .child(limit_caption(max, theme)),
        )
        .into_any_element();
    (element, moving)
}

fn limit_caption(value: f64, theme: &Theme) -> Div {
    div()
        .w(px(44.0))
        .flex_none()
        .font_family(MONO)
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_dim)
        .child(format_value(value))
}

fn big_number(
    tag: &Tag,
    own: Hsla,
    now: Instant,
    height: f32,
    theme: &Theme,
) -> (AnyElement, bool) {
    let info = &tag.info;
    let (value, moving) = tweened(tag, now);
    let color = reading_color(tag, own, theme);
    // The web client's bar: the value against the critical limit (the
    // warning one when there is none).
    let scale = if info.crit_val != 0.0 {
        info.crit_val
    } else {
        info.warn_val
    };
    let fill_share = if info.warn_val > 0.0 && scale != 0.0 && value.is_finite() {
        (value / scale).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let reading = if value.is_finite() {
        format!("{:.*}", decimals(tag), value)
    } else {
        "—".to_owned()
    };
    let font = px((height * 0.24).clamp(26.0, 64.0));
    #[allow(clippy::cast_possible_truncation)]
    let fill_share = fill_share as f32;
    let element = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap(SPACE_3)
        .child(
            div()
                .font_family(MONO)
                .text_size(font)
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .whitespace_nowrap()
                .child(reading),
        )
        .when(!info.unit.is_empty(), |this| {
            this.child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(info.unit.to_uppercase()),
            )
        })
        .child(
            div()
                .mt(SPACE_4)
                .w(gpui::relative(0.8))
                .h(px(4.0))
                .rounded_full()
                .bg(theme.border)
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(fill_share))
                        .rounded_full()
                        .bg(color),
                ),
        )
        .into_any_element();
    (element, moving)
}

fn note(icon: Icon, text: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(SPACE_3)
        .text_size(FONT_SMALL)
        .text_color(theme.fg_dim)
        .child(vt_ui::icon(icon, ICON_LG, theme.fg_dim))
        .child(text.into())
        .into_any_element()
}

fn picture_card(picture: Option<&Picture>, theme: &Theme) -> AnyElement {
    match picture {
        None => note(Icon::File, "No image configured", theme),
        Some(Picture::Loading) => note(Icon::Download, "Loading the image…", theme),
        Some(Picture::Failed(why)) => note(Icon::CircleAlert, why.clone(), theme),
        Some(Picture::Ready(image)) => div()
            .flex_1()
            .min_h_0()
            .rounded(RADIUS_MD)
            .overflow_hidden()
            .child(
                img(image.clone())
                    .size_full()
                    .object_fit(ObjectFit::Contain),
            )
            .into_any_element(),
    }
}

/// Markdown's inline marks dropped: emphasis, code and link targets.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let (before, from) = rest.split_at(start);
        out.push_str(before);
        match from
            .find("](")
            .and_then(|middle| from[middle..].find(')').map(|end| (middle, middle + end)))
        {
            Some((middle, end)) => {
                out.push_str(&from[1..middle]);
                rest = &from[end + 1..];
            }
            None => {
                out.push('[');
                rest = &from[1..];
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace("__", "").replace('`', "")
}

/// A text card: its Markdown note with headings, bullets and paragraphs
/// (inline marks are dropped).
fn text_card(text: &str, theme: &Theme) -> AnyElement {
    if text.trim().is_empty() {
        return note(Icon::ScrollText, "No text configured", theme);
    }
    let lines = text.lines().map(|line| {
        let line = line.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            return div().h(SPACE_3);
        }
        let heading = |level: &str| trimmed.strip_prefix(level).map(plain);
        if let Some(title) = heading("### ") {
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(FONT_EMPHASIS)
                .child(title)
        } else if let Some(title) = heading("## ") {
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(FONT_TITLE)
                .child(title)
        } else if let Some(title) = heading("# ") {
            div()
                .font_weight(FontWeight::BOLD)
                .text_size(FONT_DISPLAY)
                .child(title)
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            div()
                .flex()
                .gap(SPACE_3)
                .child(div().text_color(theme.fg_dim).child("•"))
                .child(div().flex_1().min_w_0().child(plain(item)))
        } else {
            div().child(plain(trimmed))
        }
    });
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .gap(SPACE_2)
        .text_size(FONT_BODY)
        .text_color(theme.fg)
        .children(lines)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_colours_parse_like_the_web_stores_them() {
        let fallback = gpui::black();
        let amber: Hsla = gpui::rgb(0xf5_9e_0b).into();
        assert_eq!(tag_color("#f59e0b", fallback), amber);
        assert_eq!(tag_color("\"#f59e0b\"", fallback), amber);
        let white: Hsla = gpui::rgb(0xff_ff_ff).into();
        assert_eq!(tag_color("#fff", fallback), white);
        assert_eq!(tag_color("teal", fallback), fallback);
        assert_eq!(tag_color("", fallback), fallback);
    }

    #[test]
    fn markdown_marks_are_dropped_from_text_cards() {
        assert_eq!(plain("**Shift A** on `inverter`"), "Shift A on inverter");
        assert_eq!(
            plain("See [the manual](https://example.com) now"),
            "See the manual now"
        );
        assert_eq!(plain("a [bracket"), "a [bracket");
    }
}
