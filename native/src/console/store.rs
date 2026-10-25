//! The console's copy of the plant: every tag's latest value, status and
//! history, fed by the runtime's events. No UI here.

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use voltrus_scada_native::api::{DataSourceSummary, HistorySeries};
use voltrus_scada_native::ws_types::TagUpdateEntry;
use vt_ui::chart::History;

/// Samples kept per tag: a day of history at one per 15 s plus live updates.
const HISTORY: usize = 8000;

/// What the server says about a tag's value (it evaluates the thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Unknown,
    Normal,
    Warning,
    Critical,
    Offline,
}

impl Status {
    pub fn parse(text: &str) -> Self {
        match text.to_ascii_lowercase().as_str() {
            "normal" | "good" | "ok" => Self::Normal,
            "warning" | "warn" => Self::Warning,
            "critical" | "alarm" => Self::Critical,
            "offline" | "stale" | "bad" => Self::Offline,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Waiting",
            Self::Normal => "Normal",
            Self::Warning => "Warning",
            Self::Critical => "Critical",
            Self::Offline => "Offline",
        }
    }

    pub fn needs_attention(self) -> bool {
        matches!(self, Self::Warning | Self::Critical | Self::Offline)
    }
}

pub struct Tag {
    pub info: DataSourceSummary,
    pub value: f64,
    pub status: Status,
    /// Unix seconds of the latest value, 0 before the first.
    pub at_secs: i64,
    pub simulated: bool,
    pub history: Rc<History>,
}

impl Tag {
    fn new(info: DataSourceSummary) -> Self {
        Self {
            info,
            value: f64::NAN,
            status: Status::Unknown,
            at_secs: 0,
            simulated: false,
            history: History::new(HISTORY),
        }
    }

    /// The value with the tag's configured decimals and unit.
    pub fn display(&self) -> String {
        if !self.value.is_finite() {
            return "—".into();
        }
        let decimals = usize::try_from(self.info.decimals.clamp(0, 6)).unwrap_or(2);
        format!("{:.*}", decimals, self.value)
    }
}

#[derive(Default)]
pub struct Store {
    pub tags: Vec<Tag>,
    index: HashMap<String, usize>,
}

impl Store {
    /// Replace the tag list, keeping the values and history of tags that
    /// are still there.
    pub fn replace(&mut self, list: Vec<DataSourceSummary>) {
        let mut previous: HashMap<String, Tag> = self
            .tags
            .drain(..)
            .map(|tag| (tag.info.key.clone(), tag))
            .collect();
        self.index.clear();
        for info in list {
            // Several devices may report the same key: the first one wins.
            if self.index.contains_key(&info.key) {
                continue;
            }
            let tag = match previous.remove(&info.key) {
                Some(mut kept) => {
                    kept.info = info;
                    kept
                }
                None => Tag::new(info),
            };
            self.index.insert(tag.info.key.clone(), self.tags.len());
            self.tags.push(tag);
        }
    }

    pub fn position(&self, key: &str) -> Option<usize> {
        self.index.get(key).copied()
    }

    pub fn keys(&self) -> Vec<String> {
        self.tags.iter().map(|tag| tag.info.key.clone()).collect()
    }

    /// Apply live updates; returns how many matched a known tag.
    pub fn apply(&mut self, updates: &[TagUpdateEntry]) -> usize {
        let mut applied = 0;
        for update in updates {
            let Some(&index) = self.index.get(&update.tag_id) else {
                continue;
            };
            let tag = &mut self.tags[index];
            tag.value = update.value;
            tag.status = Status::parse(&update.quality);
            tag.at_secs = update.ts;
            tag.simulated = update.simulated;
            #[allow(clippy::cast_precision_loss)]
            let at_ms = (update.ts as f64) * 1000.0;
            // Keep the history in time order: an update older than the last
            // sample (clock skew, replays) is shown but not plotted.
            if tag.history.latest().is_none_or(|(last, _)| at_ms >= last) {
                tag.history.push(
                    at_ms,
                    if tag.status == Status::Offline {
                        f64::NAN
                    } else {
                        update.value
                    },
                );
            }
            applied += 1;
        }
        applied
    }

    /// Load server history, replacing what each tag had.
    pub fn backfill(&mut self, series: &HashMap<String, HistorySeries>) {
        for tag in &mut self.tags {
            let Some(samples) = series.get(&tag.info.key) else {
                continue;
            };
            let latest = tag.history.latest();
            tag.history.clear();
            for (at_ms, value) in samples.samples() {
                #[allow(clippy::cast_precision_loss)]
                tag.history.push(at_ms as f64, value);
            }
            // Keep the newest live sample if the history ends before it.
            if let Some((at, value)) = latest
                && tag.history.latest().is_none_or(|(last, _)| at > last)
            {
                tag.history.push(at, value);
            }
        }
    }

    pub fn count(&self, status: Status) -> usize {
        self.tags.iter().filter(|tag| tag.status == status).count()
    }
}

/// Tag updates per second over the last second.
#[derive(Default)]
pub struct Rate {
    window: VecDeque<(Instant, usize)>,
}

impl Rate {
    pub fn record(&mut self, count: usize, now: Instant) {
        self.window.push_back((now, count));
        self.trim(now);
    }

    pub fn per_second(&mut self, now: Instant) -> usize {
        self.trim(now);
        self.window.iter().map(|(_, count)| count).sum()
    }

    fn trim(&mut self, now: Instant) {
        while self
            .window
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > Duration::from_secs(1))
        {
            self.window.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(key: &str) -> DataSourceSummary {
        DataSourceSummary {
            key: key.into(),
            name: key.into(),
            decimals: 1,
            ..DataSourceSummary::default()
        }
    }

    fn update(key: &str, value: f64, ts: i64, quality: &str) -> TagUpdateEntry {
        TagUpdateEntry {
            tag_id: key.into(),
            value,
            quality: quality.into(),
            ts,
            simulated: false,
        }
    }

    #[test]
    fn updates_land_on_their_tag_and_unknown_keys_are_ignored() {
        let mut store = Store::default();
        store.replace(vec![info("a"), info("b")]);
        let applied = store.apply(&[
            update("b", 4.25, 100, "warning"),
            update("zzz", 1.0, 100, "normal"),
        ]);
        assert_eq!(applied, 1);
        let b = &store.tags[store.position("b").unwrap()];
        assert_eq!(
            (b.value, b.status, b.display().as_str()),
            (4.25, Status::Warning, "4.2")
        );
        assert_eq!(b.history.latest(), Some((100_000.0, 4.25)));
    }

    #[test]
    fn a_new_tag_list_keeps_surviving_history_and_drops_duplicates() {
        let mut store = Store::default();
        store.replace(vec![info("a"), info("b")]);
        store.apply(&[update("a", 1.0, 10, "normal")]);
        store.replace(vec![info("c"), info("a"), info("a")]);
        assert_eq!(store.keys(), ["c", "a"]);
        assert_eq!(store.tags[1].history.len(), 1);
        assert_eq!(store.tags[1].value, 1.0);
    }

    #[test]
    fn offline_values_break_the_trend_and_old_updates_are_not_plotted() {
        let mut store = Store::default();
        store.replace(vec![info("a")]);
        store.apply(&[update("a", 5.0, 20, "normal")]);
        store.apply(&[update("a", 6.0, 10, "normal")]);
        assert_eq!(
            store.tags[0].history.len(),
            1,
            "an older update is not plotted"
        );
        assert_eq!(store.tags[0].value, 6.0, "but it is shown");
        store.apply(&[update("a", 0.0, 30, "offline")]);
        assert!(store.tags[0].history.latest().unwrap().1.is_nan());
    }

    #[test]
    fn backfill_replaces_history_but_keeps_a_newer_live_sample() {
        let mut store = Store::default();
        store.replace(vec![info("a")]);
        store.apply(&[update("a", 9.0, 100, "normal")]);
        let series = HashMap::from([(
            "a".to_string(),
            HistorySeries {
                values: vec![1.0, 2.0],
                start: 50_000,
                interval: 10_000,
            },
        )]);
        store.backfill(&series);
        let history = &store.tags[0].history;
        assert_eq!(history.len(), 3);
        assert_eq!(history.latest(), Some((100_000.0, 9.0)));
    }

    #[test]
    fn the_rate_counts_only_the_last_second() {
        let start = Instant::now();
        let mut rate = Rate::default();
        rate.record(10, start);
        rate.record(5, start + Duration::from_millis(600));
        assert_eq!(rate.per_second(start + Duration::from_millis(900)), 15);
        assert_eq!(rate.per_second(start + Duration::from_millis(1500)), 5);
    }
}
