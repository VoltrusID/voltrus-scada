//! Size benchmark for the packed historian. Ignored by default because it
//! writes a few hundred MB of temporary database:
//!
//! ```text
//! cargo test --lib storage::tests::historian_bench -- --ignored --nocapture
//! ```
//!
//! It generates one hour of synthetic plant data for 1 000 tags polled every
//! second (the tag mix is in `CLASSES`), writes it through the real insert
//! path, packs it with the real compaction code, and reports the size of the
//! database file before and after. A second run applies the per-tag history
//! deadband first. The numbers are extrapolated to 10 000 tags for one year;
//! the extrapolation is only as good as the assumed signal mix.

use super::*;
use crate::deadband::{DeadbandKind, HistoryFilter, HistoryRule};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;

const BASE: i64 = 1_699_999_200; // hour aligned
const SECONDS: i64 = 3600;
const TAGS: usize = 1000;

#[derive(Clone, Copy, Debug)]
enum Class {
    /// Slow process value (temperature, level) read as a scaled 16 bit register.
    SlowScaled,
    /// Slow process value read as an IEEE float32 register.
    SlowFloat32,
    /// Setpoint or status word: constant with an occasional step.
    Setpoint,
    /// Pump/valve/alarm bit.
    Digital,
    /// Integer pulse counter.
    Counter,
    /// Energy/volume totaliser: float accumulation.
    Totaliser,
    /// Fast noisy signal (flow, vibration, current) as float32.
    Noisy,
}

/// (class, tags out of 1000, deadband as fraction of nominal span, heartbeat s)
const CLASSES: [(Class, usize, f64, i64); 7] = [
    (Class::SlowScaled, 250, 0.005, 300),
    (Class::SlowFloat32, 200, 0.005, 300),
    (Class::Setpoint, 150, 0.0, 3600),
    (Class::Digital, 200, 0.0, 3600),
    (Class::Counter, 50, 0.0, 300),
    (Class::Totaliser, 50, 0.0001, 300),
    (Class::Noisy, 100, 0.01, 300),
];

struct Signal {
    class: Class,
    state: f64,
    aux: f64,
    scale: f64,
}

impl Signal {
    fn new(class: Class, rng: &mut StdRng) -> Self {
        Self {
            class,
            state: match class {
                Class::Digital => (rng.random_bool(0.5)) as i64 as f64,
                Class::Setpoint => rng.random_range(10.0..90.0f64).round(),
                Class::Counter => rng.random_range(0..1_000_000) as f64,
                Class::Totaliser => rng.random_range(1000.0..50_000.0),
                _ => rng.random_range(20_000.0..40_000.0f64).round(), // register counts
            },
            aux: rng.random_range(0.0..std::f64::consts::TAU),
            scale: [0.1, 0.01, 0.001][rng.random_range(0..3)],
        }
    }

    /// Nominal span, used to size the deadband.
    fn span(&self) -> f64 {
        match self.class {
            Class::SlowScaled => 65_535.0 * self.scale,
            Class::SlowFloat32 | Class::Noisy => 100.0,
            Class::Setpoint => 100.0,
            Class::Digital => 1.0,
            Class::Counter => 1000.0,
            Class::Totaliser => 50_000.0,
        }
    }

    fn next(&mut self, t: i64, rng: &mut StdRng) -> f64 {
        match self.class {
            Class::SlowScaled => {
                // Register counts wander by a count or two, with a slow swell.
                self.state += rng.random_range(-2.0..=2.0f64).round();
                self.state += (self.aux + t as f64 / 900.0).sin().round();
                (self.state * self.scale).clamp(0.0, 6553.5)
            }
            Class::SlowFloat32 => {
                self.state += rng.random_range(-2.0..=2.0f64).round();
                let v =
                    50.0 + (self.state - 30_000.0) * 0.0005 + (self.aux + t as f64 / 1800.0).sin();
                v as f32 as f64
            }
            Class::Setpoint => {
                if rng.random_bool(1.0 / 1800.0) {
                    self.state = rng.random_range(10.0..90.0f64).round();
                }
                self.state
            }
            Class::Digital => {
                if rng.random_bool(1.0 / 1200.0) {
                    self.state = 1.0 - self.state;
                }
                self.state
            }
            Class::Counter => {
                self.state += rng.random_range(0..3) as f64;
                self.state
            }
            Class::Totaliser => {
                self.state += 0.01 * (1.0 + rng.random_range(0.0..0.2));
                self.state
            }
            Class::Noisy => (50.0 + rng.random_range(-8.0..8.0f64)) as f32 as f64,
        }
    }
}

struct Outcome {
    samples: u64,
    raw_bytes: u64,
    packed_db_bytes: u64,
    payload_bytes: u64,
    per_class_payload: HashMap<String, (u64, u64)>,
}

async fn db_bytes(storage: &Storage) -> u64 {
    storage
        .with_conn(|conn| {
            let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
            let free: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
            let size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
            Ok(((pages - free) * size) as u64)
        })
        .await
        .unwrap()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Every,
    Banded,
    Tuned,
}

async fn run(mode: Mode) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(dir.path().join("bench.db").to_str().unwrap()).unwrap();
    let empty = db_bytes(&storage).await;

    let mut rng = StdRng::seed_from_u64(42);
    let mut signals: Vec<Signal> = Vec::new();
    let mut rules: HashMap<(String, String), HistoryRule> = HashMap::new();
    let mut names: Vec<(String, String)> = Vec::new();
    for (class, count, band, heartbeat) in CLASSES {
        // "Tuned": an engineer who looked at the trends sets the band above
        // the noise of the noisy tags and thins the pulse counters.
        let band = match (mode, class) {
            (Mode::Tuned, Class::Noisy) => 0.2,
            (Mode::Tuned, Class::Counter) => 0.01,
            _ => band,
        };
        for _ in 0..count {
            let i = signals.len();
            let key = (format!("plc{:02}", i / 50), format!("tag{i:04}"));
            let sig = Signal::new(class, &mut rng);
            rules.insert(
                key.clone(),
                HistoryRule {
                    kind: DeadbandKind::Absolute,
                    deadband: band * sig.span(),
                    max_interval_secs: Some(heartbeat),
                },
            );
            names.push(key);
            signals.push(sig);
        }
    }
    assert_eq!(signals.len(), TAGS);

    let mut filter = HistoryFilter::new();
    if mode != Mode::Every {
        filter.set_rules(rules);
    }
    let mut samples = 0u64;
    let mut kept_per_class: HashMap<String, u64> = HashMap::new();
    for t in 0..SECONDS {
        let ts = BASE + t;
        let mut batch = Vec::with_capacity(TAGS);
        for (i, sig) in signals.iter_mut().enumerate() {
            let v = sig.next(t, &mut rng);
            batch.push((names[i].0.clone(), ts, names[i].1.clone(), v));
        }
        let kept = filter.filter(batch);
        for r in &kept {
            let i: usize = r.2[3..].parse().unwrap();
            *kept_per_class
                .entry(format!("{:?}", signals[i].class))
                .or_default() += 1;
        }
        samples += kept.len() as u64;
        storage.insert_batch(&kept).await.unwrap();
    }
    let raw_bytes = db_bytes(&storage).await - empty;

    let report = storage
        .pack_history(BASE + SECONDS, usize::MAX)
        .await
        .unwrap();
    assert_eq!(report.samples, samples);
    storage
        .with_conn(|c| Ok(c.execute_batch("VACUUM")?))
        .await
        .unwrap();
    let packed_db_bytes = db_bytes(&storage).await - empty;
    let stats = storage.get_compression_stats().await.unwrap();

    // Payload per class, from the blocks themselves.
    let class_of: HashMap<String, String> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.1.clone(), format!("{:?}", signals[i].class)))
        .collect();
    let per_tag: Vec<(String, i64)> = storage
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT metric, SUM(LENGTH(data)) FROM telemetry_blocks GROUP BY metric",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap();
    let mut per_class_payload: HashMap<String, (u64, u64)> = HashMap::new();
    for (metric, bytes) in per_tag {
        let class = class_of[&metric].clone();
        per_class_payload.entry(class).or_default().0 += bytes as u64;
    }
    for (class, kept) in kept_per_class {
        per_class_payload.entry(class).or_default().1 = kept;
    }

    Outcome {
        samples,
        raw_bytes,
        packed_db_bytes,
        payload_bytes: stats.packed.payload_bytes as u64,
        per_class_payload,
    }
}

fn gib(bytes: f64) -> f64 {
    bytes / (1024.0 * 1024.0 * 1024.0)
}

fn report(title: &str, o: &Outcome) {
    let n = o.samples as f64;
    println!("\n=== {title} ===");
    println!(
        "samples written: {} ({:.0} per tag-hour)",
        o.samples,
        n / TAGS as f64
    );
    println!(
        "raw rows in SQLite:   {:>6.2} bytes/sample ({:.1} MB)",
        o.raw_bytes as f64 / n,
        o.raw_bytes as f64 / 1e6
    );
    println!(
        "packed block payload: {:>6.2} bytes/sample ({:.2} MB)",
        o.payload_bytes as f64 / n,
        o.payload_bytes as f64 / 1e6
    );
    println!(
        "packed DB file:       {:>6.2} bytes/sample ({:.2} MB, incl. keys, pages)",
        o.packed_db_bytes as f64 / n,
        o.packed_db_bytes as f64 / 1e6
    );
    println!(
        "compression vs raw rows: {:.1}x on payload, {:.1}x on file",
        o.raw_bytes as f64 / o.payload_bytes as f64,
        o.raw_bytes as f64 / o.packed_db_bytes as f64
    );
    let mut classes: Vec<_> = o.per_class_payload.iter().collect();
    classes.sort();
    for (class, (bytes, kept)) in classes {
        let per_tag_hour = *kept as f64 / class_tags(class) as f64;
        let bytes_per_sample = *bytes as f64 / *kept as f64;
        println!(
            "  {class:<12} {bytes_per_sample:>5.2} bytes/sample, {per_tag_hour:>6.0} samples per tag-hour, {:>7.1} KB per tag-year",
            bytes_per_sample * per_tag_hour * 8760.0 / 1024.0
        );
    }
    let tag_hours_year = 10_000.0 * 8760.0;
    let per_tag_hour_samples = n / TAGS as f64;
    let year_samples = per_tag_hour_samples * tag_hours_year;
    println!(
        "10 000 tags x 1 year: {:.3e} samples -> {:.0} GB payload, {:.0} GB as database file",
        year_samples,
        gib(year_samples * o.payload_bytes as f64 / n),
        gib(year_samples * o.packed_db_bytes as f64 / n)
    );
}

fn class_tags(name: &str) -> usize {
    CLASSES
        .iter()
        .find(|c| format!("{:?}", c.0) == name)
        .map(|c| c.1)
        .unwrap()
}

#[tokio::test]
#[ignore = "benchmark: writes a few hundred MB, run with --ignored --nocapture"]
async fn size_of_one_hour_of_1000_tags() {
    let every = run(Mode::Every).await;
    report("every sample stored (1 s polling, no deadband)", &every);
    let banded = run(Mode::Banded).await;
    report(
        "history deadband (0.5 % of span, 300 s heartbeat; counters/digitals: changes only)",
        &banded,
    );
    let tuned = run(Mode::Tuned).await;
    report(
        "deadband tuned to the signals (noisy tags banded above their noise, counters 10 counts)",
        &tuned,
    );
}
