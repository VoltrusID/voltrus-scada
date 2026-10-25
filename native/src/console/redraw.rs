//! Which parts of the window a batch of live values redraws. Every part
//! (navbar, page, status bar, toasts, overlay, HUD) is a cached view that
//! redraws when the console notifies; a stream of tag updates notifies a
//! hundred times a second, and nothing but the page, the status bar and an
//! open overlay shows tag values. So a batch of nothing but updates
//! notifies those alone: the navbar and the toasts replay their last frame
//! instead of being rendered, laid out and painted again on every frame.

use gpui::Context;
use voltrus_scada_native::runtime::Event;

use super::Console;

/// Whether a batch only moved tag values (and device links), which the
/// page, the status bar and an overlay show.
pub(crate) fn values_only<'a>(batch: impl IntoIterator<Item = &'a Event>) -> bool {
    batch
        .into_iter()
        .all(|event| matches!(event, Event::Updates(_) | Event::DeviceStatus { .. }))
}

impl Console {
    /// Redraw what shows live values; everything else keeps its frame.
    pub(crate) fn values_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.views.pages.get(&self.page) {
            page.update(cx, |_, cx| cx.notify());
        }
        self.views.status_bar.update(cx, |_, cx| cx.notify());
        if self.overlay.is_some() {
            self.views.overlay.update(cx, |_, cx| cx.notify());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_value_batches_skip_the_rest_of_the_window() {
        let updates = Event::Updates(Vec::new());
        let device = Event::DeviceStatus {
            device_id: "inv1".into(),
            online: true,
            latency_ms: None,
        };
        assert!(values_only([&updates, &device]));
        assert!(values_only([]), "nothing changed, nothing more to draw");
        let alarms = Event::Link(voltrus_scada_native::runtime::Link::Live);
        assert!(!values_only([&updates, &alarms]));
    }
}
