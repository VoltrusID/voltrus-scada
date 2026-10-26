//! Toasts in the bottom-right corner, newest at the bottom; each slides in
//! once and then holds still.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{Animation, AnimationExt as _, AnyElement, Context, ElementId, Pixels, Window, div, px};
use vt_ui::Icon;
use vt_ui::theme::*;

use crate::console::{Console, anim};

/// The toast column's box: as wide as a toast, tall enough for three.
pub(crate) const WIDTH: Pixels = px(380.0);
pub(crate) const HEIGHT: Pixels = px(240.0);

pub fn render(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let now = Instant::now();
    if console
        .toasts
        .iter()
        .any(|toast| anim::progress(toast.shown_at, now, anim::TOAST_IN).is_some())
    {
        // `with_animation` asks for the frames; the meter counts them.
        console.meter.borrow_mut().expect_frame(now);
    }
    let toasts = console.toasts.iter().map(|toast| {
        let color = if toast.error {
            theme.danger
        } else {
            theme.success
        };
        div()
            .relative()
            .w_full()
            .flex()
            .items_start()
            .gap(SPACE_4)
            .px(SPACE_6)
            .py(SPACE_5)
            .rounded(RADIUS_LG)
            .bg(theme.elevated)
            .border_1()
            .border_color(color.alpha(0.4))
            .shadow(theme.shadow(Elevation::Popover))
            .child(vt_ui::icon(
                if toast.error {
                    Icon::CircleAlert
                } else {
                    Icon::CircleCheck
                },
                ICON_LG,
                color,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(FONT_BODY)
                    .child(toast.message.clone()),
            )
            .with_animation(
                ElementId::NamedInteger("toast".into(), toast.id),
                Animation::new(anim::TOAST_IN).with_easing(anim::ease_out),
                |this, t| this.opacity(t).left(px((1.0 - t) * 32.0)),
            )
    });
    div()
        .size_full()
        .flex()
        .flex_col()
        .justify_end()
        .gap(SPACE_4)
        .children(toasts)
        .into_any_element()
}
