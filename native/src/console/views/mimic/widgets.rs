//! The mimic's non-symbol widgets (`scada2/widgets/*.tsx`): the process
//! area, gauges, bars, lights, labels, trends, tables, charts, the
//! annunciator, the PID faceplate and the operator controls, each in its
//! current state.
//!
//! Local units are the widget box's CSS pixels; the web's SVG view boxes are
//! fitted like `preserveAspectRatio="xMidYMid meet"` and its flex layouts
//! are laid out here by hand. Controls draw their state only: the page opens
//! a faceplate on a click.

use std::fmt::Write as _;

use gpui::Hsla;
use voltrus_scada_native::mimic::{
    Level, Light, Reading, WidgetKind, fraction, is_on, status_light, zone,
};

use super::paint::{Align, Canvas, Hmi, Line, Paint, Shape, Text, Xf, fade};
use super::scene::{Item, TagView, motion};

/// Draw `item` (any widget but the library's equipment) in its box.
pub(crate) fn paint(canvas: &mut Canvas<'_>, item: &Item, xf: &Xf, t: f32) {
    match &item.widget.kind {
        WidgetKind::Area => area(canvas, item, xf),
        WidgetKind::Gauge => gauge(canvas, item, xf),
        WidgetKind::Indicator => indicator(canvas, item, xf),
        WidgetKind::Bar => bar(canvas, item, xf),
        WidgetKind::StatusLight => light(canvas, item, xf),
        WidgetKind::OnOff => on_off(canvas, item, xf),
        WidgetKind::Label => label(canvas, item, xf),
        WidgetKind::Trend => trend(canvas, item, xf),
        WidgetKind::BigNumber => big_number(canvas, item, xf),
        WidgetKind::DataTable => data_table(canvas, item, xf),
        WidgetKind::Slider => slider(canvas, item, xf),
        WidgetKind::NumberInput => number_input(canvas, item, xf),
        WidgetKind::Toggle => toggle(canvas, item, xf),
        WidgetKind::Meter => meter(canvas, item, xf),
        WidgetKind::SemicircleGauge => semicircle(canvas, item, xf),
        WidgetKind::Donut => donut(canvas, item, xf),
        WidgetKind::LedDisplay => led(canvas, item, xf),
        WidgetKind::PushButton => push_button(canvas, item, xf),
        WidgetKind::BarChart => bar_chart(canvas, item, xf),
        WidgetKind::PieChart => pie_chart(canvas, item, xf),
        WidgetKind::MultiTrend => multi_trend(canvas, item, xf),
        WidgetKind::Annunciator => annunciator(canvas, item, xf, t),
        WidgetKind::PidFaceplate => pid(canvas, item, xf),
        WidgetKind::ProgressBar => progress_bar(canvas, item, xf),
        _ => {}
    }
}

/// Whether the widget blinks (`hmi-blink`): an annunciator with a critical
/// window. The page schedules blink frames while any does.
pub(crate) fn blinking(item: &Item) -> bool {
    item.widget.kind == WidgetKind::Annunciator
        && item.tags.iter().any(|tag| tag.level == Level::Critical)
}

// ── Shared ────────────────────────────────────────────────────────────────

fn size(item: &Item) -> (f32, f32) {
    (item.widget.width as f32, item.widget.height as f32)
}

/// The bound tag's scale with the web's fallbacks (`sensor?.min_val ?? 0`…).
struct Scale<'a> {
    min: f64,
    max: f64,
    warn: f64,
    crit: f64,
    decimals: usize,
    unit: &'a str,
    name: &'a str,
    value: Option<f64>,
}

impl<'a> Scale<'a> {
    fn of(tag: Option<&'a TagView>) -> Self {
        match tag {
            Some(tag) => Self {
                min: tag.min,
                max: tag.max,
                warn: tag.warn,
                crit: tag.crit,
                decimals: tag.decimals,
                unit: tag.unit.as_str(),
                name: tag.name.as_str(),
                value: tag.value,
            },
            None => Self {
                min: 0.0,
                max: 100.0,
                warn: 75.0,
                crit: 90.0,
                decimals: 1,
                unit: "",
                name: "",
                value: None,
            },
        }
    }

    /// `data?.value ?? min`.
    fn shown(&self) -> f64 {
        self.value.unwrap_or(self.min)
    }

    /// `max - min || 1`.
    fn range(&self) -> f64 {
        let range = self.max - self.min;
        if range == 0.0 { 1.0 } else { range }
    }

    fn at(&self, value: f64) -> f32 {
        fraction(value, self.min, self.max) as f32
    }

    fn pct(&self) -> f32 {
        self.at(self.shown())
    }

    fn zone(&self) -> Level {
        zone(self.shown(), self.min, self.max, self.warn, self.crit)
    }

    /// The value with the tag's decimals, `--` without one.
    fn text(&self) -> String {
        self.value
            .map_or_else(|| "--".to_owned(), |v| format!("{v:.*}", self.decimals))
    }
}

/// `--ok` / `--warn` / `--crit` for a band or a status.
fn level_color(hmi: &Hmi, level: Level) -> Hsla {
    match level {
        Level::Critical => hmi.danger,
        Level::Warning => hmi.warning,
        Level::Normal => hmi.ok,
    }
}

/// The CSS variable a colour setting names (`var(--ok)` → `ok`).
fn var_name(setting: &str) -> Option<&str> {
    setting
        .trim()
        .strip_prefix("var(--")?
        .strip_suffix(')')
        .map(str::trim)
}

/// A colour setting: a known theme variable as its token, anything else
/// (a literal colour) as the default.
fn token(hmi: &Hmi, setting: Option<&str>, default: Hsla) -> Hsla {
    let Some(name) = setting.and_then(var_name) else {
        return default;
    };
    match name {
        "ok" | "success" => hmi.ok,
        "warn" | "warning" => hmi.warning,
        "crit" | "danger" => hmi.danger,
        "cyan" | "accent" => hmi.accent,
        "muted" | "text-muted" => hmi.fg_muted,
        "text-dim" => hmi.fg_dim,
        "fg" | "text" => hmi.fg,
        "border" => hmi.border,
        "bg" => hmi.bg,
        "card" => hmi.surface,
        "hmi-run" => hmi.run,
        "hmi-fault" => hmi.fault,
        "hmi-warn" => hmi.warn,
        "hmi-stale" => hmi.stale,
        "hmi-stroke" => hmi.stroke,
        _ => default,
    }
}

/// `preserveAspectRatio="xMidYMid meet"`: the scale of a `view` box fitted
/// in `area`, and its offset.
fn meet(area: (f32, f32), view: (f32, f32)) -> (f32, f32, f32) {
    if view.0 <= 0.0 || view.1 <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let scale = (area.0 / view.0).min(area.1 / view.1).max(0.0);
    (
        scale,
        (area.0 - view.0 * scale) / 2.0,
        (area.1 - view.1 * scale) / 2.0,
    )
}

/// The transform of an SVG view box drawn in a local rectangle.
fn view_box(xf: &Xf, (x, y, w, h): (f32, f32, f32, f32), view: (f32, f32)) -> Xf {
    let (scale, ox, oy) = meet((w, h), view);
    xf.then(&Xf::translate(x + ox, y + oy))
        .then(&Xf::scale(scale, scale))
}

/// The centre of SVG text whose baseline is at `y`.
fn base(y: f32, size: f32) -> f32 {
    y - size * 0.35
}

/// A number as JavaScript prints it (`${min}`).
fn js_num(value: f64) -> String {
    format!("{value}")
}

/// `toFixed(v % 1 === 0 ? 0 : 1)`.
fn auto_fixed(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// The first `keep` characters and `..` when longer than `limit`.
fn short(name: &str, limit: usize, keep: usize) -> String {
    if name.chars().count() > limit {
        name.chars().take(keep).collect::<String>() + ".."
    } else {
        name.to_owned()
    }
}

fn segment(c: &mut Canvas<'_>, xf: &Xf, from: (f32, f32), to: (f32, f32), color: Hsla, line: Line) {
    c.stroke(xf, &Shape::polyline(&[from, to], false), color, line);
}

/// Point at `degrees` (counter-clockwise, y up, as the web's gauges).
fn polar(cx: f32, cy: f32, r: f32, degrees: f32) -> (f32, f32) {
    let (sin, cos) = degrees.to_radians().sin_cos();
    (cx + r * cos, cy - r * sin)
}

/// A gauge needle: a triangle from a tail behind the hub to the tip.
fn needle(cx: f32, cy: f32, degrees: f32, length: f32, tail: f32, half: f32) -> Shape {
    let rad = degrees.to_radians();
    let tip = (cx + length * rad.cos(), cy - length * rad.sin());
    let back = (cx - tail * rad.cos(), cy + tail * rad.sin());
    let perp = rad + std::f32::consts::FRAC_PI_2;
    let b1 = (back.0 + half * perp.cos(), back.1 - half * perp.sin());
    let b2 = (back.0 - half * perp.cos(), back.1 + half * perp.sin());
    Shape::polyline(&[tip, b1, b2], true)
}

/// A drop shadow under `shape`: the shape a unit lower, translucent black.
fn shadow(c: &mut Canvas<'_>, xf: &Xf, shape: &Shape, alpha: f32) {
    let black = c.hmi.black;
    c.fill(
        &xf.then(&Xf::translate(0.0, 1.0)),
        shape,
        fade(black, alpha),
    );
}

// ── Area ──────────────────────────────────────────────────────────────────

fn area(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    c.rect(
        xf,
        (0.0, 0.0, w, h),
        4.0,
        hmi.area,
        Some((hmi.area_edge, 1.0)),
    );
    let title = widget
        .text("title")
        .unwrap_or("Process Area")
        .to_uppercase();
    let subtitle = widget.text("subtitle").unwrap_or("");
    let title_style = Text::new(12.0, hmi.fg_dim).bold().align(Align::Start);
    let sub_style = Text::new(11.0, hmi.fg_muted).mono().align(Align::Start);
    // `letter-spacing: .08em` widens the title.
    let title_w =
        c.text_width(xf, &title, title_style) + 12.0 * 0.08 * title.chars().count() as f32;
    let sub_w = if subtitle.is_empty() {
        0.0
    } else {
        8.0 + c.text_width(xf, subtitle, sub_style)
    };
    let (tw, th) = (10.0 + title_w + sub_w + 10.0, 5.0 + 14.4 + 4.0);
    let tab = format!("M0 0 H{tw} V{} A4 4 0 0 1 {} {th} H0 Z", th - 4.0, tw - 4.0);
    c.fill(xf, &Shape::svg_owned(&tab), fade(hmi.stroke, 0.1));
    let edge = format!("M{tw} 0 V{} A4 4 0 0 1 {} {th} H0", th - 4.0, tw - 4.0);
    c.stroke(
        xf,
        &Shape::svg_owned(&edge),
        hmi.area_edge,
        Line::scaled(1.0),
    );
    let mid = 5.0 + 7.2;
    c.text(xf, (10.0, mid), &title, title_style);
    if !subtitle.is_empty() {
        c.text(xf, (10.0 + title_w + 8.0, mid + 0.5), subtitle, sub_style);
    }
}

// ── Gauges ────────────────────────────────────────────────────────────────

fn gauge(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let s = Scale::of(item.primary.as_ref());
    let labels = item.widget.flag("showLabels") != Some(false);
    let named = labels && !s.name.is_empty();
    let content = h * 0.9 + if named { 2.0 + 13.2 } else { 0.0 };
    let top = (h - content) / 2.0;
    let v = view_box(xf, (0.0, top, w, h * 0.9), (200.0, 165.0));
    let (cx, cy, r, arc_w) = (100.0, 100.0, 75.0, 10.0);
    let pct = s.pct();
    let (warn, crit) = (s.at(s.warn), s.at(s.crit));
    let color = level_color(hmi, s.zone());
    // The 270° dial, clockwise on screen from 135° (the web's 225° to -45°).
    let arc =
        |from: f32, to: f32| Shape::arc(cx, cy, r, -225.0 + 270.0 * from, -225.0 + 270.0 * to);
    c.stroke(
        &v,
        &arc(0.0, 1.0),
        fade(hmi.border, 0.375),
        Line::scaled(arc_w).round(),
    );
    let zone_line = Line::scaled(arc_w);
    if warn > 0.0 {
        c.stroke(&v, &arc(0.0, warn), fade(hmi.ok, 0.5), zone_line);
    }
    if crit > warn {
        c.stroke(&v, &arc(warn, crit), fade(hmi.warning, 0.5), zone_line);
    }
    if crit < 1.0 {
        c.stroke(&v, &arc(crit, 1.0), fade(hmi.danger, 0.5), zone_line);
    }
    if pct > 0.0 {
        c.stroke(
            &v,
            &arc(0.0, pct),
            fade(color, 0.95),
            Line::scaled(arc_w).round(),
        );
    }
    // Ticks: 10 majors of 5.
    let (mut major, mut minor) = (String::new(), String::new());
    for i in 0..=50u8 {
        let angle = 225.0 - 270.0 * f32::from(i) / 50.0;
        let is_major = i % 5 == 0;
        let outer = r + arc_w / 2.0 + 2.0;
        let inner = if is_major {
            r - arc_w / 2.0 - 6.0
        } else {
            r - arc_w / 2.0 - 2.0
        };
        let (x1, y1) = polar(cx, cy, inner, angle);
        let (x2, y2) = polar(cx, cy, outer, angle);
        let path = if is_major { &mut major } else { &mut minor };
        let _ = write!(path, "M{x1} {y1} L{x2} {y2} ");
    }
    c.stroke(
        &v,
        &Shape::svg_owned(&minor),
        fade(hmi.fg_muted, 0.4),
        Line::scaled(0.6),
    );
    c.stroke(
        &v,
        &Shape::svg_owned(&major),
        fade(hmi.fg_muted, 0.8),
        Line::scaled(1.5),
    );
    if labels {
        let style = Text::new(7.0, fade(hmi.fg_muted, 0.7)).mono();
        for i in 0..=10u8 {
            let p = f64::from(i) / 10.0;
            let angle = 225.0 - 270.0 * f32::from(i) / 10.0;
            let (x, y) = polar(cx, cy, r + arc_w / 2.0 + 14.0, angle);
            let value = s.min + s.range() * p;
            c.text(&v, (x, base(y + 3.0, 7.0)), &auto_fixed(value), style);
        }
    }
    let hand = needle(cx, cy, 225.0 - 270.0 * pct, r - 14.0, 12.0, 3.0);
    shadow(c, &v, &hand, 0.3);
    c.fill(&v, &hand, color);
    c.fill(&v, &Shape::circle(cx, cy, 7.0), hmi.surface);
    c.stroke(
        &v,
        &Shape::circle(cx, cy, 7.0),
        hmi.border,
        Line::scaled(1.0),
    );
    c.fill(&v, &Shape::circle(cx, cy, 3.0), hmi.fg);
    c.text(
        &v,
        (cx, base(cy + 30.0, 18.0)),
        &s.text(),
        Text::new(18.0, hmi.fg).mono().bold(),
    );
    if !s.unit.is_empty() {
        c.text(
            &v,
            (cx, base(cy + 43.0, 10.0)),
            s.unit,
            Text::new(10.0, hmi.fg_muted),
        );
    }
    if named {
        c.text(
            xf,
            (w / 2.0, top + h * 0.9 + 2.0 + 6.6),
            s.name,
            Text::new(11.0, hmi.fg_muted).max_width(w),
        );
    }
}

fn semicircle(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let s = Scale::of(item.primary.as_ref());
    let thick = widget.number("thickness").unwrap_or(12.0) as f32;
    let v = view_box(xf, (0.0, 0.0, w, h), (200.0, 130.0));
    let (cx, cy, r) = (100.0, 85.0, 65.0);
    let pct = s.pct();
    let (warn, crit) = (s.at(s.warn), s.at(s.crit));
    let color = level_color(hmi, s.zone());
    // Left to right over the top.
    let arc =
        |from: f32, to: f32| Shape::arc(cx, cy, r, -180.0 + 180.0 * from, -180.0 + 180.0 * to);
    c.stroke(
        &v,
        &arc(0.0, 1.0),
        fade(hmi.border, 0.3),
        Line::scaled(thick).round(),
    );
    let zone_line = Line::scaled(thick);
    if warn > 0.0 {
        c.stroke(&v, &arc(0.0, warn), fade(hmi.ok, 0.45), zone_line);
    }
    if crit > warn {
        c.stroke(&v, &arc(warn, crit), fade(hmi.warning, 0.45), zone_line);
    }
    if crit < 1.0 {
        c.stroke(&v, &arc(crit, 1.0), fade(hmi.danger, 0.45), zone_line);
    }
    if pct > 0.0 {
        c.stroke(
            &v,
            &arc(0.0, pct),
            fade(color, 0.95),
            Line::scaled(thick).round(),
        );
    }
    if widget.flag("showTicks") != Some(false) {
        let (mut major, mut minor) = (String::new(), String::new());
        let label = Text::new(6.5, fade(hmi.fg_muted, 0.7)).mono();
        for i in 0..=50u8 {
            let p = f32::from(i) / 50.0;
            let angle = 180.0 * (1.0 - p);
            let is_major = i % 5 == 0;
            let outer = r + thick / 2.0 + 2.0;
            let inner = if is_major {
                r - thick / 2.0 - 5.0
            } else {
                r - thick / 2.0 - 1.0
            };
            let (x1, y1) = polar(cx, cy, outer, angle);
            let (x2, y2) = polar(cx, cy, inner, angle);
            let path = if is_major { &mut major } else { &mut minor };
            let _ = write!(path, "M{x1} {y1} L{x2} {y2} ");
            if i % 10 == 0 {
                let (lx, ly) = polar(cx, cy, r + thick / 2.0 + 12.0, angle);
                let value = s.min + s.range() * f64::from(i) / 50.0;
                c.text(&v, (lx, base(ly + 3.0, 6.5)), &format!("{value:.0}"), label);
            }
        }
        c.stroke(
            &v,
            &Shape::svg_owned(&minor),
            fade(hmi.fg_muted, 0.35),
            Line::scaled(0.5),
        );
        c.stroke(
            &v,
            &Shape::svg_owned(&major),
            fade(hmi.fg_muted, 0.7),
            Line::scaled(1.2),
        );
    }
    let hand = needle(cx, cy, 180.0 * (1.0 - pct), r - 16.0, 8.0, 2.5);
    shadow(c, &v, &hand, 0.25);
    c.fill(&v, &hand, color);
    c.fill(&v, &Shape::circle(cx, cy, 5.0), hmi.surface);
    c.stroke(
        &v,
        &Shape::circle(cx, cy, 5.0),
        hmi.border,
        Line::scaled(0.8),
    );
    c.fill(&v, &Shape::circle(cx, cy, 2.0), hmi.fg);
    c.text(
        &v,
        (cx, base(cy + 18.0, 20.0)),
        &s.text(),
        Text::new(20.0, hmi.fg).mono().bold(),
    );
    if !s.unit.is_empty() {
        c.text(
            &v,
            (cx, base(cy + 30.0, 9.0)),
            s.unit,
            Text::new(9.0, hmi.fg_muted),
        );
    }
}

fn donut(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let s = Scale::of(item.primary.as_ref());
    let fill = token(hmi, widget.text("fillColor"), hmi.accent);
    let thick = widget.number("thickness").unwrap_or(14.0) as f32;
    // Only an explicit `true` shows the name.
    let named = widget.get("showName").and_then(|value| value.as_bool()) == Some(true)
        && !s.name.is_empty();
    let content = h * 0.9 + if named { 12.0 } else { 0.0 };
    let top = (h - content) / 2.0;
    let v = view_box(xf, (0.0, top, w, h * 0.9), (200.0, 200.0));
    let (cx, cy, r) = (100.0, 100.0, 75.0);
    let pct = s.pct();
    let color = match s.zone() {
        Level::Normal => fill,
        level => level_color(hmi, level),
    };
    let ring = Shape::circle(cx, cy, r);
    c.stroke(&v, &ring, fade(hmi.border, 0.1), Line::scaled(thick + 4.0));
    c.stroke(&v, &ring, fade(hmi.border, 0.2), Line::scaled(thick));
    let edge = fade(hmi.border, 0.3);
    c.stroke(
        &v,
        &Shape::circle(cx, cy, r + thick / 2.0),
        edge,
        Line::scaled(0.5),
    );
    c.stroke(
        &v,
        &Shape::circle(cx, cy, r - thick / 2.0),
        edge,
        Line::scaled(0.5),
    );
    if pct >= 1.0 {
        c.stroke(&v, &ring, color, Line::scaled(thick));
    } else if pct > 0.0 {
        let arc = Shape::arc(cx, cy, r, -90.0, -90.0 + 360.0 * pct);
        c.stroke(&v, &arc, color, Line::scaled(thick).round());
    }
    c.stroke(
        &v,
        &Shape::circle(cx, cy, r - thick / 2.0 - 2.0),
        fade(hmi.fg, 0.08),
        Line::scaled(0.3),
    );
    c.text(
        &v,
        (cx, base(cy - 6.0, 28.0)),
        &s.text(),
        Text::new(28.0, hmi.fg).mono().bold(),
    );
    c.text(
        &v,
        (cx, base(cy + 12.0, 11.0)),
        s.unit,
        Text::new(11.0, hmi.fg_muted),
    );
    if widget.flag("showPercentage") != Some(false) {
        let text = if s.value.is_some() {
            format!("{:.0}%", pct * 100.0)
        } else {
            "--%".to_owned()
        };
        c.text(
            &v,
            (cx, base(cy + 27.0, 10.0)),
            &text,
            Text::new(10.0, fade(hmi.fg_muted, 0.7)).mono(),
        );
    }
    if named {
        c.text(
            xf,
            (w / 2.0, top + h * 0.9 + 6.0),
            s.name,
            Text::new(10.0, hmi.fg_muted).max_width(w),
        );
    }
}

fn meter(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let s = Scale::of(item.primary.as_ref());
    let fill = token(hmi, widget.text("fillColor"), hmi.ok);
    let color = match s.zone() {
        Level::Normal => fill,
        level => level_color(hmi, level),
    };
    let v = view_box(xf, (0.0, 0.0, w, h), (200.0, 100.0));
    let (bx, by, bw, bh) = (10.0, 30.0, 180.0, 24.0);
    let pct = s.pct();
    let track = Paint::gradient(180.0, (hmi.border, 0.0), (fade(hmi.border, 0.6), 1.0));
    c.fill(&v, &Shape::round_rect(bx, by, bw, bh, bh / 2.0), track);
    if pct > 0.0 {
        let paint = Paint::gradient(90.0, (fade(color, 0.8), 0.0), (color, 1.0));
        c.fill(
            &v,
            &Shape::round_rect(bx, by, (bw * pct).max(bh), bh, bh / 2.0),
            paint,
        );
    }
    if widget.flag("showTicks") != Some(false) {
        let label = Text::new(7.0, hmi.fg_muted).mono();
        for i in 0..=10u8 {
            let x = bx + bw * f32::from(i) / 10.0;
            let major = i % 5 == 0;
            let (y2, width) = if major { (by, 1.5) } else { (by - 1.0, 0.5) };
            segment(
                c,
                &v,
                (x, by - 3.0),
                (x, y2),
                hmi.fg_muted,
                Line::scaled(width),
            );
            if major {
                let value = s.min + s.range() * f64::from(i) / 10.0;
                c.text(&v, (x, base(by - 7.0, 7.0)), &format!("{value:.0}"), label);
            }
        }
    }
    let knob = bx + bw * pct;
    c.fill(
        &v,
        &Shape::circle(knob, by + bh / 2.0, bh / 2.0 - 2.0),
        hmi.fg,
    );
    c.fill(
        &v,
        &Shape::circle(knob, by + bh / 2.0, bh / 2.0 - 5.0),
        color,
    );
    c.text(
        &v,
        (bx + bw / 2.0, base(by + bh + 16.0, 14.0)),
        &s.text(),
        Text::new(14.0, hmi.fg).mono().bold(),
    );
    if !s.unit.is_empty() {
        c.text(
            &v,
            (bx + bw / 2.0, base(by + bh + 28.0, 9.0)),
            s.unit,
            Text::new(9.0, hmi.fg_muted),
        );
    }
}

fn progress_bar(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let s = Scale::of(item.primary.as_ref());
    let fill = token(hmi, widget.text("fillColor"), hmi.ok);
    let color = match s.zone() {
        Level::Normal => fill,
        level => level_color(hmi, level),
    };
    let direction = widget.text("direction").unwrap_or("up");
    let vertical = matches!(direction, "up" | "down");
    let inverted = matches!(direction, "down" | "left");
    let ticks = widget.flag("showTicks") != Some(false);
    let pct = s.pct();
    let tick_style = Text::new(7.0, hmi.fg_muted).mono();
    let marks = [0.0f32, 0.25, 0.5, 0.75, 1.0];
    let mark_text = |mark: f32| format!("{:.0}", s.min + s.range() * f64::from(mark));
    if vertical {
        let v = view_box(xf, (0.0, 0.0, w, h), (100.0, 170.0));
        let (bx, by, bw, bh) = (35.0, 10.0, 40.0, 140.0);
        let outline = Shape::round_rect(bx, by, bw, bh, 3.0);
        c.fill(&v, &outline, fade(hmi.border, 0.3));
        if pct > 0.0 {
            let fh = bh * pct;
            let fy = if inverted { by } else { by + bh - fh };
            let shape = Shape::rect(bx, fy, bw, fh).within(&outline);
            c.fill(&v, &shape, fade(color, 0.85));
        }
        c.stroke(&v, &outline, hmi.border, Line::scaled(1.5));
        if ticks {
            for mark in marks {
                let y = if inverted {
                    by + bh * mark
                } else {
                    by + bh * (1.0 - mark)
                };
                segment(
                    c,
                    &v,
                    (bx + bw, y),
                    (bx + bw + 4.0, y),
                    hmi.fg_muted,
                    Line::scaled(0.7),
                );
                c.text(
                    &v,
                    (bx + bw + 6.0, base(y + 3.0, 7.0)),
                    &mark_text(mark),
                    tick_style.align(Align::Start),
                );
            }
        }
        c.text(
            &v,
            (bx + bw / 2.0, base(by + bh + 16.0, 12.0)),
            &s.text(),
            Text::new(12.0, hmi.fg).mono().bold(),
        );
        if !s.unit.is_empty() {
            c.text(
                &v,
                (bx + bw / 2.0, base(by + bh + 28.0, 9.0)),
                s.unit,
                Text::new(9.0, hmi.fg_muted),
            );
        }
        return;
    }
    let v = view_box(xf, (0.0, 0.0, w, h), (200.0, 80.0));
    let (bx, by, bw, bh) = (10.0, 20.0, 180.0, 30.0);
    let outline = Shape::round_rect(bx, by, bw, bh, 3.0);
    c.fill(&v, &outline, fade(hmi.border, 0.3));
    if pct > 0.0 {
        let fw = bw * pct;
        let fx = if inverted { bx + bw - fw } else { bx };
        let shape = Shape::rect(fx, by, fw, bh).within(&outline);
        c.fill(&v, &shape, fade(color, 0.85));
    }
    c.stroke(&v, &outline, hmi.border, Line::scaled(1.5));
    if ticks {
        for mark in marks {
            let x = if inverted {
                bx + bw * (1.0 - mark)
            } else {
                bx + bw * mark
            };
            segment(
                c,
                &v,
                (x, by + bh),
                (x, by + bh + 4.0),
                hmi.fg_muted,
                Line::scaled(0.7),
            );
            c.text(
                &v,
                (x, base(by + bh + 12.0, 7.0)),
                &mark_text(mark),
                tick_style,
            );
        }
    }
    let text = match s.value {
        Some(value) => format!("{value:.*} {}", s.decimals, s.unit),
        None => "--".to_owned(),
    };
    c.text(
        &v,
        (bx + bw / 2.0, base(by + bh / 2.0 + 4.0, 12.0)),
        &text,
        Text::new(12.0, hmi.fg).mono().bold(),
    );
}

// ── Values ────────────────────────────────────────────────────────────────

fn indicator(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let tag = item.primary.as_ref();
    let s = Scale::of(tag);
    let font = widget.number("fontSize").unwrap_or(28.0) as f32;
    let unit_size = (font * 0.4).max(10.0);
    let show_unit = widget.flag("showUnit") != Some(false) && !s.unit.is_empty();
    let show_status = widget.flag("showStatus") != Some(false) && s.value.is_some();
    let value_h = font * 1.2;
    let unit_h = if show_unit {
        2.0 + unit_size * 1.2
    } else {
        0.0
    };
    let badge_h = if show_status { 4.0 + 16.0 } else { 0.0 };
    let mut y = (h - value_h - unit_h - badge_h) / 2.0;
    c.text(
        xf,
        (w / 2.0, y + value_h / 2.0),
        &s.text(),
        Text::new(font, hmi.fg)
            .mono()
            .bold()
            .max_width((w - 16.0).max(0.0)),
    );
    y += value_h;
    if show_unit {
        c.text(
            xf,
            (w / 2.0, y + 2.0 + unit_size * 0.6),
            s.unit,
            Text::new(unit_size, hmi.fg_muted),
        );
        y += unit_h;
    }
    if show_status {
        let level = tag.map_or(Level::Normal, |tag| tag.level);
        let text = match level {
            Level::Critical => "CRIT",
            Level::Warning => "WARN",
            Level::Normal => "OK",
        };
        let style = Text::new(10.0, hmi.bg).bold();
        let bw = c.text_width(xf, text, style) + 16.0;
        let top = y + 4.0;
        c.rect(
            xf,
            ((w - bw) / 2.0, top, bw, 16.0),
            8.0,
            level_color(hmi, level),
            None,
        );
        c.text(xf, (w / 2.0, top + 8.0), text, style);
    }
}

fn bar(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let s = Scale::of(item.primary.as_ref());
    let (tw, th) = ((w - 20.0).max(0.0), (h - 12.0).max(20.0));
    let (tx, ty) = (10.0, (h - th) / 2.0);
    c.fill(xf, &Shape::round_rect(tx, ty, tw, th, 6.0), hmi.border);
    let pct = s.pct();
    if pct > 0.0 {
        let fw = (tw * pct).max(2.0);
        c.fill(
            xf,
            &Shape::round_rect(tx, ty, fw, th, 6.0),
            level_color(hmi, s.zone()),
        );
    }
    if item.widget.flag("showValue") != Some(false) {
        let text = match s.value {
            Some(value) => format!("{value:.*} {}", s.decimals, s.unit)
                .trim()
                .to_owned(),
            None => "--".to_owned(),
        };
        c.text(
            xf,
            (w / 2.0, ty + th / 2.0),
            &text,
            Text::new(13.0, hmi.fg).mono().semibold().halo(hmi.bg),
        );
    }
}

fn light(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let tag = item.primary.as_ref();
    let reading = tag.map(|tag| Reading {
        value: tag.value,
        level: tag.level,
        min: tag.min,
        max: tag.max,
    });
    let digital = tag.is_some_and(|tag| tag.digital);
    let threshold = item.widget.number("onThreshold").unwrap_or(0.5);
    let state = status_light(reading, digital, threshold);
    let color = match state {
        Light::Dark | Light::Off => hmi.fg_muted,
        Light::On => hmi.ok,
        Light::Warning => hmi.warning,
        Light::Critical => hmi.danger,
    };
    let d = w.min(h) * 0.6;
    let (cx, cy, r) = (w / 2.0, h / 2.0, d / 2.0);
    if state != Light::Dark {
        // `box-shadow: 0 0 .4d .15d`: a solid spread fading out over the blur.
        let spread = r + d * 0.15;
        let blur = d * 0.4;
        for (reach, alpha) in [(0.45, 0.12), (0.25, 0.18), (0.05, 0.3)] {
            c.fill(
                xf,
                &Shape::circle(cx, cy, spread + blur * reach),
                fade(color, alpha),
            );
        }
    }
    c.fill(xf, &Shape::circle(cx, cy, r), color);
}

fn on_off(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let value = item.primary.as_ref().and_then(|tag| tag.value);
    let on = is_on(value, widget.number("onThreshold").unwrap_or(0.5));
    let top = (h - (26.0 + 6.0 + 14.4)) / 2.0;
    let left = (w - 48.0) / 2.0;
    c.rect(
        xf,
        (left, top, 48.0, 26.0),
        13.0,
        if on { hmi.ok } else { hmi.border },
        None,
    );
    let knob = if on { 25.0 } else { 3.0 };
    c.fill(
        xf,
        &Shape::circle(left + knob + 10.0, top + 13.0, 10.0),
        hmi.fg,
    );
    let text = match value {
        Some(_) if on => widget.text("onLabel").unwrap_or("ON").to_uppercase(),
        Some(_) => widget.text("offLabel").unwrap_or("OFF").to_uppercase(),
        None => "--".to_owned(),
    };
    c.text(
        xf,
        (w / 2.0, top + 32.0 + 7.2),
        &text,
        Text::new(12.0, if on { hmi.ok } else { hmi.fg_muted }).semibold(),
    );
}

fn label(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let text = widget
        .text("text")
        .filter(|text| !text.is_empty())
        .or_else(|| item.primary.as_ref().map(|tag| tag.name.as_str()))
        .filter(|text| !text.is_empty())
        .unwrap_or("Label");
    let font = widget.number("fontSize").unwrap_or(14.0) as f32;
    let color = if widget.flag("muted") == Some(true) {
        hmi.fg_muted
    } else {
        hmi.fg
    };
    let mut style = Text::new(font, color).max_width((w - 16.0).max(0.0));
    if widget.flag("bold") == Some(true) {
        style = style.bold();
    }
    let (x, align) = match widget.text("align") {
        Some("left") => (8.0, Align::Start),
        Some("right") => (w - 8.0, Align::End),
        _ => (w / 2.0, Align::Middle),
    };
    c.text(xf, (x, h / 2.0), text, style.align(align));
}

fn big_number(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let tag = item.primary.as_ref();
    let s = Scale::of(tag);
    let font = widget.number("fontSize").unwrap_or(48.0) as f32;
    let decimals = widget
        .number("decimals")
        .map_or(s.decimals, |d| d.clamp(0.0, 20.0) as usize);
    let named = widget.flag("showName") != Some(false) && !s.name.is_empty();
    let unit = widget.flag("showUnit") != Some(false) && !s.unit.is_empty();
    let level = tag.map_or(Level::Normal, |tag| tag.level);
    let status = level != Level::Normal;
    let name_h = if named { 13.2 + 2.0 } else { 0.0 };
    let status_h = if status { 4.0 + 16.0 } else { 0.0 };
    let mut y = (h - name_h - font - status_h) / 2.0;
    if named {
        c.text(
            xf,
            (w / 2.0, y + 6.6),
            &s.name.to_uppercase(),
            Text::new(11.0, hmi.fg_muted).max_width((w - 16.0).max(0.0)),
        );
        y += name_h;
    }
    let text = s
        .value
        .map_or_else(|| "--".to_owned(), |v| format!("{v:.decimals$}"));
    let value_style = Text::new(font, hmi.fg).mono().bold().align(Align::Start);
    let unit_size = font * 0.35;
    let unit_style = Text::new(unit_size, hmi.fg_muted)
        .mono()
        .align(Align::Start);
    let vw = c.text_width(xf, &text, value_style);
    let uw = if unit {
        4.0 + c.text_width(xf, s.unit, unit_style)
    } else {
        0.0
    };
    let x = (w - vw - uw) / 2.0;
    let mid = y + font / 2.0;
    c.text(xf, (x, mid), &text, value_style);
    if unit {
        // Baseline-aligned with the value.
        let baseline = mid + font * 0.32;
        c.text(
            xf,
            (x + vw + 4.0, baseline - unit_size * 0.32),
            s.unit,
            unit_style,
        );
    }
    y += font;
    if status {
        let color = level_color(hmi, level);
        let word = if level == Level::Critical {
            "CRIT"
        } else {
            "WARN"
        };
        let style = Text::new(10.0, color).semibold();
        let bw = c.text_width(xf, word, style) + 16.0;
        c.rect(
            xf,
            ((w - bw) / 2.0, y + 4.0, bw, 16.0),
            4.0,
            fade(color, 0.12),
            None,
        );
        c.text(xf, (w / 2.0, y + 12.0), word, style);
    }
}

fn led(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let s = Scale::of(item.primary.as_ref());
    let color = token(hmi, widget.text("color"), hmi.accent);
    c.rect(xf, (0.0, 0.0, w, h), 4.0, fade(hmi.black, 0.15), None);
    let named = widget.flag("showName") != Some(false) && !s.name.is_empty();
    let content = h * 0.7 + if named { 2.0 + 10.8 } else { 0.0 };
    let top = (h - content) / 2.0;
    let text = s.text();
    let chars: Vec<char> = text.chars().collect();
    let digits_w = chars.len() as f32 * 28.0;
    let total = digits_w + s.unit.chars().count() as f32 * 8.0;
    let view_w = (total + 16.0).max(80.0);
    let v = view_box(xf, (w * 0.05, top, w * 0.9, h * 0.7), (view_w, 52.0))
        .then(&Xf::translate(8.0, 6.0));
    let (dw, dh) = (22.0, 38.0);
    let line = Line::scaled(3.0).round();
    for (index, ch) in chars.iter().enumerate() {
        let ox = index as f32 * 28.0;
        let lit = segments(*ch);
        let lines = [
            ((ox + 3.0, 1.0), (ox + dw - 3.0, 1.0)),
            ((ox + dw + 1.0, 3.0), (ox + dw + 1.0, dh / 2.0 - 1.0)),
            ((ox + dw + 1.0, dh / 2.0 + 1.0), (ox + dw + 1.0, dh - 3.0)),
            ((ox + 3.0, dh / 2.0), (ox + dw - 3.0, dh / 2.0)),
            ((ox - 1.0, dh / 2.0 + 1.0), (ox - 1.0, dh - 3.0)),
            ((ox - 1.0, 3.0), (ox - 1.0, dh / 2.0 - 1.0)),
            ((ox + 3.0, dh - 1.0), (ox + dw - 3.0, dh - 1.0)),
        ];
        let paint_of = |on: bool| {
            if on {
                fade(color, 0.9)
            } else {
                fade(hmi.border, 0.15)
            }
        };
        // The web draws segment a twice.
        segment(c, &v, lines[0].0, lines[0].1, paint_of(lit[0]), line);
        for (seg, on) in lines.iter().zip(lit) {
            segment(c, &v, seg.0, seg.1, paint_of(on), line);
        }
    }
    if !s.unit.is_empty() {
        c.text(
            &v,
            (digits_w + 4.0, base(30.0, 10.0)),
            s.unit,
            Text::new(10.0, hmi.fg_muted).align(Align::Start),
        );
    }
    if named {
        c.text(
            xf,
            (w / 2.0, top + h * 0.7 + 2.0 + 5.4),
            s.name,
            Text::new(9.0, hmi.fg_muted).max_width(w),
        );
    }
}

/// The seven segments (a, b, c, d, e, f, g as the web orders them) lit for
/// a character; anything else is blank.
fn segments(ch: char) -> [bool; 7] {
    match ch {
        '0' => [true, true, true, false, true, true, true],
        '1' => [false, false, true, false, false, true, false],
        '2' => [true, false, true, true, true, false, true],
        '3' => [true, false, true, true, false, true, true],
        '4' => [false, true, true, true, false, true, false],
        '5' => [true, true, false, true, false, true, true],
        '6' => [true, true, false, true, true, true, true],
        '7' => [true, false, true, false, false, true, false],
        '8' => [true; 7],
        '9' => [true, true, true, true, false, true, true],
        '-' => [false, false, false, true, false, false, false],
        _ => [false; 7],
    }
}

// ── Controls ──────────────────────────────────────────────────────────────

fn slider(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let s = Scale::of(item.primary.as_ref());
    let color = token(hmi, widget.text("sliderColor"), hmi.accent);
    let total = 15.6 + 2.0 + 22.0 + 2.0 + 10.8;
    let top = (h - total) / 2.0;
    let (left, width) = (10.0, (w - 20.0).max(0.0));
    let head = top + 7.8;
    c.text(
        xf,
        (left, head),
        s.name,
        Text::new(10.0, hmi.fg_muted)
            .align(Align::Start)
            .max_width(width * 0.6),
    );
    if widget.flag("showValue") != Some(false) {
        let mut text = format!("{:.*}", s.decimals, s.shown());
        if !s.unit.is_empty() {
            text = format!("{text} {}", s.unit);
        }
        c.text(
            xf,
            (left + width, head),
            &text,
            Text::new(13.0, hmi.fg).mono().bold().align(Align::End),
        );
    }
    let mid = top + 15.6 + 2.0 + 11.0;
    let pct = s.pct();
    c.rect(xf, (left, mid - 3.0, width, 6.0), 3.0, hmi.border, None);
    if pct > 0.0 {
        c.rect(xf, (left, mid - 3.0, width * pct, 6.0), 3.0, color, None);
    }
    let knob = Shape::circle(left + width * pct, mid, 8.0);
    shadow(c, xf, &knob, 0.25);
    c.fill(xf, &knob, color);
    let foot = top + 15.6 + 2.0 + 22.0 + 2.0 + 5.4;
    let small = Text::new(9.0, hmi.fg_muted).mono();
    c.text(xf, (left, foot), &js_num(s.min), small.align(Align::Start));
    c.text(
        xf,
        (left + width, foot),
        &js_num(s.max),
        small.align(Align::End),
    );
}

fn number_input(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let s = Scale::of(item.primary.as_ref());
    let buttons = widget.flag("showButtons") != Some(false);
    let name_h = if s.name.is_empty() { 0.0 } else { 12.0 };
    let value_h = 22.0 * 1.2 + 1.0;
    let buttons_h = if buttons { 4.0 + 24.0 } else { 0.0 };
    let mut y = (h - (name_h + 4.0 + value_h + buttons_h)) / 2.0;
    if name_h > 0.0 {
        c.text(
            xf,
            (w / 2.0, y + 6.0),
            s.name,
            Text::new(10.0, hmi.fg_muted).max_width((w - 12.0).max(0.0)),
        );
        y += name_h + 4.0;
    }
    let value_style = Text::new(
        22.0,
        if s.value.is_some() {
            hmi.fg
        } else {
            hmi.fg_muted
        },
    )
    .mono()
    .bold()
    .align(Align::Start);
    let unit_style = Text::new(12.0, hmi.fg_muted).align(Align::Start);
    let text = s.text();
    let vw = c.text_width(xf, &text, value_style);
    let uw = if s.unit.is_empty() {
        0.0
    } else {
        3.0 + c.text_width(xf, s.unit, unit_style)
    };
    let inner = (vw + uw).max(60.0);
    let x = (w - inner) / 2.0 + (inner - vw - uw) / 2.0;
    let mid = y + 13.2;
    c.text(xf, (x, mid), &text, value_style);
    if uw > 0.0 {
        let baseline = mid + 22.0 * 0.32;
        c.text(
            xf,
            (x + vw + 3.0, baseline - 12.0 * 0.32),
            s.unit,
            unit_style,
        );
    }
    let under = y + 26.4;
    let line_w = inner + 8.0;
    segment(
        c,
        xf,
        ((w - line_w) / 2.0, under + 0.5),
        ((w + line_w) / 2.0, under + 0.5),
        hmi.border,
        Line::scaled(1.0).dashed(3.0, 3.0, 0.0),
    );
    if buttons {
        let by = y + value_h + 4.0;
        for (bx, sign) in [(w / 2.0 - 30.0, "−"), (w / 2.0 + 2.0, "+")] {
            c.rect(
                xf,
                (bx, by, 28.0, 24.0),
                4.0,
                hmi.surface,
                Some((hmi.border, 1.0)),
            );
            c.text(
                xf,
                (bx + 14.0, by + 12.0),
                sign,
                Text::new(14.0, hmi.fg).bold(),
            );
        }
    }
}

fn toggle(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let tag = item.primary.as_ref();
    let name = tag.map_or("", |tag| tag.name.as_str());
    let value = tag.and_then(|tag| tag.value);
    let on = is_on(value, widget.number("onThreshold").unwrap_or(0.5));
    let name_h = if name.is_empty() { 0.0 } else { 12.0 + 6.0 };
    let mut y = (h - (name_h + 30.0 + 6.0 + 14.4)) / 2.0;
    if !name.is_empty() {
        c.text(
            xf,
            (w / 2.0, y + 6.0),
            name,
            Text::new(10.0, hmi.fg_muted).max_width(w),
        );
        y += name_h;
    }
    let left = (w - 56.0) / 2.0;
    c.rect(
        xf,
        (left, y, 56.0, 30.0),
        15.0,
        if on { hmi.ok } else { hmi.border },
        None,
    );
    let knob_x = left + if on { 29.0 } else { 3.0 };
    let knob = Shape::circle(knob_x + 12.0, y + 15.0, 12.0);
    shadow(c, xf, &knob, 0.3);
    c.fill(xf, &knob, hmi.white);
    let text = match value {
        Some(_) if on => widget.text("onLabel").unwrap_or("ON").to_uppercase(),
        Some(_) => widget.text("offLabel").unwrap_or("OFF").to_uppercase(),
        None => "--".to_owned(),
    };
    c.text(
        xf,
        (w / 2.0, y + 36.0 + 7.2),
        &text,
        Text::new(12.0, if on { hmi.ok } else { hmi.fg_muted }).bold(),
    );
}

fn push_button(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let tag = item.primary.as_ref();
    let name = tag.map_or("", |tag| tag.name.as_str());
    let on_value = widget.number("onValue").unwrap_or(1.0);
    #[allow(clippy::float_cmp)]
    let active = tag.and_then(|tag| tag.value).is_some_and(|v| v == on_value);
    let accent = token(hmi, widget.text("buttonColor"), hmi.ok);
    let color = if active { accent } else { hmi.fg };
    let named = widget.flag("showLabel") != Some(false) && !name.is_empty();
    let d = w * 0.6;
    let top = (h - d - if named { 6.0 + 10.8 } else { 0.0 }) / 2.0;
    let (cx, cy, r) = (w / 2.0, top + d / 2.0, d / 2.0);
    let body = Shape::circle(cx, cy, r);
    if active {
        c.fill(xf, &Shape::circle(cx, cy, r + 6.0), fade(color, 0.08));
        c.fill(xf, &Shape::circle(cx, cy, r + 3.0), fade(color, 0.12));
    }
    let lower = xf.then(&Xf::translate(0.0, 2.0));
    c.fill(&lower, &body, fade(hmi.black, 0.1));
    c.fill(xf, &body, hmi.surface);
    if active {
        c.fill(xf, &body, fade(color, 0.13));
    }
    c.stroke(
        xf,
        &Shape::circle(cx, cy, (r - 1.5).max(0.0)),
        if active { color } else { hmi.border },
        Line::scaled(3.0),
    );
    let text = if active {
        widget.text("onLabel").unwrap_or("START")
    } else {
        widget.text("offLabel").unwrap_or("STOP")
    };
    c.text(
        xf,
        (cx, cy),
        text,
        Text::new(11.0, color).bold().max_width((d - 6.0).max(0.0)),
    );
    if named {
        c.text(
            xf,
            (w / 2.0, top + d + 6.0 + 5.4),
            name,
            Text::new(9.0, hmi.fg_muted).max_width(w),
        );
    }
}

// ── Tables ────────────────────────────────────────────────────────────────

fn data_table(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    c.rect(xf, (0.0, 0.0, w, h), 6.0, hmi.surface, None);
    if item.tags.is_empty() {
        c.text(
            xf,
            (w / 2.0, h / 2.0),
            "Drop tags to add rows",
            Text::new(12.0, hmi.fg_muted),
        );
        return;
    }
    let status = widget.flag("showStatus") != Some(false);
    let compact = widget.flag("compact") == Some(true);
    let font = if compact { 11.0 } else { 12.0 };
    let (px, py) = if compact { (6.0, 3.0) } else { (8.0, 5.0) };
    let status_w = if status { 16.0 + 2.0 * px } else { 0.0 };
    let value_style = Text::new(font, hmi.fg).mono().align(Align::End);
    let unit_style = Text::new(font - 1.0, hmi.fg_muted).mono().align(Align::End);
    let mut value_w: f32 = c.text_width(xf, "VALUE", unit_style);
    let cells: Vec<(String, f32)> = item
        .tags
        .iter()
        .map(|tag| {
            let text = tag.fixed(tag.decimals);
            let unit_w = if tag.unit.is_empty() {
                0.0
            } else {
                3.0 + c.text_width(xf, tag.unit.as_str(), unit_style)
            };
            value_w = value_w.max(c.text_width(xf, &text, value_style) + unit_w);
            (text, unit_w)
        })
        .collect();
    let name_x = status_w + px;
    let name_max = (w - status_w - 4.0 * px - value_w).clamp(0.0, 120.0);
    let head_h = (font - 1.0) * 1.25 + 2.0 * py;
    let head = Text::new(font - 1.0, hmi.fg_muted).mono().semibold();
    c.text(xf, (name_x, head_h / 2.0), "TAG", head.align(Align::Start));
    c.text(xf, (w - px, head_h / 2.0), "VALUE", head.align(Align::End));
    c.rect(xf, (0.0, head_h, w, 1.0), 0.0, hmi.border, None);
    let row_h = font * 1.25 + 2.0 * py;
    let mut y = head_h + 1.0;
    for (tag, (text, unit_w)) in item.tags.iter().zip(&cells) {
        if y > h {
            break;
        }
        let mid = y + row_h / 2.0;
        if status {
            c.fill(
                xf,
                &Shape::circle(px + 3.0, mid, 3.0),
                level_color(hmi, tag.level),
            );
        }
        c.text(
            xf,
            (name_x, mid),
            tag.name.as_str(),
            Text::new(font, hmi.fg)
                .mono()
                .align(Align::Start)
                .max_width(name_max),
        );
        let color = match tag.level {
            Level::Critical => hmi.danger,
            Level::Warning => hmi.warning,
            Level::Normal => hmi.fg,
        };
        if *unit_w > 0.0 {
            c.text(xf, (w - px, mid), tag.unit.as_str(), unit_style);
        }
        c.text(
            xf,
            (w - px - unit_w, mid),
            text,
            Text {
                color,
                ..value_style
            },
        );
        y += row_h;
        c.rect(xf, (0.0, y, w, 1.0), 0.0, hmi.border, None);
        y += 1.0;
    }
}

/// Columns of a `repeat(auto-fill, minmax(min, 1fr))` grid `width` wide
/// with `gap`, and their width.
fn grid_columns(width: f32, min: f32, gap: f32) -> (usize, f32) {
    let fit = ((width + gap) / (min + gap)).floor();
    if fit < 2.0 {
        return (1, width.max(min));
    }
    let columns = fit as usize;
    (columns, (width - gap * (fit - 1.0)) / fit)
}

fn annunciator(c: &mut Canvas<'_>, item: &Item, xf: &Xf, t: f32) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    if item.tags.is_empty() {
        c.text(
            xf,
            (w / 2.0, h / 2.0),
            "Drop tags here",
            Text::new(11.0, hmi.fg_muted),
        );
        return;
    }
    let gap = 4.0;
    let (columns, col_w) = grid_columns(w, 150.0, gap);
    let (name_h, row_h) = (13.2, 14.0);
    let cell_h = (2.0 + 10.0 + name_h + 2.0 + row_h).max(40.0);
    for (index, tag) in item.tags.iter().enumerate() {
        let (col, row) = (index % columns, index / columns);
        let x = col as f32 * (col_w + gap);
        let y = row as f32 * (cell_h + gap);
        if y > h {
            break;
        }
        let draw = |c: &mut Canvas<'_>| {
            let color = match tag.level {
                Level::Critical => hmi.fault,
                Level::Warning => hmi.warn,
                Level::Normal => hmi.border,
            };
            let alarm = tag.level != Level::Normal;
            let fill = if alarm { fade(color, 0.2) } else { hmi.badge };
            c.rect(xf, (x, y, col_w, cell_h), 3.0, fill, Some((color, 1.0)));
            let inner = col_w - 18.0;
            let top = y + (cell_h - name_h - 2.0 - row_h) / 2.0;
            c.text(
                xf,
                (x + 9.0, top + name_h / 2.0),
                tag.name.as_str(),
                Text::new(11.0, if alarm { hmi.fg } else { hmi.fg_dim })
                    .semibold()
                    .align(Align::Start)
                    .max_width(inner.max(0.0)),
            );
            let mid = top + name_h + 2.0 + row_h / 2.0;
            let text = tag.value.map_or_else(
                || "----".to_owned(),
                |v| format!("{v:.*} {}", tag.decimals, tag.unit),
            );
            c.text(
                xf,
                (x + 9.0, mid),
                &text,
                Text::new(11.0, if alarm { hmi.fg } else { hmi.fg_muted })
                    .mono()
                    .align(Align::Start)
                    .max_width(inner.max(0.0)),
            );
            if alarm {
                let word = if tag.level == Level::Critical {
                    "ALARM"
                } else {
                    "WARN"
                };
                c.text(
                    xf,
                    (x + col_w - 9.0, mid),
                    word,
                    Text::new(9.0, color).bold().align(Align::End),
                );
            }
        };
        if tag.level == Level::Critical {
            c.faded(motion::blink(t), draw);
        } else {
            draw(c);
        }
    }
}

fn pid(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let s = Scale::of(item.primary.as_ref());
    let bars = widget.flag("showBars") != Some(false);
    let output_shown = widget.flag("showOutput") != Some(false);
    let value = s.shown();
    let pct = s.pct();
    let setpoint = widget.number("setpoint").unwrap_or((s.min + s.max) / 2.0);
    let sp_pct = s.at(setpoint);
    let out_pct = s.at(widget.number("output").unwrap_or(value));
    let value_color = level_color(hmi, s.zone());
    let mode = widget.text("mode").unwrap_or("AUTO");
    let (x0, width) = (6.0, (w - 12.0).max(0.0));
    // Heights: the title, the rows and bars, the gains, the mode pills.
    let header_h = 9.0 * 1.2 + 3.0 + 1.0;
    let mut body: Vec<f32> = vec![16.8];
    if bars {
        body.push(8.0);
    }
    body.push(12.0);
    if bars {
        body.push(8.0);
    }
    if output_shown {
        body.push(12.0);
        if bars {
            body.push(8.0);
        }
    }
    body.push(2.0 + 8.4);
    let body_h: f32 = body.iter().sum::<f32>() + 3.0 * (body.len() as f32 - 1.0);
    let pills_h = 2.0 + 10.4;
    let total = header_h + 4.0 + body_h + 4.0 + pills_h;
    let mut y = (h - total) / 2.0;
    let muted = |size: f32| Text::new(size, hmi.fg_muted).mono();
    c.text(xf, (w / 2.0, y + 5.4), "PID", muted(9.0).bold());
    c.rect(
        xf,
        (x0, y + header_h - 1.0, width, 1.0),
        0.0,
        hmi.border,
        None,
    );
    y += header_h + 4.0;
    let pv = s.text();
    row3(
        c,
        xf,
        (x0, width, y + 8.4),
        "PV",
        &pv,
        Text::new(14.0, value_color).mono().bold(),
        s.unit,
    );
    y += 16.8 + 3.0;
    if bars {
        c.rect(xf, (x0, y, width, 8.0), 4.0, hmi.border, None);
        if pct > 0.0 {
            c.rect(xf, (x0, y, width * pct, 8.0), 4.0, value_color, None);
        }
        c.rect(
            xf,
            (x0 + width * sp_pct, y - 2.0, 2.0, 12.0),
            0.0,
            fade(hmi.fg, 0.7),
            None,
        );
        y += 8.0 + 3.0;
    }
    let sp = format!("{setpoint:.*}", s.decimals);
    row3(
        c,
        xf,
        (x0, width, y + 6.0),
        "SP",
        &sp,
        Text::new(10.0, hmi.fg).mono().semibold(),
        s.unit,
    );
    y += 12.0 + 3.0;
    if bars {
        c.rect(xf, (x0, y, width, 8.0), 4.0, hmi.border, None);
        if sp_pct > 0.0 {
            c.rect(
                xf,
                (x0, y, width * sp_pct, 8.0),
                4.0,
                fade(hmi.fg, 0.4),
                None,
            );
        }
        y += 8.0 + 3.0;
    }
    if output_shown {
        c.text(xf, (x0, y + 6.0), "OUT", muted(8.0).align(Align::Start));
        c.text(
            xf,
            (x0 + width, y + 6.0),
            &format!("{:.1}%", out_pct * 100.0),
            Text::new(10.0, hmi.accent)
                .mono()
                .semibold()
                .align(Align::End),
        );
        y += 12.0 + 3.0;
        if bars {
            c.rect(xf, (x0, y, width, 8.0), 4.0, hmi.border, None);
            if out_pct > 0.0 {
                c.rect(
                    xf,
                    (x0, y, width * out_pct, 8.0),
                    4.0,
                    fade(hmi.accent, 0.6),
                    None,
                );
            }
            y += 8.0 + 3.0;
        }
    }
    let gain = |key: &str, default: &str| {
        widget.get(key).map_or_else(
            || default.to_owned(),
            |value| match value.as_f64() {
                Some(number) => js_num(number),
                None => value.as_str().unwrap_or(default).to_owned(),
            },
        )
    };
    let gains = format!(
        "P: {} I: {} D: {}",
        gain("pGain", "1.0"),
        gain("iGain", "0.1"),
        gain("dGain", "0.0")
    );
    let foot = y + 2.0 + 4.2;
    c.text(
        xf,
        (x0, foot),
        &format!("Mode: {mode}"),
        muted(7.0).align(Align::Start),
    );
    c.text(xf, (x0 + width, foot), &gains, muted(7.0).align(Align::End));
    y += 2.0 + 8.4 + 4.0 + 2.0;
    let pills = [
        ("AUTO", mode == "AUTO", hmi.ok),
        ("MAN", mode == "MAN", hmi.warning),
        ("CAS", mode == "CAS", hmi.accent),
    ];
    let style = Text::new(7.0, hmi.fg_muted).mono().semibold();
    let widths: Vec<f32> = pills
        .iter()
        .map(|(text, _, _)| c.text_width(xf, text, style) + 12.0)
        .collect();
    let mut x = (w - widths.iter().sum::<f32>() - 12.0) / 2.0;
    for ((text, on, color), pill_w) in pills.iter().zip(widths) {
        let (fill, ink) = if *on {
            (*color, hmi.bg)
        } else {
            (hmi.border, hmi.fg_muted)
        };
        c.rect(xf, (x, y, pill_w, 10.4), 3.0, fill, None);
        c.text(
            xf,
            (x + pill_w / 2.0, y + 5.2),
            text,
            Text {
                color: ink,
                ..style
            },
        );
        x += pill_w + 6.0;
    }
}

/// A faceplate row: a label left, a value in the middle of the free space,
/// a unit right (`justify-content: space-between`).
fn row3(
    c: &mut Canvas<'_>,
    xf: &Xf,
    (x0, width, mid): (f32, f32, f32),
    label: &str,
    value: &str,
    value_style: Text,
    unit: &str,
) {
    let small = Text::new(8.0, c.hmi.fg_muted).mono();
    let lw = c.text_width(xf, label, small);
    let vw = c.text_width(xf, value, value_style);
    let uw = if unit.is_empty() {
        0.0
    } else {
        c.text_width(xf, unit, small)
    };
    let free = ((width - lw - vw - uw) / 2.0).max(0.0);
    c.text(xf, (x0, mid), label, small.align(Align::Start));
    c.text(
        xf,
        (x0 + lw + free, mid),
        value,
        value_style.align(Align::Start),
    );
    if !unit.is_empty() {
        c.text(xf, (x0 + width, mid), unit, small.align(Align::End));
    }
}

// ── Charts ────────────────────────────────────────────────────────────────

/// The pie and line palettes: the web's variables and literal colours as
/// the nearest theme tokens.
fn palette(hmi: &Hmi) -> [Hsla; 8] {
    [
        hmi.accent,
        hmi.ok,
        hmi.warning,
        hmi.danger,
        hmi.series[3],
        hmi.series[2],
        hmi.series[4],
        hmi.series[5],
    ]
}

fn bar_chart(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    if item.tags.is_empty() {
        c.text(
            xf,
            (w / 2.0, h / 2.0),
            "Drop tags here",
            Text::new(11.0, hmi.fg_muted),
        );
        return;
    }
    let v = view_box(xf, (0.0, 0.0, w, h), (200.0, 120.0));
    let mut low = item
        .tags
        .iter()
        .map(|tag| tag.min)
        .fold(f64::INFINITY, f64::min);
    let high = item
        .tags
        .iter()
        .map(|tag| tag.max)
        .fold(f64::NEG_INFINITY, f64::max);
    if low > 0.0 {
        low = 0.0;
    }
    let span = if high - low == 0.0 { 1.0 } else { high - low };
    let (left, top) = (28.0, 10.0);
    let (chart_w, chart_h) = (200.0 - 28.0 - 8.0, 120.0 - 10.0 - 22.0);
    let n = item.tags.len() as f32;
    let gap = (chart_w / n * 0.15).clamp(2.0, 6.0);
    let bar_w = ((chart_w - gap * (n + 1.0)) / n).max(8.0);
    let grid = Line::scaled(0.4).dashed(2.0, 2.0, 0.0);
    let axis = Text::new(6.0, fade(hmi.fg_muted, 0.7))
        .mono()
        .align(Align::End);
    for p in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let y = top + chart_h * (1.0 - p);
        segment(
            c,
            &v,
            (left, y),
            (left + chart_w, y),
            fade(hmi.border, 0.6),
            grid,
        );
        let value = low + span * f64::from(p);
        c.text(
            &v,
            (left - 4.0, base(y + 3.0, 6.0)),
            &format!("{value:.0}"),
            axis,
        );
    }
    let show_values = item.widget.flag("showValues") != Some(false);
    for (index, tag) in item.tags.iter().enumerate() {
        let value = tag.value.unwrap_or(0.0);
        let pct = fraction(value, low, low + span) as f32;
        let bar_h = chart_h * pct;
        let x = left + gap + index as f32 * (bar_w + gap);
        let y = top + chart_h - bar_h;
        let color = if tag.value.is_some() {
            level_color(hmi, tag.level)
        } else {
            hmi.accent
        };
        c.fill(
            &v,
            &Shape::round_rect(x, top, bar_w, chart_h, 2.0),
            fade(hmi.border, 0.08),
        );
        if bar_h > 0.0 {
            c.fill(
                &v,
                &Shape::round_rect(x, y, bar_w, bar_h, 2.0),
                fade(color, 0.85),
            );
            c.fill(
                &v,
                &Shape::round_rect(x + 1.0, y, bar_w - 2.0, bar_h.min(3.0), 1.0),
                fade(hmi.white, 0.15),
            );
            if show_values {
                c.text(
                    &v,
                    (x + bar_w / 2.0, base(y - 3.0, 6.0)),
                    &format!("{value:.1}"),
                    Text::new(6.0, hmi.fg).mono().semibold(),
                );
            }
        }
        c.text(
            &v,
            (x + bar_w / 2.0, base(top + chart_h + 11.0, 5.5)),
            &short(tag.name.as_str(), 6, 5),
            Text::new(5.5, hmi.fg_muted),
        );
    }
}

fn pie_chart(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    if item.tags.is_empty() {
        c.text(
            xf,
            (w / 2.0, h / 2.0),
            "Drop tags here",
            Text::new(11.0, hmi.fg_muted),
        );
        return;
    }
    let v = view_box(xf, (0.0, 0.0, w, h), (200.0, 170.0));
    let colors = palette(hmi);
    let values: Vec<f64> = item
        .tags
        .iter()
        .map(|tag| tag.value.unwrap_or(0.0).max(0.0))
        .collect();
    let sum: f64 = values.iter().sum();
    let total = if sum == 0.0 { 1.0 } else { sum };
    let (cx, cy, r, gap) = (80.0f32, 75.0f32, 55.0f32, 1.5f32);
    c.stroke(
        &v,
        &Shape::circle(cx, cy, r + 2.0),
        fade(hmi.black, 0.06),
        Line::scaled(4.0),
    );
    let mut done = 0.0f32;
    let mut labels = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let pct = (value / total) as f32;
        let sweep = pct * 360.0;
        let start = done * 360.0 + gap / 2.0;
        let end = done * 360.0 + sweep - gap / 2.0;
        done += pct;
        let at = |deg: f32| {
            let (sin, cos) = (deg - 90.0).to_radians().sin_cos();
            (cx + r * cos, cy + r * sin)
        };
        let ((x1, y1), (x2, y2)) = (at(start), at(end));
        let large = u8::from(sweep - gap > 180.0);
        let path = if pct >= 0.99 {
            format!("M{cx} {} A{r} {r} 0 1 1 {} {} Z", cy - r, cx - 0.01, cy - r)
        } else {
            format!("M{cx} {cy} L{x1} {y1} A{r} {r} 0 {large} 1 {x2} {y2} Z")
        };
        let shape = Shape::svg_owned(&path);
        let color = colors[index % colors.len()];
        c.fill(&v, &shape, color);
        c.stroke(&v, &shape, hmi.bg, Line::scaled(1.5));
        let mid = ((start + end) / 2.0 - 90.0).to_radians();
        labels.push((pct, cx + r * 0.65 * mid.cos(), cy + r * 0.65 * mid.sin()));
    }
    c.fill(&v, &Shape::circle(cx, cy, r * 0.35), hmi.bg);
    c.fill(&v, &Shape::circle(cx, cy, r * 0.35), fade(hmi.surface, 0.5));
    let halo = fade(hmi.black, 0.3);
    for (tag, (pct, lx, ly)) in item.tags.iter().zip(&labels) {
        if *pct <= 0.1 {
            continue;
        }
        c.text(
            &v,
            (*lx, base(ly - 1.0, 6.5)),
            &short(tag.name.as_str(), 5, 4),
            Text::new(6.5, hmi.white).semibold().halo(halo),
        );
        c.text(
            &v,
            (*lx, base(ly + 7.0, 5.5)),
            &format!("{:.0}%", pct * 100.0),
            Text::new(5.5, fade(hmi.white, 0.9)).mono().halo(halo),
        );
    }
    let columns = item.tags.len().min(4);
    let col_w = 200.0 / columns as f32;
    for (index, tag) in item.tags.iter().enumerate() {
        let x = (index % columns) as f32 * col_w + 4.0;
        let y = 145.0 + (index / columns) as f32 * 12.0;
        c.fill(
            &v,
            &Shape::round_rect(x, y, 6.0, 6.0, 1.5),
            colors[index % colors.len()],
        );
        c.text(
            &v,
            (x + 9.0, base(y + 5.5, 5.5)),
            tag.name.as_str(),
            Text::new(5.5, hmi.fg_muted).align(Align::Start),
        );
    }
}

// ── Trends ────────────────────────────────────────────────────────────────

/// Cardinal-spline tension of the web's trends.
const TENSION: f32 = 0.3;

/// SVG path data of the web's smooth line through `points`.
fn spline(points: &[(f32, f32)]) -> String {
    let mut d = String::new();
    let Some(&(x0, y0)) = points.first() else {
        return d;
    };
    let _ = write!(d, "M{x0} {y0}");
    if points.len() == 2 {
        let (x, y) = points[1];
        let _ = write!(d, " L{x} {y}");
        return d;
    }
    let last = points.len() - 1;
    for (i, pair) in points.windows(2).enumerate() {
        let (p0, p1, p2, p3) = (
            points[i.saturating_sub(1)],
            pair[0],
            pair[1],
            points[(i + 2).min(last)],
        );
        let c1 = (
            p1.0 + (p2.0 - p0.0) * TENSION,
            p1.1 + (p2.1 - p0.1) * TENSION,
        );
        let c2 = (
            p2.0 - (p3.0 - p1.0) * TENSION,
            p2.1 - (p3.1 - p1.1) * TENSION,
        );
        let _ = write!(d, " C{} {} {} {} {} {}", c1.0, c1.1, c2.0, c2.1, p2.0, p2.1);
    }
    d
}

/// Enough decimals that the five grid labels across `range` differ.
fn axis_decimals(range: f64) -> usize {
    (-(range / 4.0).log10()).ceil().clamp(0.0, 4.0) as usize
}

/// The trend's y range: the samples' extent, at least 5 % of the tag's
/// span (`span`) wide, with 5 % head room each way.
fn trend_range(values: &[f64], span: f64) -> (f64, f64) {
    let mut low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let mut high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_span = if span * 0.05 == 0.0 {
        1e-6
    } else {
        span * 0.05
    };
    if high - low < min_span {
        let mid = (high + low) / 2.0;
        low = mid - min_span / 2.0;
        high = mid + min_span / 2.0;
    }
    let range = high - low;
    (low - range * 0.05, high + range * 0.05)
}

#[derive(Clone, Copy)]
struct Plot {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl Plot {
    fn points(&self, values: &[f64], low: f64, range: f64) -> Vec<(f32, f32)> {
        let step = self.width / (values.len().max(2) - 1) as f32;
        values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let at = ((v - low) / range) as f32;
                (
                    self.left + i as f32 * step,
                    self.top + self.height - at * self.height,
                )
            })
            .collect()
    }

    /// Four dashed grid lines and their labels.
    fn grid(
        &self,
        c: &mut Canvas<'_>,
        xf: &Xf,
        low: f64,
        range: f64,
        label: impl Fn(f64) -> String,
    ) {
        let hmi = c.hmi;
        let style = Text::new(9.0, fade(hmi.fg_muted, 0.6))
            .mono()
            .align(Align::End);
        for i in 0..=4u8 {
            let p = f32::from(i) / 4.0;
            let y = self.top + self.height * (1.0 - p);
            segment(
                c,
                xf,
                (self.left, y),
                (self.left + self.width, y),
                hmi.border,
                Line::scaled(0.5).dashed(3.0, 3.0, 0.0),
            );
            let value = low + range * f64::from(p);
            c.text(xf, (self.left - 4.0, y), &label(value), style);
        }
    }

    /// A smooth line with its fill and the last point's dot.
    fn line(
        &self,
        c: &mut Canvas<'_>,
        xf: &Xf,
        points: &[(f32, f32)],
        color: Hsla,
        fill: (f32, f32),
    ) {
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            return;
        };
        let d = spline(points);
        let bottom = self.top + self.height;
        let area = format!("{d} L{} {bottom} L{} {bottom} Z", last.0, first.0);
        let paint = Paint::gradient(
            180.0,
            (fade(color, fill.0), 0.0),
            (fade(color, fill.1), 1.0),
        );
        c.fill(xf, &Shape::svg_owned(&area), paint);
        c.stroke(xf, &Shape::svg_owned(&d), color, Line::scaled(1.5).round());
        c.fill(xf, &Shape::circle(last.0, last.1, 3.0), color);
    }

    fn placeholder(&self, c: &mut Canvas<'_>, xf: &Xf, y: f32) {
        let muted = c.hmi.fg_muted;
        segment(
            c,
            xf,
            (self.left, y),
            (self.left + self.width, y),
            muted,
            Line::scaled(1.0).dashed(4.0, 4.0, 0.0),
        );
    }
}

fn nonzero(range: f64) -> f64 {
    if range == 0.0 { 1.0 } else { range }
}

fn trend(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    let widget = &item.widget;
    let color = token(hmi, widget.text("lineColor"), hmi.accent);
    let tag = item.primary.as_ref();
    let values: Vec<f64> =
        tag.map_or_else(Vec::new, |tag| tag.samples.iter().map(|s| s.1).collect());
    let plot = Plot {
        left: 32.0,
        top: 18.0,
        width: w - 40.0,
        height: h - 32.0,
    };
    let (low, high) = if values.len() < 2 {
        tag.map_or((0.0, 100.0), |tag| (tag.min, tag.max))
    } else {
        trend_range(&values, tag.map_or(0.0, |tag| (tag.max - tag.min).abs()))
    };
    let range = nonzero(high - low);
    let decimals = axis_decimals(range);
    plot.grid(c, xf, low, range, |v| format!("{v:.decimals$}"));
    if values.len() < 2 {
        plot.placeholder(c, xf, plot.top + plot.height / 2.0);
        return;
    }
    let points = plot.points(&values, low, range);
    plot.line(c, xf, &points, color, (0.19, 0.02));
    if let Some(last) = points.last() {
        c.stroke(
            xf,
            &Shape::circle(last.0, last.1, 5.0),
            fade(color, 0.25),
            Line::scaled(1.0),
        );
    }
    let Some(tag) = tag else {
        return;
    };
    let current = values.last().copied().unwrap_or(0.0);
    let mut text = format!("{current:.*}", tag.decimals);
    if !tag.unit.is_empty() {
        text = format!("{text} {}", tag.unit);
    }
    let value_style = Text::new(10.0, hmi.fg).mono().bold().align(Align::End);
    c.text(xf, (plot.left + plot.width, 7.5), &text, value_style);
    if widget.flag("showName") != Some(false) && !tag.name.is_empty() {
        let room = plot.width - c.text_width(xf, &text, value_style) - 12.0;
        if room > 0.0 {
            c.text(
                xf,
                (plot.left, 7.5),
                tag.name.as_str(),
                Text::new(10.0, hmi.fg_muted)
                    .semibold()
                    .align(Align::Start)
                    .max_width(room),
            );
        }
    }
}

fn multi_trend(c: &mut Canvas<'_>, item: &Item, xf: &Xf) {
    let (w, h) = size(item);
    let hmi = c.hmi;
    if item.tags.is_empty() {
        c.text(
            xf,
            (w / 2.0, h / 2.0),
            "Drop tags here",
            Text::new(11.0, hmi.fg_muted),
        );
    }
    let all = palette(hmi);
    let colors = [all[0], all[1], all[2], all[4], all[5]];
    let plot = Plot {
        left: 32.0,
        top: 8.0,
        width: w - 40.0,
        height: h - 26.0,
    };
    let series: Vec<Vec<f64>> = item
        .tags
        .iter()
        .map(|tag| tag.samples.iter().map(|s| s.1).collect())
        .collect();
    let low = series
        .iter()
        .flatten()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let high = series
        .iter()
        .flatten()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    if !low.is_finite() {
        plot.placeholder(c, xf, h / 2.0);
        return;
    }
    let pad = nonzero(high - low) * 0.05;
    let (low, high) = (low - pad, high + pad);
    let range = nonzero(high - low);
    plot.grid(c, xf, low, range, auto_fixed);
    for (index, values) in series.iter().enumerate() {
        if values.len() < 2 {
            continue;
        }
        let points = plot.points(values, low, range);
        plot.line(c, xf, &points, colors[index % colors.len()], (0.125, 0.012));
    }
    let y = h - 4.0;
    let mut x = plot.left;
    let style = Text::new(9.0, hmi.fg_muted).align(Align::Start);
    for (index, tag) in item.tags.iter().enumerate() {
        let name = short(tag.name.as_str(), 10, 9);
        c.fill(
            xf,
            &Shape::circle(x + 4.0, y, 3.0),
            colors[index % colors.len()],
        );
        x += 10.0;
        c.text(xf, (x, y), &name, style);
        x += c.text_width(xf, &name, style) + 12.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_boxes_fit_like_meet() {
        let (scale, ox, oy) = meet((400.0, 165.0), (200.0, 165.0));
        assert!((scale - 1.0).abs() < 1e-6);
        assert!((ox - 100.0).abs() < 1e-6 && oy.abs() < 1e-6);
        let (scale, ox, oy) = meet((100.0, 300.0), (200.0, 100.0));
        assert!((scale - 0.5).abs() < 1e-6);
        assert!(ox.abs() < 1e-6 && (oy - 125.0).abs() < 1e-6);
        let (empty, _, _) = meet((10.0, 10.0), (0.0, 5.0));
        assert!(empty.abs() < 1e-9);
    }

    #[test]
    fn axis_labels_get_enough_decimals() {
        assert_eq!(axis_decimals(100.0), 0);
        assert_eq!(axis_decimals(4.0), 0);
        assert_eq!(axis_decimals(2.0), 1);
        assert_eq!(axis_decimals(0.02), 3);
        assert_eq!(axis_decimals(1e-9), 4);
        assert_eq!(auto_fixed(5.0), "5");
        assert_eq!(auto_fixed(5.26), "5.3");
        assert_eq!(js_num(100.0), "100");
        assert_eq!(js_num(0.5), "0.5");
    }

    #[test]
    fn a_flat_trend_keeps_a_minimum_span() {
        let (low, high) = trend_range(&[10.0, 10.0], 100.0);
        assert!((high - low - 5.5).abs() < 1e-9, "{low} {high}");
        assert!(((low + high) / 2.0 - 10.0).abs() < 1e-9);
        let (low, high) = trend_range(&[0.0, 20.0], 100.0);
        assert!((low + 1.0).abs() < 1e-9 && (high - 21.0).abs() < 1e-9);
        let (low, high) = trend_range(&[3.0, 3.0], 0.0);
        assert!(high > low);
    }

    #[test]
    fn the_spline_runs_through_every_point() {
        assert_eq!(spline(&[]), "");
        assert_eq!(spline(&[(0.0, 0.0), (10.0, 5.0)]), "M0 0 L10 5");
        let d = spline(&[(0.0, 0.0), (10.0, 10.0), (20.0, 0.0)]);
        assert!(d.starts_with("M0 0 C"), "{d}");
        assert_eq!(d.matches('C').count(), 2);
        assert!(d.ends_with(" 20 0"), "{d}");
    }

    #[test]
    fn settings_name_theme_variables() {
        assert_eq!(var_name("var(--ok)"), Some("ok"));
        assert_eq!(var_name(" var(--hmi-fault) "), Some("hmi-fault"));
        assert_eq!(var_name("#22d3ee"), None);
        assert_eq!(short("Temperature", 6, 5), "Tempe..");
        assert_eq!(short("Flow", 6, 5), "Flow");
    }

    #[test]
    fn annunciator_grids_fill_columns() {
        assert_eq!(grid_columns(280.0, 150.0, 4.0), (1, 280.0));
        assert_eq!(grid_columns(100.0, 150.0, 4.0), (1, 150.0));
        let (columns, width) = grid_columns(400.0, 150.0, 4.0);
        assert_eq!(columns, 2);
        assert!((width - 198.0).abs() < 1e-4);
    }

    #[test]
    fn seven_segments_light_digits() {
        assert_eq!(segments('8'), [true; 7]);
        assert_eq!(
            segments('-'),
            [false, false, false, true, false, false, false]
        );
        assert_eq!(segments('.'), [false; 7]);
    }
}
