//! A small DNP3 outstation for the simulator binary and the master's tests.
//!
//! It answers class and static reads with g1v2, g10v2, g30v1 and g40v2,
//! reports g2v2/g32v1 events that stay buffered until confirmed, sends
//! unsolicited responses once enabled, executes g12v1 and g41 controls
//! (select-before-operate and direct operate), and raises IIN1.7 and IIN1.4
//! at start until the master clears them.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::dnp3::application::{AppControl, Fragment, FunctionCode, Iin, qualifier};
use crate::dnp3::channel::Channel;
use crate::dnp3::objects::{ObjectBlock, parse_object_headers, parse_read_headers};

#[derive(Debug, Clone)]
pub struct OutstationConfig {
    pub address: u16,
    pub master_address: u16,
    pub binary_inputs: usize,
    pub analog_inputs: usize,
    pub binary_outputs: usize,
    pub analog_outputs: usize,
    /// Largest APDU it sends; small values force multi-fragment responses.
    pub max_fragment: usize,
    /// Change some inputs this often to produce events; `None` for never.
    pub event_period: Option<Duration>,
    pub select_timeout: Duration,
    pub confirm_timeout: Duration,
}

impl Default for OutstationConfig {
    fn default() -> Self {
        Self {
            address: 10,
            master_address: 1,
            binary_inputs: 4,
            analog_inputs: 4,
            binary_outputs: 4,
            analog_outputs: 2,
            max_fragment: 2048,
            event_period: Some(Duration::from_secs(2)),
            select_timeout: Duration::from_secs(5),
            confirm_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum EventKind {
    Binary { index: u16, value: bool, time: u64 },
    Analog { index: u16, value: i32 },
}

#[derive(Debug, Clone)]
struct Event {
    id: u64,
    class: u8,
    kind: EventKind,
}

/// The outstation's point database, shared by all connections.
#[derive(Debug)]
pub struct Database {
    pub binary_inputs: Vec<bool>,
    pub analog_inputs: Vec<i32>,
    pub binary_outputs: Vec<bool>,
    pub analog_outputs: Vec<i16>,
    events: Vec<Event>,
    next_event: u64,
    restart: bool,
    need_time: bool,
    unsolicited: [bool; 3],
    /// Controls executed, for tests.
    pub operations: u32,
}

const ONLINE: u8 = 0x01;
const STATE: u8 = 0x80;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Database {
    fn new(cfg: &OutstationConfig) -> Self {
        Self {
            binary_inputs: vec![false; cfg.binary_inputs],
            analog_inputs: (0..cfg.analog_inputs as i32).map(|i| i * 100).collect(),
            binary_outputs: vec![false; cfg.binary_outputs],
            analog_outputs: vec![0; cfg.analog_outputs],
            events: Vec::new(),
            next_event: 0,
            restart: true,
            need_time: true,
            unsolicited: [false; 3],
            operations: 0,
        }
    }

    fn push_event(&mut self, class: u8, kind: EventKind) {
        self.next_event += 1;
        self.events.push(Event {
            id: self.next_event,
            class,
            kind,
        });
    }

    /// Change a binary input, queueing a class 1 event.
    pub fn set_binary_input(&mut self, index: u16, value: bool) {
        if let Some(v) = self.binary_inputs.get_mut(index as usize)
            && *v != value
        {
            *v = value;
            let time = now_ms();
            self.push_event(1, EventKind::Binary { index, value, time });
        }
    }

    /// Change an analog input, queueing a class 2 event.
    pub fn set_analog_input(&mut self, index: u16, value: i32) {
        if let Some(v) = self.analog_inputs.get_mut(index as usize)
            && *v != value
        {
            *v = value;
            self.push_event(2, EventKind::Analog { index, value });
        }
    }

    pub fn pending_events(&self) -> usize {
        self.events.len()
    }

    pub fn restart_bit(&self) -> bool {
        self.restart
    }

    pub fn need_time_bit(&self) -> bool {
        self.need_time
    }

    pub fn unsolicited_enabled(&self) -> bool {
        self.unsolicited.iter().any(|&b| b)
    }

    fn iin(&self) -> Iin {
        let mut iin = Iin::default();
        iin.set(Iin::DEVICE_RESTART, self.restart);
        iin.set(Iin::NEED_TIME, self.need_time);
        for e in &self.events {
            iin.set(
                match e.class {
                    1 => Iin::CLASS_1_EVENTS,
                    2 => Iin::CLASS_2_EVENTS,
                    _ => Iin::CLASS_3_EVENTS,
                },
                true,
            );
        }
        iin
    }
}

#[derive(Clone)]
pub struct OutstationHandle {
    pub db: Arc<Mutex<Database>>,
    changed: Arc<Notify>,
}

impl OutstationHandle {
    /// Run `f` on the database and wake connections so they can report
    /// new events unsolicited.
    pub fn update<R>(&self, f: impl FnOnce(&mut Database) -> R) -> R {
        let r = f(&mut self.db.lock().unwrap_or_else(|e| e.into_inner()));
        self.changed.notify_waiters();
        r
    }

    pub fn read<R>(&self, f: impl FnOnce(&Database) -> R) -> R {
        f(&self.db.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Serve on `listener` until the task is dropped.
pub async fn serve(listener: TcpListener, cfg: OutstationConfig) -> OutstationHandle {
    let handle = OutstationHandle {
        db: Arc::new(Mutex::new(Database::new(&cfg))),
        changed: Arc::new(Notify::new()),
    };
    if let Some(period) = cfg.event_period {
        let h = handle.clone();
        tokio::spawn(async move {
            let mut tick = 0i32;
            loop {
                tokio::time::sleep(period).await;
                tick += 1;
                h.update(|db| {
                    if !db.analog_inputs.is_empty() {
                        db.set_analog_input(0, 1000 + (tick % 50) * 10);
                    }
                    if !db.binary_inputs.is_empty() && tick % 3 == 0 {
                        let v = !db.binary_inputs[0];
                        db.set_binary_input(0, v);
                    }
                });
            }
        });
    }
    let h = handle.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                continue;
            };
            tracing::info!("DNP3 sim: connection from {peer}");
            let h = h.clone();
            let cfg = cfg.clone();
            tokio::spawn(async move {
                if let Err(e) = Connection::new(stream, cfg, h).run().await {
                    tracing::info!("DNP3 sim: connection ended: {e}");
                }
            });
        }
    });
    handle
}

struct Selected {
    objects: Vec<u8>,
    seq: u8,
    at: Instant,
}

struct Connection {
    ch: Channel,
    cfg: OutstationConfig,
    h: OutstationHandle,
    unsol_seq: u8,
    selected: Option<Selected>,
    stash: VecDeque<Fragment>,
    last_request: Option<(Vec<u8>, Vec<Vec<u8>>)>,
}

/// One object header ready to send, with the events it carries.
struct Chunk {
    bytes: Vec<u8>,
    events: Vec<u64>,
}

impl Connection {
    fn new(stream: TcpStream, cfg: OutstationConfig, h: OutstationHandle) -> Self {
        stream.set_nodelay(true).ok();
        Self {
            ch: Channel::new(stream, cfg.address, cfg.master_address, false),
            cfg,
            h,
            unsol_seq: 0,
            selected: None,
            stash: VecDeque::new(),
            last_request: None,
        }
    }

    async fn run(mut self) -> Result<()> {
        // A restarted outstation announces itself with a null unsolicited
        // response before the master enables reporting.
        self.send_unsolicited(Vec::new()).await?;
        let changed = self.h.changed.clone();
        loop {
            let req = if let Some(f) = self.stash.pop_front().or_else(|| self.ch.pop_ready()) {
                f
            } else {
                tokio::select! {
                    r = self.ch.read_more() => { r?; self.ch.process().await?; }
                    _ = changed.notified() => {}
                    _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                }
                self.maybe_unsolicited().await?;
                continue;
            };
            self.on_request(req).await?;
        }
    }

    async fn maybe_unsolicited(&mut self) -> Result<()> {
        let chunks = {
            let db = self.h.db.lock().unwrap_or_else(|e| e.into_inner());
            let classes = db.unsolicited;
            if !classes.iter().any(|&b| b) {
                return Ok(());
            }
            event_chunks(&db, |c| classes[(c - 1) as usize])
        };
        if !chunks.is_empty() {
            self.send_unsolicited(chunks).await?;
        }
        Ok(())
    }

    async fn send_unsolicited(&mut self, chunks: Vec<Chunk>) -> Result<()> {
        let seq = self.unsol_seq;
        self.unsol_seq = (seq + 1) & 0x0F;
        let mut objects = Vec::new();
        let mut events = Vec::new();
        for c in chunks {
            if objects.len() + c.bytes.len() + 4 > self.cfg.max_fragment {
                break;
            }
            objects.extend(c.bytes);
            events.extend(c.events);
        }
        let frag = Fragment {
            control: AppControl {
                fir: true,
                fin: true,
                con: true,
                uns: true,
                seq,
            },
            function: FunctionCode::UnsolicitedResponse,
            iin: Some(self.iin()),
            objects,
        };
        self.ch.send(&frag).await?;
        if self.await_confirm(seq, true).await? {
            self.release_events(&events);
        }
        Ok(())
    }

    fn iin(&self) -> Iin {
        self.h.read(|db| db.iin())
    }

    fn release_events(&self, ids: &[u64]) {
        if !ids.is_empty() {
            self.h
                .update(|db| db.events.retain(|e| !ids.contains(&e.id)));
        }
    }

    /// Wait for a confirm; requests arriving meanwhile are kept for later.
    async fn await_confirm(&mut self, seq: u8, uns: bool) -> Result<bool> {
        let deadline = Instant::now() + self.cfg.confirm_timeout;
        loop {
            let Some(f) = self.ch.recv(deadline).await? else {
                return Ok(false);
            };
            if f.function == FunctionCode::Confirm {
                if f.control.seq == seq && f.control.uns == uns {
                    return Ok(true);
                }
                continue;
            }
            self.stash.push_back(f);
        }
    }

    async fn on_request(&mut self, req: Fragment) -> Result<()> {
        if req.function == FunctionCode::Confirm {
            return Ok(());
        }
        let seq = req.control.seq;
        // Spec: a repeated request (same seq and bytes) gets the same answer
        // again without being executed twice.
        let raw = req.encode();
        if let Some((last, responses)) = &self.last_request
            && *last == raw
            && req.function != FunctionCode::Read
        {
            for r in responses.clone() {
                let f = Fragment::decode(&r)?;
                self.ch.send(&f).await?;
            }
            return Ok(());
        }
        let (chunks, iin2) = self.execute(&req);
        let sent = self.respond(seq, chunks, iin2).await?;
        self.last_request = Some((raw, sent));
        Ok(())
    }

    /// Send the response, splitting it into fragments of at most
    /// `max_fragment` and waiting for each confirm the spec requires.
    async fn respond(&mut self, seq: u8, chunks: Vec<Chunk>, iin2: u16) -> Result<Vec<Vec<u8>>> {
        let room = self.cfg.max_fragment.saturating_sub(4).max(16);
        let mut frags: Vec<(Vec<u8>, Vec<u64>)> = vec![(Vec::new(), Vec::new())];
        for c in chunks {
            let last = frags.last_mut().expect("non-empty");
            if !last.0.is_empty() && last.0.len() + c.bytes.len() > room {
                frags.push((Vec::new(), Vec::new()));
            }
            let last = frags.last_mut().expect("non-empty");
            last.0.extend(c.bytes);
            last.1.extend(c.events);
        }
        let n = frags.len();
        let mut seq = seq;
        let mut sent = Vec::new();
        for (i, (objects, events)) in frags.into_iter().enumerate() {
            let fin = i + 1 == n;
            let mut iin = self.iin();
            iin.0 |= iin2;
            let frag = Fragment {
                control: AppControl {
                    fir: i == 0,
                    fin,
                    con: !fin || !events.is_empty(),
                    uns: false,
                    seq,
                },
                function: FunctionCode::Response,
                iin: Some(iin),
                objects,
            };
            self.ch.send(&frag).await?;
            sent.push(frag.encode());
            if frag.control.con {
                if !self.await_confirm(seq, false).await? {
                    tracing::info!("DNP3 sim: no confirm for seq {seq}; events kept");
                    break;
                }
                self.release_events(&events);
            }
            seq = (seq + 1) & 0x0F;
        }
        Ok(sent)
    }

    /// Run a request; returns the response objects and IIN2 error bits.
    fn execute(&mut self, req: &Fragment) -> (Vec<Chunk>, u16) {
        match req.function {
            FunctionCode::Read => match parse_read_headers(&req.objects) {
                Ok(blocks) => self.read(&blocks),
                Err(_) => (Vec::new(), Iin::OBJECT_UNKNOWN),
            },
            FunctionCode::Write => self.write(req),
            FunctionCode::EnableUnsolicited | FunctionCode::DisableUnsolicited => {
                let enable = req.function == FunctionCode::EnableUnsolicited;
                let Ok(blocks) = parse_read_headers(&req.objects) else {
                    return (Vec::new(), Iin::OBJECT_UNKNOWN);
                };
                self.h.update(|db| {
                    for b in &blocks {
                        if b.group == 60 && (2..=4).contains(&b.variation) {
                            db.unsolicited[(b.variation - 2) as usize] = enable;
                        }
                    }
                });
                (Vec::new(), 0)
            }
            FunctionCode::DelayMeasure => {
                // g52v2: processing time in ms, ~0 for this simulator.
                let bytes = vec![52, 2, qualifier::COUNT_8, 1, 0, 0];
                (
                    vec![Chunk {
                        bytes,
                        events: vec![],
                    }],
                    0,
                )
            }
            FunctionCode::RecordCurrentTime => (Vec::new(), 0),
            FunctionCode::Select | FunctionCode::Operate | FunctionCode::DirectOperate => {
                self.control(req)
            }
            _ => (Vec::new(), Iin::NO_FUNC_CODE_SUPPORT),
        }
    }

    fn read(&self, blocks: &[ObjectBlock]) -> (Vec<Chunk>, u16) {
        let db = self.h.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut classes = [false; 4];
        let mut statics = Vec::new();
        for b in blocks {
            match (b.group, b.variation) {
                (60, v @ 1..=4) => classes[(v - 1) as usize] = true,
                (1 | 10 | 30 | 40, _) if b.qualifier == qualifier::ALL_OBJECTS => {
                    statics.push(b.group)
                }
                _ => return (Vec::new(), Iin::OBJECT_UNKNOWN),
            }
        }
        let mut chunks = event_chunks(&db, |c| classes[c as usize]);
        if classes[0] {
            statics.extend([1, 10, 30, 40]);
        }
        for g in statics {
            chunks.extend(static_chunks(&db, g));
        }
        (chunks, 0)
    }

    fn write(&self, req: &Fragment) -> (Vec<Chunk>, u16) {
        let Ok(blocks) = parse_object_headers(&req.objects) else {
            return (Vec::new(), Iin::OBJECT_UNKNOWN);
        };
        let mut iin2 = 0;
        self.h.update(|db| {
            for b in &blocks {
                match (b.group, b.variation) {
                    (80, 1) => {
                        for item in &b.items {
                            if item.index == 7 && item.data[0] == 0 {
                                db.restart = false;
                            } else {
                                iin2 |= Iin::PARAMETER_ERROR;
                            }
                        }
                    }
                    (50, 1) | (50, 3) => db.need_time = false,
                    _ => iin2 |= Iin::OBJECT_UNKNOWN,
                }
            }
        });
        (Vec::new(), iin2)
    }

    fn control(&mut self, req: &Fragment) -> (Vec<Chunk>, u16) {
        let Ok(blocks) = parse_object_headers(&req.objects) else {
            return (Vec::new(), Iin::OBJECT_UNKNOWN);
        };
        let seq = req.control.seq;
        // SBO rule: OPERATE must repeat the selected objects exactly, with
        // the next sequence number, before the select times out.
        let status = match req.function {
            FunctionCode::Select => None,
            FunctionCode::Operate => match self.selected.take() {
                Some(s)
                    if s.objects == req.objects
                        && s.seq.wrapping_add(1) & 0x0F == seq
                        && s.at.elapsed() <= self.cfg.select_timeout =>
                {
                    None
                }
                Some(s) if s.at.elapsed() > self.cfg.select_timeout => Some(1), // timeout
                _ => Some(2),                                                   // no select
            },
            _ => None,
        };
        let execute = req.function != FunctionCode::Select;
        let mut out = req.objects.clone();
        let mut iin2 = 0;
        let mut pos = 0;
        for b in &blocks {
            // Header (3) + count (2) for qualifier 0x28 per object block.
            pos += 3 + match b.qualifier {
                0x17 => 1,
                0x28 => 2,
                _ => {
                    iin2 |= Iin::PARAMETER_ERROR;
                    continue;
                }
            };
            let prefix = if b.qualifier == 0x17 { 1 } else { 2 };
            for item in &b.items {
                pos += prefix + item.data.len();
                let st = status.unwrap_or_else(|| self.check_and_apply(b, item, execute));
                out[pos - 1] = st;
            }
        }
        if req.function == FunctionCode::Select && iin2 == 0 {
            self.selected = Some(Selected {
                objects: req.objects.clone(),
                seq,
                at: Instant::now(),
            });
        }
        (
            vec![Chunk {
                bytes: out,
                events: vec![],
            }],
            iin2,
        )
    }

    /// Validate one control and, when `execute`, apply it. Returns the
    /// status code to echo.
    fn check_and_apply(
        &self,
        b: &ObjectBlock,
        item: &crate::dnp3::objects::ObjectItem,
        execute: bool,
    ) -> u8 {
        let index = item.index as usize;
        let d = &item.data;
        self.h.update(|db| match (b.group, b.variation) {
            (12, 1) => {
                if index >= db.binary_outputs.len() {
                    return 4;
                }
                let on = match d[0] & 0x0F {
                    0x01 | 0x03 => d[0] & 0x80 == 0, // trip pulses mean off
                    0x02 | 0x04 => false,
                    _ => return 3,
                };
                if execute {
                    db.binary_outputs[index] = on;
                    db.operations += 1;
                }
                0
            }
            (41, v @ 1..=4) => {
                if index >= db.analog_outputs.len() {
                    return 4;
                }
                let value: f64 = match v {
                    1 => i32::from_le_bytes([d[0], d[1], d[2], d[3]]) as f64,
                    2 => i16::from_le_bytes([d[0], d[1]]) as f64,
                    3 => f32::from_le_bytes([d[0], d[1], d[2], d[3]]) as f64,
                    _ => f64::from_le_bytes(d[..8].try_into().unwrap_or([0; 8])),
                };
                if !(i16::MIN as f64..=i16::MAX as f64).contains(&value) {
                    return 12;
                }
                if execute {
                    db.analog_outputs[index] = value as i16;
                    db.operations += 1;
                }
                0
            }
            _ => 4,
        })
    }
}

/// Event headers for the classes `want` selects (by class number 1..3).
fn event_chunks(db: &Database, want: impl Fn(u8) -> bool) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    for e in db.events.iter().filter(|e| want(e.class)) {
        let mut bytes;
        match &e.kind {
            EventKind::Binary { index, value, time } => {
                bytes = vec![2, 2, qualifier::COUNT_16_INDEX_16, 1, 0];
                bytes.extend_from_slice(&index.to_le_bytes());
                bytes.push(ONLINE | if *value { STATE } else { 0 });
                bytes.extend_from_slice(&time.to_le_bytes()[..6]);
            }
            EventKind::Analog { index, value } => {
                bytes = vec![32, 1, qualifier::COUNT_16_INDEX_16, 1, 0];
                bytes.extend_from_slice(&index.to_le_bytes());
                bytes.push(ONLINE);
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        chunks.push(Chunk {
            bytes,
            events: vec![e.id],
        });
    }
    chunks
}

/// Static data for one group, in headers of at most 32 points so a big
/// database splits cleanly across fragments.
fn static_chunks(db: &Database, group: u8) -> Vec<Chunk> {
    let (variation, n) = match group {
        1 => (2, db.binary_inputs.len()),
        10 => (2, db.binary_outputs.len()),
        30 => (1, db.analog_inputs.len()),
        40 => (2, db.analog_outputs.len()),
        _ => return Vec::new(),
    };
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < n {
        let stop = (start + 32).min(n) - 1;
        let mut bytes =
            crate::dnp3::application::header::range(group, variation, start as u16, stop as u16);
        for i in start..=stop {
            match group {
                1 => bytes.push(ONLINE | if db.binary_inputs[i] { STATE } else { 0 }),
                10 => bytes.push(ONLINE | if db.binary_outputs[i] { STATE } else { 0 }),
                30 => {
                    bytes.push(ONLINE);
                    bytes.extend_from_slice(&db.analog_inputs[i].to_le_bytes());
                }
                _ => {
                    bytes.push(ONLINE);
                    bytes.extend_from_slice(&db.analog_outputs[i].to_le_bytes());
                }
            }
        }
        chunks.push(Chunk {
            bytes,
            events: vec![],
        });
        start = stop + 1;
    }
    chunks
}
