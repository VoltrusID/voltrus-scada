//! What the editor draws over the stage: the grid under the widgets, and
//! over them the selection with its handles and knob, the pointer's
//! widget, the selected connection, the connect tool's source and route,
//! guide lines, the rubber band and a dragged palette item's landing box.
//! The marks keep their window-pixel sizes at every zoom.

use std::cell::Cell;
use std::fmt::Write as _;
use std::rc::Rc;

use gpui::{App, Hsla, Window, transparent_black};
use voltrus_scada_native::mimic::edit::{self, DOTS, Guide, Handle, Rect};
use voltrus_scada_native::mimic::{self, STAGE_H, STAGE_W};

use super::super::Drawn;
use super::super::paint::{Canvas, Hmi, Line, Shape, Tessellations, Xf, fade};
use super::{Editor, KNOB_GAP, Tool};

/// The grid's dots: one shape, tessellated once per zoom.
const GRID_KEY: u64 = 0x6d69_6d69_6367_7264;
/// Below this zoom the dots would crowd: none are drawn.
const GRID_MIN_ZOOM: f32 = 0.25;
/// A handle's side, window pixels.
const HANDLE: f32 = 8.0;

/// The grid, a dot every [`DOTS`] stage pixels, under the widgets.
pub(crate) fn paint_grid(canvas: &mut Canvas<'_>, stage: &Xf) {
    if canvas.zoom < GRID_MIN_ZOOM {
        return;
    }
    let dots = Shape::cached(GRID_KEY, || {
        let mut data = String::new();
        let mut y = DOTS;
        while y < STAGE_H {
            let mut x = DOTS;
            while x < STAGE_W {
                let _ = write!(data, "M{} {}h2v2h-2Z", x - 1.0, y - 1.0);
                x += DOTS;
            }
            y += DOTS;
        }
        data
    });
    let hmi = *canvas.hmi;
    canvas.fill(stage, &dots, fade(hmi.stroke, 0.28));
}

/// The marks of one frame, in stage units.
pub(super) struct Marks {
    hmi: Hmi,
    cache: Rc<Tessellations>,
    drawn: Rc<Cell<Option<Drawn>>>,
    selected: Vec<Rect>,
    /// The single selection: its box, and whether it turns (the knob).
    single: Option<(Rect, bool)>,
    hover: Option<Rect>,
    line: Option<Vec<(f64, f64)>>,
    source: Option<Rect>,
    pending: Option<Vec<(f64, f64)>>,
    guides: Vec<Guide>,
    band: Option<Rect>,
    ghost: Option<Rect>,
}

pub(super) fn marks(
    editor: &Editor,
    hmi: Hmi,
    cache: Rc<Tessellations>,
    drawn: Rc<Cell<Option<Drawn>>>,
) -> Marks {
    let layout = editor.layout();
    let connecting = editor.tool == Tool::Connect;
    let source_widget = editor
        .from
        .as_ref()
        .filter(|_| connecting)
        .and_then(|id| layout.widget(id));
    Marks {
        hmi,
        cache,
        drawn,
        selected: edit::picked(layout, &editor.selection)
            .map(Rect::of)
            .collect(),
        single: editor
            .single()
            .filter(|_| !connecting)
            .map(|widget| (Rect::of(widget), edit::turns(widget))),
        hover: editor
            .hover
            .as_ref()
            .filter(|id| !editor.selection.contains(id))
            .and_then(|id| layout.widget(id))
            .map(Rect::of),
        line: editor
            .line
            .as_ref()
            .and_then(|id| layout.connections.iter().find(|line| line.id == *id))
            .and_then(|line| {
                Some(mimic::route(
                    layout.widget(&line.from)?,
                    layout.widget(&line.to)?,
                    &line.waypoints,
                ))
            }),
        source: source_widget.map(Rect::of),
        pending: source_widget
            .zip(editor.pointer)
            .map(|(widget, (x, y))| edit::pending_route(widget, x, y)),
        guides: editor.guides.clone(),
        band: editor.band,
        ghost: editor.ghost.map(|(placing, x, y)| {
            let (width, height) = placing.size();
            Rect {
                x,
                y,
                width,
                height,
            }
        }),
    }
}

/// Paint the marks over the frame just painted.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn paint(marks: &Marks, window: &mut Window, cx: &mut App) {
    let Some(drawn) = marks.drawn.get() else {
        return;
    };
    let camera = drawn.camera;
    let (ox, oy) = (
        f32::from(drawn.bounds.origin.x) + camera.x as f32,
        f32::from(drawn.bounds.origin.y) + camera.y as f32,
    );
    let zoom = camera.zoom as f32;
    let at = |x: f64, y: f64| (ox + x as f32 * zoom, oy + y as f32 * zoom);
    let rect = |r: &Rect| {
        let (x, y) = at(r.x, r.y);
        (x, y, r.width as f32 * zoom, r.height as f32 * zoom)
    };
    let route = |points: &[(f64, f64)]| {
        let points: Vec<(f32, f32)> = points.iter().map(|(x, y)| at(*x, *y)).collect();
        Shape::polyline(&points, false)
    };
    let hmi = marks.hmi;
    let accent = hmi.accent;
    let flat = Xf::translate(0.0, 0.0);
    let mut canvas = Canvas {
        window,
        cx,
        hmi: &hmi,
        zoom: 1.0,
        alpha: 1.0,
        cache: &marks.cache,
    };
    if let Some(points) = &marks.line {
        let shape = route(points);
        canvas.stroke(
            &flat,
            &shape,
            fade(accent, 0.3),
            Line::hairline(8.0).round(),
        );
        canvas.stroke(&flat, &shape, accent, Line::hairline(1.5));
    }
    if let Some(hover) = &marks.hover {
        dashed(&mut canvas, rect(hover), fade(accent, 0.6));
    }
    for selected in &marks.selected {
        outline(&mut canvas, rect(selected), accent, 1.5);
    }
    if let Some(source) = &marks.source {
        outline(&mut canvas, rect(source), accent, 2.0);
    }
    if let Some(points) = &marks.pending {
        canvas.stroke(
            &flat,
            &route(points),
            accent,
            Line::hairline(1.5).dashed(6.0, 4.0, 0.0),
        );
    }
    if let Some((single, turns)) = marks.single {
        for handle in Handle::ALL {
            let (x, y) = handle.point(single);
            let (x, y) = at(x, y);
            canvas.rect(
                &flat,
                (x - HANDLE / 2.0, y - HANDLE / 2.0, HANDLE, HANDLE),
                2.0,
                hmi.surface,
                Some((accent, 1.5)),
            );
        }
        if turns {
            let (x, y) = at(single.center_x(), single.y);
            let gap = KNOB_GAP as f32;
            canvas.rect(
                &flat,
                (x - 0.5, y - gap, 1.0, gap - HANDLE / 2.0),
                0.0,
                accent,
                None,
            );
            canvas.rect(
                &flat,
                (x - 5.0, y - gap - 5.0, 10.0, 10.0),
                5.0,
                hmi.surface,
                Some((accent, 1.5)),
            );
        }
    }
    for guide in &marks.guides {
        let (a, b) = if guide.vertical {
            (at(guide.at, guide.from), at(guide.at, guide.to))
        } else {
            (at(guide.from, guide.at), at(guide.to, guide.at))
        };
        canvas.stroke(
            &flat,
            &Shape::polyline(&[a, b], false),
            hmi.danger,
            Line::hairline(1.0),
        );
    }
    if let Some(band) = &marks.band {
        canvas.rect(
            &flat,
            rect(band),
            0.0,
            fade(accent, 0.08),
            Some((accent, 1.0)),
        );
    }
    if let Some(ghost) = &marks.ghost {
        let r = rect(ghost);
        canvas.rect(&flat, r, 0.0, fade(accent, 0.08), None);
        dashed(&mut canvas, r, accent);
    }
}

/// A box's outline, window pixels.
fn outline(canvas: &mut Canvas<'_>, r: (f32, f32, f32, f32), color: Hsla, width: f32) {
    canvas.rect(
        &Xf::translate(0.0, 0.0),
        r,
        0.0,
        transparent_black(),
        Some((color, width)),
    );
}

/// A box's dashed outline, window pixels.
fn dashed(canvas: &mut Canvas<'_>, r: (f32, f32, f32, f32), color: Hsla) {
    canvas.stroke(
        &Xf::translate(0.0, 0.0),
        &Shape::rect(r.0, r.1, r.2, r.3),
        color,
        Line::hairline(1.0).dashed(4.0, 3.0, 0.0),
    );
}
