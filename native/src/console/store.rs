//! The console's copy of the plant: every tag's latest value, status and
//! history, fed by the runtime's events. No UI here.

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use voltrus_scada_native::api::{DataSourceSummary, HistorySeries};
use voltrus_scada_native::ws_types::TagUpdateEntry;
use vt_ui::chart::History;

/// Samples kept per tag at most. Live values arrive about once a second;
/// thinning them with age (see [`Tag::thin`]) keeps a day within this.
const HISTORY: usize = 8000;

/// Every live sample of this long before the newest is kept: the shortest
/// trend range and the table's sparklines draw at full rate.
const DENSE_MS: f64 = 15.0 * 60_000.0;

/// Older live samples are thinned to about one per this long. A day of them
/// plus up to two dense spans fits [`HISTORY`] comfortably.
const COARSE_MS: f64 = 20_000.0;

/// Samples older than this before the newest are dropped: the longest trend
/// range and an hour to spare.
const KEEP_MS: f64 = 25.0 * 3_600_000.0;

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
    /// What the charts draw.
    pub history: Rc<History>,
    /// The same samples as `history`, which can only be appended to and
    /// cleared: thinning and backfill rebuild it from these.
    samples: VecDeque<(f64, f64)>,
    /// Unix ms up to which live samples are already thinned.
    thinned_to: f64,
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
            samples: VecDeque::new(),
            thinned_to: f64::NAN,
        }
    }

    /// The value with the tag's configured decimals (the unit is separate).
    pub fn display(&self) -> String {
        if !self.value.is_finite() {
            return "—".into();
        }
        let decimals = usize::try_from(self.info.decimals.clamp(0, 6)).unwrap_or(2);
        format!("{:.*}", decimals, self.value)
    }

    /// How far back the history reaches from `now_ms`, if it holds anything.
    pub fn span_ms(&self, now_ms: f64) -> Option<f64> {
        self.samples
            .front()
            .map(|(first, _)| (now_ms - first).max(0.0))
    }

    /// Add a sample in time order. The server sends every tag after each
    /// device poll, all stamped with the same second, so only the first
    /// sample of a second is plotted (the shown value still follows every
    /// update); an older one (clock skew, replays) is not plotted either.
    fn record(&mut self, at_ms: f64, value: f64) {
        let Some(&(last_at, _)) = self.samples.back() else {
            self.thinned_to = at_ms;
            self.append(at_ms, value);
            return;
        };
        if at_ms > last_at {
            self.append(at_ms, value);
            if at_ms - self.thinned_to >= 2.0 * DENSE_MS {
                self.thin(at_ms);
            }
        }
    }

    fn append(&mut self, at_ms: f64, value: f64) {
        if self.samples.len() == HISTORY {
            self.samples.pop_front();
        }
        self.samples.push_back((at_ms, value));
        self.history.push(at_ms, value);
    }

    /// Thin the samples between the last thinning and [`DENSE_MS`] before
    /// `newest` to about one per [`COARSE_MS`], keeping every break in the
    /// line, and drop what is older than [`KEEP_MS`].
    fn thin(&mut self, newest: f64) {
        let dense_from = newest - DENSE_MS;
        let oldest = newest - KEEP_MS;
        let thinned_to = self.thinned_to;
        let mut kept: Option<(f64, f64)> = None;
        self.samples.retain(|&(at, value)| {
            let keep = at >= oldest
                && (at < thinned_to
                    || at >= dense_from
                    || kept.is_none_or(|(last, last_value)| {
                        at - last >= COARSE_MS || value.is_nan() != last_value.is_nan()
                    }));
            if keep {
                kept = Some((at, value));
            }
            keep
        });
        self.thinned_to = dense_from;
        self.rebuild();
    }

    fn rebuild(&self) {
        self.history.clear();
        for &(at, value) in &self.samples {
            self.history.push(at, value);
        }
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
            // An offline value breaks the line rather than drawing it.
            let value = if tag.status == Status::Offline {
                f64::NAN
            } else {
                update.value
            };
            tag.record(at_ms, value);
            applied += 1;
        }
        applied
    }

    /// Load server history, replacing what each tag had up to its last
    /// bucket; the live samples after that bucket stay. A tag the server has
    /// no samples for keeps its own.
    pub fn backfill(&mut self, series: &HashMap<String, HistorySeries>) {
        for tag in &mut self.tags {
            let Some(series) = series
                .get(&tag.info.key)
                .filter(|series| !series.values.is_empty())
            else {
                continue;
            };
            let Some((last_bucket, _)) = series.samples().last() else {
                continue;
            };
            #[allow(clippy::cast_precision_loss)]
            let last_bucket = last_bucket as f64;
            #[allow(clippy::cast_precision_loss)]
            let buckets = series.samples().map(|(at_ms, value)| (at_ms as f64, value));
            let older = tag
                .samples
                .partition_point(|(at_ms, _)| *at_ms <= last_bucket);
            tag.samples.drain(..older);
            let live = tag.samples.len();
            tag.samples.extend(buckets);
            tag.samples.rotate_left(live);
            let excess = tag.samples.len().saturating_sub(HISTORY);
            tag.samples.drain(..excess);
            // Server buckets are coarse already; thinning starts after them.
            tag.thinned_to = last_bucket;
            tag.rebuild();
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
    fn the_span_starts_at_the_first_sample_and_empty_history_keeps_live_values() {
        let mut store = Store::default();
        store.replace(vec![info("a")]);
        assert_eq!(store.tags[0].span_ms(1000.0), None);
        store.apply(&[
            update("a", 1.0, 100, "normal"),
            update("a", 2.0, 160, "normal"),
        ]);
        assert_eq!(store.tags[0].span_ms(200_000.0), Some(100_000.0));
        let empty = HashMap::from([("a".to_string(), HistorySeries::default())]);
        store.backfill(&empty);
        assert_eq!(store.tags[0].history.len(), 2);
    }

    #[test]
    fn only_the_first_update_of_a_second_is_plotted_but_the_value_follows_all() {
        let mut store = Store::default();
        store.replace(vec![info("a")]);
        store.apply(&[update("a", 1.0, 100, "normal")]);
        store.apply(&[update("a", 1.0, 100, "normal")]);
        store.apply(&[update("a", 2.0, 100, "warning")]);
        let tag = &store.tags[0];
        assert_eq!(tag.history.len(), 1, "no duplicates");
        assert_eq!(tag.history.latest(), Some((100_000.0, 1.0)), "first kept");
        assert_eq!(
            (tag.value, tag.status),
            (2.0, Status::Warning),
            "the shown value and status follow the newest update"
        );
        store.apply(&[update("a", 3.0, 101, "normal")]);
        assert_eq!(store.tags[0].history.len(), 2);
        assert_eq!(store.tags[0].samples.len(), 2);
        assert_eq!(store.tags[0].history.latest(), Some((101_000.0, 3.0)));
    }

    #[test]
    fn a_day_of_live_values_fits_and_older_ones_are_thinned_or_dropped() {
        let mut store = Store::default();
        store.replace(vec![info("a")]);
        let day: i64 = 26 * 3600;
        for second in 0..day {
            // Two broadcasts a second, as with two polled devices.
            #[allow(clippy::cast_precision_loss)]
            let value = second as f64;
            store.apply(&[
                update("a", value, second, "normal"),
                update("a", value, second, "normal"),
            ]);
        }
        let tag = &store.tags[0];
        #[allow(clippy::cast_precision_loss)]
        let now = (day as f64) * 1000.0;
        assert!(tag.history.len() <= HISTORY);
        assert_eq!(tag.history.len(), tag.samples.len());
        let span = tag.span_ms(now).unwrap();
        assert!(span >= 24.0 * 3_600_000.0, "a day is kept, got {span}");
        assert!(
            span <= KEEP_MS + 2.0 * DENSE_MS,
            "older is dropped, got {span}"
        );
        let summary = tag.history.summary(now - DENSE_MS).unwrap();
        assert!(summary.count >= 900, "the last minutes stay at full rate");
        let hour = tag.history.summary(now - 3_600_000.0).unwrap();
        assert!(hour.count < 3600, "older live values are thinned");
    }

    #[test]
    fn a_backfilled_day_survives_hours_of_live_values() {
        let mut store = Store::default();
        store.replace(vec![info("a")]);
        let start = 1_000_000;
        let interval = 8 * 60_000;
        let series = HashMap::from([(
            "a".to_string(),
            HistorySeries {
                values: vec![5.0; 180],
                start,
                interval,
            },
        )]);
        store.backfill(&series);
        let end_secs = (start + 180 * 8 * 60_000) / 1000;
        let live_secs = 2 * 3600;
        for second in end_secs..end_secs + live_secs {
            store.apply(&[
                update("a", 1.0, second, "normal"),
                update("a", 1.0, second, "normal"),
            ]);
        }
        let tag = &store.tags[0];
        #[allow(clippy::cast_precision_loss)]
        let now = ((end_secs + live_secs) as f64) * 1000.0;
        let span = tag.span_ms(now).unwrap();
        assert!(
            span >= 24.0 * 3_600_000.0,
            "the day still shows, got {span}"
        );
        assert_eq!(tag.samples.front().map(|(_, value)| *value), Some(5.0));
    }

    #[test]
    fn backfill_keeps_every_live_sample_after_the_last_bucket() {
        let mut store = Store::default();
        store.replace(vec![info("a")]);
        for second in 100..104 {
            #[allow(clippy::cast_precision_loss)]
            let value = second as f64;
            store.apply(&[update("a", value, second, "normal")]);
        }
        let series = HashMap::from([(
            "a".to_string(),
            HistorySeries {
                values: vec![1.0, 2.0, 3.0],
                start: 81_000,
                interval: 10_000,
            },
        )]);
        store.backfill(&series);
        let tag = &store.tags[0];
        let times: Vec<f64> = tag.samples.iter().map(|(at, _)| *at).collect();
        assert_eq!(
            times,
            [81_000.0, 91_000.0, 101_000.0, 102_000.0, 103_000.0],
            "live samples up to the last bucket give way to it"
        );
        assert_eq!(tag.history.len(), 5);
        assert_eq!(tag.history.latest(), Some((103_000.0, 103.0)));
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
