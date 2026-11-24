//! DNP3 master session over TCP: polls, unsolicited handling, controls and
//! time synchronisation for a single outstation.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::dnp3::application::{self, Control, Fragment, FunctionCode, Iin};
use crate::dnp3::channel::Channel;
use crate::dnp3::objects::{self, Dnp3PointValue};
use crate::models::WriteResult;

/// How time is set when the outstation raises IIN1.4 NEED_TIME.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeSyncMode {
    /// Delay Measure (FC 23) then WRITE g50v1 corrected by half the round trip.
    #[default]
    NonLan,
    /// Record Current Time (FC 24) then WRITE g50v3. Recommended on LANs, but
    /// fewer outstations implement it.
    Lan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperateMode {
    SelectBeforeOperate,
    DirectOperate,
}

pub struct Dnp3MasterSession {
    channel: Channel,
    app_seq: u8,
    response_timeout: Duration,
    pub time_sync: TimeSyncMode,
    /// IIN from the most recent response, solicited or not.
    pub iin: Iin,
    unsolicited: Vec<Dnp3PointValue>,
    last_unsolicited: Option<Vec<u8>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn points_of(fragments: &[Fragment]) -> Vec<Dnp3PointValue> {
    let mut out = Vec::new();
    for f in fragments {
        // Points before a bad header are kept: they came from well-formed
        // headers, and dropping them would blank good tags.
        let (blocks, err) = objects::parse_object_headers_lenient(&f.objects);
        if let Some(e) = err {
            tracing::warn!("DNP3 response objects: {e}");
        }
        out.extend(objects::points_from_blocks(&blocks));
    }
    out
}

impl Dnp3MasterSession {
    pub async fn connect(
        ip: &str,
        port: u16,
        master_addr: u16,
        outstation_addr: u16,
    ) -> Result<Self> {
        let socket_addr: SocketAddr = format!("{ip}:{port}")
            .parse()
            .context("Invalid DNP3 address")?;
        let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(socket_addr))
            .await
            .context("DNP3 connection timeout")?
            .context("DNP3 connection failed")?;
        stream.set_nodelay(true).ok();
        Ok(Self::from_stream(stream, master_addr, outstation_addr))
    }

    pub fn from_stream(stream: TcpStream, master_addr: u16, outstation_addr: u16) -> Self {
        Self {
            channel: Channel::new(stream, master_addr, outstation_addr, true),
            app_seq: 0,
            response_timeout: Duration::from_secs(5),
            time_sync: TimeSyncMode::default(),
            iin: Iin::default(),
            unsolicited: Vec::new(),
            last_unsolicited: None,
        }
    }

    pub fn set_response_timeout(&mut self, t: Duration) {
        self.response_timeout = t;
    }

    fn next_seq(&mut self) -> u8 {
        let s = self.app_seq;
        self.app_seq = (self.app_seq + 1) & 0x0F;
        s
    }

    /// Send the request `build` makes for the next sequence number and collect
    /// every fragment of the response, confirming those that ask for it.
    async fn request(&mut self, build: impl FnOnce(u8) -> Fragment) -> Result<Vec<Fragment>> {
        let seq = self.next_seq();
        let req = build(seq);
        self.channel.send(&req).await?;
        let mut expected = seq;
        let mut fragments: Vec<Fragment> = Vec::new();
        let mut deadline = Instant::now() + self.response_timeout;
        loop {
            let Some(f) = self.channel.recv(deadline).await? else {
                anyhow::bail!(
                    "DNP3 response timeout ({:?}) to {:?}",
                    self.response_timeout,
                    req.function
                );
            };
            match f.function {
                FunctionCode::UnsolicitedResponse => {
                    self.on_unsolicited(f).await?;
                    continue;
                }
                FunctionCode::Response => {}
                other => {
                    tracing::debug!("DNP3 master: ignoring {other:?} from outstation");
                    continue;
                }
            }
            let c = f.control;
            if c.seq != expected {
                tracing::debug!("DNP3: stale response seq {} (want {expected})", c.seq);
                continue;
            }
            if fragments.is_empty() != c.fir {
                anyhow::bail!("DNP3 response fragment out of order (FIR={})", c.fir);
            }
            if c.con {
                self.channel.send(&Fragment::confirm(c.seq, false)).await?;
            }
            if let Some(iin) = f.iin {
                self.iin = iin;
            }
            let fin = c.fin;
            fragments.push(f);
            if fin {
                return Ok(fragments);
            }
            expected = (expected + 1) & 0x0F;
            deadline = Instant::now() + self.response_timeout;
        }
    }

    async fn on_unsolicited(&mut self, f: Fragment) -> Result<()> {
        let raw = f.encode();
        if f.control.con {
            self.channel
                .send(&Fragment::confirm(f.control.seq, true))
                .await?;
        }
        // A repeat means our confirm was lost; it is confirmed again but its
        // events were already taken.
        if self.last_unsolicited.as_deref() == Some(&raw[..]) {
            return Ok(());
        }
        if let Some(iin) = f.iin {
            self.iin = iin;
        }
        self.unsolicited.extend(points_of(std::slice::from_ref(&f)));
        self.last_unsolicited = Some(raw);
        Ok(())
    }

    /// Points received in unsolicited responses since the last call.
    pub fn take_unsolicited(&mut self) -> Vec<Dnp3PointValue> {
        std::mem::take(&mut self.unsolicited)
    }

    /// Wait until `deadline` handling unsolicited traffic. Returns early with
    /// a command from `commands`, or with `None` once unsolicited points are
    /// waiting in `take_unsolicited`.
    pub async fn idle_until<T>(
        &mut self,
        deadline: Instant,
        commands: &mut tokio::sync::mpsc::Receiver<T>,
    ) -> Result<Option<T>> {
        loop {
            while let Some(f) = self.channel.pop_ready() {
                if f.function == FunctionCode::UnsolicitedResponse {
                    self.on_unsolicited(f).await?;
                }
            }
            // Hand unsolicited data back now rather than at the deadline,
            // which may be a whole scan interval away.
            if !self.unsolicited.is_empty() {
                return Ok(None);
            }
            tokio::select! {
                r = self.channel.read_more() => r?,
                c = commands.recv() => return Ok(c),
                _ = tokio::time::sleep_until(deadline) => return Ok(None),
            }
            self.channel.process().await?;
        }
    }

    fn check_iin2(&self, what: &str) -> Result<()> {
        if let Some(e) = self.iin.request_error() {
            anyhow::bail!("DNP3 {what} rejected by outstation: {e}");
        }
        Ok(())
    }

    /// Class 1, 2, 3 and 0 read.
    pub async fn integrity_poll(&mut self) -> Result<Vec<Dnp3PointValue>> {
        let frags = self.request(application::integrity_poll).await?;
        Ok(points_of(&frags))
    }

    /// Class 1, 2 and 3 read.
    pub async fn event_poll(&mut self) -> Result<Vec<Dnp3PointValue>> {
        let frags = self.request(application::event_poll).await?;
        Ok(points_of(&frags))
    }

    pub async fn set_unsolicited(&mut self, enable: bool) -> Result<()> {
        self.request(|s| application::set_unsolicited(s, enable))
            .await?;
        self.check_iin2(if enable {
            "enable unsolicited"
        } else {
            "disable unsolicited"
        })
    }

    pub async fn clear_restart(&mut self) -> Result<()> {
        self.request(application::clear_restart).await?;
        self.check_iin2("clear restart")
    }

    pub async fn time_sync(&mut self) -> Result<()> {
        match self.time_sync {
            TimeSyncMode::NonLan => {
                let t0 = Instant::now();
                let frags = self
                    .request(|s| Fragment::request(s, FunctionCode::DelayMeasure, vec![]))
                    .await?;
                let round_trip = t0.elapsed().as_millis() as u64;
                self.check_iin2("delay measure")?;
                let mut outstation_ms = 0u64;
                for f in &frags {
                    for b in objects::parse_object_headers(&f.objects)? {
                        if let (52, v @ (1 | 2), Some(item)) =
                            (b.group, b.variation, b.items.first())
                        {
                            let d = u16::from_le_bytes([item.data[0], item.data[1]]) as u64;
                            outstation_ms = if v == 1 { d * 1000 } else { d };
                        }
                    }
                }
                let delay = round_trip.saturating_sub(outstation_ms) / 2;
                self.request(|s| application::write_time(s, now_ms() + delay))
                    .await?;
                self.check_iin2("time write")
            }
            TimeSyncMode::Lan => {
                let mut recorded = 0;
                self.request(|s| {
                    recorded = now_ms();
                    Fragment::request(s, FunctionCode::RecordCurrentTime, vec![])
                })
                .await?;
                self.check_iin2("record current time")?;
                self.request(|s| application::write_last_recorded_time(s, recorded))
                    .await?;
                self.check_iin2("time write")
            }
        }
    }

    /// Spec startup: disable unsolicited, integrity poll, clear the restart
    /// bit and set the time if asked, then enable unsolicited.
    pub async fn startup(&mut self, unsolicited: bool) -> Result<Vec<Dnp3PointValue>> {
        if unsolicited && let Err(e) = self.set_unsolicited(false).await {
            tracing::info!("DNP3: disable unsolicited failed ({e}); continuing");
        }
        let mut points = self.integrity_poll().await?;
        points.extend(self.housekeeping().await?);
        if unsolicited && let Err(e) = self.set_unsolicited(true).await {
            tracing::warn!("DNP3: enable unsolicited failed ({e}); polling only");
        }
        Ok(points)
    }

    /// Act on IIN: clear a restart (and re-read everything, since the
    /// outstation's values may have been reset), then set the time if needed.
    pub async fn housekeeping(&mut self) -> Result<Vec<Dnp3PointValue>> {
        let mut points = Vec::new();
        if self.iin.device_restart() {
            self.clear_restart().await?;
            points = self.integrity_poll().await?;
            if self.iin.device_restart() {
                tracing::warn!("DNP3: outstation still reports DEVICE_RESTART");
            }
        }
        if self.iin.need_time()
            && let Err(e) = self.time_sync().await
        {
            tracing::warn!("DNP3 time sync failed: {e}");
        }
        Ok(points)
    }

    /// SELECT then OPERATE, or DIRECT OPERATE, one CROB or analog output.
    pub async fn operate(&mut self, control: Control, mode: OperateMode) -> Result<WriteResult> {
        let start = std::time::Instant::now();
        let outcome = match mode {
            OperateMode::SelectBeforeOperate => {
                match self.control_step(control, FunctionCode::Select).await? {
                    Ok(()) => self.control_step(control, FunctionCode::Operate).await?,
                    Err(e) => Err(format!("select: {e}")),
                }
            }
            OperateMode::DirectOperate => {
                self.control_step(control, FunctionCode::DirectOperate)
                    .await?
            }
        };
        let latency_ms = start.elapsed().as_millis() as u64;
        Ok(match outcome {
            Ok(()) => WriteResult {
                success: true,
                device_response: Some(format!("{control:?} {mode:?}: success")),
                error: None,
                latency_ms,
            },
            Err(e) => WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("DNP3 {mode:?} failed: {e}")),
                latency_ms,
            },
        })
    }

    /// The outer `Result` is a link failure; the inner one is the
    /// outstation refusing the control.
    async fn control_step(
        &mut self,
        control: Control,
        fc: FunctionCode,
    ) -> Result<std::result::Result<(), String>> {
        let frags = self.request(|s| control.fragment(s, fc)).await?;
        if let Some(e) = self.iin.request_error() {
            return Ok(Err(e.to_string()));
        }
        let Some(f) = frags.first() else {
            return Ok(Err("empty response".into()));
        };
        match application::check_control_echo(&control.object_headers(), &f.objects) {
            Ok(0) => Ok(Ok(())),
            Ok(status) => Ok(Err(format!(
                "status {status} ({})",
                application::control_status_text(status)
            ))),
            Err(e) => Ok(Err(e.to_string())),
        }
    }
}
