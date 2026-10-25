//! A part of the window drawn by its own view, as in muxy and Voltrus
//! Tools, so GPUI can reuse its last frame while something else animates.
//!
//! GPUI redraws a notified view and its ancestors. Without this, a trend
//! asking for a frame would rebuild the whole window: the navbar, the page,
//! the status bar. Wrapped in [`CachedView`]s and drawn with
//! `AnyView::cached`, only the animating part and the thin root rebuild;
//! everything else replays its previous frame until the console changes.

use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, Render, Subscription, WeakEntity,
    Window, div,
};

use crate::console::Console;

type RenderFn = Box<dyn Fn(&mut Console, &mut Window, &mut Context<Console>) -> AnyElement>;

pub struct CachedView {
    model: WeakEntity<Console>,
    render: RenderFn,
    _model_changes: Subscription,
}

impl CachedView {
    /// A view drawing `render` over the console, redrawn whenever the
    /// console notifies (and when `render` asks for an animation frame).
    pub fn new(
        render: impl Fn(&mut Console, &mut Window, &mut Context<Console>) -> AnyElement + 'static,
        cx: &mut Context<Console>,
    ) -> Entity<Self> {
        let model = cx.entity();
        cx.new(|cx| Self {
            model: model.downgrade(),
            render: Box::new(render),
            _model_changes: cx.observe(&model, |_, _, cx| cx.notify()),
        })
    }
}

impl Render for CachedView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render = &self.render;
        self.model
            .update(cx, |model, cx| render(model, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}
