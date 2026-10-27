//! The console's copy of the plant: every tag's latest value, status and
//! history, fed by the runtime's events. No UI here.
//!
//! Tags are known by their data source id. The WebSocket names them by key
//! (the history is read by id), and several devices may share a key (two inverters'
//! `frequency`): the server sends one update per data source, in the order
//! of its data source list, so the n-th update for a key in a batch belongs
//! to the n-th tag with that key in the list (`GET /api/v1/data-sources`
//! has the same order).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::SharedString;
use voltrus_scada_native::api::{DataSource, HistorySeries};
use voltrus_scada_native::ws_types::TagUpdateEntry;
use vt_ui::chart::History;

/// Filter results kept at once (the Tags filter, the Trends picker's, and
/// the queries typed on the way to them).
const FILTERS: usize = 8;

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
    /// Without allocating: the stream parses one per update.
    pub fn parse(text: &str) -> Self {
        let is = |names: &[&str]| names.iter().any(|name| text.eq_ignore_ascii_case(name));
        if is(&["normal", "good", "ok"]) {
            Self::Normal
        } else if is(&["warning", "warn"]) {
            Self::Warning
        } else if is(&["critical", "alarm"]) {
            Self::Critical
        } else if is(&["offline", "stale", "bad"]) {
            Self::Offline
        } else {
            Self::Unknown
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
        self.rank().is_some()
    }

    /// Where a tag needing attention sorts, worst first: critical, warning,
    /// offline.
    pub fn rank(self) -> Option<u8> {
        match self {
            Self::Critical => Some(0),
            Self::Warning => Some(1),
            Self::Offline => Some(2),
            Self::Normal | Self::Unknown => None,
        }
    }
}

pub struct Tag {
    pub info: DataSource,
    /// `info.name` and `info.key` as shared strings: views draw them every
    /// frame without copying.
    pub label: SharedString,
    pub key_label: SharedString,
    pub value: f64,
    /// The value before the latest change, for tweens.
    pub previous: f64,
    /// When the value last changed, for the change flash.
    pub changed_at: Option<Instant>,
    /// Set by the store only: it keeps the status counts and the attention
    /// list in step.
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
    fn new(info: DataSource) -> Self {
        Self {
            label: SharedString::from(info.name.clone()),
            key_label: SharedString::from(info.key.clone()),
            info,
            value: f64::NAN,
            previous: f64::NAN,
            changed_at: None,
            status: Status::Unknown,
            at_secs: 0,
            simulated: false,
            history: History::new(HISTORY),
            samples: VecDeque::new(),
            thinned_to: f64::NAN,
        }
    }

    /// Where the value sits between the tag's minimum and maximum, 0 to 1,
    /// when both are set and the value is known.
    pub fn level(value: f64, info: &DataSource) -> Option<f64> {
        let span = info.max_val - info.min_val;
        (value.is_finite() && span > 0.0).then(|| ((value - info.min_val) / span).clamp(0.0, 1.0))
    }

    /// The value with the tag's configured decimals (the unit is separate).
    pub fn display(&self) -> String {
        if !self.value.is_finite() {
            return "—".into();
        }
        let decimals = usize::try_from(self.info.decimals.clamp(0, 6)).unwrap_or(2);
        format!("{:.*}", decimals, self.value)
    }

    /// How far back the history reaches from `now_ms`, if it holds anything
    /// (views ask the store's [`Store::span_ms`], kept for all tags).
    #[cfg(test)]
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

    /// The plotted samples, oldest first: `(Unix ms, value)`.
    pub fn samples(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.samples.iter().copied()
    }

    /// The plotted samples from `since_ms` on (found by binary search, so a
    /// sparkline over the last minutes does not walk a day of history).
    pub fn samples_since(&self, since_ms: f64) -> impl Iterator<Item = (f64, f64)> + Clone + '_ {
        let from = self.samples.partition_point(|(at, _)| *at < since_ms);
        self.samples.range(from..).copied()
    }
}

/// The tag list plus what the views ask of all of it every frame, kept up
/// to date as updates arrive rather than recounted over every tag: status
/// counts, the tags needing attention in order, the oldest sample, and
/// filter results.
#[derive(Default)]
pub struct Store {
    pub tags: Vec<Tag>,
    /// Data source id → position in `tags`.
    by_id: HashMap<i64, usize>,
    /// Key → positions of the tags with that key, in list order.
    by_key: HashMap<String, Vec<usize>>,
    /// Per batch: how many updates each shared key has had so far.
    shared_seen: HashMap<usize, usize>,
    /// Tags per status, indexed by `Status as usize`.
    counts: [usize; 5],
    /// Tags needing attention as (rank, position): worst first, then in
    /// list order.
    attention: BTreeSet<(u8, usize)>,
    /// Per tag, its name and key lowercased, for filters.
    haystacks: Vec<String>,
    /// Every position, the answer to an empty filter.
    all: Rc<[usize]>,
    /// Recent filter results by lowercased query, newest last; the tag
    /// list changing clears them (values and statuses do not matter).
    filters: RefCell<Vec<(String, Rc<[usize]>)>>,
    /// The oldest sample of any tag: `None` until worked out again,
    /// `Some(None)` with no samples at all.
    earliest: Cell<Option<Option<f64>>>,
    /// Counts tag list changes ([`Store::replace`]), for view caches.
    generation: u64,
}

impl Store {
    /// Replace the tag list, keeping the values and history of tags that
    /// are still there (by id).
    pub fn replace(&mut self, list: Vec<DataSource>) {
        let mut previous: HashMap<i64, Tag> =
            self.tags.drain(..).map(|tag| (tag.info.id, tag)).collect();
        self.generation += 1;
        self.by_id.clear();
        self.by_key.clear();
        for info in list {
            if self.by_id.contains_key(&info.id) {
                continue;
            }
            let tag = match previous.remove(&info.id) {
                Some(mut kept) => {
                    kept.label = SharedString::from(info.name.clone());
                    kept.key_label = SharedString::from(info.key.clone());
                    kept.info = info;
                    kept
                }
                None => Tag::new(info),
            };
            let position = self.tags.len();
            self.by_id.insert(tag.info.id, position);
            self.by_key
                .entry(tag.info.key.clone())
                .or_default()
                .push(position);
            self.tags.push(tag);
        }
        self.counts = [0; 5];
        self.attention.clear();
        for (position, tag) in self.tags.iter().enumerate() {
            self.counts[tag.status as usize] += 1;
            if let Some(rank) = tag.status.rank() {
                self.attention.insert((rank, position));
            }
        }
        self.haystacks = self
            .tags
            .iter()
            .map(|tag| format!("{}\n{}", tag.info.name, tag.info.key).to_lowercase())
            .collect();
        self.all = (0..self.tags.len()).collect();
        self.filters.borrow_mut().clear();
        self.earliest.set(None);
    }

    /// Counts tag list changes, for view caches keyed on the tag list.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The position in `tags` of the tag with data source id `id`.
    pub fn position(&self, id: i64) -> Option<usize> {
        self.by_id.get(&id).copied()
    }

    pub fn get(&self, id: i64) -> Option<&Tag> {
        self.position(id).map(|index| &self.tags[index])
    }

    /// The first tag with this key (alarms and `--tag` name tags by key).
    pub fn by_key(&self, key: &str) -> Option<usize> {
        self.by_key
            .get(key)
            .and_then(|positions| positions.first().copied())
    }

    /// Every key once, in list order: what the WebSocket subscribes to.
    pub fn keys(&self) -> Vec<String> {
        self.tags
            .iter()
            .enumerate()
            .filter(|(index, tag)| self.by_key(&tag.info.key) == Some(*index))
            .map(|(_, tag)| tag.info.key.clone())
            .collect()
    }

    /// Apply live updates; returns how many matched a known tag.
    pub fn apply(&mut self, updates: &[TagUpdateEntry]) -> usize {
        self.apply_at(updates, Instant::now())
    }

    /// Apply live updates arriving at `now` (the change flash's start).
    pub fn apply_at(&mut self, updates: &[TagUpdateEntry], now: Instant) -> usize {
        let mut applied = 0;
        self.shared_seen.clear();
        for update in updates {
            let Some(positions) = self.by_key.get(&update.tag_id) else {
                continue;
            };
            let index = match positions.as_slice() {
                [only] => *only,
                shared => {
                    // The n-th update for a shared key in this batch is the
                    // n-th tag with it.
                    let seen = self.shared_seen.entry(shared[0]).or_insert(0);
                    let index = shared[(*seen).min(shared.len() - 1)];
                    *seen += 1;
                    index
                }
            };
            let status = Status::parse(&update.quality);
            self.set_status(index, status);
            let tag = &mut self.tags[index];
            if tag.value.to_bits() != update.value.to_bits() {
                tag.previous = tag.value;
                tag.changed_at = Some(now);
            }
            tag.value = update.value;
            tag.at_secs = update.ts;
            tag.simulated = update.simulated;
            #[allow(clippy::cast_precision_loss)]
            let at_ms = (update.ts as f64) * 1000.0;
            // An offline value breaks the line rather than drawing it.
            let value = if status == Status::Offline {
                f64::NAN
            } else {
                update.value
            };
            let first_before = tag.samples.front().map(|(at, _)| *at);
            tag.record(at_ms, value);
            let first_after = tag.samples.front().map(|(at, _)| *at);
            if first_after != first_before {
                self.first_sample_moved(first_before, first_after);
            }
            applied += 1;
        }
        applied
    }

    /// Move tag `index` to `status`, keeping the counts and the attention
    /// list in step.
    fn set_status(&mut self, index: usize, status: Status) {
        let old = self.tags[index].status;
        if old == status {
            return;
        }
        self.counts[old as usize] -= 1;
        self.counts[status as usize] += 1;
        if let Some(rank) = old.rank() {
            self.attention.remove(&(rank, index));
        }
        if let Some(rank) = status.rank() {
            self.attention.insert((rank, index));
        }
        self.tags[index].status = status;
    }

    /// A tag's oldest sample changed: a first sample can only lower the
    /// oldest of all; anything else (old samples dropped) works it out
    /// again when next asked.
    fn first_sample_moved(&self, before: Option<f64>, after: Option<f64>) {
        match (before, after, self.earliest.get()) {
            (None, Some(at), Some(earliest)) => self
                .earliest
                .set(Some(Some(earliest.map_or(at, |earliest| earliest.min(at))))),
            _ => self.earliest.set(None),
        }
    }

    /// Load server history, replacing what each tag had up to its last
    /// bucket; the live samples after that bucket stay. A tag the server has
    /// no samples for keeps its own. The history is keyed by data source id
    /// (`/history?keyed_by=id`), so tags sharing a key each get their own.
    pub fn backfill(&mut self, series: &HashMap<String, HistorySeries>) {
        for (id, series) in series {
            let Some(index) = id.parse::<i64>().ok().and_then(|id| self.position(id)) else {
                continue;
            };
            let tag = &mut self.tags[index];
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
        self.earliest.set(None);
    }

    /// How many tags have `status`.
    pub fn count(&self, status: Status) -> usize {
        self.counts[status as usize]
    }

    /// How many tags need attention (warning, critical or offline).
    pub fn attention_count(&self) -> usize {
        self.attention.len()
    }

    /// The positions of the tags needing attention, worst first (critical,
    /// warning, offline), then in list order.
    pub fn attention(&self) -> impl Iterator<Item = usize> + '_ {
        self.attention.iter().map(|(_, position)| *position)
    }

    /// How far back the oldest sample of any tag reaches from `now_ms`.
    pub fn span_ms(&self, now_ms: f64) -> Option<f64> {
        let earliest = self.earliest.get().unwrap_or_else(|| {
            let earliest = self
                .tags
                .iter()
                .filter_map(|tag| tag.samples.front().map(|(at, _)| *at))
                .reduce(f64::min);
            self.earliest.set(Some(earliest));
            earliest
        });
        earliest.map(|first| (now_ms - first).max(0.0))
    }

    /// The positions of the tags whose name or key contains `query`
    /// (trimmed, any case), in list order. The answer is kept until the
    /// tag list changes, so a view may ask for it every frame.
    pub fn filter(&self, query: &str) -> Rc<[usize]> {
        let query = query.trim();
        if query.is_empty() {
            return Rc::clone(&self.all);
        }
        let query = query.to_lowercase();
        let mut filters = self.filters.borrow_mut();
        if let Some((_, rows)) = filters.iter().find(|(kept, _)| *kept == query) {
            return Rc::clone(rows);
        }
        let rows: Rc<[usize]> = self
            .haystacks
            .iter()
            .enumerate()
            .filter(|(_, haystack)| haystack.contains(&query))
            .map(|(position, _)| position)
            .collect();
        if filters.len() == FILTERS {
            filters.remove(0);
        }
        filters.push((query, Rc::clone(&rows)));
        rows
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

    /// A tag whose id derives from its key, so tests can name tags by key.
    fn info(key: &str) -> DataSource {
        let id = key
            .bytes()
            .fold(0_i64, |id, byte| id * 131 + i64::from(byte));
        info_with(id, key)
    }

    fn info_with(id: i64, key: &str) -> DataSource {
        DataSource {
            id,
            key: key.into(),
            name: key.into(),
            decimals: 1,
            ..DataSource::default()
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
        let b = &store.tags[store.by_key("b").unwrap()];
        assert_eq!(
            (b.value, b.status, b.display().as_str()),
            (4.25, Status::Warning, "4.2")
        );
        assert_eq!(b.history.latest(), Some((100_000.0, 4.25)));
        assert_eq!(store.get(info("b").id).map(|tag| tag.value), Some(4.25));
    }

    #[test]
    fn a_new_tag_list_keeps_surviving_history_by_id() {
        let mut store = Store::default();
        store.replace(vec![info("a"), info("b")]);
        store.apply(&[update("a", 1.0, 10, "normal")]);
        // `a` moved, a repeated id is dropped.
        store.replace(vec![info("c"), info("a"), info("a")]);
        assert_eq!(store.keys(), ["c", "a"]);
        assert_eq!(store.tags.len(), 2);
        assert_eq!(store.tags[1].history.len(), 1);
        assert_eq!(store.tags[1].value, 1.0);
        assert_eq!(store.position(info("a").id), Some(1));
        assert_eq!(store.position(info("b").id), None);
    }

    #[test]
    fn devices_sharing_a_key_each_keep_their_own_value() {
        // Two inverters both report `frequency`; the server sends one
        // update per data source, in list order.
        let mut store = Store::default();
        store.replace(vec![
            info_with(1, "frequency"),
            info_with(2, "power"),
            info_with(3, "frequency"),
        ]);
        assert_eq!(store.keys(), ["frequency", "power"], "subscribed once");
        store.apply(&[
            update("frequency", 50.01, 100, "normal"),
            update("power", 7.0, 100, "normal"),
            update("frequency", 49.98, 100, "warning"),
        ]);
        let value = |store: &Store, id| store.get(id).map(|tag| (tag.value, tag.status));
        assert_eq!(value(&store, 1), Some((50.01, Status::Normal)));
        assert_eq!(value(&store, 3), Some((49.98, Status::Warning)));
        // The next batch counts afresh; a lone update is the first tag's.
        store.apply(&[update("frequency", 50.5, 101, "normal")]);
        assert_eq!(value(&store, 1), Some((50.5, Status::Normal)));
        assert_eq!(value(&store, 3), Some((49.98, Status::Warning)));
        assert_eq!(store.by_key("frequency"), Some(0));
    }

    #[test]
    fn a_changed_value_is_timed_for_its_flash_and_level() {
        let mut store = Store::default();
        let mut tag = info("a");
        tag.min_val = 0.0;
        tag.max_val = 200.0;
        store.replace(vec![tag]);
        let t0 = Instant::now();
        store.apply_at(&[update("a", 50.0, 1, "normal")], t0);
        let later = t0 + Duration::from_secs(1);
        store.apply_at(&[update("a", 50.0, 2, "normal")], later);
        assert_eq!(
            store.tags[0].changed_at,
            Some(t0),
            "the same value is no change"
        );
        store.apply_at(&[update("a", 150.0, 3, "normal")], later);
        let tag = &store.tags[0];
        assert_eq!((tag.previous, tag.changed_at), (50.0, Some(later)));
        assert_eq!(Tag::level(tag.value, &tag.info), Some(0.75));
        assert_eq!(Tag::level(f64::NAN, &tag.info), None);
        assert_eq!(
            Tag::level(1.0, &DataSource::default()),
            None,
            "no range set"
        );
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
            info("a").id.to_string(),
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
    fn tags_sharing_a_key_each_get_their_own_backfill() {
        let mut store = Store::default();
        store.replace(vec![info_with(1, "frequency"), info_with(2, "frequency")]);
        let series = |value: f64| HistorySeries {
            values: vec![value],
            start: 50_000,
            interval: 10_000,
        };
        store.backfill(&HashMap::from([
            ("1".to_string(), series(49.9)),
            ("2".to_string(), series(50.1)),
            ("not-an-id".to_string(), series(0.0)),
        ]));
        assert_eq!(store.tags[0].history.latest(), Some((50_000.0, 49.9)));
        assert_eq!(store.tags[1].history.latest(), Some((50_000.0, 50.1)));
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
        let empty = HashMap::from([(info("a").id.to_string(), HistorySeries::default())]);
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
            info("a").id.to_string(),
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
            info("a").id.to_string(),
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

    #[test]
    fn qualities_parse_in_any_case() {
        assert_eq!(Status::parse("GOOD"), Status::Normal);
        assert_eq!(Status::parse("Warn"), Status::Warning);
        assert_eq!(Status::parse("alarm"), Status::Critical);
        assert_eq!(Status::parse("Stale"), Status::Offline);
        assert_eq!(Status::parse("normalish"), Status::Unknown);
        assert_eq!(Status::parse(""), Status::Unknown);
    }

    /// The counts and the attention list as a full scan works them out.
    fn scanned(store: &Store) -> ([usize; 5], Vec<usize>) {
        let mut counts = [0; 5];
        for tag in &store.tags {
            counts[tag.status as usize] += 1;
        }
        let mut attention: Vec<usize> = (0..store.tags.len())
            .filter(|index| store.tags[*index].status.needs_attention())
            .collect();
        attention.sort_by_key(|index| store.tags[*index].status.rank());
        (counts, attention)
    }

    fn kept(store: &Store) -> ([usize; 5], Vec<usize>) {
        let counts = [
            Status::Unknown,
            Status::Normal,
            Status::Warning,
            Status::Critical,
            Status::Offline,
        ]
        .map(|status| store.count(status));
        (counts, store.attention().collect())
    }

    #[test]
    fn counts_and_attention_follow_every_update_like_a_full_scan() {
        let mut store = Store::default();
        let keys: Vec<String> = (0..40).map(|index| format!("t{index}")).collect();
        store.replace(keys.iter().map(String::as_str).map(info).collect());
        assert_eq!(store.count(Status::Unknown), 40);
        let qualities = ["normal", "warning", "critical", "offline", "bad", "ok"];
        let mut seed: u64 = 12345;
        for second in 0..200 {
            let batch: Vec<TagUpdateEntry> = (0..7)
                .map(|_| {
                    seed = seed
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    let tag = usize::try_from(seed >> 33).unwrap() % keys.len();
                    let quality = qualities[usize::try_from(seed >> 20).unwrap() % qualities.len()];
                    update(&keys[tag], 1.0, second, quality)
                })
                .collect();
            store.apply(&batch);
            assert_eq!(kept(&store), scanned(&store), "after second {second}");
        }
        assert!(store.attention_count() > 0);
        // A new list keeps surviving statuses and recounts.
        store.replace(keys[10..].iter().map(String::as_str).map(info).collect());
        assert_eq!(kept(&store), scanned(&store));
    }

    #[test]
    fn attention_is_worst_first_then_in_list_order() {
        let mut store = Store::default();
        store.replace(vec![info("a"), info("b"), info("c"), info("d")]);
        store.apply(&[
            update("a", 1.0, 1, "offline"),
            update("b", 1.0, 1, "warning"),
            update("c", 1.0, 1, "critical"),
            update("d", 1.0, 1, "warning"),
        ]);
        assert_eq!(store.attention().collect::<Vec<_>>(), [2, 1, 3, 0]);
        store.apply(&[update("c", 1.0, 2, "normal")]);
        assert_eq!(store.attention().collect::<Vec<_>>(), [1, 3, 0]);
        assert_eq!(
            (store.count(Status::Normal), store.attention_count()),
            (1, 3)
        );
    }

    #[test]
    fn filters_match_name_or_key_in_any_case_and_follow_the_list() {
        let mut store = Store::default();
        store.replace(vec![
            info_with(1, "inv1.power"),
            DataSource {
                name: "Grid Frequency".into(),
                ..info_with(2, "grid.hz")
            },
            info_with(3, "meter.energy"),
        ]);
        assert_eq!(&*store.filter("  "), [0, 1, 2], "empty is everything");
        assert_eq!(&*store.filter("FREQ"), [1], "by name, any case");
        assert_eq!(&*store.filter("grid.h"), [1], "by key");
        assert_eq!(&*store.filter("power"), [0]);
        assert!(
            store.filter("frequencygrid").is_empty(),
            "never across name and key"
        );
        let first = store.filter("e");
        assert!(Rc::ptr_eq(&first, &store.filter(" E ")), "kept per query");
        store.replace(vec![info_with(4, "freq.backup")]);
        assert_eq!(&*store.filter("freq"), [0], "a new list answers afresh");
    }

    #[test]
    fn the_overall_span_follows_first_samples_and_backfill() {
        let mut store = Store::default();
        store.replace(vec![info("a"), info("b")]);
        assert_eq!(store.span_ms(1_000_000.0), None);
        store.apply(&[update("a", 1.0, 500, "normal")]);
        assert_eq!(store.span_ms(600_000.0), Some(100_000.0));
        // An older first sample on another tag reaches further back.
        store.apply(&[update("b", 1.0, 400, "normal")]);
        assert_eq!(store.span_ms(600_000.0), Some(200_000.0));
        // Later samples do not move it.
        store.apply(&[update("a", 2.0, 550, "normal")]);
        assert_eq!(store.span_ms(600_000.0), Some(200_000.0));
        let series = HashMap::from([(
            info("a").id.to_string(),
            HistorySeries {
                values: vec![1.0],
                start: 100_000,
                interval: 10_000,
            },
        )]);
        store.backfill(&series);
        assert_eq!(store.span_ms(600_000.0), Some(500_000.0));
    }
}
