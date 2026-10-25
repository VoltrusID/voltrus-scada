//! The alarm sound's controls: the navbar's speaker button (⇧⌘M, also in
//! the palette) and the glue between the alarm list and the annunciator
//! (`console::audio`).

use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, Context, Hsla, PathBuilder, Pixels, Point, Window, canvas, div, point, px,
};
use vt_ui::components::Tooltip;
use vt_ui::theme::*;

use crate::console::Console;
use crate::console::audio::{Annunciator, Tone};
use crate::console::keys::{AlarmsToggleMute, hint};

/// `user@server`: whose mute choice applies.
fn user(console: &Console) -> Option<String> {
    console
        .user
        .as_ref()
        .map(|(username, _)| format!("{username}@{}", console.server))
}

/// Make the sound follow the alarm list (and silence it when signed out).
pub(super) fn sync(console: &mut Console) {
    let user = user(console);
    let alarms = std::mem::take(&mut console.alarms);
    console.state::<Annunciator>().sync(user, &alarms);
    console.alarms = alarms;
}

pub(super) fn toggle_mute(console: &mut Console, cx: &mut Context<Console>) {
    if console.user.is_none() {
        return;
    }
    let muted = console.state::<Annunciator>().toggle_mute();
    console.show_toast(
        if muted {
            "Alarm sound muted"
        } else {
            "Alarm sound on"
        },
        false,
        cx,
    );
    cx.notify();
}

pub(super) fn muted(console: &Console) -> bool {
    console
        .state_ref::<Annunciator>()
        .is_some_and(Annunciator::muted)
}

pub(super) fn sounding(console: &Console) -> Option<Tone> {
    console
        .state_ref::<Annunciator>()
        .and_then(Annunciator::sounding)
}

/// The navbar's speaker: struck through while muted, tinted while a tone
/// sounds. Click or ⇧⌘M toggles.
pub(crate) fn mute_button(console: &Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let annunciator = console.state_ref::<Annunciator>();
    let muted = annunciator.is_some_and(Annunciator::muted);
    let wanted = annunciator.and_then(Annunciator::wanted);
    let unavailable = annunciator.and_then(|a| a.unavailable().map(str::to_owned));
    let keys = hint(&AlarmsToggleMute).unwrap_or_default();
    let tooltip = match (&unavailable, muted, wanted) {
        (Some(error), _, _) => format!("No alarm sound: {error}"),
        (None, true, Some(tone)) => {
            format!("Alarm sound muted ({} alarm waits)  {keys}", tone.label())
        }
        (None, true, None) => format!("Alarm sound muted  {keys}"),
        (None, false, Some(tone)) => format!("Mute the {} alarm tone  {keys}", tone.label()),
        (None, false, None) => format!("Alarm sound on: sounds while an alarm waits  {keys}"),
    };
    let color = match (muted, wanted) {
        _ if unavailable.is_some() => theme.fg_dim,
        (false, Some(Tone::Critical)) => theme.danger,
        (false, Some(_)) => theme.warning,
        (true, Some(_)) => theme.warning,
        _ => theme.fg_muted,
    };
    let hover = theme.hover;
    let ring = theme.accent;
    div()
        .id("alarm-sound")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(CONTROL_SM)
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(gpui::transparent_black())
        .when(muted, |this| this.bg(theme.selected))
        .cursor_pointer()
        .tab_index(0)
        .focus(move |style| style.border_color(ring))
        .hover(move |style| style.bg(hover))
        .tooltip(move |_, cx| Tooltip::view(tooltip.clone().into(), cx))
        .on_click(cx.listener(|_, _, window, cx| {
            window.dispatch_action(Box::new(AlarmsToggleMute), cx);
        }))
        .child(speaker(muted || unavailable.is_some(), ICON_MD, color))
        .into_any_element()
}

/// A speaker glyph (vt-ui has no sound icons): the cone, then waves when
/// on or a cross when off.
fn speaker(off: bool, size: Pixels, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _| {
            let unit = f32::from(size);
            let at = |x: f32, y: f32| -> Point<Pixels> {
                point(
                    bounds.origin.x + px(x * unit),
                    bounds.origin.y + px(y * unit),
                )
            };
            let mut cone = PathBuilder::fill();
            cone.add_polygon(
                &[
                    at(0.08, 0.36),
                    at(0.28, 0.36),
                    at(0.52, 0.14),
                    at(0.52, 0.86),
                    at(0.28, 0.64),
                    at(0.08, 0.64),
                ],
                true,
            );
            if let Ok(path) = cone.build() {
                window.paint_path(path, color);
            }
            let mut lines = PathBuilder::stroke(px(1.3));
            if off {
                lines.move_to(at(0.66, 0.36));
                lines.line_to(at(0.94, 0.64));
                lines.move_to(at(0.94, 0.36));
                lines.line_to(at(0.66, 0.64));
            } else {
                lines.move_to(at(0.66, 0.34));
                lines.curve_to(at(0.66, 0.66), at(0.78, 0.5));
                lines.move_to(at(0.76, 0.2));
                lines.curve_to(at(0.76, 0.8), at(0.98, 0.5));
            }
            if let Ok(path) = lines.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(size)
}
