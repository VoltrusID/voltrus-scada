//! A synthetic plant for stress tests: thousands of tags whose values follow
//! deterministic waves, delivered in the batches the server's WebSocket
//! sends. The runtime feeds them through the same event channel as the real
//! stream (`Command::Stress`), so the console's store, views and frame
//! budget see exactly the work a large plant would cause, without a server.
//!
//! Determinism: the tag list and every batch depend only on the options and
//! the tick number, never on the clock (the timestamp is passed in), so two
//! runs with the same seed apply the same values in the same order.

use std::time::Duration;

use crate::api::DataSource;
use crate::ws_types::TagUpdateEntry;

/// How often the feed sends a batch: 100 batches a second.
pub const TICK: Duration = Duration::from_millis(10);
const TICKS_PER_SECOND: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StressOptions {
    /// How many tags the synthetic plant has.
    pub tags: usize,
    /// Tag updates per second, spread evenly over the ticks.
    pub rate: u32,
    pub seed: u64,
}

impl Default for StressOptions {
    fn default() -> Self {
        Self {
            tags: 10_000,
            rate: 20_000,
            seed: 0x5CADA,
        }
    }
}

/// splitmix64: a fast, well-mixed hash of one number.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A number in `[0, 1)` from a hash.
#[allow(clippy::cast_precision_loss)]
fn unit(hash: u64) -> f64 {
    (hash >> 11) as f64 / (1_u64 << 53) as f64
}

const UNITS: [&str; 6] = ["kW", "V", "A", "°C", "Hz", "%"];

/// One synthetic tag's wave.
#[derive(Debug, Clone, Copy)]
struct Wave {
    base: f64,
    amplitude: f64,
    /// Ticks per cycle.
    period: f64,
    phase: f64,
    /// One tag in about a thousand reports offline.
    offline: bool,
}

impl Wave {
    fn new(seed: u64, index: usize) -> Self {
        let hash = mix(seed ^ (index as u64).wrapping_mul(0xA24B_AED4_963E_E407));
        let base = 10.0 + (unit(hash) * 890.0).round();
        Self {
            base,
            amplitude: base * (0.05 + unit(mix(hash ^ 1)) * 0.2),
            period: 300.0 + (unit(mix(hash ^ 2)) * 3000.0).round(),
            phase: unit(mix(hash ^ 3)) * std::f64::consts::TAU,
            offline: hash.is_multiple_of(997),
        }
    }

    fn warn(&self) -> f64 {
        self.base + self.amplitude * 0.85
    }

    fn crit(&self) -> f64 {
        self.base + self.amplitude * 0.97
    }
}

pub struct StressFeed {
    options: StressOptions,
    waves: Vec<Wave>,
    tick: u64,
    /// The next tag to update: batches walk the tags round-robin.
    cursor: usize,
}

impl StressFeed {
    pub fn new(options: StressOptions) -> Self {
        let waves = (0..options.tags)
            .map(|index| Wave::new(options.seed, index))
            .collect();
        Self {
            options,
            waves,
            tick: 0,
            cursor: 0,
        }
    }

    pub fn options(&self) -> StressOptions {
        self.options
    }

    /// The tag list, as `GET /api/v1/data-sources` would answer: ids from 1,
    /// a hundred tags per synthetic device.
    pub fn tags(&self) -> Vec<DataSource> {
        self.waves
            .iter()
            .enumerate()
            .map(|(index, wave)| {
                let device = index / 100;
                DataSource {
                    id: i64::try_from(index + 1).unwrap_or(i64::MAX),
                    key: Self::key(index),
                    name: format!("Stress {device:03}/{:02}", index % 100),
                    unit: UNITS[index % UNITS.len()].to_owned(),
                    color: String::new(),
                    min_val: wave.base - wave.amplitude * 1.2,
                    max_val: wave.base + wave.amplitude * 1.2,
                    warn_val: wave.warn(),
                    crit_val: wave.crit(),
                    base_val: wave.base,
                    sort_order: i64::try_from(index).unwrap_or(i64::MAX),
                    decimals: 1 + i64::from(index.is_multiple_of(2)),
                    source_type: "stress".to_owned(),
                    device_id: format!("stress-{device:03}"),
                    chart_type: "sparkline".to_owned(),
                    path: Some(format!("stress/{device:03}")),
                    ..DataSource::default()
                }
            })
            .collect()
    }

    fn key(index: usize) -> String {
        format!("stress.{:03}.{index:05}", index / 100)
    }

    /// Updates in tick `tick`: the rate spread over [`TICK`]s, carrying the
    /// remainder so a second holds exactly `rate` updates.
    pub fn updates_in_tick(&self, tick: u64) -> usize {
        let rate = u64::from(self.options.rate);
        let before = rate * tick / TICKS_PER_SECOND;
        let after = rate * (tick + 1) / TICKS_PER_SECOND;
        usize::try_from(after - before).unwrap_or(usize::MAX)
    }

    /// The next batch, stamped with `now_secs`.
    pub fn next_batch(&mut self, now_secs: i64) -> Vec<TagUpdateEntry> {
        let tags = self.waves.len();
        if tags == 0 {
            self.tick += 1;
            return Vec::new();
        }
        let count = self.updates_in_tick(self.tick);
        #[allow(clippy::cast_precision_loss)]
        let at = self.tick as f64;
        let mut batch = Vec::with_capacity(count);
        for _ in 0..count {
            let index = self.cursor;
            self.cursor = (self.cursor + 1) % tags;
            let wave = self.waves[index];
            let (value, quality) = if wave.offline {
                (f64::NAN, "offline")
            } else {
                let noise = (unit(mix(self.options.seed ^ (index as u64) ^ (self.tick << 20)))
                    - 0.5)
                    * wave.amplitude
                    * 0.08;
                let value = wave.base
                    + wave.amplitude
                        * (wave.phase + at * std::f64::consts::TAU / wave.period).sin()
                    + noise;
                let quality = if value >= wave.crit() {
                    "critical"
                } else if value >= wave.warn() {
                    "warning"
                } else {
                    "normal"
                };
                (value, quality)
            };
            batch.push(TagUpdateEntry {
                tag_id: Self::key(index),
                value,
                quality: quality.to_owned(),
                ts: now_secs,
                simulated: true,
            });
        }
        self.tick += 1;
        batch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(tags: usize, rate: u32) -> StressOptions {
        StressOptions {
            tags,
            rate,
            seed: 7,
        }
    }

    #[test]
    fn a_second_of_ticks_holds_exactly_the_rate() {
        for rate in [1, 99, 150, 20_000, 33_333] {
            let feed = StressFeed::new(options(10, rate));
            let second: usize = (0..TICKS_PER_SECOND)
                .map(|tick| feed.updates_in_tick(tick))
                .sum();
            assert_eq!(second, rate as usize, "rate {rate}");
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_stream_and_another_seed_does_not() {
        let run = |seed: u64| {
            let mut feed = StressFeed::new(StressOptions {
                seed,
                ..options(500, 9_000)
            });
            (0..200)
                .flat_map(|_| feed.next_batch(1_700_000_000))
                .map(|update| (update.tag_id, update.value.to_bits(), update.quality))
                .collect::<Vec<_>>()
        };
        let first = run(7);
        assert_eq!(first.len(), 18_000, "two seconds at 9000 updates/s");
        assert_eq!(first, run(7));
        assert_ne!(first, run(8));
    }

    #[test]
    fn batches_walk_every_tag_and_statuses_follow_the_limits() {
        let mut feed = StressFeed::new(options(1_000, 100_000));
        let tags = feed.tags();
        assert_eq!(tags.len(), 1_000);
        let ids: std::collections::HashSet<i64> = tags.iter().map(|tag| tag.id).collect();
        assert_eq!(ids.len(), 1_000, "ids are unique");
        let batch = feed.next_batch(0);
        assert_eq!(
            batch.len(),
            1_000,
            "one tick at 100k/s covers all 1000 tags"
        );
        for (tag, update) in tags.iter().zip(&batch) {
            assert_eq!(tag.key, update.tag_id);
            match update.quality.as_str() {
                "critical" => assert!(update.value >= tag.crit_val),
                "warning" => assert!(update.value >= tag.warn_val && update.value < tag.crit_val),
                "normal" => assert!(update.value < tag.warn_val),
                "offline" => assert!(update.value.is_nan()),
                other => panic!("unexpected quality {other}"),
            }
            assert!(
                update.value.is_nan() || (update.value > tag.min_val && update.value < tag.max_val)
            );
        }
        assert_eq!(
            feed.next_batch(0)[0].tag_id,
            tags[0].key,
            "round-robin wraps"
        );
    }
}
