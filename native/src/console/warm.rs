//! Warming the pages after sign-in: each page the user may open is drawn
//! once, out of sight, one page per frame. It is rendered, laid out,
//! prepainted and painted like a shown page (its view made and kept, its
//! fonts loaded, its text shaped and its glyphs rasterized into the atlas)
//! inside a clip of no size, so nothing of it reaches the screen or the
//! mouse. The first switch to it then costs what any later one does. The
//! meter keeps these frames out of its figures and reports each page's
//! warm-up apart.

use gpui::prelude::*;
use gpui::{AnyElement, AnyView, Context, Entity, Pixels, Size, Window, div, px};
use vt_ui::theme::TITLE_BAR_HEIGHT;

use super::views::cached::CachedView;
use super::{Console, Phase, pages};

/// A page view drawn in full where pages go, `size` large, but clipped to
/// nothing: GPUI drops every primitive and hitbox outside the clip, after
/// the work of making them is done.
pub(crate) fn hidden(view: Entity<CachedView>, size: Size<Pixels>) -> AnyElement {
    div()
        .absolute()
        .top(TITLE_BAR_HEIGHT)
        .left_0()
        .w(px(0.0))
        .h(px(0.0))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w(size.width)
                .h(size.height)
                .flex()
                .flex_col()
                .child(AnyView::from(view).cached(div().size_full().style().clone())),
        )
        .into_any_element()
}

impl Console {
    /// Queue every other page the user may open for warming.
    pub(crate) fn warm_up(&mut self) {
        self.warm = pages::PAGES
            .iter()
            .rev()
            .filter(|spec| spec.page != self.page && self.allows(spec.role))
            .map(|spec| spec.page)
            .collect();
        self.meter.borrow_mut().set_warming(!self.warm.is_empty());
    }

    /// The page view to draw hidden in this frame, if any are left; asks
    /// for the next frame while more wait. Before the console shows (a
    /// stress run signing in) nothing is warmed and the warm-up goes on.
    pub(crate) fn warm_step(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<CachedView>> {
        if self.phase != Phase::Console {
            return None;
        }
        let Some(page) = self.warm.pop() else {
            self.meter.borrow_mut().set_warming(false);
            return None;
        };
        // Drawn clipped, its cached frame is not the one to replay: the
        // switch to it notifies the console, so its view draws afresh in
        // the page slot.
        window.request_animation_frame();
        Some(self.page_view(page, cx))
    }
}
