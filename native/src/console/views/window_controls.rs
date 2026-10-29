//! Window controls where the system draws none. The window's title bar is
//! transparent (`main`), which on macOS keeps the traffic lights over the
//! navbar but on Windows and Linux hides the caption: its minimize,
//! maximize and close buttons and the area that moves the window. This
//! draws them: the buttons at the window's top right, and a drag area over
//! the navbar's empty middle (or the whole top strip at the sign-in and
//! station screens). On Windows they are the system's own controls by hit
//! test (`WindowControlArea`: snap layouts, double-click to maximize);
//! elsewhere clicks call the window.
//!
//! Drawn by the console's root, which redraws every frame: GPUI does not
//! replay a cached view's control areas, and the navbar is cached.

use gpui::prelude::*;
use gpui::{
    Action, AnyElement, Bounds, Context, MouseButton, Pixels, Window, WindowControlArea, div, px,
};
use vt_ui::Icon;
use vt_ui::theme::*;

use crate::console::Console;
use crate::console::keys::Quit;

/// Whether the console draws the window's controls (not on macOS).
pub const SHOWN: bool = cfg!(not(target_os = "macos"));
/// One button's width, as Windows draws its caption buttons.
const BUTTON: Pixels = px(46.0);
/// The three buttons' width: the room the navbar leaves at its right.
pub const WIDTH: Pixels = px(46.0 * 3.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Minimize,
    Maximize,
    Close,
}

/// Make `area` move the window, and maximize it on a double click.
fn drag<E: InteractiveElement>(area: E) -> E {
    if cfg!(windows) {
        area.window_control_area(WindowControlArea::Drag)
    } else {
        area.on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count >= 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        })
    }
}

fn button(control: Control, window: &Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let (id, hover, hover_fg) = match control {
        Control::Minimize => ("window-minimize", theme.hover, theme.fg),
        Control::Maximize => ("window-maximize", theme.hover, theme.fg),
        Control::Close => ("window-close", theme.danger, gpui::white()),
    };
    let fg = theme.fg_muted;
    let icon = |icon: Icon| {
        vt_ui::icon(icon, ICON_SM, fg)
            .group_hover(id, move |style| style.text_color(hover_fg))
            .into_any_element()
    };
    let glyph = match control {
        Control::Minimize => div()
            .w(px(10.0))
            .h(px(1.0))
            .bg(fg)
            .group_hover(id, move |style| style.bg(hover_fg))
            .into_any_element(),
        Control::Maximize if window.is_maximized() => icon(Icon::Copy),
        Control::Maximize => icon(Icon::Square),
        Control::Close => icon(Icon::X),
    };
    let element = div()
        .id(id)
        .group(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(BUTTON)
        .h_full()
        .debug_selector(move || id.into())
        .hover(move |style| style.bg(hover))
        .child(glyph);
    if cfg!(windows) {
        element
            .window_control_area(match control {
                Control::Minimize => WindowControlArea::Min,
                Control::Maximize => WindowControlArea::Max,
                Control::Close => WindowControlArea::Close,
            })
            .into_any_element()
    } else {
        element
            .on_click(move |_, window, cx| match control {
                Control::Minimize => window.minimize_window(),
                Control::Maximize => window.zoom_window(),
                // As closing the window: the kiosk lock may keep it open.
                Control::Close => window.dispatch_action(Quit.boxed_clone(), cx),
            })
            .into_any_element()
    }
}

fn buttons(window: &Window, cx: &mut Context<Console>) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .h(TITLE_BAR_HEIGHT)
        .child(button(Control::Minimize, window, cx))
        .child(button(Control::Maximize, window, cx))
        .child(button(Control::Close, window, cx))
        .into_any_element()
}

/// The window's controls over the top strip of the window: in the console,
/// the buttons right of the navbar and a drag area over its empty middle
/// (`navbar_gap`, as the navbar last drew it); else the whole strip drags.
pub fn render(
    in_console: bool,
    navbar_gap: Option<Bounds<Pixels>>,
    window: &Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let strip = div()
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h(TITLE_BAR_HEIGHT);
    if in_console {
        strip
            .when_some(navbar_gap, |this, gap| {
                this.child(drag(
                    div()
                        .absolute()
                        .left(gap.origin.x)
                        .top(gap.origin.y)
                        .w(gap.size.width)
                        .h(gap.size.height),
                ))
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .child(buttons(window, cx)),
            )
            .into_any_element()
    } else {
        strip
            .flex()
            .child(drag(div().flex_1().h_full()))
            .child(buttons(window, cx))
            .into_any_element()
    }
}
