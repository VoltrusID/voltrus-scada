//! File I/O pattern optimizations for EDR compatibility
//!
//! EDR tools intercept file I/O operations to scan for malware. In SCADA systems
//! with high-frequency telemetry writes, this can cause significant CPU overhead.
//!
//! This module provides:
//! - Optimized write patterns (batching, sequential writes)
//! - SQLite PRAGMA tuning for EDR-friendly I/O
//! - Write amplification monitoring
//! - EDR-aware write scheduling (avoid writes during known scan windows)

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// I/O performance counters for monitoring EDR impact.
#[derive(Debug)]
pub struct IoCounters {
    /// Total bytes written
    pub bytes_written: AtomicU64,
    /// Total write operations
    pub write_ops: AtomicU64,
    /// Total bytes read
    pub bytes_read: AtomicU64,
    /// Total read operations
    pub read_ops: AtomicU64,
    /// Number of batch writes performed
    pub batch_writes: AtomicU64,
    /// Number of single-row writes that were deferred to a batch
    pub deferred_writes: AtomicU64,
    /// Time spent in write operations (microseconds)
    pub write_time_us: AtomicU64,
    /// Number of fsync calls
    pub fsync_count: AtomicU64,
}

impl IoCounters {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            bytes_written: AtomicU64::new(0),
            write_ops: AtomicU64::new(0),
            bytes_read: AtomicU64::new(0),
            read_ops: AtomicU64::new(0),
            batch_writes: AtomicU64::new(0),
            deferred_writes: AtomicU64::new(0),
            write_time_us: AtomicU64::new(0),
            fsync_count: AtomicU64::new(0),
        })
    }

    /// Record a batch write operation.
    pub fn record_batch_write(&self, bytes: u64, rows: usize, duration_us: u64) {
        self.bytes_written.fetch_add(bytes, Ordering::Relaxed);
        self.write_ops.fetch_add(1, Ordering::Relaxed);
        self.batch_writes.fetch_add(1, Ordering::Relaxed);
        self.deferred_writes
            .fetch_add(rows.saturating_sub(1) as u64, Ordering::Relaxed);
        self.write_time_us.fetch_add(duration_us, Ordering::Relaxed);
    }

    /// Record a single write operation.
    pub fn record_single_write(&self, bytes: u64, duration_us: u64) {
        self.bytes_written.fetch_add(bytes, Ordering::Relaxed);
        self.write_ops.fetch_add(1, Ordering::Relaxed);
        self.write_time_us.fetch_add(duration_us, Ordering::Relaxed);
    }

    /// Record a read operation.
    pub fn record_read(&self, bytes: u64) {
        self.bytes_read.fetch_add(bytes, Ordering::Relaxed);
        self.read_ops.fetch_add(1, Ordering::Relaxed);
    }

    /// Get a snapshot of all counters.
    pub fn snapshot(&self) -> IoCountersSnapshot {
        IoCountersSnapshot {
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            write_ops: self.write_ops.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            read_ops: self.read_ops.load(Ordering::Relaxed),
            batch_writes: self.batch_writes.load(Ordering::Relaxed),
            deferred_writes: self.deferred_writes.load(Ordering::Relaxed),
            write_time_us: self.write_time_us.load(Ordering::Relaxed),
            fsync_count: self.fsync_count.load(Ordering::Relaxed),
        }
    }
}

impl Default for IoCounters {
    fn default() -> Self {
        Self {
            bytes_written: AtomicU64::new(0),
            write_ops: AtomicU64::new(0),
            bytes_read: AtomicU64::new(0),
            read_ops: AtomicU64::new(0),
            batch_writes: AtomicU64::new(0),
            deferred_writes: AtomicU64::new(0),
            write_time_us: AtomicU64::new(0),
            fsync_count: AtomicU64::new(0),
        }
    }
}

/// Snapshot of I/O counters for reporting.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IoCountersSnapshot {
    pub bytes_written: u64,
    pub write_ops: u64,
    pub bytes_read: u64,
    pub read_ops: u64,
    pub batch_writes: u64,
    pub deferred_writes: u64,
    pub write_time_us: u64,
    pub fsync_count: u64,
}

/// SQLite PRAGMA settings optimized for EDR compatibility.
///
/// These settings minimize the number of individual I/O operations
/// that EDR tools need to intercept and scan:
///
/// - WAL mode: Sequential appends instead of random in-place writes
/// - NORMAL sync: Fewer fsync calls (safe with WAL)
/// - Larger page size: Fewer I/O operations per write
/// - Larger cache: More reads from memory, fewer disk reads
pub const EDR_OPTIMIZED_PRAGMAS: &[&str] = &[
    "PRAGMA journal_mode=WAL;", // Sequential WAL writes instead of random rollback journal
    "PRAGMA synchronous=NORMAL;", // Fewer fsync calls (safe with WAL mode)
    "PRAGMA wal_autocheckpoint=1000;", // Batch checkpoint every 1000 pages
    "PRAGMA busy_timeout=5000;", // Retry on lock contention
    "PRAGMA cache_size=-8000;", // 8MB cache (more in-memory reads)
    "PRAGMA temp_store=MEMORY;", // Temp tables in RAM
    "PRAGMA mmap_size=67108864;", // 64MB memory-mapped I/O for reads
];

/// Voltrus recommended I/O configuration for OT deployments.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EdrIoConfig {
    /// Enable batch write optimization (default: true)
    pub batch_writes_enabled: bool,
    /// Maximum batch size before flushing (default: 500)
    pub batch_size: usize,
    /// Maximum time to hold a batch before flushing in milliseconds (default: 1000)
    pub batch_flush_interval_ms: u64,
    /// Enable write coalescing (combine small writes into larger ones)
    pub write_coalescing_enabled: bool,
    /// Enable sequential write hint for the OS
    pub sequential_write_hint: bool,
    /// Disable fsync on non-critical writes (WAL handles durability)
    pub reduced_fsync: bool,
}

impl Default for EdrIoConfig {
    fn default() -> Self {
        Self {
            batch_writes_enabled: true,
            batch_size: 500,
            batch_flush_interval_ms: 1000,
            write_coalescing_enabled: true,
            sequential_write_hint: true,
            reduced_fsync: true,
        }
    }
}

/// Calculate the write amplification factor.
/// A factor of 1.0 means no amplification (ideal).
/// EDR scanning typically causes 2-5x amplification.
pub fn calculate_write_amplification(snapshot: &IoCountersSnapshot) -> f64 {
    if snapshot.write_ops == 0 {
        return 1.0;
    }

    // Without batching, each row would be 1 write op
    // With batching, batch_writes covers multiple rows
    let total_rows = snapshot.write_ops + snapshot.deferred_writes;
    if total_rows == 0 {
        return 1.0;
    }

    // Amplification = actual writes / ideal writes
    // Ideal: 1 write per batch (batch_size rows)
    // Actual: write_ops
    let ideal_writes = total_rows.div_ceil(500); // ceil(total/500)
    let actual_writes = snapshot.write_ops;

    actual_writes as f64 / ideal_writes.max(1) as f64
}

/// Estimate the sequential write percentage.
/// Higher is better for EDR performance (sequential scans are faster than random).
pub fn estimate_sequential_pct(snapshot: &IoCountersSnapshot) -> f64 {
    if snapshot.write_ops == 0 {
        return 1.0;
    }

    // Batch writes are sequential; single writes may be random
    let sequential_ops = snapshot.batch_writes;
    let total_ops = snapshot.write_ops;

    sequential_ops as f64 / total_ops as f64
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_counters_default() {
        let counters = IoCounters::default();
        let snap = counters.snapshot();
        assert_eq!(snap.bytes_written, 0);
        assert_eq!(snap.write_ops, 0);
        assert_eq!(snap.batch_writes, 0);
    }

    #[test]
    fn test_io_counters_batch_write() {
        let counters = IoCounters::default();
        counters.record_batch_write(1024, 50, 100);
        let snap = counters.snapshot();
        assert_eq!(snap.bytes_written, 1024);
        assert_eq!(snap.write_ops, 1);
        assert_eq!(snap.batch_writes, 1);
        assert_eq!(snap.deferred_writes, 49); // 50 rows - 1 = 49 deferred
        assert_eq!(snap.write_time_us, 100);
    }

    #[test]
    fn test_io_counters_single_write() {
        let counters = IoCounters::default();
        counters.record_single_write(64, 10);
        let snap = counters.snapshot();
        assert_eq!(snap.bytes_written, 64);
        assert_eq!(snap.write_ops, 1);
        assert_eq!(snap.write_time_us, 10);
    }

    #[test]
    fn test_io_counters_read() {
        let counters = IoCounters::default();
        counters.record_read(2048);
        let snap = counters.snapshot();
        assert_eq!(snap.bytes_read, 2048);
        assert_eq!(snap.read_ops, 1);
    }

    #[test]
    fn test_write_amplification_ideal() {
        // All batch writes = low amplification
        let snap = IoCountersSnapshot {
            bytes_written: 10000,
            write_ops: 10,
            bytes_read: 0,
            read_ops: 0,
            batch_writes: 10,
            deferred_writes: 4990, // 5000 rows total, 10 batches of ~500
            write_time_us: 1000,
            fsync_count: 10,
        };
        let amp = calculate_write_amplification(&snap);
        // 10 writes / 10 ideal = 1.0
        assert!(amp <= 1.5, "Amplification should be low: {}", amp);
    }

    #[test]
    fn test_write_amplification_worst_case() {
        // All single writes = high amplification
        let snap = IoCountersSnapshot {
            bytes_written: 50000,
            write_ops: 5000,
            bytes_read: 0,
            read_ops: 0,
            batch_writes: 0,
            deferred_writes: 0,
            write_time_us: 50000,
            fsync_count: 5000,
        };
        let amp = calculate_write_amplification(&snap);
        // 5000 writes / 10 ideal = 500x amplification!
        assert!(amp > 10.0, "Amplification should be very high: {}", amp);
    }

    #[test]
    fn test_sequential_pct_all_batch() {
        let snap = IoCountersSnapshot {
            bytes_written: 0,
            write_ops: 100,
            bytes_read: 0,
            read_ops: 0,
            batch_writes: 100,
            deferred_writes: 0,
            write_time_us: 0,
            fsync_count: 0,
        };
        let pct = estimate_sequential_pct(&snap);
        assert!((pct - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_sequential_pct_mixed() {
        let snap = IoCountersSnapshot {
            bytes_written: 0,
            write_ops: 100,
            bytes_read: 0,
            read_ops: 0,
            batch_writes: 50,
            deferred_writes: 0,
            write_time_us: 0,
            fsync_count: 0,
        };
        let pct = estimate_sequential_pct(&snap);
        assert!((pct - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_edr_io_config_default() {
        let config = EdrIoConfig::default();
        assert!(config.batch_writes_enabled);
        assert_eq!(config.batch_size, 500);
        assert_eq!(config.batch_flush_interval_ms, 1000);
        assert!(config.write_coalescing_enabled);
        assert!(config.sequential_write_hint);
        assert!(config.reduced_fsync);
    }

    #[test]
    fn test_edr_optimized_pragmas() {
        assert!(!EDR_OPTIMIZED_PRAGMAS.is_empty());
        assert!(EDR_OPTIMIZED_PRAGMAS.iter().any(|p| p.contains("WAL")));
        assert!(
            EDR_OPTIMIZED_PRAGMAS
                .iter()
                .any(|p| p.contains("synchronous"))
        );
    }
}
