//! Drawing for the mimic: a local coordinate system mapped onto the window
//! ([`Xf`]), shapes from SVG path data or primitives ([`Shape`]), fills,
//! strokes (scaling or not, dashed with an offset), clipped level fills,
//! quads and text, in the ISA-101 palette ([`Hmi`]) derived from the theme.
//!
//! Shapes are tessellated with lyon (GPUI's own) after the transform, so
//! curves stay smooth at any zoom. Tessellations of the library's shapes
//! are cached without their translation: panning re-uses them and only
//! moves their vertices; zooming or a state change tessellates again.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use gpui::{
    App, Background, Bounds, ContentMask, Corners, FontWeight, Hsla, PathBuilder, Pixels, Point,
    TextRun, Window, font, linear_color_stop, linear_gradient, point, px, size,
};
use lyon::algorithms::measure::{PathMeasurements, SampleType};
use lyon::math::{Angle, Box2D, Transform, Vector, point as lpoint, vector};
use lyon::path::builder::BorderRadii;
use lyon::path::iterator::PathIterator;
use lyon::path::traits::SvgPathBuilder;
use lyon::path::{ArcFlags, Path, PathEvent, Winding};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, LineCap, LineJoin,
    StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};
use vt_ui::theme::{MONO, SANS, Theme};

use crate::console::anim::mix;

// ── The palette ───────────────────────────────────────────────────────────

/// The HMI palette (the web client's `--hmi-*` variables): grey equipment,
/// colour kept for state and alarms. Derived from the theme so it follows
/// the console's appearance; plus the theme tokens the widgets use.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Hmi {
    pub stage: Hsla,
    pub area: Hsla,
    pub area_edge: Hsla,
    pub body_hi: Hsla,
    pub body_mid: Hsla,
    pub body_lo: Hsla,
    pub stroke: Hsla,
    pub detail: Hsla,
    pub pipe_hi: Hsla,
    pub pipe_lo: Hsla,
    pub run: Hsla,
    pub fault: Hsla,
    pub warn: Hsla,
    pub stale: Hsla,
    pub liquid_hi: Hsla,
    pub liquid_lo: Hsla,
    pub bulk_hi: Hsla,
    pub bulk_lo: Hsla,
    pub lcd: Hsla,
    pub lcd_text: Hsla,
    pub badge: Hsla,
    pub badge_text: Hsla,
    pub badge_unit: Hsla,
    pub solar: Hsla,
    pub solar_grid: Hsla,
    pub power_line: Hsla,
    pub data_line: Hsla,
    // Theme tokens, for the widgets.
    pub bg: Hsla,
    pub surface: Hsla,
    pub fg: Hsla,
    pub fg_muted: Hsla,
    pub fg_dim: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub ok: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub series: [Hsla; 6],
    pub white: Hsla,
    pub black: Hsla,
}

impl Hmi {
    pub fn new(theme: &Theme) -> Self {
        let dark = theme.dark;
        let on = |amount: f32| mix(theme.bg, theme.fg, amount);
        let grey = on(0.5);
        let white = gpui::white();
        let black = gpui::black();
        Self {
            stage: if dark { on(0.045) } else { on(0.11) },
            area: theme.fg.opacity(if dark { 0.025 } else { 0.035 }),
            area_edge: on(if dark { 0.62 } else { 0.7 }).opacity(0.22),
            body_hi: if dark { on(0.56) } else { on(0.015) },
            body_mid: if dark { on(0.35) } else { on(0.15) },
            body_lo: if dark { on(0.16) } else { on(0.34) },
            stroke: if dark { on(0.66) } else { on(0.7) },
            detail: if dark { on(0.07) } else { on(0.55) },
            pipe_hi: if dark { on(0.6) } else { on(0.08) },
            pipe_lo: if dark { on(0.21) } else { on(0.44) },
            run: theme.success,
            fault: theme.danger,
            warn: theme.warning,
            stale: theme.series(3),
            liquid_hi: mix(theme.info, white, 0.25),
            liquid_lo: mix(theme.info, black, 0.45),
            bulk_hi: mix(theme.warning, grey, 0.55),
            bulk_lo: mix(mix(theme.warning, grey, 0.5), black, 0.3),
            lcd: mix(black, theme.success, 0.06),
            lcd_text: mix(theme.success, white, 0.35),
            badge: if dark {
                mix(theme.bg, black, 0.15)
            } else {
                on(0.88)
            },
            badge_text: mix(on(1.0), white, 0.6),
            badge_unit: grey,
            solar: mix(theme.info, black, 0.62),
            solar_grid: mix(theme.info, white, 0.35),
            power_line: mix(theme.warning, grey, 0.35),
            data_line: mix(theme.info, grey, 0.3),
            bg: theme.bg,
            surface: theme.surface,
            fg: theme.fg,
            fg_muted: theme.fg_muted,
            fg_dim: theme.fg_dim,
            border: theme.border,
            accent: theme.accent,
            ok: theme.success,
            warning: theme.warning,
            danger: theme.danger,
            series: theme.series,
            white,
            black,
        }
    }

    /// The SVG gradients of the library (`Defs` in `hmi/symbols.tsx`),
    /// with two stops each. A cylinder lit from above, across y.
    pub fn cv(&self) -> Paint {
        Paint::gradient(180.0, (self.body_hi, 0.28), (self.body_lo, 1.0))
    }

    /// A cylinder across x.
    pub fn ch(&self) -> Paint {
        Paint::gradient(90.0, (self.body_hi, 0.28), (self.body_lo, 1.0))
    }

    /// A sphere lit from the top left.
    pub fn sp(&self) -> Paint {
        Paint::gradient(135.0, (self.body_hi, 0.1), (self.body_lo, 0.95))
    }

    /// A pipe across y (a horizontal pipe's shading).
    pub fn pv(&self) -> Paint {
        Paint::gradient(180.0, (self.pipe_hi, 0.35), (self.pipe_lo, 1.0))
    }

    /// A pipe across x (a vertical pipe's shading).
    pub fn ph(&self) -> Paint {
        Paint::gradient(90.0, (self.pipe_hi, 0.35), (self.pipe_lo, 1.0))
    }

    /// Liquid.
    pub fn lq(&self) -> Paint {
        Paint::gradient(90.0, (self.liquid_hi, 0.3), (self.liquid_lo, 1.0))
    }

    /// Bulk solids.
    pub fn bk(&self) -> Paint {
        Paint::gradient(90.0, (self.bulk_hi, 0.3), (self.bulk_lo, 1.0))
    }
}

// ── Transforms ────────────────────────────────────────────────────────────

/// An affine map from local units to window pixels:
/// `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Xf {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Xf {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn translate(x: f32, y: f32) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    pub fn scale(x: f32, y: f32) -> Self {
        Self {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

    /// A turn of `degrees` (clockwise on screen) about `(cx, cy)`.
    pub fn rotate_about(degrees: f32, cx: f32, cy: f32) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let turn = Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        };
        Self::translate(cx, cy)
            .then(&turn)
            .then(&Self::translate(-cx, -cy))
    }

    /// A horizontal mirror about `x = cx`, scaled by `sx` (agitators).
    pub fn scale_x_about(sx: f32, cx: f32) -> Self {
        Self::translate(cx, 0.0)
            .then(&Self::scale(sx, 1.0))
            .then(&Self::translate(-cx, 0.0))
    }

    /// `self` after `inner`: inner's units are mapped by inner, then self.
    #[must_use]
    pub fn then(&self, inner: &Self) -> Self {
        Self {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    pub fn apply(&self, x: f32, y: f32) -> Point<Pixels> {
        point(
            px(self.a * x + self.c * y + self.e),
            px(self.b * x + self.d * y + self.f),
        )
    }

    /// How many pixels one local unit is (the geometric mean of the axes).
    pub fn unit(&self) -> f32 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }

    /// The window rectangle of a local rectangle; exact for the quarter
    /// turns and mirrors the mimic uses.
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        let p = self.apply(x, y);
        let q = self.apply(x + w, y + h);
        let (x0, x1) = (p.x.min(q.x), p.x.max(q.x));
        let (y0, y1) = (p.y.min(q.y), p.y.max(q.y));
        Bounds {
            origin: point(x0, y0),
            size: size(x1 - x0, y1 - y0),
        }
    }

    /// A CSS gradient angle in local units, on screen.
    fn angle(&self, degrees: f32) -> f32 {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let (dx, dy) = (sin, -cos);
        let x = self.a * dx + self.c * dy;
        let y = self.b * dx + self.d * dy;
        x.atan2(-y).to_degrees().rem_euclid(360.0)
    }

    fn lyon(&self) -> Transform {
        Transform::new(self.a, self.b, self.c, self.d, self.e, self.f)
    }

    /// The transform without its translation, rounded so near-equal ones
    /// share a cache entry.
    fn linear_key(&self) -> [i32; 4] {
        #[allow(clippy::cast_possible_truncation)]
        let q = |v: f32| (v * 4096.0).round() as i32;
        [q(self.a), q(self.b), q(self.c), q(self.d)]
    }
}

// ── Paint ─────────────────────────────────────────────────────────────────

/// A fill: one colour, or a two-stop gradient whose angle is in local
/// units (CSS degrees: 180 runs top to bottom, 90 left to right).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Paint {
    Solid(Hsla),
    Gradient {
        angle: f32,
        from: (Hsla, f32),
        to: (Hsla, f32),
    },
}

impl Paint {
    pub fn gradient(angle: f32, from: (Hsla, f32), to: (Hsla, f32)) -> Self {
        Self::Gradient { angle, from, to }
    }

    fn background(self, xf: &Xf, alpha: f32) -> Background {
        match self {
            Self::Solid(color) => fade(color, alpha).into(),
            Self::Gradient { angle, from, to } => linear_gradient(
                xf.angle(angle),
                linear_color_stop(fade(from.0, alpha), from.1),
                linear_color_stop(fade(to.0, alpha), to.1),
            ),
        }
    }
}

impl From<Hsla> for Paint {
    fn from(color: Hsla) -> Self {
        Self::Solid(color)
    }
}

/// `color` with its opacity multiplied by `alpha`.
pub(crate) fn fade(color: Hsla, alpha: f32) -> Hsla {
    Hsla {
        a: color.a * alpha,
        ..color
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Cap {
    Butt,
    Round,
}

/// A stroke. `width` is in local units when `scaling`, otherwise in stage
/// pixels (SVG's `non-scaling-stroke`: outlines keep their weight however
/// big the symbol is drawn). Dashes are in the same units as the width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Line {
    pub width: f32,
    pub scaling: bool,
    pub cap: Cap,
    pub dash: Option<Dash>,
}

impl Line {
    /// A stroke that scales with the drawing, butt caps, round joins.
    pub fn scaled(width: f32) -> Self {
        Self {
            width,
            scaling: true,
            cap: Cap::Butt,
            dash: None,
        }
    }

    /// A stroke of `width` stage pixels whatever the drawing's size.
    pub fn hairline(width: f32) -> Self {
        Self {
            scaling: false,
            ..Self::scaled(width)
        }
    }

    #[must_use]
    pub fn round(self) -> Self {
        Self {
            cap: Cap::Round,
            ..self
        }
    }

    #[must_use]
    pub fn dashed(self, on: f32, off: f32, offset: f32) -> Self {
        Self {
            dash: Some(Dash { on, off, offset }),
            ..self
        }
    }
}

/// Dashes of `on`, gaps of `off`, the pattern shifted back by `offset`
/// (SVG's `stroke-dashoffset`; animating it moves the dashes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Dash {
    pub on: f32,
    pub off: f32,
    pub offset: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    Start,
    Middle,
    End,
}

/// Text: its size in local units, centred vertically on the anchor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Text {
    pub size: f32,
    pub color: Hsla,
    pub mono: bool,
    pub weight: FontWeight,
    pub align: Align,
    /// Cut with an ellipsis past this width (local units).
    pub max_width: Option<f32>,
    /// Drawn around the glyphs first, to keep them legible over lines.
    pub halo: Option<Hsla>,
}

impl Text {
    pub fn new(size: f32, color: Hsla) -> Self {
        Self {
            size,
            color,
            mono: false,
            weight: FontWeight::NORMAL,
            align: Align::Middle,
            max_width: None,
            halo: None,
        }
    }

    #[must_use]
    pub fn mono(self) -> Self {
        Self { mono: true, ..self }
    }

    #[must_use]
    pub fn bold(self) -> Self {
        Self {
            weight: FontWeight::BOLD,
            ..self
        }
    }

    #[must_use]
    pub fn semibold(self) -> Self {
        Self {
            weight: FontWeight::SEMIBOLD,
            ..self
        }
    }

    #[must_use]
    pub fn align(self, align: Align) -> Self {
        Self { align, ..self }
    }

    #[must_use]
    pub fn max_width(self, width: f32) -> Self {
        Self {
            max_width: Some(width),
            ..self
        }
    }

    #[must_use]
    pub fn halo(self, color: Hsla) -> Self {
        Self {
            halo: Some(color),
            ..self
        }
    }
}

// ── Shapes ────────────────────────────────────────────────────────────────

/// A path in local units. Shapes from [`Shape::svg`] are made once and
/// their tessellations cached; the others are made each time.
#[derive(Clone)]
pub(crate) struct Shape {
    path: Rc<Path>,
    /// Identifies a cached shape.
    key: Option<u64>,
}

/// Shapes made at run time but kept by a key (connection routes); cleared
/// when it grows past this.
const MAX_KEYED: usize = 4096;

thread_local! {
    static SHAPES: RefCell<HashMap<(usize, usize), Shape>> = RefCell::new(HashMap::new());
    static KEYED: RefCell<HashMap<u64, Shape>> = RefCell::new(HashMap::new());
}

impl Shape {
    fn new(path: Path) -> Self {
        Self {
            path: Rc::new(path),
            key: None,
        }
    }

    /// SVG path data (`M`, `L`, `H`, `V`, `C`, `S`, `Q`, `T`, `A`, `Z`, and
    /// their relative forms), parsed once per string.
    pub fn svg(data: &'static str) -> Self {
        let id = (data.as_ptr() as usize, data.len());
        SHAPES.with(|shapes| {
            shapes
                .borrow_mut()
                .entry(id)
                .or_insert_with(|| Self {
                    path: Rc::new(parse_svg(data)),
                    key: Some((id.0 as u64) ^ ((id.1 as u64) << 48)),
                })
                .clone()
        })
    }

    /// SVG path data made at run time (not cached).
    pub fn svg_owned(data: &str) -> Self {
        Self::new(parse_svg(data))
    }

    /// SVG path data made at run time, kept under `key` (a hash of what
    /// it was made from): `make` runs only the first time, and the shape's
    /// tessellations are cached like the library's.
    pub fn cached(key: u64, make: impl FnOnce() -> String) -> Self {
        // Apart from the library's pointer keys.
        let key = key | (1 << 63);
        KEYED.with(|shapes| {
            let mut shapes = shapes.borrow_mut();
            if let Some(shape) = shapes.get(&key) {
                return shape.clone();
            }
            if shapes.len() >= MAX_KEYED {
                shapes.clear();
            }
            let shape = Self {
                path: Rc::new(parse_svg(&make())),
                key: Some(key),
            };
            shapes.insert(key, shape.clone());
            shape
        })
    }

    pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Self {
        let mut builder = Path::builder();
        builder.add_rectangle(
            &Box2D::new(lpoint(x, y), lpoint(x + w, y + h)),
            Winding::Positive,
        );
        Self::new(builder.build())
    }

    pub fn round_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Self {
        let radius = radius.min(w / 2.0).min(h / 2.0).max(0.0);
        let mut builder = Path::builder();
        builder.add_rounded_rectangle(
            &Box2D::new(lpoint(x, y), lpoint(x + w, y + h)),
            &BorderRadii::new(radius),
            Winding::Positive,
        );
        Self::new(builder.build())
    }

    pub fn circle(cx: f32, cy: f32, r: f32) -> Self {
        let mut builder = Path::builder();
        builder.add_circle(lpoint(cx, cy), r.max(0.0), Winding::Positive);
        Self::new(builder.build())
    }

    /// Straight lines through `points`, closed into a polygon or not.
    pub fn polyline(points: &[(f32, f32)], closed: bool) -> Self {
        let mut builder = Path::builder();
        if let Some((first, rest)) = points.split_first() {
            builder.begin(lpoint(first.0, first.1));
            for p in rest {
                builder.line_to(lpoint(p.0, p.1));
            }
            builder.end(closed);
        }
        Self::new(builder.build())
    }

    /// A circular arc from `from` to `to` degrees (0 along +x, clockwise on
    /// screen), the shorter or longer way as the sweep says.
    pub fn arc(cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Self {
        let point_at = |degrees: f32| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            lpoint(cx + r * cos, cy + r * sin)
        };
        let sweep = to - from;
        let mut builder = Path::svg_builder();
        builder.move_to(point_at(from));
        // Split anything past half a turn so the arc is never ambiguous.
        let steps = (sweep.abs() / 170.0).ceil().max(1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        for step in 1..=(steps as u32) {
            #[allow(clippy::cast_precision_loss)]
            let at = from + sweep * step as f32 / steps;
            builder.arc_to(
                vector(r, r),
                Angle::zero(),
                ArcFlags {
                    large_arc: false,
                    sweep: sweep > 0.0,
                },
                point_at(at),
            );
        }
        Self::new(builder.build())
    }

    /// This shape under a further local transform (a blade turned about the
    /// hub).
    #[must_use]
    pub fn transformed(&self, xf: &Xf) -> Self {
        Self::new((*self.path).clone().transformed(&xf.lyon()))
    }

    /// The part of this shape at or below `y` (a vessel's level fill).
    /// Exact for convex outlines, which every vessel is.
    #[must_use]
    pub fn below(&self, y: f32) -> Self {
        let polygons = flatten(&self.path)
            .into_iter()
            .map(|polygon| clip(&polygon, |p| p.1 - y))
            .collect::<Vec<_>>();
        Self::from_polygons(&polygons)
    }

    /// The part of this shape inside the convex `outline`.
    #[must_use]
    pub fn within(&self, outline: &Shape) -> Self {
        let mut polygons = flatten(&self.path);
        for edge_polygon in flatten(&outline.path) {
            let orientation = signed_area(&edge_polygon).signum();
            for (index, a) in edge_polygon.iter().enumerate() {
                let b = edge_polygon[(index + 1) % edge_polygon.len()];
                let inside = |p: (f32, f32)| {
                    orientation * ((b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0))
                };
                polygons = polygons
                    .iter()
                    .map(|polygon| clip(polygon, inside))
                    .collect();
            }
        }
        Self::from_polygons(&polygons)
    }

    fn from_polygons(polygons: &[Vec<(f32, f32)>]) -> Self {
        let mut builder = Path::builder();
        for polygon in polygons.iter().filter(|polygon| polygon.len() >= 3) {
            builder.begin(lpoint(polygon[0].0, polygon[0].1));
            for p in &polygon[1..] {
                builder.line_to(lpoint(p.0, p.1));
            }
            builder.end(true);
        }
        Self::new(builder.build())
    }
}

/// The path as closed polygons (one per sub-path).
fn flatten(path: &Path) -> Vec<Vec<(f32, f32)>> {
    let mut polygons = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    for event in path.iter().flattened(0.05) {
        match event {
            PathEvent::Begin { at } => {
                current = vec![(at.x, at.y)];
            }
            PathEvent::Line { to, .. } => current.push((to.x, to.y)),
            PathEvent::End { .. } => {
                if current.len() >= 3 {
                    polygons.push(std::mem::take(&mut current));
                }
            }
            PathEvent::Quadratic { to, .. } | PathEvent::Cubic { to, .. } => {
                current.push((to.x, to.y));
            }
        }
    }
    polygons
}

fn signed_area(polygon: &[(f32, f32)]) -> f32 {
    let mut area = 0.0;
    for (index, a) in polygon.iter().enumerate() {
        let b = polygon[(index + 1) % polygon.len()];
        area += a.0 * b.1 - b.0 * a.1;
    }
    area / 2.0
}

/// Sutherland–Hodgman against one half-plane: keeps points where
/// `inside(p) >= 0`.
fn clip(polygon: &[(f32, f32)], inside: impl Fn((f32, f32)) -> f32) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(polygon.len() + 2);
    for (index, &a) in polygon.iter().enumerate() {
        let b = polygon[(index + 1) % polygon.len()];
        let (da, db) = (inside(a), inside(b));
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    out
}

/// SVG path data into a lyon path. Stops at the first thing it cannot
/// read, keeping what came before.
fn parse_svg(data: &str) -> Path {
    let mut scan = Scan {
        bytes: data.as_bytes(),
        at: 0,
    };
    let mut builder = Path::svg_builder();
    let mut command = b'M';
    loop {
        if let Some(next) = scan.command() {
            command = next;
            if command.eq_ignore_ascii_case(&b'z') {
                builder.close();
                continue;
            }
        } else if !scan.at_number() || command.eq_ignore_ascii_case(&b'z') {
            break;
        }
        let read = match command {
            b'M' => scan.point().map(|p| {
                builder.move_to(p);
                command = b'L';
            }),
            b'm' => scan.vector().map(|v| {
                builder.relative_move_to(v);
                command = b'l';
            }),
            b'L' => scan.point().map(|p| {
                builder.line_to(p);
            }),
            b'l' => scan.vector().map(|v| {
                builder.relative_line_to(v);
            }),
            b'H' => scan.number().map(|x| builder.horizontal_line_to(x)),
            b'h' => scan
                .number()
                .map(|x| builder.relative_horizontal_line_to(x)),
            b'V' => scan.number().map(|y| builder.vertical_line_to(y)),
            b'v' => scan.number().map(|y| builder.relative_vertical_line_to(y)),
            b'C' => scan.points3().map(|(a, b, to)| {
                builder.cubic_bezier_to(a, b, to);
            }),
            b'c' => scan.vectors3().map(|(a, b, to)| {
                builder.relative_cubic_bezier_to(a, b, to);
            }),
            b'S' => scan.points2().map(|(b, to)| {
                builder.smooth_cubic_bezier_to(b, to);
            }),
            b's' => scan.vectors2().map(|(b, to)| {
                builder.smooth_relative_cubic_bezier_to(b, to);
            }),
            b'Q' => scan.points2().map(|(c, to)| {
                builder.quadratic_bezier_to(c, to);
            }),
            b'q' => scan.vectors2().map(|(c, to)| {
                builder.relative_quadratic_bezier_to(c, to);
            }),
            b'T' => scan.point().map(|to| {
                builder.smooth_quadratic_bezier_to(to);
            }),
            b't' => scan.vector().map(|to| {
                builder.smooth_relative_quadratic_bezier_to(to);
            }),
            b'A' | b'a' => scan.arc().map(|(radii, rotation, flags, x, y)| {
                if command == b'A' {
                    builder.arc_to(radii, rotation, flags, lpoint(x, y));
                } else {
                    builder.relative_arc_to(radii, rotation, flags, vector(x, y));
                }
            }),
            _ => None,
        };
        if read.is_none() {
            break;
        }
    }
    builder.build()
}

struct Scan<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Scan<'_> {
    fn skip(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            self.at += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let byte = *self.bytes.get(self.at)?;
        (byte.is_ascii_alphabetic() && byte != b'e' && byte != b'E').then(|| {
            self.at += 1;
            byte
        })
    }

    fn at_number(&mut self) -> bool {
        self.skip();
        self.bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.'))
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.at;
        let mut end = start;
        let bytes = self.bytes;
        if matches!(bytes.get(end), Some(b'-' | b'+')) {
            end += 1;
        }
        let mut dot = false;
        while let Some(&b) = bytes.get(end) {
            if b.is_ascii_digit() {
                end += 1;
            } else if b == b'.' && !dot {
                dot = true;
                end += 1;
            } else {
                break;
            }
        }
        if matches!(bytes.get(end), Some(b'e' | b'E')) {
            let mut exponent = end + 1;
            if matches!(bytes.get(exponent), Some(b'-' | b'+')) {
                exponent += 1;
            }
            if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
                end = exponent;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
            }
        }
        let value = std::str::from_utf8(&bytes[start..end]).ok()?.parse().ok()?;
        self.at = end;
        Some(value)
    }

    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let byte = *self.bytes.get(self.at)?;
        let flag = match byte {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.at += 1;
        Some(flag)
    }

    fn point(&mut self) -> Option<lyon::math::Point> {
        Some(lpoint(self.number()?, self.number()?))
    }

    fn vector(&mut self) -> Option<Vector> {
        Some(vector(self.number()?, self.number()?))
    }

    fn points2(&mut self) -> Option<(lyon::math::Point, lyon::math::Point)> {
        Some((self.point()?, self.point()?))
    }

    fn points3(&mut self) -> Option<(lyon::math::Point, lyon::math::Point, lyon::math::Point)> {
        Some((self.point()?, self.point()?, self.point()?))
    }

    fn vectors2(&mut self) -> Option<(Vector, Vector)> {
        Some((self.vector()?, self.vector()?))
    }

    fn vectors3(&mut self) -> Option<(Vector, Vector, Vector)> {
        Some((self.vector()?, self.vector()?, self.vector()?))
    }

    fn arc(&mut self) -> Option<(Vector, Angle, ArcFlags, f32, f32)> {
        let radii = self.vector()?;
        let rotation = Angle::degrees(self.number()?);
        let large_arc = self.flag()?;
        let sweep = self.flag()?;
        let x = self.number()?;
        let y = self.number()?;
        Some((radii, rotation, ArcFlags { large_arc, sweep }, x, y))
    }
}

// ── The canvas ────────────────────────────────────────────────────────────

/// A tessellated shape, in window pixels less the translation.
type Buffers = Rc<VertexBuffers<lyon::math::Point, u16>>;

/// Tessellations kept between frames, by shape, transform (without its
/// translation) and style, with the frame each was last used in. Entries
/// unused for a while are dropped.
#[derive(Default)]
pub(crate) struct Tessellations {
    entries: RefCell<HashMap<u64, (u64, Buffers)>>,
    frame: Cell<u64>,
}

/// Frames an unused tessellation is kept.
const KEEP_FRAMES: u64 = 240;
/// More entries than this and the cache starts afresh.
const MAX_ENTRIES: usize = 8192;

impl Tessellations {
    /// A new frame: forget what was not drawn lately.
    pub fn next_frame(&self) {
        let frame = self.frame.get() + 1;
        self.frame.set(frame);
        if frame.is_multiple_of(60) {
            self.entries
                .borrow_mut()
                .retain(|_, (used, _)| frame.saturating_sub(*used) < KEEP_FRAMES);
        }
    }

    fn get_or(
        &self,
        key: u64,
        make: impl FnOnce() -> Option<VertexBuffers<lyon::math::Point, u16>>,
    ) -> Option<Rc<VertexBuffers<lyon::math::Point, u16>>> {
        let frame = self.frame.get();
        if let Some((used, buffers)) = self.entries.borrow_mut().get_mut(&key) {
            *used = frame;
            return Some(buffers.clone());
        }
        let buffers = Rc::new(make()?);
        let mut entries = self.entries.borrow_mut();
        if entries.len() >= MAX_ENTRIES {
            entries.clear();
        }
        entries.insert(key, (frame, buffers.clone()));
        Some(buffers)
    }
}

/// What a draw call paints with: the window, the zoom (stage pixels to
/// window pixels), an opacity for everything (blinking), the palette and
/// the tessellation cache.
pub(crate) struct Canvas<'a> {
    pub window: &'a mut Window,
    pub cx: &'a mut App,
    pub hmi: &'a Hmi,
    pub zoom: f32,
    pub alpha: f32,
    pub cache: &'a Tessellations,
}

impl Canvas<'_> {
    /// Fill a shape.
    pub fn fill(&mut self, xf: &Xf, shape: &Shape, paint: impl Into<Paint>) {
        let paint = paint.into();
        let key = shape.key.map(|id| {
            let mut hasher = DefaultHasher::new();
            (0u8, id, xf.linear_key()).hash(&mut hasher);
            hasher.finish()
        });
        let make = |transform: &Transform| {
            let path = (*shape.path).clone().transformed(transform);
            let mut buffers = VertexBuffers::new();
            FillTessellator::new()
                .tessellate_path(
                    &path,
                    &FillOptions::tolerance(0.15).with_fill_rule(FillRule::NonZero),
                    &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| vertex.position()),
                )
                .ok()?;
            Some(buffers)
        };
        self.paint_buffers(xf, key, make, paint);
    }

    /// Stroke a shape's outline.
    pub fn stroke(&mut self, xf: &Xf, shape: &Shape, color: Hsla, line: Line) {
        let width = if line.scaling {
            line.width * xf.unit()
        } else {
            line.width * self.zoom
        };
        if width <= 0.01 {
            return;
        }
        // Dashes scale like the stroke: local units, or stage pixels.
        let unit = if line.scaling { xf.unit() } else { self.zoom };
        let key = shape.key.filter(|_| line.dash.is_none()).map(|id| {
            let mut hasher = DefaultHasher::new();
            (1u8, id, xf.linear_key(), width.to_bits(), line.cap).hash(&mut hasher);
            hasher.finish()
        });
        let make = |transform: &Transform| {
            let path = (*shape.path).clone().transformed(transform);
            let path = match line.dash {
                Some(dash) => dashed(&path, dash.on * unit, dash.off * unit, dash.offset * unit),
                None => path,
            };
            let cap = match line.cap {
                Cap::Butt => LineCap::Butt,
                Cap::Round => LineCap::Round,
            };
            let options = StrokeOptions::tolerance(0.1)
                .with_line_width(width)
                .with_line_cap(cap)
                .with_line_join(LineJoin::Round);
            let mut buffers = VertexBuffers::new();
            StrokeTessellator::new()
                .tessellate_path(
                    &path,
                    &options,
                    &mut BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex| {
                        vertex.position()
                    }),
                )
                .ok()?;
            Some(buffers)
        };
        self.paint_buffers(xf, key, make, Paint::Solid(color));
    }

    /// Tessellate (or take from the cache, translated) and paint.
    fn paint_buffers(
        &mut self,
        xf: &Xf,
        key: Option<u64>,
        make: impl Fn(&Transform) -> Option<VertexBuffers<lyon::math::Point, u16>>,
        paint: Paint,
    ) {
        let (buffers, offset) = match key {
            Some(key) => {
                let linear = Xf {
                    e: 0.0,
                    f: 0.0,
                    ..*xf
                };
                let Some(buffers) = self.cache.get_or(key, || make(&linear.lyon())) else {
                    return;
                };
                (buffers, (xf.e, xf.f))
            }
            None => match make(&xf.lyon()) {
                Some(buffers) => (Rc::new(buffers), (0.0, 0.0)),
                None => return,
            },
        };
        if buffers.indices.is_empty() {
            return;
        }
        let moved = VertexBuffers {
            vertices: buffers
                .vertices
                .iter()
                .map(|v| lpoint(v.x + offset.0, v.y + offset.1))
                .collect(),
            indices: buffers.indices.clone(),
        };
        let path = PathBuilder::build_path(moved);
        self.window
            .paint_path(path, paint.background(xf, self.alpha));
    }

    /// A rectangle as a quad (fast; for axis-aligned boxes), with rounded
    /// corners (local units) and an optional border (stage pixels).
    #[allow(clippy::too_many_arguments)]
    pub fn rect(
        &mut self,
        xf: &Xf,
        (x, y, w, h): (f32, f32, f32, f32),
        radius: f32,
        paint: impl Into<Paint>,
        border: Option<(Hsla, f32)>,
    ) {
        let bounds = xf.rect(x, y, w, h);
        let mut quad = gpui::fill(bounds, paint.into().background(xf, self.alpha))
            .corner_radii(Corners::all(px(radius * xf.unit())));
        if let Some((color, width)) = border {
            quad = quad
                .border_widths(px(width * self.zoom))
                .border_color(fade(color, self.alpha));
        }
        self.window.paint_quad(quad);
    }

    /// Draw inside a local rectangle only (quarter turns keep it a
    /// rectangle on screen).
    pub fn clip<R>(
        &mut self,
        xf: &Xf,
        (x, y, w, h): (f32, f32, f32, f32),
        draw: impl FnOnce(&mut Canvas<'_>) -> R,
    ) -> R {
        let bounds = xf.rect(x, y, w, h);
        let Canvas {
            window,
            cx,
            hmi,
            zoom,
            alpha,
            cache,
        } = self;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            draw(&mut Canvas {
                window,
                cx,
                hmi,
                zoom: *zoom,
                alpha: *alpha,
                cache,
            })
        })
    }

    /// Draw with everything faded by `alpha` (a blink's dim phase).
    pub fn faded<R>(&mut self, alpha: f32, draw: impl FnOnce(&mut Self) -> R) -> R {
        let before = self.alpha;
        self.alpha *= alpha;
        let result = draw(self);
        self.alpha = before;
        result
    }

    /// Text centred vertically on `(x, y)` in local units.
    pub fn text(&mut self, xf: &Xf, (x, y): (f32, f32), text: &str, style: Text) {
        let size = style.size * xf.unit();
        if size < 3.0 || text.is_empty() {
            return;
        }
        let mut font = font(if style.mono { MONO } else { SANS });
        font.weight = style.weight;
        let shape = |window: &Window, text: String, color: Hsla| {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text.into(), px(size), &[run], None)
        };
        let color = fade(style.color, self.alpha);
        let mut line = shape(self.window, text.to_owned(), color);
        if let Some(max) = style.max_width.map(|max| max * xf.unit())
            && f32::from(line.width) > max
        {
            let chars: Vec<char> = text.chars().collect();
            let (mut low, mut high) = (0usize, chars.len());
            while low < high {
                let mid = (low + high).div_ceil(2);
                let candidate: String = chars[..mid].iter().collect::<String>() + "…";
                if f32::from(shape(self.window, candidate, color).width) <= max {
                    low = mid;
                } else {
                    high = mid - 1;
                }
            }
            let cut: String = chars[..low].iter().collect::<String>() + "…";
            line = shape(self.window, cut, color);
        }
        let anchor = xf.apply(x, y);
        let width = line.width;
        let left = match style.align {
            Align::Start => anchor.x,
            Align::Middle => anchor.x - width / 2.0,
            Align::End => anchor.x - width,
        };
        let line_height = px(size * 1.25);
        let origin = point(left, anchor.y - line_height / 2.0);
        if let Some(halo) = style.halo {
            let halo_line = shape(self.window, line.text.to_string(), fade(halo, self.alpha));
            let reach = (size / 9.0).clamp(0.6, 2.0);
            for (dx, dy) in [(-reach, 0.0), (reach, 0.0), (0.0, -reach), (0.0, reach)] {
                let at = point(origin.x + px(dx), origin.y + px(dy));
                let _ = halo_line.paint(at, line_height, self.window, self.cx);
            }
        }
        let _ = line.paint(origin, line_height, self.window, self.cx);
    }

    /// How wide `text` is drawn, in local units.
    pub fn text_width(&mut self, xf: &Xf, text: &str, style: Text) -> f32 {
        let unit = xf.unit();
        let size = style.size * unit;
        if size <= 0.0 || unit <= 0.0 {
            return 0.0;
        }
        let mut font = font(if style.mono { MONO } else { SANS });
        font.weight = style.weight;
        let run = TextRun {
            len: text.len(),
            font,
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line =
            self.window
                .text_system()
                .shape_line(text.to_owned().into(), px(size), &[run], None);
        f32::from(line.width) / unit
    }
}

/// The dashes of `path` (window pixels), the pattern shifted back by
/// `offset`.
fn dashed(path: &Path, on: f32, off: f32, offset: f32) -> Path {
    let period = on + off;
    if on <= 0.0 || period <= 0.0 {
        return path.clone();
    }
    let measurements = PathMeasurements::from_path(path, 0.05);
    let mut sampler = measurements.create_sampler(path, SampleType::Distance);
    let total = sampler.length();
    let mut builder = Path::builder();
    let mut start = -offset.rem_euclid(period);
    while start < total {
        let (from, to) = (start.max(0.0), (start + on).min(total));
        if to > from {
            sampler.split_range(from..to, &mut builder);
        }
        start += period;
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(shape: &Shape) -> Vec<(f32, f32)> {
        flatten(&shape.path).into_iter().flatten().collect()
    }

    #[test]
    fn svg_data_reads_every_command() {
        let shape = Shape::svg_owned(
            "M12 30 Q12 6 50 6 L88 30 H90 V40 C90 50, 80 60, 70 60 S50 70 40 60 T20 40 A10 10 0 0 1 12 30 Z",
        );
        let pts = points(&shape);
        assert!(pts.len() > 10, "curves flatten to many points");
        assert!(pts.iter().all(|p| p.0 >= 0.0 && p.0 <= 100.0));
        let relative = Shape::svg_owned("m10 10 l10 0 h5 v5 z");
        let pts = points(&relative);
        assert_eq!(pts[0], (10.0, 10.0));
        assert!(pts.contains(&(25.0, 15.0)));
        // Numbers packed the way minifiers write them.
        let packed = Shape::svg_owned("M0-5L10.5.5");
        assert!(points(&packed).is_empty(), "an open line is no polygon");
    }

    #[test]
    fn a_level_fill_is_the_vessel_below_the_surface() {
        let vessel = Shape::rect(0.0, 0.0, 10.0, 100.0);
        let fill = vessel.below(60.0);
        let pts = points(&fill);
        assert!(pts.iter().all(|p| p.1 >= 60.0 - 1e-3));
        let area = signed_area(&pts).abs();
        assert!((area - 400.0).abs() < 1e-2, "{area}");
        let triangle = Shape::polyline(&[(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)], true);
        let inside = Shape::rect(-5.0, -5.0, 20.0, 20.0).within(&triangle);
        assert!((signed_area(&points(&inside)).abs() - 50.0).abs() < 1e-2);
    }

    #[test]
    fn transforms_compose_and_turn_gradients() {
        let xf = Xf::translate(10.0, 20.0).then(&Xf::scale(2.0, 2.0));
        assert_eq!(xf.apply(1.0, 1.0), point(px(12.0), px(22.0)));
        assert!((xf.unit() - 2.0).abs() < 1e-6);
        let turned = Xf::rotate_about(90.0, 0.0, 0.0);
        let p = turned.apply(1.0, 0.0);
        assert!((f32::from(p.x)).abs() < 1e-5 && (f32::from(p.y) - 1.0).abs() < 1e-5);
        // Top-to-bottom turned a quarter clockwise runs right to left.
        assert!((turned.angle(180.0) - 270.0).abs() < 1e-3);
        let mirrored = Xf::scale(-1.0, 1.0);
        assert!((mirrored.angle(90.0) - 270.0).abs() < 1e-3);
        let r = Xf::scale(-1.0, 1.0).rect(0.0, 0.0, 10.0, 5.0);
        assert_eq!(r.origin.x, px(-10.0));
    }

    #[test]
    fn dashes_follow_the_offset() {
        let line = Shape::polyline(&[(0.0, 0.0), (100.0, 0.0)], false);
        let still = dashed(&line.path, 8.0, 16.0, 0.0);
        let moved = dashed(&line.path, 8.0, 16.0, 12.0);
        let first = |path: &Path| {
            path.iter()
                .find_map(|event| match event {
                    PathEvent::Begin { at } => Some(at.x),
                    _ => None,
                })
                .unwrap()
        };
        assert!(first(&still).abs() < 1e-3);
        assert!((first(&moved) - 12.0).abs() < 1e-2, "{}", first(&moved));
    }
}
