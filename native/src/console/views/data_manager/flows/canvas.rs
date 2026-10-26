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

use super::editor;
use super::model::{Bezier, NODE_H, NODE_W, PORT_R, Vec2, View, input_port, output_port};
use crate::console::Console;

/// One step's card as drawn.
pub(crate) struct Card {
    /// Top-left corner, world units.
    pub at: Vec2,
    pub color: Hsla,
    pub kicker: SharedString,
    pub title: SharedString,
    pub preview: SharedString,
    pub selected: bool,
    /// A wire being dragged would land here.
    pub target: bool,
    pub input: bool,
    pub output: bool,
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
}

/// What one frame of the canvas draws.
pub(crate) struct Scene {
    pub view: View,
    pub cards: Vec<Card>,
    /// Curves in world units; selected or not.
    pub wires: Vec<(Bezier, bool)>,
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

    for (curve, selected) in &scene.wires {
        let color = if *selected {
            colors.accent
        } else {
            colors.wire
        };
        stroke(
            *curve,
            if *selected { 3.0 } else { 2.0 },
            color,
            None,
            &screen,
            window,
        );
        arrow(screen(curve.to), color, window);
    }
    if let Some((from, to)) = scene.pending {
        let curve = super::model::wire_curve(from, to);
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
        let top_left = screen(card.at);
        let card_size = size(px(NODE_W * zoom), px(NODE_H * zoom));
        let radius = px(8.0 * zoom);
        let (border, width) = if card.selected {
            (card.color, 2.0)
        } else if card.target {
            (colors.accent, 2.0)
        } else {
            (colors.border, 1.0)
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
                (&card.kicker, &ui, FONT_CAPTION, colors.fg_dim, 10.0),
                (&card.title, &title_font, FONT_BODY, colors.fg, 26.0),
                (&card.preview, &mono, FONT_CAPTION, colors.fg_dim, 48.0),
            ];
            for (text, face, text_size, color, top) in lines {
                let at = point(left, top_left.y + px(top * zoom));
                label(text, face, text_size * zoom, color, at, room, window, cx);
            }
        }
        if card.input {
            port(
                screen(input_port(card.at)),
                card.color,
                colors.surface,
                window,
            );
        }
        if card.output {
            port(
                screen(output_port(card.at)),
                card.color,
                colors.surface,
                window,
            );
        }
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
