//! Packed full-resolution history.
//!
//! Raw samples live as rows of `telemetry` while they are young. The
//! compaction worker moves every complete hour older than the hot window into
//! `telemetry_blocks`: one Gorilla/RLE block per tag and hour (see
//! `compression::block`) and deletes the raw rows of that hour in the same
//! transaction.
//!
//! Everything that reads full-resolution samples goes through the functions
//! here, which merge the raw rows with the decoded blocks so a reader cannot
//! tell whether a sample has been packed yet. Ordering is the same either
//! way: ascending timestamp, and for equal timestamps of one tag the order in
//! which the samples were written (packed samples are always older than the
//! raw rows that share their timestamp).

use super::Storage;
use crate::compression::{MAX_BLOCK_SAMPLES, decode_series, encode_series};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

/// Width of one packed block in seconds. Equal to the hourly tier's bucket so
/// a block is exactly one hourly aggregate.
pub const BUCKET_SECS: i64 = 3600;

pub fn bucket_start(ts: i64) -> i64 {
    ts.div_euclid(BUCKET_SECS).saturating_mul(BUCKET_SECS)
}

/// One `(timestamp, value)` sample.
pub type Sample = (i64, f64);

// ── Reading ──

/// Stream the samples of one tag in `[from, to]`, packed and raw merged,
/// ascending by timestamp (equal timestamps in write order), to `visit`.
/// `visit` returns `false` to stop early. Only one decoded block is held at a
/// time, so a year of history can be aggregated without loading it.
pub(super) fn scan_series(
    conn: &Connection,
    device_id: &str,
    metric: &str,
    from: i64,
    to: i64,
    mut visit: impl FnMut(i64, f64) -> bool,
) -> Result<()> {
    let mut raw_stmt = conn
        .prepare_cached(
            "SELECT timestamp, value FROM telemetry
             WHERE device_id = ?1 AND metric = ?2 AND timestamp >= ?3 AND timestamp <= ?4
             ORDER BY timestamp, id",
        )
        .context("Prepare raw series query failed")?;
    let mut raw = raw_stmt
        .query(params![device_id, metric, from, to])
        .context("Raw series query failed")?;
    let mut raw_next: Option<Sample> = next_raw(&mut raw)?;

    let mut block_stmt = conn
        .prepare_cached(
            "SELECT data FROM telemetry_blocks
             WHERE device_id = ?1 AND metric = ?2
               AND bucket_start >= ?3 AND bucket_start <= ?4
               AND last_ts >= ?5 AND first_ts <= ?6
             ORDER BY bucket_start, seq",
        )
        .context("Prepare packed series query failed")?;
    let mut blocks = block_stmt
        .query(params![device_id, metric, bucket_start(from), to, from, to])
        .context("Packed series query failed")?;

    while let Some(row) = blocks.next().context("Packed series row failed")? {
        let data = row.get_ref(0)?.as_blob().context("Block is not a blob")?;
        for (ts, v) in decode_series(data).context("Corrupt history block")? {
            if ts < from || ts > to {
                continue;
            }
            // Raw rows that precede this packed sample (late arrivals).
            while let Some((rts, rv)) = raw_next {
                if rts >= ts {
                    break;
                }
                if !visit(rts, rv) {
                    return Ok(());
                }
                raw_next = next_raw(&mut raw)?;
            }
            if !visit(ts, v) {
                return Ok(());
            }
        }
    }
    while let Some((rts, rv)) = raw_next {
        if !visit(rts, rv) {
            return Ok(());
        }
        raw_next = next_raw(&mut raw)?;
    }
    Ok(())
}

fn next_raw(rows: &mut rusqlite::Rows<'_>) -> Result<Option<Sample>> {
    match rows.next().context("Raw series row failed")? {
        Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
        None => Ok(None),
    }
}

/// Sample range of a single tag, packed and raw merged, ascending by
/// timestamp. `limit` keeps the first `limit` samples.
pub(super) fn read_series(
    conn: &Connection,
    device_id: &str,
    metric: &str,
    from: i64,
    to: i64,
    limit: Option<usize>,
) -> Result<Vec<Sample>> {
    let cap = limit.unwrap_or(usize::MAX);
    let mut out = Vec::new();
    if cap == 0 {
        return Ok(out);
    }
    scan_series(conn, device_id, metric, from, to, |ts, v| {
        out.push((ts, v));
        out.len() < cap
    })?;
    Ok(out)
}

/// Stable merge of two ascending runs; on equal timestamps `older` comes first.
fn merge_by_timestamp(older: Vec<Sample>, newer: Vec<Sample>, cap: usize) -> Vec<Sample> {
    if older.is_empty() {
        let mut newer = newer;
        newer.truncate(cap);
        return newer;
    }
    if newer.is_empty() {
        let mut older = older;
        older.truncate(cap);
        return older;
    }
    let mut out = Vec::with_capacity((older.len() + newer.len()).min(cap));
    let (mut a, mut b) = (older.into_iter().peekable(), newer.into_iter().peekable());
    while out.len() < cap {
        let take_older = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => x.0 <= y.0,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        out.push(if take_older { a.next() } else { b.next() }.expect("peeked"));
    }
    out
}

/// True when a packed block of `device_id` (and `metric`, when given) overlaps
/// `[from, to]`. Readers use it to keep their plain SQL for unpacked ranges.
pub(super) fn has_packed(
    conn: &Connection,
    device_id: Option<&str>,
    metric: Option<&str>,
    from: i64,
    to: i64,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM telemetry_blocks
         WHERE (?1 IS NULL OR device_id = ?1) AND (?2 IS NULL OR metric = ?2)
           AND bucket_start >= ?3 AND bucket_start <= ?4
           AND last_ts >= ?5 AND first_ts <= ?6)",
        params![device_id, metric, bucket_start(from), to, from, to],
        |row| row.get(0),
    )
    .context("Packed overlap check failed")
}

/// All full-resolution samples of a device in `[from, to]` as
/// `(timestamp, metric, value)`, ascending by timestamp (ties by metric name,
/// then write order). Fails instead of truncating when the range holds more
/// than `max_samples`: callers are exports, and a silently short export is
/// worse than a refused one.
pub(super) fn read_device_all(
    conn: &Connection,
    device_id: &str,
    from: i64,
    to: i64,
    max_samples: usize,
) -> Result<Vec<(i64, String, f64)>> {
    let mut all: Vec<(i64, String, f64)> = Vec::new();
    for metric in metrics_in_range(conn, device_id, from, to)? {
        let mut too_many = false;
        scan_series(conn, device_id, &metric, from, to, |ts, v| {
            if all.len() >= max_samples {
                too_many = true;
                return false;
            }
            all.push((ts, metric.clone(), v));
            true
        })?;
        if too_many {
            bail!(
                "{device_id} holds more than {max_samples} samples between {from} and {to}; narrow the time range"
            );
        }
    }
    // Stable: samples of one metric keep their write order.
    all.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    Ok(all)
}

fn metrics_in_range(conn: &Connection, device_id: &str, from: i64, to: i64) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT metric FROM telemetry_blocks
             WHERE device_id = ?1 AND bucket_start >= ?2 AND bucket_start <= ?3
               AND last_ts >= ?4 AND first_ts <= ?5
             UNION
             SELECT metric FROM telemetry
             WHERE device_id = ?1 AND timestamp >= ?4 AND timestamp <= ?5",
        )
        .context("Prepare metric list failed")?;
    stmt.query_map(
        params![device_id, bucket_start(from), to, from, to],
        |row| row.get(0),
    )
    .context("Metric list query failed")?
    .collect::<Result<Vec<String>, _>>()
    .context("Metric list row mapping failed")
}

/// Samples matching the export filters, newest first, in the canonical order:
/// timestamp descending, then device, metric, and for identical keys the
/// latest write first. `offset` and `limit` page through that order.
pub(super) fn export_page(
    conn: &Connection,
    device_id: Option<&str>,
    metric: Option<&str>,
    from: Option<i64>,
    to: Option<i64>,
    limit: usize,
    offset: usize,
) -> Result<Vec<(i64, String, String, f64)>> {
    let (lo, hi) = (from.unwrap_or(i64::MIN), to.unwrap_or(i64::MAX));
    let want = offset.saturating_add(limit);

    let mut raw_stmt = conn
        .prepare_cached(
            "SELECT timestamp, device_id, metric, value, id FROM telemetry
             WHERE (?1 IS NULL OR device_id = ?1) AND (?2 IS NULL OR metric = ?2)
               AND timestamp >= ?3 AND timestamp <= ?4
             ORDER BY timestamp DESC, device_id, metric, id DESC
             LIMIT ?5",
        )
        .context("Prepare raw export failed")?;
    let mut cand: Vec<ExportRow> = raw_stmt
        .query_map(params![device_id, metric, lo, hi, want as i64], |row| {
            Ok(ExportRow {
                ts: row.get(0)?,
                device: row.get(1)?,
                metric: row.get(2)?,
                value: row.get(3)?,
                // Raw rows are newer than any packed sample of their tag.
                order: (1u64 << 63) | row.get::<_, i64>(4)? as u64,
            })
        })
        .context("Raw export query failed")?
        .collect::<Result<_, _>>()
        .context("Raw export row mapping failed")?;

    if has_packed(conn, device_id, metric, lo, hi)? {
        let mut meta_stmt = conn
            .prepare_cached(
                "SELECT device_id, metric, bucket_start, seq, last_ts FROM telemetry_blocks
                 WHERE (?1 IS NULL OR device_id = ?1) AND (?2 IS NULL OR metric = ?2)
                   AND bucket_start >= ?3 AND bucket_start <= ?4
                   AND last_ts >= ?5 AND first_ts <= ?6
                 ORDER BY last_ts DESC, bucket_start DESC, device_id, metric",
            )
            .context("Prepare packed export failed")?;
        let metas: Vec<(String, String, i64, i64, i64)> = meta_stmt
            .query_map(
                params![device_id, metric, bucket_start(lo), hi, lo, hi],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .context("Packed export query failed")?
            .collect::<Result<_, _>>()
            .context("Packed export row mapping failed")?;
        let mut block_stmt = conn
            .prepare_cached(
                "SELECT data FROM telemetry_blocks
                 WHERE device_id = ?1 AND metric = ?2 AND bucket_start = ?3 AND seq = ?4",
            )
            .context("Prepare block fetch failed")?;

        let mut threshold: Option<i64> = None;
        for (dev, met, bucket, seq, last_ts) in metas {
            // Blocks come newest first: once `want` rows are held and the
            // next block ends before the want-th newest, nothing later can
            // enter the page.
            if let Some(t) = threshold
                && last_ts < t
            {
                break;
            }
            let data: Vec<u8> = block_stmt
                .query_row(params![dev, met, bucket, seq], |row| row.get(0))
                .context("Block fetch failed")?;
            for (i, (ts, v)) in decode_series(&data)
                .context("Corrupt history block")?
                .into_iter()
                .enumerate()
            {
                if ts >= lo && ts <= hi {
                    cand.push(ExportRow {
                        ts,
                        device: dev.clone(),
                        metric: met.clone(),
                        value: v,
                        order: ((seq as u64) << 32) | i as u64,
                    });
                }
            }
            if cand.len() >= want {
                cand.sort_by(ExportRow::canonical);
                cand.truncate(want);
                threshold = cand.last().map(|r| r.ts);
            }
        }
    }

    cand.sort_by(ExportRow::canonical);
    Ok(cand
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|r| (r.ts, r.device, r.metric, r.value))
        .collect())
}

struct ExportRow {
    ts: i64,
    device: String,
    metric: String,
    value: f64,
    /// Write order within one tag; larger is newer.
    order: u64,
}

impl ExportRow {
    fn canonical(a: &Self, b: &Self) -> std::cmp::Ordering {
        b.ts.cmp(&a.ts)
            .then_with(|| a.device.cmp(&b.device))
            .then_with(|| a.metric.cmp(&b.metric))
            .then_with(|| b.order.cmp(&a.order))
    }
}

/// Number of samples matching the filters, packed and raw.
pub(super) fn count_samples(
    conn: &Connection,
    device_id: Option<&str>,
    metric: Option<&str>,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<i64> {
    let (lo, hi) = (from.unwrap_or(i64::MIN), to.unwrap_or(i64::MAX));
    let raw: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM telemetry
             WHERE (?1 IS NULL OR device_id = ?1) AND (?2 IS NULL OR metric = ?2)
               AND timestamp >= ?3 AND timestamp <= ?4",
            params![device_id, metric, lo, hi],
            |row| row.get(0),
        )
        .context("Count raw failed")?;

    // Blocks wholly inside the range are counted from their header; the few
    // that straddle an edge are decoded.
    let inside: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(sample_count), 0) FROM telemetry_blocks
             WHERE (?1 IS NULL OR device_id = ?1) AND (?2 IS NULL OR metric = ?2)
               AND first_ts >= ?3 AND last_ts <= ?4",
            params![device_id, metric, lo, hi],
            |row| row.get(0),
        )
        .context("Count packed failed")?;

    let mut stmt = conn
        .prepare_cached(
            "SELECT data FROM telemetry_blocks
             WHERE (?1 IS NULL OR device_id = ?1) AND (?2 IS NULL OR metric = ?2)
               AND bucket_start >= ?3 AND bucket_start <= ?4
               AND last_ts >= ?5 AND first_ts <= ?6
               AND NOT (first_ts >= ?5 AND last_ts <= ?6)",
        )
        .context("Prepare edge count failed")?;
    let mut rows = stmt
        .query(params![device_id, metric, bucket_start(lo), hi, lo, hi])
        .context("Edge count query failed")?;
    let mut edge = 0i64;
    while let Some(row) = rows.next()? {
        let data = row.get_ref(0)?.as_blob().context("Block is not a blob")?;
        edge += decode_series(data)
            .context("Corrupt history block")?
            .iter()
            .filter(|s| s.0 >= lo && s.0 <= hi)
            .count() as i64;
    }
    Ok(raw + inside + edge)
}

// ── Aggregation ──

/// Running min/max/mean/count with compensated summation, so a mean is the
/// same number whether the samples came from raw rows or from blocks.
#[derive(Debug, Clone, Copy)]
pub struct Agg {
    sum: f64,
    err: f64,
    pub min: f64,
    pub max: f64,
    pub count: u64,
    pub first_ts: i64,
    pub last_ts: i64,
}

impl Agg {
    pub fn new() -> Self {
        Self {
            sum: 0.0,
            err: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            count: 0,
            first_ts: i64::MAX,
            last_ts: i64::MIN,
        }
    }

    fn add_to_sum(&mut self, v: f64) {
        let t = self.sum + v;
        if self.sum.abs() > v.abs() {
            self.err += (self.sum - t) + v;
        } else {
            self.err += (v - t) + self.sum;
        }
        self.sum = t;
    }

    pub fn add(&mut self, ts: i64, v: f64) {
        self.add_to_sum(v);
        if v < self.min {
            self.min = v;
        }
        if v > self.max {
            self.max = v;
        }
        self.count += 1;
        self.first_ts = self.first_ts.min(ts);
        self.last_ts = self.last_ts.max(ts);
    }

    /// Fold in an aggregate of `count` samples (an hourly or daily row).
    pub fn add_aggregate(&mut self, ts: i64, mean: f64, min: f64, max: f64, count: u64) {
        self.add_to_sum(mean * count as f64);
        self.min = self.min.min(min);
        self.max = self.max.max(max);
        self.count += count;
        self.first_ts = self.first_ts.min(ts);
        self.last_ts = self.last_ts.max(ts);
    }

    pub fn mean(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let total = if self.err.is_finite() {
            self.sum + self.err
        } else {
            self.sum
        };
        total / self.count as f64
    }
}

impl Default for Agg {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-hour aggregates of every sample of a tag older than `before`, packed
/// and raw together. Keys are hour starts.
pub(super) fn aggregate_hours_before(
    conn: &Connection,
    device_id: &str,
    metric: &str,
    before: i64,
) -> Result<std::collections::BTreeMap<i64, Agg>> {
    let mut hours: std::collections::BTreeMap<i64, Agg> = std::collections::BTreeMap::new();
    scan_series(
        conn,
        device_id,
        metric,
        i64::MIN,
        before.saturating_sub(1),
        |ts, v| {
            hours.entry(ts / 3600 * 3600).or_default().add(ts, v);
            true
        },
    )?;
    Ok(hours)
}

/// Tags with any full-resolution sample (packed or raw) older than `before`.
pub(super) fn series_before(conn: &Connection, before: i64) -> Result<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT device_id, metric FROM telemetry_blocks WHERE first_ts < ?1
             UNION
             SELECT device_id, metric FROM telemetry WHERE timestamp < ?1
             ORDER BY 1, 2",
        )
        .context("Prepare series list failed")?;
    stmt.query_map(params![before], |row| Ok((row.get(0)?, row.get(1)?)))
        .context("Series list query failed")?
        .collect::<Result<_, _>>()
        .context("Series list row mapping failed")
}

// ── Deleting ──

/// Remove packed samples of `device_id` (all devices when `None`) and
/// `metric` (all when `None`) with `from <= timestamp <= to`. Blocks wholly in
/// the range are dropped, blocks straddling it are rewritten without the
/// removed samples. Returns the number of samples removed.
pub(super) fn delete_packed_range(
    conn: &Connection,
    device_id: Option<&str>,
    metric: Option<&str>,
    from: i64,
    to: i64,
) -> Result<u64> {
    let inside: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(sample_count), 0) FROM telemetry_blocks
             WHERE (?1 IS NULL OR device_id = ?1) AND (?2 IS NULL OR metric = ?2)
               AND first_ts >= ?3 AND last_ts <= ?4",
            params![device_id, metric, from, to],
            |row| row.get(0),
        )
        .context("Count packed range failed")?;
    conn.execute(
        "DELETE FROM telemetry_blocks
         WHERE (?1 IS NULL OR device_id = ?1) AND (?2 IS NULL OR metric = ?2)
           AND first_ts >= ?3 AND last_ts <= ?4",
        params![device_id, metric, from, to],
    )
    .context("Delete packed range failed")?;
    let mut removed = inside as u64;

    // Whatever still overlaps straddles an edge.
    let edge: Vec<(String, String, i64, i64, Vec<u8>)> = {
        let mut stmt = conn
            .prepare(
                "SELECT device_id, metric, bucket_start, seq, data FROM telemetry_blocks
                 WHERE (?1 IS NULL OR device_id = ?1) AND (?2 IS NULL OR metric = ?2)
                   AND bucket_start >= ?3 AND bucket_start <= ?4
                   AND last_ts >= ?5 AND first_ts <= ?4",
            )
            .context("Prepare edge blocks failed")?;
        stmt.query_map(
            params![device_id, metric, bucket_start(from), to, from],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .context("Edge blocks query failed")?
        .collect::<Result<_, _>>()
        .context("Edge blocks row mapping failed")?
    };
    for (dev, met, bucket, seq, data) in edge {
        let samples = decode_series(&data).context("Corrupt history block")?;
        let kept: Vec<Sample> = samples
            .iter()
            .copied()
            .filter(|s| s.0 < from || s.0 > to)
            .collect();
        removed += (samples.len() - kept.len()) as u64;
        if kept.len() == samples.len() {
            continue;
        }
        if kept.is_empty() {
            conn.execute(
                "DELETE FROM telemetry_blocks
                 WHERE device_id = ?1 AND metric = ?2 AND bucket_start = ?3 AND seq = ?4",
                params![dev, met, bucket, seq],
            )?;
        } else {
            conn.execute(
                "UPDATE telemetry_blocks SET first_ts = ?5, last_ts = ?6, sample_count = ?7, data = ?8
                 WHERE device_id = ?1 AND metric = ?2 AND bucket_start = ?3 AND seq = ?4",
                params![
                    dev,
                    met,
                    bucket,
                    seq,
                    kept[0].0,
                    kept[kept.len() - 1].0,
                    kept.len() as i64,
                    encode_checked(&kept)?
                ],
            )?;
        }
    }
    Ok(removed)
}

// ── Packing ──

/// What one compaction pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PackReport {
    /// Tag-hours packed.
    pub buckets: usize,
    /// Samples moved from raw rows into blocks.
    pub samples: u64,
    /// True when the pass stopped at its budget with work left.
    pub more: bool,
}

/// Encode and verify: a block is only written if it decodes back to exactly
/// the samples it was built from.
fn encode_checked(samples: &[Sample]) -> Result<Vec<u8>> {
    let data = encode_series(samples)?;
    let back = decode_series(&data).context("Packed block does not decode")?;
    if back.len() != samples.len()
        || back
            .iter()
            .zip(samples)
            .any(|(a, b)| a.0 != b.0 || a.1.to_bits() != b.1.to_bits())
    {
        bail!("Packed block does not round-trip; keeping raw rows");
    }
    Ok(data)
}

/// Pack the raw rows of one tag-hour into blocks, merging with blocks already
/// stored for that hour (late samples). One transaction; either the blocks are
/// written and the raw rows gone, or nothing changed.
fn pack_bucket(conn: &Connection, device_id: &str, metric: &str, bucket: i64) -> Result<u64> {
    let tx = conn
        .unchecked_transaction()
        .context("Begin pack tx failed")?;
    let end = bucket + BUCKET_SECS;

    let raw: Vec<Sample> = {
        let mut stmt = tx.prepare_cached(
            "SELECT timestamp, value FROM telemetry
             WHERE device_id = ?1 AND metric = ?2 AND timestamp >= ?3 AND timestamp < ?4
             ORDER BY timestamp, id",
        )?;
        stmt.query_map(params![device_id, metric, bucket, end], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })?
        .collect::<Result<_, _>>()?
    };
    if raw.is_empty() {
        return Ok(0);
    }

    let existing = read_bucket(&tx, device_id, metric, bucket)?;

    let packed_raw = raw.len() as u64;
    let samples = merge_by_timestamp(existing, raw, usize::MAX);

    let encoded: Vec<(i64, i64, usize, Vec<u8>)> = samples
        .chunks(MAX_BLOCK_SAMPLES)
        .map(|chunk| {
            Ok((
                chunk[0].0,
                chunk[chunk.len() - 1].0,
                chunk.len(),
                encode_checked(chunk)?,
            ))
        })
        .collect::<Result<_>>()?;

    tx.execute(
        "DELETE FROM telemetry_blocks WHERE device_id = ?1 AND metric = ?2 AND bucket_start = ?3",
        params![device_id, metric, bucket],
    )?;
    for (seq, (first_ts, last_ts, count, data)) in encoded.iter().enumerate() {
        tx.execute(
            "INSERT INTO telemetry_blocks
                 (device_id, metric, bucket_start, seq, first_ts, last_ts, sample_count, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                device_id,
                metric,
                bucket,
                seq as i64,
                first_ts,
                last_ts,
                *count as i64,
                data
            ],
        )?;
    }
    tx.execute(
        "DELETE FROM telemetry
         WHERE device_id = ?1 AND metric = ?2 AND timestamp >= ?3 AND timestamp < ?4",
        params![device_id, metric, bucket, end],
    )?;
    tx.commit().context("Commit pack tx failed")?;
    Ok(packed_raw)
}

impl Storage {
    /// Pack the raw samples of every complete hour that ends at or before
    /// `before` into blocks. Handles at most `max_buckets` tag-hours (oldest
    /// first per tag) and reports whether work remains. The connection lock is
    /// released between tag-hours so the poller's inserts are never starved.
    pub async fn pack_history(&self, before: i64, max_buckets: usize) -> Result<PackReport> {
        let storage = self.clone();
        tokio::task::spawn_blocking(move || storage.pack_history_blocking(before, max_buckets))
            .await
            .context("Task join failed")?
    }

    pub(super) fn pack_history_blocking(
        &self,
        before: i64,
        max_buckets: usize,
    ) -> Result<PackReport> {
        let limit = bucket_start(before);
        let series: Vec<(String, String)> = {
            let conn = self.lock_conn();
            let mut stmt = conn
                .prepare("SELECT DISTINCT device_id, metric FROM telemetry WHERE timestamp < ?1")
                .context("Prepare pack series list failed")?;
            stmt.query_map(params![limit], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?
        };

        let mut report = PackReport::default();
        for (device_id, metric) in &series {
            loop {
                if report.buckets >= max_buckets {
                    report.more = true;
                    return Ok(report);
                }
                let conn = self.lock_conn();
                let oldest: Option<i64> = conn
                    .query_row(
                        "SELECT MIN(timestamp) FROM telemetry
                         WHERE device_id = ?1 AND metric = ?2 AND timestamp < ?3",
                        params![device_id, metric, limit],
                        |row| row.get(0),
                    )
                    .optional()?
                    .flatten();
                let Some(oldest) = oldest else { break };
                let packed = match pack_bucket(&conn, device_id, metric, bucket_start(oldest)) {
                    Ok(n) => n,
                    Err(e) => {
                        // The transaction rolled back, nothing changed. Leave
                        // this tag for the next pass instead of retrying it
                        // in a loop, and carry on with the others.
                        tracing::error!("Packing {device_id}/{metric} failed: {e:#}");
                        break;
                    }
                };
                drop(conn);
                report.buckets += 1;
                report.samples += packed;
                // std's mutex is not fair: without a pause this thread could
                // take the lock straight back and starve the pollers' inserts.
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(report)
    }
}

// ── Statistics ──

/// Size of the packed store.
#[derive(Debug, Clone, Copy, Default)]
pub struct PackedStats {
    pub blocks: i64,
    pub samples: i64,
    /// Bytes of block payload (excludes SQLite page and key overhead).
    pub payload_bytes: i64,
}

pub(super) fn packed_stats(conn: &Connection) -> Result<PackedStats> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(sample_count), 0), COALESCE(SUM(LENGTH(data)), 0)
         FROM telemetry_blocks",
        [],
        |row| {
            Ok(PackedStats {
                blocks: row.get(0)?,
                samples: row.get(1)?,
                payload_bytes: row.get(2)?,
            })
        },
    )
    .context("Packed stats query failed")
}

// ── Expiry ──

/// Receives full-resolution samples just before they expire, one hour at a
/// time, so they can be archived. The hour's blocks are deleted only after
/// `commit` returned `Ok`.
pub trait ArchiveSink: Send {
    fn begin(&mut self, bucket_start: i64) -> Result<()>;
    fn series(&mut self, device_id: &str, metric: &str, samples: &[Sample]) -> Result<()>;
    fn commit(&mut self) -> Result<()>;
}

/// What one expiry pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExpireReport {
    /// Tag-hours removed.
    pub buckets: usize,
    /// Full-resolution samples removed.
    pub samples: u64,
}

/// Decoded samples of the blocks of one tag-hour, in order.
fn read_bucket(
    conn: &Connection,
    device_id: &str,
    metric: &str,
    bucket: i64,
) -> Result<Vec<Sample>> {
    let mut stmt = conn.prepare_cached(
        "SELECT data FROM telemetry_blocks
         WHERE device_id = ?1 AND metric = ?2 AND bucket_start = ?3
         ORDER BY seq",
    )?;
    let mut rows = stmt.query(params![device_id, metric, bucket])?;
    let mut all = Vec::new();
    while let Some(row) = rows.next()? {
        let data = row.get_ref(0)?.as_blob()?;
        all.extend(decode_series(data).context("Corrupt history block")?);
    }
    Ok(all)
}

impl Storage {
    /// End the full-resolution life of every hour that lies before `cutoff`:
    /// hand its samples to `sink` (if any), write its hourly aggregate, and
    /// delete the samples. `cutoff` is rounded down to an hour. Raw rows older
    /// than that are packed first, so each hour is handled from its block in
    /// one transaction per tag; an interrupted pass resumes where it stopped.
    pub async fn expire_full_resolution(
        &self,
        cutoff: i64,
        sink: Option<Box<dyn ArchiveSink>>,
    ) -> Result<ExpireReport> {
        let storage = self.clone();
        tokio::task::spawn_blocking(move || storage.expire_blocking(cutoff, sink))
            .await
            .context("Task join failed")?
    }

    fn expire_blocking(
        &self,
        cutoff: i64,
        mut sink: Option<Box<dyn ArchiveSink>>,
    ) -> Result<ExpireReport> {
        let limit = bucket_start(cutoff);
        while self.pack_history_blocking(limit, 5000)?.more {}

        let mut report = ExpireReport::default();
        loop {
            let (bucket, series) = {
                let conn = self.lock_conn();
                let bucket: Option<i64> = conn
                    .query_row(
                        "SELECT MIN(bucket_start) FROM telemetry_blocks WHERE bucket_start < ?1",
                        params![limit],
                        |row| row.get(0),
                    )
                    .context("Find expiring hour failed")?;
                let Some(bucket) = bucket else { break };
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT device_id, metric FROM telemetry_blocks WHERE bucket_start = ?1",
                )?;
                let series: Vec<(String, String)> = stmt
                    .query_map(params![bucket], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<_, _>>()?;
                (bucket, series)
            };

            if let Some(sink) = sink.as_mut() {
                sink.begin(bucket)?;
                for (device_id, metric) in &series {
                    let samples = {
                        let conn = self.lock_conn();
                        read_bucket(&conn, device_id, metric, bucket)?
                    };
                    sink.series(device_id, metric, &samples)?;
                }
                sink.commit()?;
            }

            for (device_id, metric) in &series {
                let conn = self.lock_conn();
                let tx = conn
                    .unchecked_transaction()
                    .context("Begin expire tx failed")?;
                let samples = read_bucket(&tx, device_id, metric, bucket)?;
                let mut agg = Agg::new();
                for &(ts, v) in &samples {
                    agg.add(ts, v);
                }
                if agg.count > 0 {
                    super::downsampling::write_hourly(&tx, device_id, metric, bucket, &agg)?;
                }
                tx.execute(
                    "DELETE FROM telemetry_blocks
                     WHERE device_id = ?1 AND metric = ?2 AND bucket_start = ?3",
                    params![device_id, metric, bucket],
                )?;
                tx.commit().context("Commit expire tx failed")?;
                report.buckets += 1;
                report.samples += samples.len() as u64;
            }
        }
        Ok(report)
    }
}

// ── Whole-window reads ──
//
// A reader that wants a window of history for a trend or a statistic does not
// want the first N samples of it: it wants the whole window at a resolution it
// can carry. These two readers give that, so no answer ever stops short of the
// end of the window.

/// Most samples a single-tag read returns before it thins the series.
pub const THIN_CAP: usize = 10_000;

/// How a series that exceeds the cap is thinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thin {
    /// Every k-th sample plus the last: an unbiased sample for statistics.
    Stride,
    /// The lowest and highest sample of every group of k: keeps the extremes.
    Extremes,
}

/// Samples of one tag across the whole of `[from, to]`, ascending. At most
/// `cap` (a little under, for `Extremes`) are returned; a longer series is
/// thinned evenly over the window instead of cut off at the end.
pub(super) fn read_series_thinned(
    conn: &Connection,
    device_id: &str,
    metric: &str,
    from: i64,
    to: i64,
    cap: usize,
    thin: Thin,
) -> Result<Vec<Sample>> {
    let n = count_samples(conn, Some(device_id), Some(metric), Some(from), Some(to))? as usize;
    if n <= cap {
        return read_series(conn, device_id, metric, from, to, None);
    }
    let mut out: Vec<Sample> = Vec::with_capacity(cap + 2);
    let mut idx = 0usize;
    match thin {
        Thin::Stride => {
            let k = n.div_ceil(cap);
            scan_series(conn, device_id, metric, from, to, |ts, v| {
                if idx.is_multiple_of(k) || idx + 1 == n {
                    out.push((ts, v));
                }
                idx += 1;
                true
            })?;
        }
        Thin::Extremes => {
            // Each group of k samples yields its lowest and highest, so the
            // output has about 2n/k samples.
            let k = (2 * n).div_ceil(cap).max(2);
            let mut lo: Option<(usize, Sample)> = None;
            let mut hi: Option<(usize, Sample)> = None;
            let flush = |out: &mut Vec<Sample>,
                         lo: &mut Option<(usize, Sample)>,
                         hi: &mut Option<(usize, Sample)>| {
                if let (Some(l), Some(h)) = (lo.take(), hi.take()) {
                    if l.0 == h.0 {
                        out.push(l.1);
                    } else if l.0 < h.0 {
                        out.extend([l.1, h.1]);
                    } else {
                        out.extend([h.1, l.1]);
                    }
                }
            };
            scan_series(conn, device_id, metric, from, to, |ts, v| {
                if lo.is_none_or(|(_, s)| v < s.1) {
                    lo = Some((idx, (ts, v)));
                }
                if hi.is_none_or(|(_, s)| v > s.1) {
                    hi = Some((idx, (ts, v)));
                }
                idx += 1;
                if idx.is_multiple_of(k) {
                    flush(&mut out, &mut lo, &mut hi);
                }
                true
            })?;
            flush(&mut out, &mut lo, &mut hi);
        }
    }
    Ok(out)
}

/// One bucket of a tag's history: the samples that fall in `[start, start + width)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesBucket {
    /// Start of the bucket, a multiple of the bucket width.
    pub start: i64,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    /// The newest sample of the bucket and when it was taken. For an hourly or
    /// daily row, which keeps no single sample, the mean and the row's start.
    pub last: f64,
    pub last_ts: i64,
    /// Samples folded into the bucket.
    pub count: u64,
}

#[derive(Clone, Copy)]
struct Acc {
    agg: Agg,
    last: f64,
}

/// Folds samples and aggregate rows into buckets of a fixed width. Input may
/// switch between tiers, but within a tier it must be in time order.
struct Bucketer {
    width: i64,
    done: std::collections::BTreeMap<i64, Acc>,
    cur: Option<(i64, Acc)>,
}

impl Bucketer {
    fn new(width: i64) -> Self {
        Self {
            width: width.max(1),
            done: Default::default(),
            cur: None,
        }
    }

    fn slot(&mut self, ts: i64) -> &mut Acc {
        let start = ts.div_euclid(self.width).saturating_mul(self.width);
        if self.cur.as_ref().is_none_or(|(s, _)| *s != start) {
            if let Some((s, acc)) = self.cur.take() {
                self.done.insert(s, acc);
            }
            let acc = self.done.remove(&start).unwrap_or(Acc {
                agg: Agg::new(),
                last: 0.0,
            });
            self.cur = Some((start, acc));
        }
        &mut self.cur.as_mut().expect("just set").1
    }

    fn sample(&mut self, ts: i64, v: f64) {
        if !v.is_finite() {
            return;
        }
        let acc = self.slot(ts);
        if ts >= acc.agg.last_ts {
            acc.last = v;
        }
        acc.agg.add(ts, v);
    }

    fn aggregate(&mut self, ts: i64, mean: f64, min: f64, max: f64, count: u64) {
        if count == 0 || !mean.is_finite() {
            return;
        }
        let acc = self.slot(ts);
        if ts >= acc.agg.last_ts {
            acc.last = mean;
        }
        acc.agg.add_aggregate(ts, mean, min, max, count);
    }

    fn finish(mut self) -> Vec<SeriesBucket> {
        if let Some((s, acc)) = self.cur.take() {
            self.done.insert(s, acc);
        }
        self.done
            .into_iter()
            .map(|(start, acc)| SeriesBucket {
                start,
                avg: acc.agg.mean(),
                min: acc.agg.min,
                max: acc.agg.max,
                last: acc.last,
                last_ts: acc.agg.last_ts,
                count: acc.agg.count,
            })
            .collect()
    }
}

/// A tag's history in `[from, to]` in buckets of `width` seconds, from every
/// tier: daily rows, then hourly rows, then full-resolution samples (packed
/// and raw), each used only for the time before the next tier begins, so a
/// moment is never counted twice. Never truncates; the result has about
/// `(to - from) / width` buckets.
pub(super) fn bucket_series(
    conn: &Connection,
    device_id: &str,
    metric: &str,
    from: i64,
    to: i64,
    width: i64,
) -> Result<Vec<SeriesBucket>> {
    let min_of = |sql: &str| -> Result<i64> {
        let v: Option<i64> = conn
            .query_row(sql, params![device_id, metric], |row| row.get(0))
            .context("Tier start query failed")?;
        Ok(v.unwrap_or(i64::MAX))
    };
    let full_start = min_of(
        "SELECT MIN(bucket_start) FROM telemetry_blocks WHERE device_id = ?1 AND metric = ?2",
    )?
    .min(min_of(
        "SELECT MIN(timestamp) FROM telemetry WHERE device_id = ?1 AND metric = ?2",
    )?);
    let hourly_start = min_of(
        "SELECT MIN(bucket_start) FROM telemetry_hourly WHERE device_id = ?1 AND metric = ?2",
    )?;

    let mut b = Bucketer::new(width);
    for (table, span, ends_before) in [
        ("telemetry_daily", 86_400, full_start.min(hourly_start)),
        ("telemetry_hourly", 3_600, full_start),
    ] {
        let mut stmt = conn
            .prepare_cached(&format!(
                "SELECT bucket_start, avg_value, min_value, max_value, sample_count FROM {table}
                 WHERE device_id = ?1 AND metric = ?2
                   AND bucket_start + {span} > ?3 AND bucket_start <= ?4
                   AND bucket_start + {span} <= ?5
                 ORDER BY bucket_start"
            ))
            .context("Prepare aggregate rows query failed")?;
        let mut rows = stmt
            .query(params![device_id, metric, from, to, ends_before])
            .context("Aggregate rows query failed")?;
        while let Some(row) = rows.next()? {
            b.aggregate(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get::<_, i64>(4)?.max(0) as u64,
            );
        }
    }
    scan_series(conn, device_id, metric, from, to, |ts, v| {
        b.sample(ts, v);
        true
    })?;
    Ok(b.finish())
}

/// Tags of a device with anything (full resolution or aggregate) in `[from, to]`.
pub(super) fn series_metrics(
    conn: &Connection,
    device_id: &str,
    from: i64,
    to: i64,
) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT metric FROM telemetry_blocks
             WHERE device_id = ?1 AND bucket_start >= ?2 AND bucket_start <= ?3
               AND last_ts >= ?4 AND first_ts <= ?5
             UNION
             SELECT metric FROM telemetry
             WHERE device_id = ?1 AND timestamp >= ?4 AND timestamp <= ?5
             UNION
             SELECT metric FROM telemetry_hourly
             WHERE device_id = ?1 AND bucket_start + 3600 > ?4 AND bucket_start <= ?5
             UNION
             SELECT metric FROM telemetry_daily
             WHERE device_id = ?1 AND bucket_start + 86400 > ?4 AND bucket_start <= ?5",
        )
        .context("Prepare series metric list failed")?;
    stmt.query_map(
        params![device_id, bucket_start(from), to, from, to],
        |row| row.get(0),
    )
    .context("Series metric list query failed")?
    .collect::<Result<Vec<String>, _>>()
    .context("Series metric list row mapping failed")
}
