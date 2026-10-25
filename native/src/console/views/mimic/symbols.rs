//! The symbol library (`hmi/symbols.tsx`) and the symbol view
//! (`hmi/SymbolView.tsx`): ISA-101 equipment drawn in its view box, fitted
//! (or stretched) into the widget's box above its footer, turned and
//! mirrored as set, with its live state, level and value; then the footer
//! rows (label, value badge, state text) under the drawing.
//!
//! Text inside a turned symbol stays upright at its mapped anchor (GPUI
//! cannot turn glyphs), where the web turns it with the drawing; text a
//! symbol right-aligns (the meter's reading) is centred on where its middle
//! falls, so it stays on its display when mirrored or turned.

use std::borrow::Cow;

use gpui::Hsla;
use voltrus_scada_native::mimic::{
    EquipState, FooterRows, Level, Placement, Widget, footer, footer_height, footer_rows,
    state_text,
};

use super::paint::{Align, Canvas, Hmi, Line, Paint, Shape, Text, Xf, fade};
use super::scene::{Item, TagView, motion};

/// Draw `item`'s library symbol and its footer; nothing for other widgets.
pub(crate) fn paint(canvas: &mut Canvas<'_>, item: &Item, xf: &Xf, t: f32) {
    let Some((resolved, live)) = item.symbol.as_ref() else {
        return;
    };
    let widget = &item.widget;
    let place = Placement::of(widget, resolved);
    let (w, h) = (place.width as f32, place.height as f32);
    // The drawing area turned, then mirrored, about its centre.
    let mut area = xf.then(&Xf::rotate_about(
        f32::from(place.quarter_turns) * 90.0,
        w / 2.0,
        h / 2.0,
    ));
    if place.flip {
        area = area.then(&Xf::scale_x_about(-1.0, w / 2.0));
    }
    let (sx, sy) = (place.scale_x as f32, place.scale_y as f32);
    let (ox, oy) = (place.offset_x as f32, place.offset_y as f32);
    let primary = item.primary.as_ref();
    let value = primary.and_then(TagView::text);
    let sym = Sym {
        key: resolved.def.key,
        xf: area.then(&Xf::translate(ox, oy)).then(&Xf::scale(sx, sy)),
        area,
        scale: (sx, sy),
        offset: (ox, oy),
        state: live.state,
        run: live.run_state,
        level: live.level.map(|level| level as f32),
        show_percent: resolved.show_percent,
        value: value.as_deref().unwrap_or(""),
        unit: primary.map_or("", |tag| tag.unit.as_str()),
        widget,
        t,
    };
    let alpha = if live.state == EquipState::Fault {
        motion::blink(t)
    } else {
        1.0
    };
    canvas.faded(alpha, |c| draw(c, &sym));

    let rows = footer_rows(widget, resolved);
    let [label, badge, status] = footer_centres(rows, widget.height as f32);
    let cx = widget.width as f32 / 2.0;
    let hmi = canvas.hmi;
    if let Some(y) = label {
        let text = widget.text("label").unwrap_or("");
        let style = Text::new(11.0, hmi.fg_dim)
            .semibold()
            .max_width(widget.width as f32)
            .halo(hmi.stage);
        canvas.text(xf, (cx, y), text, style);
    }
    if let Some(y) = badge {
        let unit = primary.map_or("", |tag| tag.unit.as_str());
        value_badge(canvas, xf, (cx, y), value.as_deref(), unit, live.alarm, t);
    }
    if let Some(y) = status {
        let color = match live.state {
            EquipState::Running => hmi.run,
            EquipState::Fault => hmi.fault,
            EquipState::Stale => hmi.stale,
            EquipState::Idle | EquipState::Stopped => hmi.fg_muted,
        };
        status_chip(
            canvas,
            xf,
            (cx, y),
            state_text(resolved.def, live.state),
            color,
        );
    }
}

/// The vertical centres of the footer rows (label, badge, state text) in a
/// box `height` tall: fixed heights under the drawing, as the web lays them
/// out (`FOOTER`).
fn footer_centres(rows: FooterRows, height: f32) -> [Option<f32>; 3] {
    let total = footer_height(rows) as f32;
    let mut y = (height - total).max(0.0) + footer::PAD as f32;
    let mut out = [None; 3];
    let shown = [
        (rows.label, footer::LABEL),
        (rows.value, footer::BADGE),
        (rows.status, footer::STATUS),
    ];
    for (slot, (on, row)) in out.iter_mut().zip(shown) {
        if on {
            let row = row as f32;
            *slot = Some(y + row / 2.0);
            y += row + footer::GAP as f32;
        }
    }
    out
}

// ── The footer ────────────────────────────────────────────────────────────

/// `ValueBadge`: the value and unit on a dark chip, its border and diamond
/// coloured by the alarm, blinking while critical; `????` without a value.
fn value_badge(
    c: &mut Canvas<'_>,
    xf: &Xf,
    (cx, cy): (f32, f32),
    value: Option<&str>,
    unit: &str,
    alarm: Option<Level>,
    t: f32,
) {
    const PAD_X: f32 = 6.0;
    const GAP: f32 = 4.0;
    const DIAMOND: f32 = 6.0;
    let h = c.hmi;
    let border = match alarm {
        Some(Level::Critical) => h.fault,
        Some(Level::Warning) => h.warn,
        Some(Level::Normal) | None => fade(h.white, 0.08),
    };
    let flagged = matches!(alarm, Some(Level::Critical | Level::Warning));
    let shown = value.unwrap_or("????");
    let value_style = Text::new(
        12.0,
        if value.is_some() {
            h.badge_text
        } else {
            h.stale
        },
    )
    .mono()
    .semibold()
    .align(Align::Start);
    let unit_style = Text::new(10.0, h.badge_unit).mono().align(Align::Start);
    let value_width = c.text_width(xf, shown, value_style);
    let unit_width = if unit.is_empty() {
        0.0
    } else {
        GAP + c.text_width(xf, unit, unit_style)
    };
    let diamond = if flagged { DIAMOND + GAP } else { 0.0 };
    let width = diamond + value_width + unit_width + 2.0 * (PAD_X + 1.0);
    let height = footer::BADGE as f32;
    let left = cx - width / 2.0;
    let alpha = if alarm == Some(Level::Critical) {
        motion::blink(t)
    } else {
        1.0
    };
    c.faded(alpha, |c| {
        c.rect(
            xf,
            (left, cy - height / 2.0, width, height),
            3.0,
            h.badge,
            Some((border, 1.0)),
        );
        let mut x = left + 1.0 + PAD_X;
        if flagged {
            let (mx, r) = (x + DIAMOND / 2.0, DIAMOND * std::f32::consts::FRAC_1_SQRT_2);
            let shape = Shape::polyline(
                &[(mx, cy - r), (mx + r, cy), (mx, cy + r), (mx - r, cy)],
                true,
            );
            c.fill(xf, &shape, border);
            x += diamond;
        }
        c.text(xf, (x, cy), shown, value_style);
        if !unit.is_empty() {
            // Baselines aligned: the smaller unit sits a little lower.
            c.text(xf, (x + value_width + GAP, cy + 0.7), unit, unit_style);
        }
    });
}

/// `StatusChip`: the state in small bold capitals, letter-spaced, with a
/// halo in the stage colour.
fn status_chip(c: &mut Canvas<'_>, xf: &Xf, (cx, cy): (f32, f32), text: &str, color: Hsla) {
    const SIZE: f32 = 9.0;
    const SPACING: f32 = 0.08 * SIZE;
    let stage = c.hmi.stage;
    let style = Text::new(SIZE, color).bold().align(Align::Start);
    let mut buf = [0u8; 4];
    let total: f32 = text
        .chars()
        .map(|ch| c.text_width(xf, ch.encode_utf8(&mut buf), style) + SPACING)
        .sum();
    // Halos first, then the glyphs, so no halo covers a neighbour.
    for pass in [true, false] {
        let mut x = cx - total / 2.0;
        for ch in text.chars() {
            let glyph = ch.encode_utf8(&mut buf);
            let width = c.text_width(xf, glyph, style);
            let drawn = if pass {
                Text::new(SIZE, fade(color, 0.0))
                    .bold()
                    .align(Align::Start)
                    .halo(stage)
            } else {
                style
            };
            c.text(xf, (x, cy), glyph, drawn);
            x += width + SPACING;
        }
    }
}

// ── Shared paint ──────────────────────────────────────────────────────────

/// One symbol being drawn: its view box mapped to the window, its live
/// state and what it shows.
struct Sym<'a> {
    key: &'static str,
    /// View box units to window pixels.
    xf: Xf,
    /// The drawing area (stage px, turned and mirrored) to window pixels.
    area: Xf,
    /// View box to drawing area: scale, then offset.
    scale: (f32, f32),
    offset: (f32, f32),
    state: EquipState,
    run: EquipState,
    level: Option<f32>,
    show_percent: bool,
    /// The formatted value, empty without one.
    value: &'a str,
    unit: &'a str,
    widget: &'a Widget,
    t: f32,
}

/// The outline of a symbol's parts: grey, red and heavier on a fault,
/// dashed when stale; never scaled (`non-scaling-stroke`).
fn outline(h: &Hmi, state: EquipState) -> (Hsla, Line) {
    match state {
        EquipState::Fault => (h.fault, Line::hairline(2.0)),
        EquipState::Stale => (h.stale, Line::hairline(1.25).dashed(4.0, 3.0, 0.0)),
        EquipState::Idle | EquipState::Running | EquipState::Stopped => {
            (h.stroke, Line::hairline(1.25))
        }
    }
}

/// The colour of the moving or indicating part.
fn accent(h: &Hmi, state: EquipState) -> Hsla {
    match state {
        EquipState::Running => h.run,
        EquipState::Fault => h.fault,
        EquipState::Stale => h.stale,
        EquipState::Idle | EquipState::Stopped => h.detail,
    }
}

fn at_rest(state: EquipState) -> bool {
    matches!(state, EquipState::Idle | EquipState::Stopped)
}

/// The square around a circle, for drawing it as a rounded quad.
fn disc(cx: f32, cy: f32, r: f32) -> (f32, f32, f32, f32) {
    (cx - r, cy - r, 2.0 * r, 2.0 * r)
}

/// `Math.round(level * 100)%`.
fn percent(level: f32) -> String {
    format!("{}%", (level.clamp(0.0, 1.0) * 100.0).round() as u32)
}

/// A text setting, numbers written out, `default` when unset (`??`).
fn config_text<'a>(widget: &'a Widget, key: &str, default: &'static str) -> Cow<'a, str> {
    match widget.get(key) {
        Some(value) => value
            .as_str()
            .map(Cow::Borrowed)
            .or_else(|| value.as_f64().map(|n| Cow::Owned(n.to_string())))
            .unwrap_or(Cow::Borrowed(default)),
        None => Cow::Borrowed(default),
    }
}

impl Sym<'_> {
    /// A filled part with the state outline.
    fn body(&self, c: &mut Canvas<'_>, shape: &Shape, paint: impl Into<Paint>) {
        c.fill(&self.xf, shape, paint);
        self.edge(c, shape);
    }

    /// The state outline alone (`fill="none"`).
    fn edge(&self, c: &mut Canvas<'_>, shape: &Shape) {
        let (color, line) = outline(c.hmi, self.state);
        c.stroke(&self.xf, shape, color, line);
    }

    /// A (rounded) rectangle part with the state outline, as a quad.
    fn boxed(
        &self,
        c: &mut Canvas<'_>,
        rect: (f32, f32, f32, f32),
        radius: f32,
        paint: impl Into<Paint>,
    ) {
        if self.state == EquipState::Stale {
            c.rect(&self.xf, rect, radius, paint, None);
            self.rim(c, rect, radius);
        } else {
            let (color, line) = outline(c.hmi, self.state);
            c.rect(&self.xf, rect, radius, paint, Some((color, line.width)));
        }
    }

    /// The state outline of a (rounded) rectangle, unfilled.
    fn rim(&self, c: &mut Canvas<'_>, (x, y, w, h): (f32, f32, f32, f32), radius: f32) {
        let (color, line) = outline(c.hmi, self.state);
        if self.state == EquipState::Stale {
            c.stroke(
                &self.xf,
                &Shape::round_rect(x, y, w, h, radius),
                color,
                line,
            );
        } else {
            let clear = fade(color, 0.0);
            c.rect(
                &self.xf,
                (x, y, w, h),
                radius,
                clear,
                Some((color, line.width)),
            );
        }
    }

    /// A circle part with the state outline.
    fn circle(&self, c: &mut Canvas<'_>, (cx, cy): (f32, f32), r: f32, paint: impl Into<Paint>) {
        self.boxed(c, disc(cx, cy, r), r, paint);
    }

    /// A plain circle (no outline, or a thin one).
    fn dot(
        &self,
        c: &mut Canvas<'_>,
        (cx, cy): (f32, f32),
        r: f32,
        paint: impl Into<Paint>,
        border: Option<(Hsla, f32)>,
    ) {
        c.rect(&self.xf, disc(cx, cy, r), r, paint, border);
    }

    fn segment(
        &self,
        c: &mut Canvas<'_>,
        from: (f32, f32),
        to: (f32, f32),
        color: Hsla,
        line: Line,
    ) {
        c.stroke(&self.xf, &Shape::polyline(&[from, to], false), color, line);
    }

    fn text(&self, c: &mut Canvas<'_>, at: (f32, f32), text: &str, style: Text) {
        c.text(&self.xf, at, text, style);
    }

    /// `shape` under a local motion. Kept as the cached shape while still;
    /// made afresh while moving, since a transform changing every frame
    /// would only fill the tessellation cache.
    fn moved(&self, shape: &Shape, motion: &Xf, moving: bool) -> (Xf, Shape) {
        if moving {
            (self.xf, shape.transformed(motion))
        } else {
            (self.xf.then(motion), shape.clone())
        }
    }

    /// `LevelFill`: the vessel below the level in liquid (or bulk solids),
    /// with a light surface line.
    fn level_fill(&self, c: &mut Canvas<'_>, vessel: &Shape, top: f32, bottom: f32, bulk: bool) {
        let Some(level) = self.level else {
            return;
        };
        let h = c.hmi;
        let y = bottom - (bottom - top) * level;
        c.fill(
            &self.xf,
            &vessel.below(y),
            if bulk { h.bk() } else { h.lq() },
        );
        let surface = Shape::rect(0.0, y, 200.0, 1.5).within(vessel);
        c.fill(&self.xf, &surface, fade(h.white, 0.35));
    }

    /// `LevelText`: the level's percentage, unless hidden.
    fn level_text(&self, c: &mut Canvas<'_>, at: (f32, f32)) {
        let Some(level) = self.level.filter(|_| self.show_percent) else {
            return;
        };
        let h = c.hmi;
        let style = Text::new(13.0, h.white)
            .mono()
            .bold()
            .halo(fade(h.black, 0.55));
        self.text(c, at, &percent(level), style);
    }

    /// `Ticks`: a level scale, long ticks every half.
    fn ticks(&self, c: &mut Canvas<'_>, x: f32, top: f32, bottom: f32) {
        let color = fade(c.hmi.detail, 0.7);
        for i in 0..=10u8 {
            let y = bottom - (bottom - top) * f32::from(i) / 10.0;
            let long = if i % 5 == 0 { 7.0 } else { 4.0 };
            self.segment(c, (x, y), (x + long, y), color, Line::hairline(1.0));
        }
    }

    /// `Lamp`: a small state light.
    fn lamp(&self, c: &mut Canvas<'_>, at: (f32, f32), r: f32) {
        let h = c.hmi;
        let border = (fade(h.black, 0.35), 0.75);
        self.dot(c, at, r, accent(h, self.state), Some(border));
    }

    /// A hub over an impeller or fan.
    fn hub(&self, c: &mut Canvas<'_>, at: (f32, f32), r: f32) {
        let h = c.hmi;
        self.dot(c, at, r, h.body_hi, Some((h.detail, 1.0)));
    }
}

// ── The symbols ───────────────────────────────────────────────────────────

fn draw(c: &mut Canvas<'_>, s: &Sym<'_>) {
    match s.key {
        "dosing_pump" => dosing_pump(c, s),
        "fan" => fan(c, s),
        "motor" => motor(c, s),
        "compressor" => compressor(c, s),
        "conveyor" => conveyor(c, s),
        "valve" => valve(c, s),
        "control_valve" => control_valve(c, s),
        "check_valve" => check_valve(c, s),
        "tank" => tank(c, s),
        "tank_horizontal" => tank_horizontal(c, s),
        "silo" => silo(c, s),
        "mixer" => mixer(c, s),
        "clarifier" => clarifier(c, s),
        "gravity_filter" => gravity_filter(c, s),
        "heat_exchanger" => heat_exchanger(c, s),
        "filter" => filter(c, s),
        "breaker" => breaker(c, s),
        "transformer" => transformer(c, s),
        "generator" => generator(c, s),
        "solar_panel" => solar_panel(c, s),
        "battery" => battery(c, s),
        "inverter" => inverter(c, s),
        "meter" => meter(c, s),
        "busbar" => busbar(c, s),
        "instrument" => instrument(c, s),
        "flow_meter" => flow_meter(c, s),
        "pipe" | "pipe_vertical" => straight_pipe(c, s),
        "pipe_elbow" => pipe_path(c, s, Shape::svg("M0 30 L20 30 Q30 30 30 40 L30 60")),
        "pipe_tee" => pipe_path(c, s, Shape::svg("M0 12 L80 12 M40 12 L40 50")),
        // `resolve` falls back to the pump for unknown keys.
        _ => pump(c, s),
    }
}

fn pump(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.body(c, &Shape::svg("M30 76 L70 76 L82 95 L18 95 Z"), h.cv());
    s.boxed(c, (2.0, 45.0, 26.0, 15.0), 0.0, h.pv());
    s.boxed(c, (50.0, 22.0, 47.0, 15.0), 0.0, h.pv());
    s.circle(c, (50.0, 52.0), 30.0, h.sp());
    s.dot(c, (50.0, 52.0), 21.0, fade(h.body_lo, 0.45), None);
    let running = s.state == EquipState::Running;
    let spin = if running { motion::spin(s.t) } else { 0.0 };
    let blade = Shape::svg("M50 52 C 57 47, 62 40, 63 33");
    let color = accent(h, s.state);
    for angle in [0.0, 120.0, 240.0] {
        let turn = Xf::rotate_about(angle + spin, 50.0, 52.0);
        let (xf, shape) = s.moved(&blade, &turn, running);
        c.stroke(&xf, &shape, color, Line::scaled(5.0).round());
    }
    s.hub(c, (50.0, 52.0), 5.0);
}

fn dosing_pump(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.body(c, &Shape::svg("M22 78 L78 78 L86 89 L14 89 Z"), h.cv());
    s.boxed(c, (0.0, 51.0, 24.0, 14.0), 0.0, h.pv());
    s.boxed(c, (76.0, 51.0, 24.0, 14.0), 0.0, h.pv());
    s.boxed(c, (20.0, 36.0, 60.0, 44.0), 6.0, h.cv());
    let diaphragm = Shape::svg("M34 58 L50 46 L66 58 L50 70 Z");
    c.fill(&s.xf, &diaphragm, accent(h, s.state));
    c.stroke(&s.xf, &diaphragm, h.detail, Line::hairline(1.0));
    s.boxed(c, (38.0, 4.0, 24.0, 34.0), 4.0, h.ch());
    for y in [14.0, 22.0] {
        s.segment(c, (44.0, y), (56.0, y), h.detail, Line::hairline(1.5));
    }
    s.lamp(c, (50.0, 30.0), 3.2);
}

fn fan(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.circle(c, (50.0, 50.0), 44.0, h.sp());
    s.dot(c, (50.0, 50.0), 36.0, fade(h.detail, 0.55), None);
    let running = s.state == EquipState::Running;
    let spin = if running { motion::spin(s.t) } else { 0.0 };
    let blade = Shape::svg("M50 50 C 56 40, 70 30, 80 36 C 80 46, 66 52, 50 50 Z");
    let color = if at_rest(s.state) {
        h.body_mid
    } else {
        accent(h, s.state)
    };
    for angle in [0.0, 90.0, 180.0, 270.0] {
        let turn = Xf::rotate_about(angle + spin, 50.0, 50.0);
        let (xf, shape) = s.moved(&blade, &turn, running);
        c.fill(&xf, &shape, fade(color, 0.9));
    }
    s.hub(c, (50.0, 50.0), 6.0);
}

fn motor(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.boxed(c, (28.0, 64.0, 13.0, 12.0), 0.0, h.cv());
    s.boxed(c, (78.0, 64.0, 13.0, 12.0), 0.0, h.cv());
    s.boxed(c, (98.0, 35.0, 19.0, 10.0), 0.0, h.pv());
    s.boxed(c, (7.0, 20.0, 16.0, 40.0), 4.0, h.cv());
    s.boxed(c, (20.0, 12.0, 79.0, 56.0), 7.0, h.cv());
    let fins = fade(h.detail, 0.35);
    for y in [22.0, 30.0, 38.0, 46.0, 54.0] {
        s.segment(
            c,
            (28.0, y + 2.0),
            (92.0, y + 2.0),
            fins,
            Line::hairline(1.0),
        );
    }
    s.boxed(c, (46.0, 2.0, 27.0, 12.0), 2.0, h.cv());
    s.lamp(c, (59.5, 8.0), 3.2);
    s.dot(
        c,
        (59.5, 40.0),
        11.0,
        h.detail,
        Some((accent(h, s.state), 2.0)),
    );
    let letter = if at_rest(s.state) {
        h.body_hi
    } else {
        accent(h, s.state)
    };
    s.text(c, (59.5, 40.5), "M", Text::new(12.0, letter).bold());
}

fn compressor(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.body(c, &Shape::svg("M30 78 L70 78 L82 95 L18 95 Z"), h.cv());
    s.boxed(c, (2.0, 43.0, 18.0, 15.0), 0.0, h.pv());
    s.boxed(c, (80.0, 43.0, 18.0, 15.0), 0.0, h.pv());
    s.circle(c, (50.0, 50.0), 32.0, h.sp());
    c.stroke(
        &s.xf,
        &Shape::svg("M26 30 L74 41 M26 70 L74 59"),
        accent(h, s.state),
        Line::scaled(3.5).round(),
    );
}

fn conveyor(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    c.stroke(
        &s.xf,
        &Shape::svg("M40 42 L34 62 M160 42 L166 62"),
        h.body_lo,
        Line::scaled(4.0),
    );
    s.boxed(c, (10.0, 12.0, 180.0, 32.0), 16.0, h.detail);
    let offset = if s.state == EquipState::Running {
        motion::flow(s.t)
    } else {
        0.0
    };
    s.segment(
        c,
        (26.0, 13.5),
        (174.0, 13.5),
        accent(h, s.state),
        Line::scaled(3.0).dashed(10.0, 14.0, offset),
    );
    for x in [60.0, 85.0, 110.0, 135.0] {
        s.dot(c, (x, 28.0), 5.0, h.sp(), Some((h.detail, 1.0)));
    }
    for x in [26.0, 174.0] {
        s.circle(c, (x, 28.0), 13.0, h.sp());
        s.dot(c, (x, 28.0), 3.0, h.detail, None);
    }
}

fn valve(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let fill: Paint = match s.state {
        EquipState::Running => h.run.into(),
        EquipState::Fault => h.fault.into(),
        EquipState::Idle | EquipState::Stopped | EquipState::Stale => h.sp(),
    };
    s.boxed(c, (0.0, 45.0, 20.0, 14.0), 0.0, h.pv());
    s.boxed(c, (80.0, 45.0, 20.0, 14.0), 0.0, h.pv());
    s.segment(c, (50.0, 52.0), (50.0, 17.0), h.stroke, Line::scaled(3.0));
    s.boxed(c, (33.0, 10.0, 34.0, 7.0), 3.5, h.cv());
    s.body(
        c,
        &Shape::svg("M18 34 L50 52 L18 70 Z M82 34 L50 52 L82 70 Z"),
        fill,
    );
}

fn control_valve(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.boxed(c, (0.0, 65.0, 20.0, 14.0), 0.0, h.pv());
    s.boxed(c, (80.0, 65.0, 20.0, 14.0), 0.0, h.pv());
    s.segment(c, (50.0, 72.0), (50.0, 32.0), h.stroke, Line::scaled(3.0));
    s.body(c, &Shape::svg("M22 32 Q50 2 78 32 Z"), h.cv());
    s.segment(c, (20.0, 32.0), (80.0, 32.0), h.stroke, Line::hairline(2.0));
    s.body(
        c,
        &Shape::svg("M18 54 L50 72 L18 90 Z M82 54 L50 72 L82 90 Z"),
        h.sp(),
    );
    if !s.value.is_empty() {
        let style = Text::new(10.0, h.detail).mono().bold();
        s.text(c, (50.0, 22.0), s.value, style);
    }
}

fn check_valve(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.boxed(c, (0.0, 23.0, 22.0, 14.0), 0.0, h.pv());
    s.boxed(c, (78.0, 23.0, 22.0, 14.0), 0.0, h.pv());
    s.body(c, &Shape::svg("M20 10 L50 30 L20 50 Z"), h.sp());
    s.body(c, &Shape::svg("M80 10 L50 30 L80 50 Z"), h.stroke);
}

/// `vesselPath(6, 152)`.
const TANK: &str = "M12 24 Q12 6 50 6 Q88 6 88 24 L88 140 Q88 152 50 152 Q12 152 12 140 Z";
/// `vesselPath(24, 160)`.
const MIXER: &str = "M12 42 Q12 24 50 24 Q88 24 88 42 L88 148 Q88 160 50 160 Q12 160 12 148 Z";

fn tank(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let vessel = Shape::svg(TANK);
    s.boxed(c, (18.0, 146.0, 8.0, 21.0), 0.0, h.ch());
    s.boxed(c, (74.0, 146.0, 8.0, 21.0), 0.0, h.ch());
    c.fill(&s.xf, &vessel, h.ch());
    s.level_fill(c, &vessel, 10.0, 152.0, false);
    c.rect(
        &s.xf,
        (21.0, 22.0, 5.0, 116.0),
        0.0,
        fade(h.white, 0.1),
        None,
    );
    s.ticks(c, 80.0, 24.0, 140.0);
    s.edge(c, &vessel);
    s.level_text(c, (50.0, 80.0));
}

fn tank_horizontal(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let vessel = Shape::svg("M34 8 L146 8 A38 38 0 0 1 146 84 L34 84 A38 38 0 0 1 34 8 Z");
    s.body(
        c,
        &Shape::svg("M44 78 L64 78 L68 97 L40 97 Z M116 78 L136 78 L140 97 L112 97 Z"),
        h.cv(),
    );
    c.fill(&s.xf, &vessel, h.cv());
    s.level_fill(c, &vessel, 8.0, 84.0, false);
    c.rect(
        &s.xf,
        (34.0, 16.0, 112.0, 4.0),
        0.0,
        fade(h.white, 0.12),
        None,
    );
    s.edge(c, &vessel);
    s.level_text(c, (90.0, 46.0));
}

fn silo(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let vessel = Shape::svg("M12 30 L50 6 L88 30 L88 108 L58 148 L42 148 L12 108 Z");
    s.boxed(c, (43.0, 146.0, 14.0, 20.0), 0.0, h.ph());
    c.fill(&s.xf, &vessel, h.ch());
    s.level_fill(c, &vessel, 6.0, 148.0, true);
    let seam = fade(h.detail, 0.5);
    for y in [30.0, 108.0] {
        s.segment(c, (12.0, y), (88.0, y), seam, Line::hairline(1.0));
    }
    s.edge(c, &vessel);
    s.level_text(c, (50.0, 70.0));
}

fn mixer(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let vessel = Shape::svg(MIXER);
    let spinning = s.run == EquipState::Running;
    c.fill(&s.xf, &vessel, h.ch());
    s.level_fill(c, &vessel, 28.0, 160.0, false);
    s.segment(c, (50.0, 18.0), (50.0, 134.0), h.stroke, Line::scaled(2.5));
    let paddles = Shape::svg("M50 134 L30 128 L30 140 Z M50 134 L70 128 L70 140 Z");
    let swing = if spinning { motion::agitate(s.t) } else { 1.0 };
    let (xf, shape) = s.moved(&paddles, &Xf::scale_x_about(swing, 50.0), spinning);
    c.fill(&xf, &shape, if spinning { h.run } else { h.body_mid });
    c.stroke(&xf, &shape, h.detail, Line::hairline(1.0));
    s.edge(c, &vessel);
    s.boxed(c, (37.0, 0.0, 26.0, 20.0), 3.0, h.ch());
    s.dot(
        c,
        (50.0, 10.0),
        3.2,
        if spinning { h.run } else { h.detail },
        None,
    );
    s.level_text(c, (50.0, 90.0));
}

fn clarifier(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let basin = Shape::svg("M14 14 L206 14 L206 58 L124 86 L96 86 L14 58 Z");
    let turning = s.run == EquipState::Running;
    s.boxed(c, (0.0, 23.0, 18.0, 14.0), 0.0, h.pv());
    s.boxed(c, (203.0, 15.0, 17.0, 14.0), 0.0, h.pv());
    s.boxed(c, (102.0, 84.0, 16.0, 14.0), 0.0, h.ph());
    c.faded(0.85, |c| c.fill(&s.xf, &basin, h.lq()));
    let sludge = Shape::svg("M14 66 L96 92 L124 92 L206 66 L206 100 L14 100 Z").within(&basin);
    c.faded(0.9, |c| c.fill(&s.xf, &sludge, h.bk()));
    c.rect(
        &s.xf,
        (14.0, 14.0, 192.0, 3.0),
        0.0,
        fade(h.white, 0.3),
        None,
    );
    c.rect(
        &s.xf,
        (94.0, 14.0, 32.0, 38.0),
        0.0,
        fade(h.body_mid, 0.6),
        Some((h.detail, 1.0)),
    );
    s.segment(c, (110.0, 4.0), (110.0, 80.0), h.stroke, Line::scaled(2.5));
    let rake = Shape::svg("M110 80 L40 60 M110 80 L180 60");
    let swing = if turning { motion::agitate(s.t) } else { 1.0 };
    let (xf, shape) = s.moved(&rake, &Xf::scale_x_about(swing, 110.0), turning);
    let color = if turning { h.run } else { h.body_hi };
    c.stroke(&xf, &shape, color, Line::scaled(3.0).round());
    s.edge(c, &basin);
    s.boxed(c, (8.0, 10.0, 204.0, 5.0), 0.0, h.cv());
    s.boxed(c, (96.0, 0.0, 28.0, 11.0), 2.0, h.cv());
    s.dot(
        c,
        (110.0, 5.5),
        3.0,
        if turning { h.run } else { h.detail },
        None,
    );
}

fn gravity_filter(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let washing = s.run == EquipState::Running;
    s.boxed(c, (0.0, 19.0, 16.0, 14.0), 0.0, h.pv());
    s.boxed(c, (124.0, 99.0, 16.0, 14.0), 0.0, h.pv());
    s.boxed(c, (63.0, 104.0, 14.0, 16.0), 0.0, h.ph());
    c.rect(&s.xf, (14.0, 10.0, 112.0, 96.0), 0.0, h.ch(), None);
    // Everything below sits inside the web's clip box (14, 10, 112, 90).
    let water = if washing {
        (14.0, 14.0, 112.0, 40.0)
    } else {
        (14.0, 22.0, 112.0, 32.0)
    };
    c.faded(0.85, |c| c.rect(&s.xf, water, 0.0, h.lq(), None));
    c.rect(&s.xf, (14.0, 54.0, 112.0, 22.0), 0.0, h.bk(), None);
    c.rect(&s.xf, (14.0, 76.0, 112.0, 14.0), 0.0, h.body_lo, None);
    for i in 0..14u8 {
        let cx = 20.0 + f32::from(i) * 8.0;
        let cy = 60.0 + f32::from(i % 3) * 5.0;
        s.dot(c, (cx, cy), 1.4, h.bulk_lo, None);
    }
    if washing {
        let color = fade(h.white, 0.7);
        for (i, x) in [24.0f32, 46.0, 70.0, 94.0, 116.0].into_iter().enumerate() {
            let delay = 0.15 * i as f32;
            c.faded(motion::blink(s.t - delay), |c| {
                s.segment(c, (x, 50.0), (x, 26.0), color, Line::hairline(1.5));
                let head = Shape::polyline(&[(x - 4.0, 31.0), (x, 25.0), (x + 4.0, 31.0)], false);
                c.stroke(&s.xf, &head, color, Line::hairline(1.5));
            });
        }
    }
    s.segment(
        c,
        (14.0, 90.0),
        (126.0, 90.0),
        h.detail,
        Line::hairline(1.0).dashed(3.0, 3.0, 0.0),
    );
    s.rim(c, (14.0, 10.0, 112.0, 96.0), 0.0);
    let hatch: Paint = if washing { h.liquid_hi.into() } else { h.cv() };
    s.boxed(c, (104.0, 0.0, 22.0, 10.0), 2.0, hatch);
}

fn heat_exchanger(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.boxed(c, (36.0, 2.0, 13.0, 16.0), 0.0, h.ph());
    s.boxed(c, (91.0, 62.0, 13.0, 16.0), 0.0, h.ph());
    s.boxed(c, (0.0, 33.0, 16.0, 14.0), 0.0, h.pv());
    s.boxed(c, (124.0, 33.0, 16.0, 14.0), 0.0, h.pv());
    s.body(
        c,
        &Shape::svg("M38 16 L102 16 A24 24 0 0 1 102 64 L38 64 A24 24 0 0 1 38 16 Z"),
        h.cv(),
    );
    c.stroke(
        &s.xf,
        &Shape::svg("M16 40 L32 40 L42 26 L56 54 L70 26 L84 54 L98 26 L108 40 L124 40"),
        h.detail,
        Line::scaled(2.5),
    );
}

fn filter(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.boxed(c, (0.0, 22.0, 16.0, 13.0), 0.0, h.pv());
    s.boxed(c, (64.0, 85.0, 16.0, 13.0), 0.0, h.pv());
    s.boxed(c, (14.0, 8.0, 52.0, 104.0), 20.0, h.ch());
    s.segment(
        c,
        (16.0, 100.0),
        (64.0, 20.0),
        h.detail,
        Line::hairline(2.0).dashed(6.0, 4.0, 0.0),
    );
}

fn breaker(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let closed = s.state == EquipState::Running;
    let line = if closed { h.run } else { h.stroke };
    s.segment(c, (30.0, 0.0), (30.0, 28.0), line, Line::scaled(3.0));
    s.segment(c, (30.0, 72.0), (30.0, 100.0), line, Line::scaled(3.0));
    let body: Paint = if closed { h.run.into() } else { h.cv() };
    s.boxed(c, (12.0, 28.0, 36.0, 44.0), 3.0, body);
    if closed {
        s.segment(c, (30.0, 34.0), (30.0, 66.0), h.detail, Line::scaled(3.0));
    } else {
        s.segment(
            c,
            (30.0, 66.0),
            (42.0, 38.0),
            h.detail,
            Line::scaled(3.0).round(),
        );
    }
}

fn transformer(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let line = if s.state == EquipState::Running {
        h.run
    } else {
        h.stroke
    };
    s.segment(c, (40.0, 0.0), (40.0, 20.0), line, Line::scaled(3.0));
    s.segment(c, (40.0, 100.0), (40.0, 120.0), line, Line::scaled(3.0));
    for (cy, alpha) in [(44.0, 0.9), (76.0, 0.75)] {
        c.faded(alpha, |c| s.dot(c, (40.0, cy), 24.0, h.sp(), None));
        s.rim(c, disc(40.0, cy, 24.0), 24.0);
    }
    s.rim(c, disc(40.0, 44.0, 24.0), 24.0);
}

fn generator(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.circle(c, (50.0, 50.0), 38.0, h.sp());
    s.text(c, (50.0, 42.0), "G", Text::new(22.0, h.detail).bold());
    c.stroke(
        &s.xf,
        &Shape::svg("M30 64 Q 37.5 52, 45 64 T 60 64 T 75 64"),
        accent(h, s.state),
        Line::scaled(3.0).round(),
    );
}

fn solar_panel(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.segment(c, (60.0, 66.0), (60.0, 92.0), h.body_lo, Line::scaled(5.0));
    s.boxed(c, (44.0, 90.0, 32.0, 7.0), 2.0, h.cv());
    s.body(c, &Shape::svg("M14 16 L106 8 L114 62 L6 70 Z"), h.solar);
    c.stroke(
        &s.xf,
        &Shape::svg("M44.7 13.3 L42 67.3 M75.3 10.7 L78 64.7 M10 43 L110 35"),
        fade(h.solar_grid, 0.6),
        Line::hairline(1.0),
    );
    c.fill(
        &s.xf,
        &Shape::svg("M14 16 L106 8 L108 22 L12 30 Z"),
        fade(h.white, 0.08),
    );
    s.lamp(c, (104.0, 70.0), 3.2);
}

fn battery(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let level = s.level.unwrap_or(0.0);
    let color = match s.level {
        None => h.body_mid,
        Some(level) if level < 0.2 => h.fault,
        Some(level) if level < 0.4 => h.warn,
        Some(_) => h.run,
    };
    s.boxed(c, (106.0, 24.0, 9.0, 22.0), 2.0, h.cv());
    s.boxed(c, (5.0, 9.0, 102.0, 52.0), 7.0, h.cv());
    c.rect(&s.xf, (12.0, 16.0, 88.0, 38.0), 3.0, h.detail, None);
    let width = 88.0 * level;
    if width > 0.0 {
        c.rect(
            &s.xf,
            (12.0, 16.0, width, 38.0),
            3.0f32.min(width / 2.0),
            color,
            None,
        );
    }
    s.level_text(c, (56.0, 35.0));
}

fn inverter(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.boxed(c, (8.0, 8.0, 74.0, 84.0), 6.0, h.cv());
    s.segment(c, (8.0, 92.0), (82.0, 8.0), h.detail, Line::hairline(1.5));
    c.stroke(
        &s.xf,
        &Shape::svg("M20 30 L40 30 M20 38 L40 38"),
        h.detail,
        Line::scaled(2.5),
    );
    c.stroke(
        &s.xf,
        &Shape::svg("M50 68 Q 55 58, 60 68 T 70 68"),
        accent(h, s.state),
        Line::scaled(2.5).round(),
    );
    s.lamp(c, (72.0, 18.0), 3.2);
}

fn meter(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.boxed(c, (4.0, 4.0, 82.0, 82.0), 6.0, h.cv());
    c.rect(
        &s.xf,
        (12.0, 16.0, 66.0, 34.0),
        3.0,
        h.lcd,
        Some((h.detail, 1.0)),
    );
    let reading = if s.value.is_empty() { "----" } else { s.value };
    let size = if s.value.chars().count() > 6 {
        11.0
    } else {
        14.0
    };
    let color = if s.state == EquipState::Stale {
        h.stale
    } else {
        h.lcd_text
    };
    let style = Text::new(size, color).mono().bold();
    // Right-aligned at x 72: centred on its middle, which a mirror or a
    // turn keeps on the display.
    let width = c.text_width(&s.xf, reading, style);
    s.text(c, (72.0 - width / 2.0, 34.0), reading, style);
    if !s.unit.is_empty() {
        s.text(c, (45.0, 64.0), s.unit, Text::new(9.0, h.detail).semibold());
    }
    s.dot(c, (20.0, 76.0), 3.0, h.detail, None);
    s.dot(c, (70.0, 76.0), 3.0, h.detail, None);
}

fn busbar(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let fill: Paint = if s.state == EquipState::Running {
        h.run.into()
    } else {
        h.pv()
    };
    s.boxed(c, (0.0, 2.0, 200.0, 12.0), 2.0, fill);
}

fn instrument(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    let mounting = config_text(s.widget, "mounting", "field");
    if mounting == "dcs" {
        s.rim(c, (8.0, 8.0, 64.0, 64.0), 0.0);
    }
    s.circle(c, (40.0, 40.0), 30.0, h.body_hi);
    if mounting != "field" {
        let (color, line) = outline(h, s.state);
        s.segment(c, (10.0, 40.0), (70.0, 40.0), color, line);
    }
    let function = config_text(s.widget, "tagFunction", "PT");
    s.text(c, (40.0, 29.0), &function, Text::new(14.0, h.detail).bold());
    let number = config_text(s.widget, "loopNumber", "101");
    s.text(c, (40.0, 52.0), &number, Text::new(12.0, h.detail).mono());
}

fn flow_meter(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let h = c.hmi;
    s.boxed(c, (0.0, 39.0, 100.0, 14.0), 0.0, h.pv());
    s.boxed(c, (20.0, 30.0, 7.0, 32.0), 1.5, h.ch());
    s.boxed(c, (73.0, 30.0, 7.0, 32.0), 1.5, h.ch());
    s.boxed(c, (31.0, 32.0, 38.0, 28.0), 12.0, h.cv());
    s.boxed(c, (40.0, 2.0, 20.0, 30.0), 3.0, h.ch());
    c.rect(&s.xf, (43.0, 7.0, 14.0, 10.0), 1.5, h.lcd, None);
    s.text(c, (50.0, 47.0), "FT", Text::new(11.0, h.detail).bold());
}

/// `PipeStroke` along `shape` in `xf`'s units: `thick` scales its width,
/// `along` its dashes (a stretched pipe keeps its look across and along).
fn pipe_stroke(
    c: &mut Canvas<'_>,
    xf: &Xf,
    shape: &Shape,
    thick: f32,
    along: f32,
    flowing: bool,
    t: f32,
) {
    let h = c.hmi;
    c.stroke(xf, shape, h.pipe_lo, Line::scaled(16.0 * thick));
    c.stroke(xf, shape, fade(h.pipe_hi, 0.75), Line::scaled(7.0 * thick));
    if flowing {
        let line = Line::scaled(4.0 * thick).round().dashed(
            8.0 * along,
            16.0 * along,
            motion::flow(t) * along,
        );
        c.stroke(xf, shape, h.liquid_hi, line);
    }
}

/// The straight pipes stretch to their box: drawn in the drawing area's
/// units so their width follows the axis across them, as SVG's
/// `preserveAspectRatio="none"` scales it.
fn straight_pipe(c: &mut Canvas<'_>, s: &Sym<'_>) {
    let (sx, sy) = s.scale;
    let (ox, oy) = s.offset;
    let (from, to, thick, along) = if s.key == "pipe_vertical" {
        let x = ox + 10.0 * sx;
        ((x, oy), (x, oy + 200.0 * sy), sx, sy)
    } else {
        let y = oy + 10.0 * sy;
        ((ox, y), (ox + 200.0 * sx, y), sy, sx)
    };
    let shape = Shape::polyline(&[from, to], false);
    let flowing = s.run == EquipState::Running;
    pipe_stroke(c, &s.area, &shape, thick, along, flowing, s.t);
}

fn pipe_path(c: &mut Canvas<'_>, s: &Sym<'_>, shape: Shape) {
    let flowing = s.run == EquipState::Running;
    pipe_stroke(c, &s.xf, &shape, 1.0, 1.0, flowing, s.t);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The web's `vesselPath`.
    fn vessel_path(top: f32, bottom: f32) -> String {
        format!(
            "M12 {} Q12 {top} 50 {top} Q88 {top} 88 {} L88 {} Q88 {bottom} 50 {bottom} Q12 {bottom} 12 {} Z",
            top + 18.0,
            top + 18.0,
            bottom - 12.0,
            bottom - 12.0,
        )
    }

    #[test]
    fn vessels_are_the_web_outlines() {
        assert_eq!(vessel_path(6.0, 152.0), TANK);
        assert_eq!(vessel_path(24.0, 160.0), MIXER);
    }

    #[test]
    fn percentages_round_like_the_web() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.624), "62%");
        assert_eq!(percent(0.625), "63%");
        assert_eq!(percent(1.0), "100%");
        assert_eq!(percent(1.2), "100%");
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn footer_rows_stack_under_the_drawing() {
        let all = FooterRows {
            label: true,
            value: true,
            status: true,
        };
        // 2 pad + 15 + 2 + 19 + 2 + 12 = 52 under a 100 tall box.
        let [label, badge, status] = footer_centres(all, 100.0);
        assert_eq!(label, Some(48.0 + 2.0 + 7.5));
        assert_eq!(badge, Some(48.0 + 2.0 + 15.0 + 2.0 + 9.5));
        assert_eq!(status, Some(100.0 - 6.0));
        let badge_only = FooterRows {
            label: false,
            value: true,
            status: false,
        };
        assert_eq!(
            footer_centres(badge_only, 50.0),
            [None, Some(50.0 - 9.5), None]
        );
        let none = FooterRows {
            label: false,
            value: false,
            status: false,
        };
        assert_eq!(footer_centres(none, 50.0), [None; 3]);
    }

    #[test]
    fn idle_and_stopped_are_at_rest() {
        assert!(at_rest(EquipState::Idle));
        assert!(at_rest(EquipState::Stopped));
        assert!(!at_rest(EquipState::Running));
        assert!(!at_rest(EquipState::Fault));
    }
}
