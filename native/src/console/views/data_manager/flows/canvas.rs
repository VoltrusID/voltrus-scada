//! The flow canvas: one GPUI `canvas()` that paints what the editor laid
//! out (a [`Scene`]: only the steps and wires in view) as quads, Bézier
//! paths and shaped text, and turns the pointer into editor calls: press,
//! drag, release and the scroll wheel (pan; ⌘ or Ctrl zooms about the
//! pointer). A drag listens to the whole window until the button is up.
//! Pointer work redraws only the page's view; nothing redraws on a timer.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Corners, CursorStyle, DispatchPhase, EntityId,
    FocusHandle, Font, FontWeight, HitboxBehavior, Hsla, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, ScrollWheelEvent, SharedString,
    Styled, TextRun, WeakEntity, Window, canvas, fill, font, point, px, quad, size,
    transparent_black,
};
use vt_ui::theme::{FONT_BODY, FONT_CAPTION, MONO};

use voltrus_scada_native::api::eng_flows::End;

use super::editor;
use super::model::{self, Bezier, NODE_W, PORT_R, Vec2, View};
use crate::console::Console;

/// How a run's trace marks a card or a wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Mark {
    /// No run shown.
    #[default]
    Plain,
    /// The run went through it.
    Taken,
    /// The run failed here.
    Failed,
    /// A run is shown and did not come here.
    Dim,
}

/// One node's card as drawn.
pub(crate) struct Drawn {
    /// Top-left corner, world units.
    pub at: Vec2,
    pub height: f32,
    pub color: Hsla,
    pub kicker: SharedString,
    pub title: SharedString,
    pub preview: SharedString,
    pub selected: bool,
    /// A wire being dragged would land here.
    pub target: bool,
    pub input: bool,
    /// The output ports' labels (drawn when they say something).
    pub outputs: Vec<SharedString>,
    pub mark: Mark,
    /// A required setting is unset.
    pub invalid: bool,
}

/// One wire as drawn.
pub(crate) struct DrawnWire {
    pub curve: Bezier,
    pub selected: bool,
    pub mark: Mark,
}

pub(crate) struct Colors {
    pub bg: Hsla,
    pub grid: Hsla,
    pub surface: Hsla,
    pub border: Hsla,
    pub fg: Hsla,
    pub fg_dim: Hsla,
    pub wire: Hsla,
    pub accent: Hsla,
    pub success: Hsla,
    pub danger: Hsla,
}

/// What one frame of the canvas draws.
pub(crate) struct Scene {
    pub view: View,
    pub cards: Vec<Drawn>,
    pub wires: Vec<DrawnWire>,
    /// A wire being dragged: from a port to the pointer, world units.
    pub pending: Option<(Vec2, Vec2)>,
    /// While the pointer drags: the cursor to show.
    pub dragging: Option<CursorStyle>,
    pub colors: Colors,
}

/// The canvas element. `bounds` learns where it was laid out (for the
/// keys and for drops); `view` is the page's view, redrawn on pointer
/// work; `focus` is the canvas's focus, taken on a press.
pub(crate) fn element(
    scene: Scene,
    console: WeakEntity<Console>,
    view: EntityId,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    focus: FocusHandle,
) -> impl IntoElement {
    canvas(
        move |laid_out, window, _| {
            bounds.set(Some(laid_out));
            window.insert_hitbox(laid_out, HitboxBehavior::Normal)
        },
        move |laid_out, hitbox, window, cx| {
            window.with_content_mask(Some(ContentMask { bounds: laid_out }), |window| {
                paint(&scene, laid_out, window, cx);
            });
            if let Some(cursor) = scene.dragging {
                window.set_cursor_style(cursor, &hitbox);
            }
            let origin = laid_out.origin;
            let local = move |at: Point<Pixels>| {
                Vec2::new(f32::from(at.x - origin.x), f32::from(at.y - origin.y))
            };

            let pressed = console.clone();
            let over = hitbox.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !over.is_hovered(window)
                {
                    return;
                }
                window.focus(&focus);
                let at = local(event.position);
                let clicks = event.click_count;
                let changed = pressed
                    .update(cx, |console, _| editor::press(console, at, clicks))
                    .unwrap_or(false);
                if changed {
                    cx.notify(view);
                }
            });

            let scrolled = console.clone();
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                    return;
                }
                let delta = event.delta.pixel_delta(px(20.0));
                let delta = Vec2::new(f32::from(delta.x), f32::from(delta.y));
                let zoom = event.modifiers.secondary() || event.modifiers.control;
                let at = local(event.position);
                let _ = scrolled.update(cx, |console, _| editor::scroll(console, at, delta, zoom));
                cx.notify(view);
                cx.stop_propagation();
            });

            if scene.dragging.is_some() {
                let moved = console.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let at = local(event.position);
                    let changed = moved
                        .update(cx, |console, _| editor::drag_to(console, at))
                        .unwrap_or(false);
                    if changed {
                        cx.notify(view);
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                        return;
                    }
                    let at = local(event.position);
                    let _ = console.update(cx, |console, _| editor::release(console, at));
                    cx.notify(view);
                });
            }
        },
    )
    .size_full()
}

fn paint(scene: &Scene, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let colors = &scene.colors;
    let view = scene.view;
    let origin = bounds.origin;
    let screen = |world: Vec2| {
        let at = view.to_screen(world);
        point(origin.x + px(at.x), origin.y + px(at.y))
    };
    window.paint_quad(fill(bounds, colors.bg));
    paint_grid(view, bounds, colors.grid, window);

    for wire in &scene.wires {
        let (color, width) = match (wire.selected, wire.mark) {
            (true, _) => (colors.accent, 3.0),
            (false, Mark::Taken) => (colors.success, 3.0),
            (false, Mark::Failed) => (colors.danger, 3.0),
            (false, Mark::Dim) => (colors.wire.opacity(0.35), 2.0),
            (false, Mark::Plain) => (colors.wire, 2.0),
        };
        stroke(wire.curve, width, color, None, &screen, window);
        arrow(screen(wire.curve.to), color, window);
    }
    if let Some((from, to)) = scene.pending {
        let curve = model::wire_curve(from, to);
        stroke(
            curve,
            2.0,
            colors.accent,
            Some(&[px(6.0), px(4.0)]),
            &screen,
            window,
        );
    }

    let zoom = view.zoom;
    let ui = window.text_style().font();
    let title_font = Font {
        weight: FontWeight::SEMIBOLD,
        ..ui.clone()
    };
    let mono = font(MONO);
    for card in &scene.cards {
        paint_card(
            card,
            colors,
            zoom,
            [&ui, &title_font, &mono],
            &screen,
            window,
            cx,
        );
    }
}

fn paint_card(
    card: &Drawn,
    colors: &Colors,
    zoom: f32,
    [ui, title_font, mono]: [&Font; 3],
    screen: &impl Fn(Vec2) -> Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let top_left = screen(card.at);
    let card_size = size(px(NODE_W * zoom), px(card.height * zoom));
    let radius = px(8.0 * zoom);
    let (border, width) = if card.selected {
        (card.color, 2.0)
    } else if card.target {
        (colors.accent, 2.0)
    } else {
        match card.mark {
            Mark::Taken => (colors.success, 2.0),
            Mark::Failed => (colors.danger, 2.0),
            Mark::Plain | Mark::Dim => (colors.border, 1.0),
        }
    };
    window.paint_quad(quad(
        Bounds::new(top_left, card_size),
        radius,
        colors.surface,
        px(width),
        border,
        BorderStyle::Solid,
    ));
    window.paint_quad(quad(
        Bounds::new(top_left, size(card_size.width, px((4.0 * zoom).max(2.0)))),
        Corners {
            top_left: radius,
            top_right: radius,
            bottom_left: px(0.0),
            bottom_right: px(0.0),
        },
        card.color,
        px(0.0),
        transparent_black(),
        BorderStyle::Solid,
    ));
    // Below 45 % the words are too small to read: colours and shapes only.
    if zoom >= 0.45 {
        let left = top_left.x + px(12.0 * zoom);
        let room = px((NODE_W - 24.0) * zoom);
        let lines = [
            (&card.kicker, ui, FONT_CAPTION, colors.fg_dim, 10.0),
            (&card.title, title_font, FONT_BODY, colors.fg, 26.0),
            (&card.preview, mono, FONT_CAPTION, colors.fg_dim, 48.0),
        ];
        for (text, face, text_size, color, top) in lines {
            let at = point(left, top_left.y + px(top * zoom));
            label(text, face, text_size * zoom, color, at, room, window, cx);
        }
    }
    if card.invalid {
        let dot = px(8.0 * zoom.max(0.6));
        window.paint_quad(quad(
            Bounds::new(
                point(
                    top_left.x + card_size.width - dot - px(8.0 * zoom),
                    top_left.y + px(10.0 * zoom),
                ),
                size(dot, dot),
            ),
            dot / 2.0,
            colors.danger,
            px(0.0),
            transparent_black(),
            BorderStyle::Solid,
        ));
    }
    let shape = model::Card {
        end: End::Trigger,
        at: card.at,
        height: card.height,
        outputs: card.outputs.len(),
        input: card.input,
    };
    if card.input {
        port(
            screen(shape.input_port()),
            card.color,
            colors.surface,
            window,
        );
    }
    let named = card.outputs.len() > 1;
    for (index, name) in card.outputs.iter().enumerate() {
        let center = screen(shape.output_port(index));
        port(center, card.color, colors.surface, window);
        if named && zoom >= 0.6 {
            let at = point(center.x + px(8.0), center.y - px(16.0 * zoom));
            label(
                name,
                ui,
                FONT_CAPTION * zoom,
                colors.fg_dim,
                at,
                px(120.0 * zoom),
                window,
                cx,
            );
        }
    }
    if card.mark == Mark::Dim {
        window.paint_quad(quad(
            Bounds::new(top_left, card_size),
            radius,
            colors.bg.opacity(0.55),
            px(0.0),
            transparent_black(),
            BorderStyle::Solid,
        ));
    }
}

/// Lines every 40 world units, doubled until they are at least 16 px apart.
fn paint_grid(view: View, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let mut step = 40.0 * view.zoom;
    while step < 16.0 {
        step *= 2.0;
    }
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let mut x = view.pan.x.rem_euclid(step);
    while x < width {
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x + px(x), bounds.origin.y),
                size(px(1.0), bounds.size.height),
            ),
            color,
        ));
        x += step;
    }
    let mut y = view.pan.y.rem_euclid(step);
    while y < height {
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, bounds.origin.y + px(y)),
                size(bounds.size.width, px(1.0)),
            ),
            color,
        ));
        y += step;
    }
}

fn stroke(
    curve: Bezier,
    width: f32,
    color: Hsla,
    dashes: Option<&[Pixels]>,
    screen: &impl Fn(Vec2) -> Point<Pixels>,
    window: &mut Window,
) {
    let mut path = PathBuilder::stroke(px(width));
    if let Some(dashes) = dashes {
        path = path.dash_array(dashes);
    }
    path.move_to(screen(curve.from));
    path.cubic_bezier_to(
        screen(curve.to),
        screen(curve.control_a),
        screen(curve.control_b),
    );
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// The arrowhead of a wire arriving at `tip` from the left, short of the
/// port.
fn arrow(tip: Point<Pixels>, color: Hsla, window: &mut Window) {
    let tip = point(tip.x - px(PORT_R), tip.y);
    let mut head = PathBuilder::fill();
    head.add_polygon(
        &[
            point(tip.x - px(8.0), tip.y - px(4.5)),
            tip,
            point(tip.x - px(8.0), tip.y + px(4.5)),
        ],
        true,
    );
    if let Ok(path) = head.build() {
        window.paint_path(path, color);
    }
}

fn port(center: Point<Pixels>, color: Hsla, fill_color: Hsla, window: &mut Window) {
    let radius = px(PORT_R);
    window.paint_quad(quad(
        Bounds::new(
            point(center.x - radius, center.y - radius),
            size(radius * 2.0, radius * 2.0),
        ),
        radius,
        fill_color,
        px(1.5),
        color,
        BorderStyle::Solid,
    ));
}

/// One line of text, cut with an ellipsis to fit `room`.
#[allow(clippy::too_many_arguments)]
fn label(
    text: &SharedString,
    face: &Font,
    text_size: Pixels,
    color: Hsla,
    at: Point<Pixels>,
    room: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let run = |len: usize| TextRun {
        len,
        font: face.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let system = window.text_system().clone();
    let mut shaped = system.shape_line(text.clone(), text_size, &[run(text.len())], None);
    if shaped.width > room {
        let mut cut = shaped.closest_index_for_x(room - text_size).min(text.len());
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        let short = SharedString::from(format!("{}…", &text[..cut]));
        shaped = system.shape_line(short.clone(), text_size, &[run(short.len())], None);
    }
    let _ = shaped.paint(at, text_size * 1.3, window, cx);
}
