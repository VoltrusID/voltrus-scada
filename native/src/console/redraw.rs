//! Which parts of the window a batch of live values redraws. Every part
//! (navbar, page, status bar, toasts, overlay, HUD) is a cached view that
//! redraws when the console notifies; a stream of tag updates notifies a
//! hundred times a second, and only some parts show tag values: the status
//! bar, an open overlay, and the pages that draw values. So a batch of
//! nothing but updates notifies those alone: the rest replay their last
//! frame instead of being rendered, laid out and painted again on every
//! frame (a Data Manager table rebuilt sixty times a second for values it
//! does not show cost more than any page that shows them).

use gpui::Context;
use voltrus_scada_native::runtime::Event;

use super::{Console, pages};

/// Pages that show no live tag values (configuration, alarms, operations
/// logs): a batch of updates leaves them as they are. Their own data comes
/// with other events (alarms, REST replies), which redraw everything, and
/// their ages move on with the once-a-second tick.
const VALUELESS: [&str; 5] = ["alarms", "data", "commands", "shift", "audit"];

/// Whether page `id` shows live tag values.
pub(crate) fn shows_values(id: &str) -> bool {
    !VALUELESS.contains(&id)
}

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
        if shows_values(pages::spec(self.page).id)
            && let Some(page) = self.views.pages.get(&self.page)
        {
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

    #[test]
    fn pages_without_live_values_keep_their_frame() {
        for id in VALUELESS {
            assert!(pages::by_id(id).is_some(), "{id} is a page");
            assert!(!shows_values(id));
        }
        for id in [
            "overview",
            "tags",
            "trends",
            "tagtree",
            "dashboard",
            "mimic",
        ] {
            assert!(shows_values(id), "{id} shows values");
        }
    }
}
