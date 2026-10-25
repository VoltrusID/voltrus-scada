//! What one frame of the mimic draws, prepared from the store before
//! painting: each visible widget with its tags' values and its symbol's
//! live state, so the painters ([`super::symbols`], [`super::widgets`])
//! never touch the console.

use std::rc::Rc;

use gpui::SharedString;
use voltrus_scada_native::mimic::{Level, Live, Reading, Resolved, Values, Widget};

use crate::console::store::{Status, Store, Tag};

/// A tag as a widget draws it.
#[derive(Debug, Clone)]
pub(crate) struct TagView {
    pub id: i64,
    pub name: SharedString,
    pub key: SharedString,
    pub unit: SharedString,
    /// The tag's decimals (0 to 6).
    pub decimals: usize,
    /// `None` until a value arrives.
    pub value: Option<f64>,
    pub level: Level,
    pub min: f64,
    pub max: f64,
    pub warn: f64,
    pub crit: f64,
    /// `source_type` is `digital`.
    pub digital: bool,
    /// The latest samples, oldest first (`(Unix ms, value)`), for the
    /// widgets that draw a line; empty for the others.
    pub samples: Rc<[(f64, f64)]>,
}

impl TagView {
    pub fn of(tag: &Tag, samples: usize) -> Self {
        let info = &tag.info;
        let kept: Rc<[(f64, f64)]> = if samples == 0 {
            Rc::from([])
        } else {
            // The newest `samples`: skipping is O(1) on the store's deque.
            let from = tag.history.len().saturating_sub(samples);
            tag.samples().skip(from).collect()
        };
        Self {
            id: info.id,
            name: tag.label.clone(),
            key: tag.key_label.clone(),
            unit: info.unit.clone().into(),
            decimals: usize::try_from(info.decimals.clamp(0, 6)).unwrap_or(1),
            value: tag.value.is_finite().then_some(tag.value),
            level: level(tag.status),
            min: info.min_val,
            max: info.max_val,
            warn: info.warn_val,
            crit: info.crit_val,
            digital: info.source_type == "digital",
            samples: kept,
        }
    }

    /// The value with the tag's decimals, as `toFixed` writes it.
    pub fn text(&self) -> Option<String> {
        self.value.map(|value| format!("{value:.*}", self.decimals))
    }

    /// The value with `decimals`, `--` without one.
    pub fn fixed(&self, decimals: usize) -> String {
        self.value
            .map_or_else(|| "--".to_owned(), |value| format!("{value:.decimals$}"))
    }
}

/// The server's status as an alarm level.
pub(crate) fn level(status: Status) -> Level {
    match status {
        Status::Critical => Level::Critical,
        Status::Warning => Level::Warning,
        Status::Normal | Status::Unknown | Status::Offline => Level::Normal,
    }
}

/// The store as the mimic's live values.
pub(crate) struct StoreValues<'a>(pub &'a Store);

impl Values for StoreValues<'_> {
    fn reading(&self, tag: i64) -> Option<Reading> {
        let tag = self.0.get(tag)?;
        Some(Reading {
            value: tag.value.is_finite().then_some(tag.value),
            level: level(tag.status),
            min: tag.info.min_val,
            max: tag.info.max_val,
        })
    }
}

/// One widget to draw this frame.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    pub widget: Widget,
    /// The tags it shows: its bindings (only those the store knows), or for
    /// the multi-tag widgets the plant's first ones when it has none
    /// (`mimic::shown_tags`). The primary tag is `tags.first()` when the
    /// widget's first binding is known.
    pub tags: Vec<TagView>,
    /// The widget's primary tag, when bound and known.
    pub primary: Option<TagView>,
    /// For library symbols: the symbol and its live state.
    pub symbol: Option<(Resolved, Live)>,
    /// The faceplate is open on it.
    pub selected: bool,
    /// The keyboard cursor is on it (drawn with a focus ring).
    pub cursor: bool,
    /// The pointer is over it and a click opens its faceplate.
    pub hovered: bool,
}

/// Time-driven motion, from the seconds since the page's epoch: the web
/// client's CSS animations (`index.css`, `.hmi-*`).
pub(crate) mod motion {
    use std::f32::consts::PI;

    /// `hmi-spin`: a turn every 1.4 s, in degrees.
    pub fn spin(t: f32) -> f32 {
        (t / 1.4).fract() * 360.0
    }

    /// `hmi-flow`: the dash offset of a flowing line (dashes 8, gaps 16),
    /// moving 24 units every 0.9 s along the line.
    pub fn flow(t: f32) -> f32 {
        -(t / 0.9).fract() * 24.0
    }

    /// `hmi-agitate`: a horizontal scale swinging 1 → −1 → 1, 0.6 s each
    /// way, eased.
    pub fn agitate(t: f32) -> f32 {
        let phase = (t / 0.6).rem_euclid(2.0);
        let u = if phase < 1.0 { phase } else { 2.0 - phase };
        let eased = 0.5 - 0.5 * (PI * u).cos();
        1.0 - 2.0 * eased
    }

    /// `hmi-blink`: full for half a second, 0.35 for the next half.
    pub fn blink(t: f32) -> f32 {
        if t.rem_euclid(1.0) < 0.5 { 1.0 } else { 0.35 }
    }

    /// Seconds until the blink next changes.
    pub fn next_blink(t: f32) -> f32 {
        let into = t.rem_euclid(0.5);
        (0.5 - into).max(0.01)
    }
}

#[cfg(test)]
mod tests {
    use super::motion;

    #[test]
    fn motions_run_like_the_css_animations() {
        assert!(motion::spin(0.0).abs() < 1e-6);
        assert!((motion::spin(0.7) - 180.0).abs() < 1e-3);
        assert!((motion::flow(0.45) + 12.0).abs() < 1e-3);
        assert!((motion::agitate(0.0) - 1.0).abs() < 1e-6);
        assert!((motion::agitate(0.6) + 1.0).abs() < 1e-6);
        assert!(motion::agitate(0.3).abs() < 1e-6);
        assert!((motion::blink(0.2) - 1.0).abs() < 1e-6);
        assert!((motion::blink(0.7) - 0.35).abs() < 1e-6);
        assert!((motion::next_blink(0.2) - 0.3).abs() < 1e-5);
    }
}
