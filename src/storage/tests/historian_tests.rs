//! Packed history: readers must not be able to tell raw rows from blocks.

use super::*;
use crate::storage::{ArchiveSink, ExpireReport, PackReport, Sample};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::BTreeMap;

/// Hour-aligned start of the generated data.
const BASE: i64 = 1_699_999_200;
const HOUR: i64 = 3600;

type Row = (String, i64, String, f64);

/// Six hours of five tags, polled every 5 s, in the order a poller writes
/// them, plus duplicated timestamps and a digital tag that changes rarely.
fn generate(seed: u64) -> Vec<Row> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut rows = Vec::new();
    let mut temp = 21.0f64;
    let mut counter = 1000.0f64;
    let mut running = true;
    for step in 0..(6 * HOUR / 5) {
        let ts = BASE + step * 5;
        temp += rng.random_range(-0.05..0.05);
        counter += rng.random_range(0..3) as f64;
        if rng.random_bool(0.003) {
            running = !running;
        }
        rows.push(("d1".into(), ts, "temp".into(), temp));
        rows.push(("d1".into(), ts, "run".into(), running as i64 as f64));
        rows.push(("d1".into(), ts, "setpoint".into(), 22.5));
        rows.push(("d1".into(), ts, "count".into(), counter));
        rows.push((
            "d2".into(),
            ts,
            "temp".into(),
            temp * 0.5 + rng.random_range(0.0..1.0),
        ));
        // A re-read of the same snapshot: same timestamp, same tag.
        if step % 997 == 0 {
            rows.push(("d1".into(), ts, "temp".into(), temp + 0.5));
        }
    }
    rows
}

async fn raw_rows(storage: &Storage) -> i64 {
    let conn = storage.conn.lock().unwrap();
    conn.query_row("SELECT COUNT(*) FROM telemetry", [], |r| r.get(0))
        .unwrap()
}

async fn block_count(storage: &Storage) -> i64 {
    let conn = storage.conn.lock().unwrap();
    conn.query_row("SELECT COUNT(*) FROM telemetry_blocks", [], |r| r.get(0))
        .unwrap()
}

/// Everything a reader can observe about full-resolution history, rendered
/// to strings so two snapshots compare exactly (floats by bit pattern).
async fn snapshot(storage: &Storage, from: i64, to: i64) -> Vec<String> {
    fn bits(v: f64) -> u64 {
        v.to_bits()
    }
    let mut out = Vec::new();

    for (dev, metric) in [
        ("d1", "temp"),
        ("d1", "run"),
        ("d1", "count"),
        ("d2", "temp"),
    ] {
        let h = storage.query_history(dev, metric, from, to).await.unwrap();
        out.push(format!(
            "history {dev}/{metric} {:?}",
            h.iter()
                .map(|p| (p.timestamp, bits(p.value)))
                .collect::<Vec<_>>()
        ));
        let v = storage
            .get_metric_values_for_range(dev, metric, from, to)
            .await
            .unwrap();
        out.push(format!(
            "values {dev}/{metric} {:?}",
            v.iter().map(|(t, x)| (*t, bits(*x))).collect::<Vec<_>>()
        ));
        let m = storage
            .query_history_multi(dev, from, to, Some(metric))
            .await
            .unwrap();
        out.push(format!(
            "multi {dev}/{metric} {:?}",
            m.iter()
                .map(|p| (p.timestamp, bits(p.value)))
                .collect::<Vec<_>>()
        ));
        for interval in [60, 3600] {
            let a = storage
                .aggregate_raw(dev, metric, from, to, interval)
                .await
                .unwrap();
            out.push(format!(
                "agg {dev}/{metric}/{interval} {:?}",
                a.iter()
                    .map(|(b, m)| (
                        *b,
                        bits(m.min),
                        bits(m.max),
                        bits(m.avg),
                        m.count,
                        m.first_ts,
                        m.last_ts
                    ))
                    .collect::<Vec<_>>()
            ));
        }
        let s = storage
            .aggregate_single_period(dev, metric, from, to)
            .await
            .unwrap();
        out.push(format!(
            "single {dev}/{metric} {:?}",
            (
                bits(s.min),
                bits(s.max),
                bits(s.avg),
                s.count,
                s.first_ts,
                s.last_ts
            )
        ));
    }

    // The whole-window readers: bucketed (one width finer than the data, one
    // coarser) and the export reader.
    for width in [1, 90] {
        let series = storage
            .query_device_series("d1", from, to, width, None)
            .await
            .unwrap();
        out.push(format!(
            "series/{width} {:?}",
            series
                .iter()
                .map(|(m, buckets)| (
                    m.clone(),
                    buckets
                        .iter()
                        .map(|b| (
                            b.start,
                            bits(b.avg),
                            bits(b.min),
                            bits(b.max),
                            bits(b.last),
                            b.last_ts,
                            b.count
                        ))
                        .collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>()
        ));
    }
    let samples = storage
        .query_device_samples("d1", from, to, usize::MAX)
        .await
        .unwrap();
    out.push(format!(
        "samples {:?}",
        samples
            .iter()
            .map(|(t, m, v)| (*t, m.clone(), bits(*v)))
            .collect::<Vec<_>>()
    ));

    for (dev, metric) in [
        (None, None),
        (Some("d1"), None),
        (Some("d1"), Some("temp")),
        (Some("d2"), Some("temp")),
    ] {
        out.push(format!(
            "count {dev:?}/{metric:?} {}",
            storage
                .count_telemetry(dev, metric, Some(from), Some(to))
                .await
                .unwrap()
        ));
        for (limit, offset) in [(40, 0), (25, 3), (60, 7000), (5, 100_000)] {
            let (rows, total) = storage
                .query_telemetry_export(
                    dev,
                    metric,
                    Some(from),
                    Some(to),
                    None,
                    None,
                    limit,
                    offset,
                )
                .await
                .unwrap();
            out.push(format!(
                "export {dev:?}/{metric:?} {limit}/{offset} total={total} {:?}",
                rows.iter()
                    .map(|(t, d, m, v)| (*t, d.clone(), m.clone(), bits(*v)))
                    .collect::<Vec<_>>()
            ));
        }
    }
    out.push(format!(
        "devices {:?} metrics {:?}",
        storage.get_telemetry_device_ids().await.unwrap(),
        storage.get_telemetry_metrics("d1").await.unwrap()
    ));
    out
}

fn assert_same(label: &str, expected: &[String], actual: &[String]) {
    assert_eq!(expected.len(), actual.len(), "{label}: snapshot size");
    for (e, a) in expected.iter().zip(actual) {
        assert_eq!(e, a, "{label}: reader output differs");
    }
}

#[tokio::test]
async fn packing_leaves_every_reader_output_unchanged() {
    let storage = Storage::new_in_memory().unwrap();
    let rows = generate(1);
    storage.insert_batch(&rows).await.unwrap();
    let total = rows.len() as i64;

    let windows = [
        (BASE, BASE + 6 * HOUR),
        (BASE + 2 * HOUR + 17, BASE + 4 * HOUR + 41),
        (BASE + HOUR - 10, BASE + HOUR + 10),
        (BASE + 5 * HOUR, BASE + 6 * HOUR),
    ];
    let mut expected = Vec::new();
    for (f, t) in windows {
        expected.push(snapshot(&storage, f, t).await);
    }
    assert_eq!(raw_rows(&storage).await, total);
    assert_eq!(block_count(&storage).await, 0);

    // Three hours packed, three still raw: window 2 reads across the edge.
    let report = storage
        .pack_history(BASE + 3 * HOUR, usize::MAX)
        .await
        .unwrap();
    assert!(!report.more);
    assert_eq!(report.buckets, 5 * 3, "five tags x three hours");
    assert!(raw_rows(&storage).await < total);
    assert_eq!(block_count(&storage).await, 5 * 3);
    for (i, (f, t)) in windows.iter().enumerate() {
        let now = snapshot(&storage, *f, *t).await;
        assert_same(&format!("half packed, window {i}"), &expected[i], &now);
    }

    // Everything packed.
    let report = storage
        .pack_history(BASE + 6 * HOUR, usize::MAX)
        .await
        .unwrap();
    assert_eq!(report.buckets, 5 * 3);
    assert_eq!(raw_rows(&storage).await, 0);
    for (i, (f, t)) in windows.iter().enumerate() {
        let now = snapshot(&storage, *f, *t).await;
        assert_same(&format!("fully packed, window {i}"), &expected[i], &now);
    }

    // Idempotent: nothing left to do.
    let again = storage
        .pack_history(BASE + 6 * HOUR, usize::MAX)
        .await
        .unwrap();
    assert_eq!(again, PackReport::default());

    // The packed form is far smaller than the rows it replaced.
    let stats = storage.get_compression_stats().await.unwrap();
    assert_eq!(stats.packed.samples, total);
    assert!(
        (stats.packed.payload_bytes as f64) < total as f64 * 3.0,
        "{} bytes for {total} samples",
        stats.packed.payload_bytes
    );
}

#[tokio::test]
async fn incremental_packing_gives_the_same_result_as_one_pass() {
    let rows = generate(2);

    let whole = Storage::new_in_memory().unwrap();
    whole.insert_batch(&rows).await.unwrap();
    whole
        .pack_history(BASE + 6 * HOUR, usize::MAX)
        .await
        .unwrap();

    // One tag-hour per call, as an interrupted-and-resumed worker would.
    let steps = Storage::new_in_memory().unwrap();
    steps.insert_batch(&rows).await.unwrap();
    let expected = snapshot(&steps, BASE, BASE + 6 * HOUR).await;
    let mut calls = 0;
    loop {
        let report = steps.pack_history(BASE + 6 * HOUR, 1).await.unwrap();
        calls += 1;
        // Between any two steps the readers see the same history.
        if calls % 4 == 0 {
            assert_same(
                &format!("after {calls} steps"),
                &expected,
                &snapshot(&steps, BASE, BASE + 6 * HOUR).await,
            );
        }
        if !report.more {
            break;
        }
        assert_eq!(report.buckets, 1);
    }
    assert_eq!(
        calls,
        5 * 6 + 1,
        "one call per tag-hour, then one that finds nothing"
    );

    let a = snapshot(&whole, BASE, BASE + 6 * HOUR).await;
    let b = snapshot(&steps, BASE, BASE + 6 * HOUR).await;
    assert_same("one pass vs steps", &a, &b);
    assert_eq!(block_count(&whole).await, block_count(&steps).await);
}

#[tokio::test]
async fn late_samples_merge_into_packed_hours() {
    let storage = Storage::new_in_memory().unwrap();
    let mut model: Vec<Row> = generate(3);
    storage.insert_batch(&model).await.unwrap();
    storage
        .pack_history(BASE + 6 * HOUR, usize::MAX)
        .await
        .unwrap();
    assert_eq!(raw_rows(&storage).await, 0);

    // An import fills a gap and repeats a timestamp that is already packed.
    let late: Vec<Row> = vec![
        ("d1".into(), BASE + 12, "temp".into(), 99.0), // between two packed samples
        ("d1".into(), BASE + 15, "temp".into(), 98.0), // same ts as a packed sample
        ("d1".into(), BASE + 2 * HOUR + 7, "temp".into(), 97.0),
        ("d3".into(), BASE + 40, "new".into(), 1.0), // a tag that has no blocks yet
    ];
    storage.insert_batch(&late).await.unwrap();
    model.extend(late.clone());

    // Expected history of d1/temp from the model: ts ascending, ties in write order.
    let expect_series = |dev: &str, metric: &str| -> Vec<(i64, u64)> {
        let mut v: Vec<(i64, u64)> = model
            .iter()
            .filter(|r| r.0 == dev && r.2 == metric)
            .map(|r| (r.1, r.3.to_bits()))
            .collect();
        v.sort_by_key(|x| x.0); // stable
        v
    };
    let check = |label: &'static str| {
        let storage = storage.clone();
        let want = expect_series("d1", "temp");
        async move {
            let got: Vec<(i64, u64)> = storage
                .query_history("d1", "temp", BASE, BASE + 6 * HOUR)
                .await
                .unwrap()
                .iter()
                .map(|p| (p.timestamp, p.value.to_bits()))
                .collect();
            assert_eq!(got, want, "{label}");
            let new = storage
                .query_history("d3", "new", 0, i64::MAX)
                .await
                .unwrap();
            assert_eq!(new.len(), 1, "{label}");
        }
    };
    check("late rows read next to blocks").await;
    assert_eq!(raw_rows(&storage).await, 4);

    // The next compaction folds them into the blocks.
    let report = storage
        .pack_history(BASE + 6 * HOUR, usize::MAX)
        .await
        .unwrap();
    // d1/temp hour 0, d1/temp hour 2 and d3/new.
    assert_eq!(report.buckets, 3);
    assert_eq!(report.samples, 4);
    assert_eq!(raw_rows(&storage).await, 0);
    check("late rows folded into blocks").await;
    let count = storage
        .count_telemetry(Some("d1"), Some("temp"), None, None)
        .await
        .unwrap();
    assert_eq!(count as usize, expect_series("d1", "temp").len());
}

#[tokio::test]
async fn long_windows_are_thinned_over_their_whole_length_not_cut_off() {
    // More samples than the 10 000 point cap; one spike and one dip hidden in
    // the middle. Half of the data ends up packed.
    let rows: Vec<Row> = (0..12_000)
        .map(|i| {
            let v = match i {
                6_001 => 500.0,
                6_002 => -500.0,
                _ => (i % 50) as f64,
            };
            ("d".into(), BASE + i, "x".into(), v)
        })
        .collect();
    let end = BASE + 11_999;
    let raw = Storage::new_in_memory().unwrap();
    raw.insert_batch(&rows).await.unwrap();
    let packed = Storage::new_in_memory().unwrap();
    packed.insert_batch(&rows).await.unwrap();
    packed
        .pack_history(BASE + 2 * HOUR, usize::MAX)
        .await
        .unwrap();
    assert!(block_count(&packed).await > 0);
    assert!(raw_rows(&packed).await > 0, "the last hour stays raw");

    for storage in [&raw, &packed] {
        // A tag's history: thinned, ends where the window ends, keeps the extremes.
        let h = storage.query_history("d", "x", BASE, end).await.unwrap();
        assert!(h.len() <= 10_000 && h.len() > 5_000, "{} points", h.len());
        assert_eq!(h[0].timestamp, BASE);
        assert_eq!(h.last().unwrap().timestamp, end);
        assert!(h.windows(2).all(|w| w[0].timestamp < w[1].timestamp));
        assert!(h.iter().any(|p| p.value == 500.0) && h.iter().any(|p| p.value == -500.0));

        // The report reader: same rule with a 5000 cap.
        let m = storage
            .query_history_multi("d", BASE, end, Some("x"))
            .await
            .unwrap();
        assert!(m.len() <= 5_000 && m.len() > 2_500, "{} points", m.len());
        assert_eq!(m.last().unwrap().timestamp, end);
        assert_eq!(m.iter().map(|p| p.value).fold(f64::MIN, f64::max), 500.0);
        assert_eq!(m.iter().map(|p| p.value).fold(f64::MAX, f64::min), -500.0);

        // The statistics reader: every k-th sample plus the last.
        let v = storage
            .get_metric_values_for_range("d", "x", BASE, end)
            .await
            .unwrap();
        assert!(v.len() <= 10_001 && v.len() > 5_000, "{} points", v.len());
        assert_eq!((v[0].0, v.last().unwrap().0), (BASE, end));

        // A window under the cap comes back untouched.
        let small = storage
            .query_history("d", "x", BASE, BASE + 99)
            .await
            .unwrap();
        assert_eq!(small.len(), 100);

        // Without a tag filter the cap is a refusal, not a short answer.
        assert!(
            storage
                .query_history_multi("d", BASE, end, None)
                .await
                .is_err()
        );
    }
    // Raw and packed give the same thinned answer.
    assert_eq!(
        raw.query_history("d", "x", BASE, end).await.unwrap(),
        packed.query_history("d", "x", BASE, end).await.unwrap()
    );
}

#[tokio::test]
async fn bucketed_series_cover_the_window_across_every_tier() {
    // 3 hours of one tag at 10 s: the first hour only as a daily row, the
    // second only as an hourly row, the rest as raw samples.
    let storage = Storage::new_in_memory().unwrap();
    let day = BASE / 86_400 * 86_400;
    let rows: Vec<Row> = (0..3 * 360)
        .map(|i| {
            (
                "d".into(),
                day + 86_400 + HOUR * 2 + i * 10,
                "x".into(),
                7.0,
            )
        })
        .collect();
    storage.insert_batch(&rows).await.unwrap();
    storage
        .with_conn(move |c| {
            for (table, start, avg, count) in [
                ("telemetry_daily", day, 1.0, 8640),
                ("telemetry_hourly", day + 86_400, 3.0, 360),
            ] {
                c.execute(
                    &format!("INSERT INTO {table} (device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count) VALUES ('d','x',?1,?2,?2,?2,?3)"),
                    rusqlite::params![start, avg, count],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let (from, to) = (day, day + 86_400 + HOUR * 5);
    let series = storage
        .query_device_series("d", from, to, 3_600, None)
        .await
        .unwrap();
    let buckets = &series["x"];
    // One daily row, one hourly row and three hours of raw, each tier only for
    // the time before the next one starts.
    assert_eq!(buckets.first().unwrap().start, day);
    assert_eq!(buckets.first().unwrap().avg, 1.0);
    assert_eq!(buckets.first().unwrap().count, 8640);
    assert_eq!(buckets.last().unwrap().start, day + 86_400 + HOUR * 4);
    assert_eq!(buckets.last().unwrap().last, 7.0);
    let starts: Vec<i64> = buckets.iter().map(|b| b.start).collect();
    assert_eq!(
        starts,
        vec![
            day,
            day + 86_400,
            day + 86_400 + HOUR * 2,
            day + 86_400 + HOUR * 3,
            day + 86_400 + HOUR * 4
        ]
    );
    let total: u64 = buckets.iter().map(|b| b.count).sum();
    assert_eq!(total, 8640 + 360 + 3 * 360);

    // A coarse bucket folds the tiers together with count-weighted means.
    let one = storage
        .query_device_series("d", from, to, 2_592_000, None)
        .await
        .unwrap();
    assert_eq!(one["x"].len(), 1);
    let b = &one["x"][0];
    let expected = (1.0 * 8640.0 + 3.0 * 360.0 + 7.0 * 1080.0) / (8640.0 + 360.0 + 1080.0);
    assert!((b.avg - expected).abs() < 1e-9, "{} vs {expected}", b.avg);
    assert_eq!((b.min, b.max), (1.0, 7.0));
}

#[tokio::test]
async fn deleting_a_range_trims_blocks_at_both_edges() {
    let storage = Storage::new_in_memory().unwrap();
    let rows = generate(4);
    storage.insert_batch(&rows).await.unwrap();
    storage
        .pack_history(BASE + 6 * HOUR, usize::MAX)
        .await
        .unwrap();

    // From the middle of hour 1 to the middle of hour 3: trims two blocks,
    // drops the one between them whole.
    let (from, to) = (BASE + HOUR + 1000, BASE + 3 * HOUR - 1000);
    let (deleted, _, _) = storage
        .delete_telemetry_range("d1", Some("temp"), from, to)
        .await
        .unwrap();
    let want: Vec<&Row> = rows
        .iter()
        .filter(|r| r.0 == "d1" && r.2 == "temp" && r.1 >= from && r.1 <= to)
        .collect();
    assert_eq!(deleted as usize, want.len());

    let got = storage
        .query_history("d1", "temp", BASE, BASE + 6 * HOUR)
        .await
        .unwrap();
    let mut expect: Vec<(i64, u64)> = rows
        .iter()
        .filter(|r| r.0 == "d1" && r.2 == "temp" && !(r.1 >= from && r.1 <= to))
        .map(|r| (r.1, r.3.to_bits()))
        .collect();
    expect.sort_by_key(|x| x.0);
    let got: Vec<(i64, u64)> = got
        .iter()
        .map(|p| (p.timestamp, p.value.to_bits()))
        .collect();
    assert_eq!(got, expect);

    // Other tags are untouched.
    let other = storage
        .count_telemetry(Some("d1"), Some("run"), None, None)
        .await
        .unwrap();
    assert_eq!(other, 6 * HOUR / 5);

    // Deleting a range that contains no sample of a block leaves it alone.
    let blocks = block_count(&storage).await;
    storage
        .delete_telemetry_range("d1", Some("temp"), BASE - 500, BASE - 100)
        .await
        .unwrap();
    assert_eq!(block_count(&storage).await, blocks);

    // Whole-device delete clears every tag of d2.
    storage
        .delete_telemetry_range("d2", None, BASE, BASE + 6 * HOUR)
        .await
        .unwrap();
    assert_eq!(
        storage
            .count_telemetry(Some("d2"), None, None, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        storage.get_telemetry_metrics("d2").await.unwrap(),
        Vec::<String>::new()
    );
}

/// Collects what the archive hook sees.
#[derive(Default)]
struct Collect {
    hours: Vec<i64>,
    rows: BTreeMap<(String, String), Vec<Sample>>,
    commits: usize,
}

struct CollectSink(std::sync::Arc<std::sync::Mutex<Collect>>);

impl ArchiveSink for CollectSink {
    fn begin(&mut self, bucket_start: i64) -> anyhow::Result<()> {
        self.0.lock().unwrap().hours.push(bucket_start);
        Ok(())
    }
    fn series(&mut self, d: &str, m: &str, s: &[Sample]) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .rows
            .entry((d.to_string(), m.to_string()))
            .or_default()
            .extend_from_slice(s);
        Ok(())
    }
    fn commit(&mut self) -> anyhow::Result<()> {
        self.0.lock().unwrap().commits += 1;
        Ok(())
    }
}

async fn hourly(storage: &Storage) -> Vec<(String, String, i64, u64, u64, u64, i64)> {
    let conn = storage.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count
             FROM telemetry_hourly ORDER BY device_id, metric, bucket_start",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get::<_, f64>(3)?.to_bits(),
            r.get::<_, f64>(4)?.to_bits(),
            r.get::<_, f64>(5)?.to_bits(),
            r.get(6)?,
        ))
    })
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap()
}

#[tokio::test]
async fn expiry_archives_aggregates_and_removes_full_resolution_hours() {
    let rows = generate(5);

    // The hourly tier the old way: aggregate raw rows, no packing involved.
    let reference = Storage::new_in_memory().unwrap();
    reference.insert_batch(&rows).await.unwrap();
    reference
        .downsample_raw_to_hourly(BASE + 3 * HOUR)
        .await
        .unwrap();

    let storage = Storage::new_in_memory().unwrap();
    storage.insert_batch(&rows).await.unwrap();
    // Half of it packed already, half still raw when expiry runs.
    storage
        .pack_history(BASE + 2 * HOUR, usize::MAX)
        .await
        .unwrap();

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Collect::default()));
    let report = storage
        .expire_full_resolution(
            BASE + 3 * HOUR + 123,
            Some(Box::new(CollectSink(seen.clone()))),
        )
        .await
        .unwrap();
    assert_eq!(report.buckets, 5 * 3);

    // Same hourly rows as aggregating the raw data directly, bit for bit.
    assert_eq!(hourly(&storage).await, hourly(&reference).await);

    // The archive got exactly the expired samples, hour by hour.
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.hours, vec![BASE, BASE + HOUR, BASE + 2 * HOUR]);
        assert_eq!(seen.commits, 3);
        let archived: usize = seen.rows.values().map(Vec::len).sum();
        let want: usize = rows.iter().filter(|r| r.1 < BASE + 3 * HOUR).count();
        assert_eq!(archived, want);
        assert_eq!(report.samples as usize, want);
        for ((dev, metric), samples) in &seen.rows {
            let expect: Vec<(i64, u64)> = rows
                .iter()
                .filter(|r| &r.0 == dev && &r.2 == metric && r.1 < BASE + 3 * HOUR)
                .map(|r| (r.1, r.3.to_bits()))
                .collect();
            let got: Vec<(i64, u64)> = samples.iter().map(|s| (s.0, s.1.to_bits())).collect();
            assert_eq!(got, expect, "{dev}/{metric}");
        }
    }

    // Expired hours are gone from full resolution; the rest is intact.
    assert_eq!(
        storage
            .count_telemetry(None, None, Some(BASE), Some(BASE + 3 * HOUR - 1))
            .await
            .unwrap(),
        0
    );
    let kept = storage
        .query_history("d1", "temp", BASE + 3 * HOUR, BASE + 6 * HOUR)
        .await
        .unwrap();
    let expect = reference
        .query_history("d1", "temp", BASE + 3 * HOUR, BASE + 6 * HOUR)
        .await
        .unwrap();
    assert_eq!(kept, expect);

    // Running it again changes nothing.
    let again = storage
        .expire_full_resolution(BASE + 3 * HOUR, None)
        .await
        .unwrap();
    assert_eq!(again, ExpireReport::default());
}

#[tokio::test]
async fn a_failing_archive_keeps_the_data() {
    struct Failing;
    impl ArchiveSink for Failing {
        fn begin(&mut self, _: i64) -> anyhow::Result<()> {
            Ok(())
        }
        fn series(&mut self, _: &str, _: &str, _: &[Sample]) -> anyhow::Result<()> {
            anyhow::bail!("disk full")
        }
        fn commit(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let storage = Storage::new_in_memory().unwrap();
    storage.insert_batch(&generate(6)).await.unwrap();
    let before = snapshot(&storage, BASE, BASE + 6 * HOUR).await;
    let err = storage
        .expire_full_resolution(BASE + 6 * HOUR, Some(Box::new(Failing)))
        .await;
    assert!(err.is_err());
    assert!(
        hourly(&storage).await.is_empty(),
        "no aggregate without the archive"
    );
    assert_same(
        "after failed archive",
        &before,
        &snapshot(&storage, BASE, BASE + 6 * HOUR).await,
    );
}

#[tokio::test]
async fn hourly_downsampling_is_the_same_for_packed_and_raw_data() {
    let rows = generate(7);
    let raw = Storage::new_in_memory().unwrap();
    raw.insert_batch(&rows).await.unwrap();
    let packed = Storage::new_in_memory().unwrap();
    packed.insert_batch(&rows).await.unwrap();
    packed
        .pack_history(BASE + 4 * HOUR, usize::MAX)
        .await
        .unwrap();

    let a = raw.downsample_raw_to_hourly(BASE + 6 * HOUR).await.unwrap();
    let b = packed
        .downsample_raw_to_hourly(BASE + 6 * HOUR)
        .await
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(a, 5 * 6);
    assert_eq!(hourly(&raw).await, hourly(&packed).await);

    // Deleting full resolution older than a cutoff covers both forms.
    let removed = packed.delete_raw_older_than(BASE + 5 * HOUR).await.unwrap();
    let want = rows.iter().filter(|r| r.1 < BASE + 5 * HOUR).count() as u64;
    assert_eq!(removed, want);
    assert_eq!(
        packed
            .count_telemetry(None, None, Some(BASE), Some(BASE + 5 * HOUR - 1))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn export_pages_are_stable_across_ties_and_packing() {
    // Two devices sample the same timestamps; pages must not repeat or skip.
    let rows: Vec<Row> = (0..200)
        .flat_map(|i| {
            [
                ("a".to_string(), BASE + i * 30, "m".to_string(), i as f64),
                ("b".to_string(), BASE + i * 30, "m".to_string(), -(i as f64)),
                ("a".to_string(), BASE + i * 30, "n".to_string(), 0.5),
            ]
        })
        .collect();
    let raw = Storage::new_in_memory().unwrap();
    raw.insert_batch(&rows).await.unwrap();
    let packed = Storage::new_in_memory().unwrap();
    packed.insert_batch(&rows).await.unwrap();
    packed.pack_history(BASE + HOUR, usize::MAX).await.unwrap();
    assert!(raw_rows(&packed).await > 0 && block_count(&packed).await > 0);

    let mut all_raw = Vec::new();
    let mut all_packed = Vec::new();
    for page in 0..12 {
        let (a, total_a) = raw
            .query_telemetry_export(None, None, None, None, None, None, 50, page * 50)
            .await
            .unwrap();
        let (b, total_b) = packed
            .query_telemetry_export(None, None, None, None, None, None, 50, page * 50)
            .await
            .unwrap();
        assert_eq!(total_a, 600);
        assert_eq!(total_b, 600);
        assert_eq!(a, b, "page {page}");
        all_raw.extend(a);
        all_packed.extend(b);
    }
    assert_eq!(all_raw.len(), 600);
    assert_eq!(all_packed, all_raw);
    // Newest first, then device and metric.
    assert!(all_raw.windows(2).all(|w| {
        let (x, y) = (&w[0], &w[1]);
        (std::cmp::Reverse(x.0), &x.1, &x.2) <= (std::cmp::Reverse(y.0), &y.1, &y.2)
    }));
}

#[tokio::test]
async fn old_database_gets_the_blocks_table_and_packs_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let path = path.to_str().unwrap();

    let rows = generate(8);
    {
        let storage = Storage::new(path).unwrap();
        storage.insert_batch(&rows).await.unwrap();
        // A database from before packing has no such table.
        let conn = storage.conn.lock().unwrap();
        conn.execute_batch("DROP TABLE telemetry_blocks").unwrap();
    }

    let storage = Storage::new(path).unwrap();
    let before = snapshot(&storage, BASE, BASE + 6 * HOUR).await;
    assert_eq!(block_count(&storage).await, 0);
    storage
        .pack_history(BASE + 6 * HOUR, usize::MAX)
        .await
        .unwrap();
    assert_eq!(raw_rows(&storage).await, 0);
    assert_same(
        "old database after packing",
        &before,
        &snapshot(&storage, BASE, BASE + 6 * HOUR).await,
    );
}

#[tokio::test]
async fn the_hot_window_stays_raw() {
    let storage = Storage::new_in_memory().unwrap();
    let rows = generate(9);
    storage.insert_batch(&rows).await.unwrap();
    // Only complete hours that end at or before `before` are packed; the
    // partial hour around it stays as rows.
    let report = storage
        .pack_history(BASE + 2 * HOUR + 1800, usize::MAX)
        .await
        .unwrap();
    assert_eq!(report.buckets, 5 * 2);
    let conn = storage.conn.lock().unwrap();
    let packed_to: i64 = conn
        .query_row("SELECT MAX(last_ts) FROM telemetry_blocks", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(packed_to < BASE + 2 * HOUR);
    let oldest_raw: i64 = conn
        .query_row("SELECT MIN(timestamp) FROM telemetry", [], |r| r.get(0))
        .unwrap();
    assert_eq!(oldest_raw, BASE + 2 * HOUR);
}

#[tokio::test]
async fn long_buckets_use_block_summaries_and_agree_with_decoding() {
    let mut rows = generate(11);
    // A tag with a non-finite sample has no summary and is always decoded.
    rows.push(("d1".into(), BASE + 100, "odd".into(), 1.0));
    rows.push(("d1".into(), BASE + 101, "odd".into(), f64::INFINITY));
    rows.push(("d1".into(), BASE + 102, "odd".into(), 3.0));
    let storage = Storage::new_in_memory().unwrap();
    storage.insert_batch(&rows).await.unwrap();
    storage
        .pack_history(BASE + 6 * HOUR, usize::MAX)
        .await
        .unwrap();

    let summarised = |s: &Storage| {
        let s = s.clone();
        async move {
            s.with_conn(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM telemetry_blocks WHERE sum_v IS NOT NULL",
                    [],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .await
            .unwrap()
        }
    };
    assert_eq!(
        summarised(&storage).await,
        5 * 6,
        "every finite block has a summary"
    );

    let width = 86_400;
    let with_summaries = storage
        .query_device_series("d1", BASE, BASE + 6 * HOUR, width, None)
        .await
        .unwrap();
    // Forget the summaries, as for blocks packed before they existed.
    storage
        .with_conn(|c| {
            c.execute_batch("UPDATE telemetry_blocks SET min_v = NULL, max_v = NULL, sum_v = NULL, last_v = NULL")?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(summarised(&storage).await, 0);
    let decoded = storage
        .query_device_series("d1", BASE, BASE + 6 * HOUR, width, None)
        .await
        .unwrap();

    assert_eq!(with_summaries.len(), decoded.len());
    for (metric, a) in &with_summaries {
        let b = &decoded[metric];
        assert_eq!(a.len(), b.len(), "{metric}");
        for (x, y) in a.iter().zip(b) {
            assert_eq!(
                (x.start, x.min, x.max, x.last, x.last_ts, x.count),
                (y.start, y.min, y.max, y.last, y.last_ts, y.count),
                "{metric}"
            );
            assert!(
                (x.avg - y.avg).abs() <= 1e-9 * y.avg.abs().max(1.0),
                "{metric}: {} vs {}",
                x.avg,
                y.avg
            );
        }
    }
    // The tag with an infinity counts its two finite samples.
    assert_eq!(decoded["odd"][0].count, 2);
    assert_eq!(decoded["odd"][0].max, 3.0);

    // Deleting inside a block rewrites its summary too.
    storage
        .pack_history(BASE + 6 * HOUR, usize::MAX)
        .await
        .unwrap();
    storage
        .delete_telemetry_range("d1", Some("temp"), BASE + 600, BASE + HOUR + 600)
        .await
        .unwrap();
    let after = storage
        .query_device_series(
            "d1",
            BASE,
            BASE + 6 * HOUR,
            width,
            Some(["temp".to_string()].into()),
        )
        .await
        .unwrap();
    let want = rows
        .iter()
        .filter(|r| {
            r.0 == "d1" && r.2 == "temp" && !(r.1 >= BASE + 600 && r.1 <= BASE + HOUR + 600)
        })
        .count() as u64;
    assert_eq!(after["temp"].iter().map(|b| b.count).sum::<u64>(), want);
}
