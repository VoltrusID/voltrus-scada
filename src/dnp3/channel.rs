//! Link + transport over one TCP stream, shared by the master and the
//! simulator outstation. Yields whole application fragments.

use std::collections::VecDeque;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::dnp3::application::Fragment;
use crate::dnp3::datalink::{ControlByte, LinkFrame, ParseResult, func, parse_frame};
use crate::dnp3::transport::{SegmentResult, TransportReassembler, segment};

pub struct Channel {
    stream: TcpStream,
    rx: Vec<u8>,
    reassembler: TransportReassembler,
    tx_seq: u8,
    local: u16,
    remote: u16,
    is_master: bool,
    ready: VecDeque<Fragment>,
}

impl Channel {
    pub fn new(stream: TcpStream, local: u16, remote: u16, is_master: bool) -> Self {
        Self {
            stream,
            rx: Vec::with_capacity(1024),
            reassembler: TransportReassembler::new(),
            tx_seq: 0,
            local,
            remote,
            is_master,
            ready: VecDeque::new(),
        }
    }

    pub fn peer_addr(&self) -> Option<std::net::SocketAddr> {
        self.stream.peer_addr().ok()
    }

    pub async fn send(&mut self, fragment: &Fragment) -> Result<()> {
        let (segments, next) = segment(&fragment.encode(), self.tx_seq);
        self.tx_seq = next;
        let mut bytes = Vec::new();
        for s in segments {
            bytes
                .extend(LinkFrame::user_data(self.is_master, self.remote, self.local, s).encode()?);
        }
        self.stream
            .write_all(&bytes)
            .await
            .context("DNP3 send failed")
    }

    async fn send_link(&mut self, func_code: u8) -> Result<()> {
        let control = ControlByte {
            dir: self.is_master,
            prm: false,
            fcb: false,
            fcv: false,
            func: func_code,
        };
        let bytes = LinkFrame::new(control, self.remote, self.local, vec![]).encode()?;
        self.stream
            .write_all(&bytes)
            .await
            .context("DNP3 link reply failed")
    }

    /// Read whatever bytes are available. Cancel safe: nothing is consumed
    /// unless the read completes, so it can sit in a `select!`.
    pub async fn read_more(&mut self) -> Result<()> {
        let mut buf = [0u8; 2048];
        let n = self
            .stream
            .read(&mut buf)
            .await
            .context("DNP3 read failed")?;
        if n == 0 {
            anyhow::bail!("DNP3 peer closed the connection");
        }
        self.rx.extend_from_slice(&buf[..n]);
        Ok(())
    }

    /// Turn buffered bytes into fragments, answering link-layer requests.
    pub async fn process(&mut self) -> Result<()> {
        loop {
            match parse_frame(&self.rx) {
                Err(e) => {
                    // Resynchronise on the next start octet.
                    tracing::debug!("DNP3 link: {e}; resynchronising");
                    let skip = self.rx[1..]
                        .iter()
                        .position(|&b| b == 0x05)
                        .map_or(self.rx.len(), |p| p + 1);
                    self.rx.drain(..skip);
                }
                Ok(ParseResult::Incomplete) => return Ok(()),
                Ok(ParseResult::Frame(frame, used)) => {
                    self.rx.drain(..used);
                    self.on_frame(frame).await?;
                }
            }
        }
    }

    async fn on_frame(&mut self, frame: LinkFrame) -> Result<()> {
        if frame.destination != self.local || frame.source != self.remote {
            tracing::debug!(
                "DNP3 link: ignoring frame {} -> {}",
                frame.source,
                frame.destination
            );
            return Ok(());
        }
        if !frame.control.prm {
            // Secondary frames only answer link requests we never make.
            return Ok(());
        }
        let data = match frame.control.func {
            func::PRI_RESET_LINK_STATES | func::PRI_TEST_LINK_STATES => {
                self.send_link(func::SEC_ACK).await?;
                return Ok(());
            }
            func::PRI_REQUEST_LINK_STATUS => {
                // Outstations use this as a keep-alive.
                self.send_link(func::SEC_LINK_STATUS).await?;
                return Ok(());
            }
            func::PRI_CONFIRMED_USER_DATA => {
                self.send_link(func::SEC_ACK).await?;
                frame.data
            }
            func::PRI_UNCONFIRMED_USER_DATA => frame.data,
            _ => {
                self.send_link(func::SEC_NOT_SUPPORTED).await?;
                return Ok(());
            }
        };
        match self.reassembler.push(&data) {
            SegmentResult::Complete(apdu) => match Fragment::decode(&apdu) {
                Ok(f) => self.ready.push_back(f),
                Err(e) => tracing::warn!("DNP3: dropping undecodable fragment: {e}"),
            },
            SegmentResult::Pending => {}
            SegmentResult::Discarded(why) => tracing::warn!("DNP3 transport: {why}"),
        }
        Ok(())
    }

    pub fn pop_ready(&mut self) -> Option<Fragment> {
        self.ready.pop_front()
    }

    /// Next fragment, or `None` once `deadline` passes.
    pub async fn recv(&mut self, deadline: Instant) -> Result<Option<Fragment>> {
        loop {
            if let Some(f) = self.ready.pop_front() {
                return Ok(Some(f));
            }
            match tokio::time::timeout_at(deadline, self.read_more()).await {
                Err(_) => return Ok(None),
                Ok(r) => r?,
            }
            self.process().await?;
        }
    }
}
