//! Per-device protocol traffic: what the drivers sent and received, kept in
//! a bounded ring per device and decoded with `voltrus-dissect` when someone
//! looks (see docs/traffic.md).
//!
//! Recording costs nothing until a device is watched: the driver taps check
//! one atomic and pass the bytes through. Fetching a device's traffic (REST
//! or MCP) starts a watch lease of [`WATCH_LEASE`]; while it runs, each
//! complete frame is copied into the ring (raw bytes only). When the lease
//! runs out, recording stops and the ring is freed. Decoding, pairing and
//! tag mapping happen on read, not on the polling path.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use voltrus_dissect::{DecodedFrame, Dir};

/// Frames kept per device.
pub const DEFAULT_CAPACITY: usize = 2_000;
/// Raw bytes kept per device, whichever limit is reached first.
pub const DEFAULT_MAX_BYTES: usize = 2 * 1024 * 1024;
/// How long one look at a device keeps its capture running.
pub const WATCH_LEASE_MS: i64 = 10 * 60 * 1000;
/// A request with no response after this long is shown as unanswered
/// (the Modbus poller's read timeout).
pub const NO_RESPONSE_MS: i64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    ModbusTcp,
}

/// One frame as it went over the wire.
#[derive(Debug, Clone)]
pub struct RawFrame {
    pub seq: u64,
    pub ts_us: i64,
    /// Connection it travelled on; transaction ids restart per connection.
    pub conn: u64,
    pub proto: Proto,
    pub dir: Dir,
    pub bytes: Box<[u8]>,
}

#[derive(Debug, Default)]
struct Ring {
    frames: VecDeque<RawFrame>,
    bytes: usize,
    next_seq: u64,
    /// Frames pushed out of the ring since the capture started.
    evicted: u64,
}

/// The capture of one device.
#[derive(Debug)]
pub struct DeviceTraffic {
    /// Capture runs while now (ms) is before this; 0 = off.
    watch_until_ms: AtomicI64,
    ring: Mutex<Ring>,
    capacity: usize,
    max_bytes: usize,
}

impl DeviceTraffic {
    fn new(capacity: usize, max_bytes: usize) -> Self {
        Self {
            watch_until_ms: AtomicI64::new(0),
            ring: Mutex::new(Ring::default()),
            capacity,
            max_bytes,
        }
    }

    /// Whether anyone watches: one relaxed load, no clock read. The taps
    /// call this first and [`DeviceTraffic::recording_at`] only when it is true.
    #[inline]
    pub fn watched(&self) -> bool {
        self.watch_until_ms.load(Ordering::Relaxed) != 0
    }

    /// Whether the taps should record now. A lease that ran out is ended
    /// (and the ring freed) by the first tap that notices.
    pub fn recording(&self) -> bool {
        self.watched() && self.recording_at(now_us())
    }

    pub fn recording_at(&self, now_us: i64) -> bool {
        let until = self.watch_until_ms.load(Ordering::Relaxed);
        if until == 0 {
            return false;
        }
        if now_us / 1000 < until {
            return true;
        }
        if self
            .watch_until_ms
            .compare_exchange(until, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            let mut ring = self.lock();
            *ring = Ring {
                next_seq: ring.next_seq,
                ..Ring::default()
            };
        }
        false
    }

    /// Keep one complete frame, seen at `ts_us`.
    pub fn record(&self, conn: u64, proto: Proto, dir: Dir, bytes: &[u8], ts_us: i64) {
        let mut ring = self.lock();
        let seq = ring.next_seq;
        ring.next_seq += 1;
        ring.bytes += bytes.len();
        ring.frames.push_back(RawFrame {
            seq,
            ts_us,
            conn,
            proto,
            dir,
            bytes: bytes.into(),
        });
        while ring.frames.len() > self.capacity || ring.bytes > self.max_bytes {
            let Some(old) = ring.frames.pop_front() else {
                break;
            };
            ring.bytes -= old.bytes.len();
            ring.evicted += 1;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start or extend the capture lease.
    pub fn watch(&self) -> i64 {
        let until = now_ms() + WATCH_LEASE_MS;
        self.watch_until_ms.store(until, Ordering::Relaxed);
        until
    }

    /// The frames kept, oldest first, and the capture state.
    pub fn snapshot(&self) -> Snapshot {
        let ring = self.lock();
        Snapshot {
            frames: ring.frames.iter().cloned().collect(),
            evicted: ring.evicted,
            watch_until_ms: self.watch_until_ms.load(Ordering::Relaxed),
            capacity: self.capacity,
        }
    }
}

pub struct Snapshot {
    pub frames: Vec<RawFrame>,
    pub evicted: u64,
    pub watch_until_ms: i64,
    pub capacity: usize,
}

/// Every device's capture. Cheap to clone; one per server, held by the
/// [`crate::cache::DeviceCache`] so pollers and API handlers share it.
#[derive(Debug, Clone)]
pub struct TrafficHub {
    devices: Arc<Mutex<HashMap<String, Arc<DeviceTraffic>>>>,
    conns: Arc<AtomicU64>,
    capacity: usize,
    max_bytes: usize,
}

impl Default for TrafficHub {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY, DEFAULT_MAX_BYTES)
    }
}

impl TrafficHub {
    pub fn new(capacity: usize, max_bytes: usize) -> Self {
        Self {
            devices: Arc::default(),
            conns: Arc::new(AtomicU64::new(1)),
            capacity: capacity.max(1),
            max_bytes,
        }
    }

    /// The capture of a device, created on first use (by its tap or a viewer).
    pub fn device(&self, device_id: &str) -> Arc<DeviceTraffic> {
        let mut map = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(device_id.to_string())
            .or_insert_with(|| Arc::new(DeviceTraffic::new(self.capacity, self.max_bytes)))
            .clone()
    }

    /// A fresh connection id for a tap.
    pub fn next_conn(&self) -> u64 {
        self.conns.fetch_add(1, Ordering::Relaxed)
    }
}

/// Decode a device's frames: pair requests and responses per connection,
/// put the pairing and latency on both frames, and mark requests that
/// stayed unanswered for longer than [`NO_RESPONSE_MS`].
pub fn decode(device_id: &str, frames: &[RawFrame], now_us: i64) -> Vec<DecodedFrame> {
    let mut sessions: HashMap<u64, voltrus_dissect::modbus::Session> = HashMap::new();
    let mut out: Vec<DecodedFrame> = frames
        .iter()
        .map(|f| {
            let mut d = match f.proto {
                Proto::ModbusTcp => sessions
                    .entry(f.conn)
                    .or_default()
                    .decode(&f.bytes, f.dir, f.ts_us, f.seq),
            };
            d.device_id = Some(device_id.to_string());
            d
        })
        .collect();
    let index: HashMap<u64, usize> = out.iter().enumerate().map(|(i, f)| (f.seq, i)).collect();
    for i in 0..out.len() {
        if out[i].dir == Dir::Rx
            && let Some(req) = out[i].pair_id.and_then(|s| index.get(&s).copied())
        {
            let (seq, latency) = (out[i].seq, out[i].latency_us);
            out[req].pair_id = Some(seq);
            out[req].latency_us = latency;
        }
    }
    let last_conn = frames.last().map(|f| f.conn);
    for (f, raw) in out.iter_mut().zip(frames) {
        let stale = now_us - f.ts_us > NO_RESPONSE_MS * 1000;
        // On a connection that was replaced, an open request will never be answered.
        let closed = Some(raw.conn) != last_conn;
        if f.dir == Dir::Tx && f.pair_id.is_none() && f.error.is_none() && (stale || closed) {
            f.error = Some(voltrus_dissect::modbus::no_response(NO_RESPONSE_MS as u64));
        }
    }
    out
}

pub(crate) fn now_ms() -> i64 {
    now_us() / 1000
}

pub(crate) fn now_us() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adu(tid: u16, pdu: &[u8]) -> Vec<u8> {
        let mut v = tid.to_be_bytes().to_vec();
        v.extend([0, 0]);
        v.extend(((pdu.len() + 1) as u16).to_be_bytes());
        v.push(1);
        v.extend(pdu);
        v
    }

    #[test]
    fn records_only_while_watched_and_stays_bounded() {
        let hub = TrafficHub::new(3, 1 << 20);
        let dev = hub.device("d");
        assert!(!dev.recording());
        dev.watch();
        assert!(dev.recording());
        for i in 0..5u16 {
            dev.record(1, Proto::ModbusTcp, Dir::Tx, &adu(i, &[3, 0, 0, 0, 1]), 0);
        }
        let snap = dev.snapshot();
        assert_eq!(snap.frames.len(), 3);
        assert_eq!(snap.evicted, 2);
        assert_eq!(snap.frames[0].seq, 2);
        // A lease that ran out stops the capture and frees the ring.
        dev.watch_until_ms.store(1, Ordering::Relaxed);
        assert!(!dev.recording());
        assert!(dev.snapshot().frames.is_empty());
        assert_eq!(dev.watch_until_ms.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn byte_limit_evicts_too() {
        let hub = TrafficHub::new(100, 30);
        let dev = hub.device("d");
        for i in 0..3u16 {
            dev.record(1, Proto::ModbusTcp, Dir::Tx, &adu(i, &[3, 0, 0, 0, 1]), 0);
        }
        assert_eq!(
            dev.snapshot().frames.len(),
            2,
            "12-byte frames, 30 bytes kept"
        );
    }

    #[test]
    fn decode_pairs_both_ways_and_flags_unanswered() {
        let now = now_us();
        let raw = |seq, ts_us, conn, dir, b: Vec<u8>| RawFrame {
            seq,
            ts_us,
            conn,
            proto: Proto::ModbusTcp,
            dir,
            bytes: b.into(),
        };
        let frames = vec![
            // Old connection: a request that never got its answer.
            raw(0, now - 1_000, 1, Dir::Tx, adu(9, &[3, 0, 0, 0, 1])),
            // New connection: transaction ids start again.
            raw(1, now - 900, 2, Dir::Tx, adu(9, &[3, 0, 0, 0, 1])),
            raw(2, now - 400, 2, Dir::Rx, adu(9, &[3, 2, 0, 42])),
            raw(3, now - 6_000_000, 2, Dir::Tx, adu(10, &[4, 0, 0, 0, 1])),
        ];
        let d = decode("dev", &frames, now);
        assert_eq!(
            d[0].error.as_ref().unwrap().kind,
            voltrus_dissect::ErrorKind::NoResponse
        );
        assert_eq!((d[1].pair_id, d[1].latency_us), (Some(2), Some(500)));
        assert_eq!((d[2].pair_id, d[2].latency_us), (Some(1), Some(500)));
        assert!(d[3].error.is_some(), "older than the timeout");
        assert_eq!(d[2].device_id.as_deref(), Some("dev"));
    }
}
