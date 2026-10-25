//! Warming the pages after sign-in: each page the user may open is drawn
//! once, hidden behind the shown one, one page per frame. Its view is made
//! and kept, its fonts are loaded and its text shaped, so the first switch
//! to it is as quick as any later one. The meter keeps these frames out of
//! its figures (they are the price of signing in, reported apart).

use gpui::prelude::*;
use gpui::{AnyElement, AnyView, Context, Entity, Visibility, Window, div};
use vt_ui::theme::{STATUS_BAR_HEIGHT, TITLE_BAR_HEIGHT};

use super::views::cached::CachedView;
use super::{Console, Phase, pages};

/// A page view laid out where pages go but not painted. Put it before the
/// shown page, so that nothing in it sits above the page for the mouse.
pub(crate) fn hidden(view: Entity<CachedView>) -> AnyElement {
    let mut layer = div()
        .absolute()
        .top(TITLE_BAR_HEIGHT)
        .left_0()
        .right_0()
        .bottom(STATUS_BAR_HEIGHT);
    layer.style().visibility = Some(Visibility::Hidden);
    layer
        .child(AnyView::from(view).cached(div().size_full().style().clone()))
        .into_any_element()
}

impl Console {
    /// Queue every other page the user may open for warming.
    pub(crate) fn warm_up(&mut self) {
        self.warm = pages::PAGES
            .iter()
            .rev()
            .filter(|spec| spec.page != self.page && pages::opens(spec, self.role()))
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
        // Drawn hidden, it was rendered but not painted: the switch to it
        // notifies the console, so its view draws afresh then rather than
        // replaying this frame.
        window.request_animation_frame();
        Some(self.page_view(page, cx))
    }
}
