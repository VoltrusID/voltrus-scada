//! Performance benchmark engine — measures throughput on current hardware.

use std::time::Instant;

use rusqlite::Connection;

use super::platform::PlatformInfo;
use super::{BenchmarkDetail, BenchmarkResults};

/// Default benchmark duration in seconds.
const DEFAULT_DURATION_SECS: u64 = 10;

/// Minimum acceptable tag write throughput (writes/second) for certification.
const MIN_TAG_WRITE_THROUGHPUT: f64 = 1000.0;

/// Minimum acceptable batch insert throughput (rows/second) for certification.
const MIN_BATCH_INSERT_THROUGHPUT: f64 = 5000.0;

/// Minimum acceptable query throughput (reads/second) for certification.
const MIN_QUERY_THROUGHPUT: f64 = 1000.0;

/// Minimum acceptable compression throughput (rows/second).
const MIN_COMPRESSION_THROUGHPUT: f64 = 10000.0;

/// Run all hardware benchmarks and return results.
pub fn run_benchmarks(
    platform: &PlatformInfo,
    duration_secs: Option<u64>,
) -> anyhow::Result<BenchmarkResults> {
    let duration = duration_secs.unwrap_or(DEFAULT_DURATION_SECS);
    let start = Instant::now();

    // Create in-memory database for benchmarking (no disk I/O interference for tag writes)
    let conn = Connection::open_in_memory()?;
    init_benchmark_schema(&conn)?;

    let mut details = Vec::new();

    // Benchmark 1: Single tag writes
    let tag_write = benchmark_tag_writes(&conn, duration / 4)?;
    details.push(tag_write.clone());

    // Benchmark 2: Batch inserts (simulating telemetry persistence)
    let batch_insert = benchmark_batch_inserts(&conn, duration / 4)?;
    details.push(batch_insert.clone());

    // Benchmark 3: Range queries
    let query = benchmark_queries(&conn, duration / 4)?;
    details.push(query.clone());

    // Benchmark 4: Compression simulation (delta encoding)
    let compression = benchmark_compression(duration / 4)?;
    details.push(compression.clone());

    // Calculate overall metrics
    let tag_write_throughput = tag_write.throughput;
    let batch_insert_throughput = batch_insert.throughput;
    let query_throughput = query.throughput;
    let compression_throughput = compression.throughput;

    // Determine recommended limits based on weakest benchmark
    let recommended_max_tags = calculate_recommended_tags(
        tag_write_throughput,
        batch_insert_throughput,
        platform.cpu_cores,
    );

    let recommended_poll_rate = if tag_write_throughput > 50000.0 && platform.cpu_cores >= 4 {
        10
    } else if tag_write_throughput > 10000.0 {
        5
    } else {
        1
    };

    // Estimate write amplification (1.0 = ideal, only batch writes)
    let write_amplification = if batch_insert_throughput > 0.0 {
        tag_write_throughput / batch_insert_throughput
    } else {
        1.0
    };

    let total_duration = start.elapsed().as_secs_f64();

    Ok(BenchmarkResults {
        platform: platform.clone(),
        tag_write_throughput,
        batch_insert_throughput,
        compression_throughput,
        query_throughput,
        peak_memory_mb: estimate_peak_memory_mb(),
        duration_secs: total_duration,
        recommended_max_tags,
        recommended_poll_rate,
        write_amplification: write_amplification.min(10.0),
        details,
    })
}

/// Initialize the benchmark database schema.
fn init_benchmark_schema(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS benchmark_telemetry (
            device_id TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            metric TEXT NOT NULL,
            value REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_bench_ts ON benchmark_telemetry(timestamp);
        CREATE INDEX IF NOT EXISTS idx_bench_device ON benchmark_telemetry(device_id, metric);",
    )?;
    Ok(())
}

/// Benchmark single tag writes (simulating individual telemetry inserts).
fn benchmark_tag_writes(conn: &Connection, duration_secs: u64) -> anyhow::Result<BenchmarkDetail> {
    let start = Instant::now();
    let duration = std::time::Duration::from_secs(duration_secs.max(1));
    let mut ops: u64 = 0;

    loop {
        if start.elapsed() >= duration {
            break;
        }

        let tx = conn.unchecked_transaction()?;
        for i in 0..100 {
            let ts = 1700000000 + ops + i as u64;
            tx.execute(
                "INSERT INTO benchmark_telemetry (device_id, timestamp, metric, value) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![format!("dev_{}", ops % 10), ts, format!("tag_{}", i % 20), 42.5 + ops as f64 * 0.001],
            )?;
        }
        tx.commit()?;
        ops += 100;
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let throughput = ops as f64 / (elapsed_ms as f64 / 1000.0);

    // Clean up
    conn.execute("DELETE FROM benchmark_telemetry", [])?;

    Ok(BenchmarkDetail {
        name: "tag_write".into(),
        operations: ops,
        duration_ms: elapsed_ms,
        throughput,
        passed: throughput >= MIN_TAG_WRITE_THROUGHPUT,
    })
}

/// Benchmark batch inserts (simulating telemetry persistence task).
fn benchmark_batch_inserts(
    conn: &Connection,
    duration_secs: u64,
) -> anyhow::Result<BenchmarkDetail> {
    let start = Instant::now();
    let duration = std::time::Duration::from_secs(duration_secs.max(1));
    let batch_size = 500;
    let mut total_rows: u64 = 0;

    loop {
        if start.elapsed() >= duration {
            break;
        }

        let tx = conn.unchecked_transaction()?;
        for _ in 0..batch_size {
            let ts = 1700000000 + total_rows;
            tx.execute(
                "INSERT INTO benchmark_telemetry (device_id, timestamp, metric, value) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![format!("dev_{}", total_rows % 50), ts, format!("tag_{}", total_rows % 100), 42.5],
            )?;
            total_rows += 1;
        }
        tx.commit()?;
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let throughput = total_rows as f64 / (elapsed_ms as f64 / 1000.0);

    // Clean up
    conn.execute("DELETE FROM benchmark_telemetry", [])?;

    Ok(BenchmarkDetail {
        name: "batch_insert".into(),
        operations: total_rows,
        duration_ms: elapsed_ms,
        throughput,
        passed: throughput >= MIN_BATCH_INSERT_THROUGHPUT,
    })
}

/// Benchmark range queries against populated data.
fn benchmark_queries(conn: &Connection, duration_secs: u64) -> anyhow::Result<BenchmarkDetail> {
    // Pre-populate with 100K rows
    let tx = conn.unchecked_transaction()?;
    for i in 0..100_000 {
        let ts = 1700000000 + i;
        tx.execute(
            "INSERT INTO benchmark_telemetry (device_id, timestamp, metric, value) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![format!("dev_{}", i % 10), ts, format!("tag_{}", i % 50), i as f64 * 0.1],
        )?;
    }
    tx.commit()?;

    let start = Instant::now();
    let duration = std::time::Duration::from_secs(duration_secs.max(1));
    let mut ops: u64 = 0;

    loop {
        if start.elapsed() >= duration {
            break;
        }

        let from_ts = 1700000000 + (ops % 90000) as i64;
        let to_ts = from_ts + 1000;
        let mut stmt = conn.prepare(
            "SELECT timestamp, value FROM benchmark_telemetry WHERE device_id = ?1 AND metric = ?2 AND timestamp >= ?3 AND timestamp <= ?4 ORDER BY timestamp LIMIT 10000",
        )?;

        let device_id = format!("dev_{}", ops % 10);
        let metric = format!("tag_{}", ops % 50);
        let rows: Vec<(i64, f64)> = stmt
            .query_map(
                rusqlite::params![device_id, metric, from_ts, to_ts],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
            .filter_map(|r| r.ok())
            .collect();

        ops += 1;
        drop(rows);
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let throughput = ops as f64 / (elapsed_ms as f64 / 1000.0);

    // Clean up
    conn.execute("DELETE FROM benchmark_telemetry", [])?;

    Ok(BenchmarkDetail {
        name: "range_query".into(),
        operations: ops,
        duration_ms: elapsed_ms,
        throughput,
        passed: throughput >= MIN_QUERY_THROUGHPUT,
    })
}

/// Benchmark compression (delta encoding simulation, no disk I/O).
fn benchmark_compression(duration_secs: u64) -> anyhow::Result<BenchmarkDetail> {
    let start = Instant::now();
    let duration = std::time::Duration::from_secs(duration_secs.max(1));
    let mut ops: u64 = 0;

    // Simulate Gorilla-style delta-of-delta compression
    let mut values: Vec<f64> = Vec::with_capacity(10000);
    for i in 0..10000u64 {
        values.push(42.0 + (i as f64 * 0.001).sin() * 5.0);
    }

    loop {
        if start.elapsed() >= duration {
            break;
        }

        // Delta encode the values
        let mut deltas: Vec<i64> = Vec::with_capacity(values.len());
        let mut prev = 0i64;
        for &v in &values {
            // Float to fixed-point for delta encoding
            let fixed = (v * 1000.0) as i64;
            let delta = fixed - prev;
            deltas.push(delta);
            prev = fixed;
        }

        // Simulate varint encoding
        let mut encoded_size: usize = 0;
        for &delta in &deltas {
            // ZigZag encode + varint size estimate
            let zigzag = ((delta << 1) ^ (delta >> 63)) as u64;
            encoded_size += varint_size(zigzag);
        }

        // Don't let the compiler optimize away
        std::hint::black_box(encoded_size);
        std::hint::black_box(&deltas);

        ops += values.len() as u64;
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let throughput = ops as f64 / (elapsed_ms as f64 / 1000.0);

    Ok(BenchmarkDetail {
        name: "compression".into(),
        operations: ops,
        duration_ms: elapsed_ms,
        throughput,
        passed: throughput >= MIN_COMPRESSION_THROUGHPUT,
    })
}

/// Estimate the varint encoded size of a u64 value.
fn varint_size(value: u64) -> usize {
    if value < 0x80 {
        1
    } else if value < 0x4000 {
        2
    } else if value < 0x200000 {
        3
    } else if value < 0x10000000 {
        4
    } else if value < 0x0800000000 {
        5
    } else {
        9
    }
}

/// Calculate recommended maximum tag count based on benchmark results.
fn calculate_recommended_tags(
    tag_write_throughput: f64,
    batch_insert_throughput: f64,
    _cpu_cores: u32,
) -> u32 {
    // At 1Hz, each tag requires 1 write/second
    // Batch inserts handle multiple tags per transaction
    // Conservative: use 30% of throughput for headroom
    let write_limited = (tag_write_throughput * 0.3) as u32;
    let batch_limited = (batch_insert_throughput * 0.3) as u32;

    let base = write_limited.min(batch_limited);
    let capped = base.min(50000); // Never recommend more than 50K tags

    capped.max(50) // At least 50 tags
}

/// Estimate peak memory usage in MB.
fn estimate_peak_memory_mb() -> f64 {
    // Use sysinfo to get current process memory
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut sys = System::new_all();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        false,
        ProcessRefreshKind::nothing().with_memory(),
    );

    let my_pid = sysinfo::Pid::from(std::process::id() as usize);
    if let Some(proc) = sys.process(my_pid) {
        proc.memory() as f64 / (1024.0 * 1024.0)
    } else {
        0.0
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_benchmarks() {
        let platform = PlatformInfo::default();
        // Quick benchmark (1 second per test = 4 seconds total)
        let results = run_benchmarks(&platform, Some(1)).unwrap();

        assert!(results.tag_write_throughput > 0.0);
        assert!(results.batch_insert_throughput > 0.0);
        assert!(results.compression_throughput > 0.0);
        assert!(results.query_throughput > 0.0);
        assert!(results.recommended_max_tags > 0);
        assert!(results.duration_secs > 0.0);
        assert_eq!(results.details.len(), 4);
    }

    #[test]
    fn test_benchmark_details() {
        let platform = PlatformInfo::default();
        let results = run_benchmarks(&platform, Some(1)).unwrap();

        for detail in &results.details {
            assert!(!detail.name.is_empty());
            assert!(detail.operations > 0);
            assert!(detail.duration_ms > 0);
            assert!(detail.throughput > 0.0);
        }
    }

    #[test]
    fn test_varint_size() {
        assert_eq!(varint_size(0), 1);
        assert_eq!(varint_size(127), 1);
        assert_eq!(varint_size(128), 2);
        assert_eq!(varint_size(16383), 2);
        assert_eq!(varint_size(16384), 3);
    }

    #[test]
    fn test_calculate_recommended_tags() {
        let tags = calculate_recommended_tags(50000.0, 100000.0, 4);
        assert!(tags > 0);
        assert!(tags <= 50000);

        // Low throughput
        let tags = calculate_recommended_tags(500.0, 1000.0, 1);
        assert!(tags >= 50);
    }
}
