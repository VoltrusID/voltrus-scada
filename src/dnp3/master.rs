#![allow(dead_code)]
// src/dnp3/master.rs

use std::collections::HashMap;
use std::net::SocketAddr;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::dnp3::application::*;
use crate::dnp3::datalink::Dnp3Frame;
use crate::dnp3::objects;
use crate::dnp3::objects::Dnp3PointValue;
use crate::models::WriteResult;

/// Tracks application layer sequence numbers (0-15, wraps).
pub struct SequenceTracker {
    seq: u8,
}

impl SequenceTracker {
    pub fn new() -> Self {
        Self { seq: 0 }
    }

    pub fn next(&mut self) -> u8 {
        let current = self.seq;
        self.seq = (self.seq + 1) & 0x0F;
        current
    }
}

/// Build a link-layer reset frame (master -> outstation).
pub fn build_link_reset_frame(master_addr: u16, outstation_addr: u16) -> Dnp3Frame {
    Dnp3Frame {
        master: true,
        fcb: false,
        fcv: false,
        sequence: 0,
        destination: outstation_addr,
        source: master_addr,
        data: vec![0xC0, 0x01],
    }
}

/// DNP3 master session connected to a single outstation.
pub struct Dnp3MasterSession {
    stream: TcpStream,
    master_addr: u16,
    outstation_addr: u16,
    seq: SequenceTracker,
    fcb: bool,
}

impl Dnp3MasterSession {
    pub async fn connect(
        ip: &str,
        port: u16,
        master_addr: u16,
        outstation_addr: u16,
    ) -> Result<Self> {
        let addr = format!("{}:{}", ip, port);
        let socket_addr: SocketAddr = addr.parse().context("Invalid DNP3 address")?;
        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            TcpStream::connect(socket_addr),
        )
        .await
        .context("DNP3 connection timeout")?
        .context("DNP3 connection failed")?;

        let mut session = Self {
            stream,
            master_addr,
            outstation_addr,
            seq: SequenceTracker::new(),
            fcb: false,
        };

        session.send_link_reset().await?;
        session.receive_frame().await?;

        Ok(session)
    }

    pub async fn integrity_poll(&mut self) -> Result<HashMap<(u8, u8, u16), Dnp3PointValue>> {
        let seq = self.seq.next();
        let asdu = build_integrity_poll_request(seq);
        self.send_asdu(&asdu).await?;
        self.read_response_objects().await
    }

    pub async fn event_scan(
        &mut self,
        class: u8,
    ) -> Result<HashMap<(u8, u8, u16), Dnp3PointValue>> {
        let seq = self.seq.next();
        let asdu = build_event_scan_request(seq, class);
        self.send_asdu(&asdu).await?;
        self.read_response_objects().await
    }

    // DNP3 spec: freeze counter command (not yet wired to poller/API).
    pub async fn freeze_counters(&mut self) -> Result<()> {
        let seq = self.seq.next();
        let asdu = build_freeze_request(seq);
        self.send_asdu(&asdu).await?;
        let _ = self.receive_frame().await;
        Ok(())
    }

    pub async fn direct_operate_binary(&mut self, index: u16, on: bool) -> Result<WriteResult> {
        let start = std::time::Instant::now();
        let seq = self.seq.next();
        let asdu = build_direct_operate_binary(seq, index, on);
        self.send_asdu(&asdu).await?;

        match tokio::time::timeout(std::time::Duration::from_secs(5), self.receive_frame()).await {
            Ok(Ok(_)) => Ok(WriteResult {
                success: true,
                device_response: Some(format!("Direct operate: index={} on={}", index, on)),
                error: None,
                latency_ms: start.elapsed().as_millis() as u64,
            }),
            Ok(Err(e)) => Ok(WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("DNP3 direct operate failed: {e}")),
                latency_ms: start.elapsed().as_millis() as u64,
            }),
            Err(_) => Ok(WriteResult {
                success: false,
                device_response: None,
                error: Some("DNP3 direct operate timeout (5s)".into()),
                latency_ms: start.elapsed().as_millis() as u64,
            }),
        }
    }

    async fn send_link_reset(&mut self) -> Result<()> {
        let frame = build_link_reset_frame(self.master_addr, self.outstation_addr);
        let bytes = frame.encode()?;
        self.stream
            .write_all(&bytes)
            .await
            .context("Send link reset failed")?;
        Ok(())
    }

    async fn send_asdu(&mut self, asdu: &[u8]) -> Result<()> {
        let frame = Dnp3Frame {
            master: true,
            fcb: self.fcb,
            fcv: true,
            sequence: 0,
            destination: self.outstation_addr,
            source: self.master_addr,
            data: asdu.to_vec(),
        };
        let bytes = frame.encode()?;
        self.stream
            .write_all(&bytes)
            .await
            .context("Send ASDU failed")?;
        self.fcb = !self.fcb;
        Ok(())
    }

    async fn receive_frame(&mut self) -> Result<Dnp3Frame> {
        let mut header = [0u8; 10];
        self.stream
            .read_exact(&mut header[0..3])
            .await
            .context("Read frame header failed")?;

        if header[0] != 0x05 || header[1] != 0x64 {
            anyhow::bail!("Invalid DNP3 start bytes");
        }

        let data_len = header[2] as usize;
        let total_frame_size = 3 + data_len + (2 * data_len.div_ceil(16));
        let mut buf = vec![0u8; total_frame_size];
        buf[0..3].copy_from_slice(&header[0..3]);
        self.stream
            .read_exact(&mut buf[3..total_frame_size])
            .await
            .context("Read frame body failed")?;

        Dnp3Frame::decode(&buf)
    }

    async fn read_response_objects(&mut self) -> Result<HashMap<(u8, u8, u16), Dnp3PointValue>> {
        let frame = self.receive_frame().await?;
        let (header, parsed_objects) = parse_response_asdu(&frame.data)?;

        if header.function != FunctionCode::Response {
            anyhow::bail!("Expected RESPONSE, got {:?}", header.function);
        }

        let mut results = HashMap::new();
        for obj in parsed_objects {
            let values = objects::parse_objects(
                obj.group,
                obj.variation,
                obj.start_index,
                obj.count as u16,
                &obj.data,
            );
            for v in values {
                results.insert((v.group, v.variation, v.index), v);
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_sequence_increment() {
        let mut seq = SequenceTracker::new();
        assert_eq!(seq.next(), 0);
        assert_eq!(seq.next(), 1);
        assert_eq!(seq.next(), 2);
    }

    #[test]
    fn test_session_sequence_wraps() {
        let mut seq = SequenceTracker::new();
        for _ in 0..15 {
            seq.next();
        }
        assert_eq!(seq.next(), 15);
        assert_eq!(seq.next(), 0);
    }

    #[test]
    fn test_build_link_reset_frame() {
        let frame = build_link_reset_frame(1, 10);
        assert_eq!(frame.master, true);
        assert_eq!(frame.destination, 10);
        assert_eq!(frame.source, 1);
        assert!(!frame.data.is_empty());
        assert_eq!(frame.fcb, false);
        assert_eq!(frame.fcv, false);
    }
}
