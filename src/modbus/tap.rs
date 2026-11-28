//! A tee around the TCP stream tokio-modbus talks over: every byte passes
//! through unchanged, and while the device is watched each complete ADU in
//! either direction is copied into the device's traffic ring.
//!
//! tokio-modbus does not expose its frames, so the tap sits under its codec
//! and cuts the byte stream into ADUs itself (MBAP length). It never delays,
//! reorders or drops bytes: polling behaves the same with or without it.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use voltrus_dissect::Dir;
use voltrus_dissect::modbus::Splitter;

use crate::traffic::{DeviceTraffic, Proto};

pub struct Tap<S> {
    inner: S,
    traffic: Arc<DeviceTraffic>,
    conn: u64,
    tx: Splitter,
    rx: Splitter,
    /// Whether the last bytes seen were recorded; after a pause the
    /// splitters start clean.
    was_recording: bool,
}

impl<S> Tap<S> {
    pub fn new(inner: S, traffic: Arc<DeviceTraffic>, conn: u64) -> Self {
        Self {
            inner,
            traffic,
            conn,
            tx: Splitter::default(),
            rx: Splitter::default(),
            was_recording: false,
        }
    }

    fn observe(&mut self, dir: Dir, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // Not watched: one atomic load, no clock read, nothing copied.
        let now_us = if self.traffic.watched() {
            crate::traffic::now_us()
        } else {
            0
        };
        if now_us == 0 || !self.traffic.recording_at(now_us) {
            self.was_recording = false;
            return;
        }
        if !self.was_recording {
            self.tx.reset();
            self.rx.reset();
            self.was_recording = true;
        }
        let (traffic, conn) = (&self.traffic, self.conn);
        let splitter = match dir {
            Dir::Tx => &mut self.tx,
            Dir::Rx => &mut self.rx,
        };
        splitter.push(bytes, |adu| {
            traffic.record(conn, Proto::ModbusTcp, dir, adu, now_us)
        });
    }
}

impl<S> std::fmt::Debug for Tap<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tap").field("conn", &self.conn).finish()
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Tap<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let res = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = res {
            let this = &mut *self;
            this.observe(Dir::Rx, &buf.filled()[before..]);
        }
        res
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Tap<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let res = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = res {
            self.observe(Dir::Tx, &buf[..n]);
        }
        res
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::TrafficHub;
    use tokio_modbus::prelude::*;
    use tokio_modbus::server::Service;
    use tokio_modbus::server::tcp::{Server, accept_tcp_connection};

    #[derive(Clone)]
    struct Regs;

    impl Service for Regs {
        type Request = Request<'static>;
        type Response = Response;
        type Exception = ExceptionCode;
        type Future = std::future::Ready<Result<Response, ExceptionCode>>;

        fn call(&self, req: Self::Request) -> Self::Future {
            std::future::ready(match req {
                Request::ReadHoldingRegisters(addr, n) if addr < 100 => {
                    Ok(Response::ReadHoldingRegisters((addr..addr + n).collect()))
                }
                Request::ReadHoldingRegisters(..) => Err(ExceptionCode::IllegalDataAddress),
                _ => Err(ExceptionCode::IllegalFunction),
            })
        }
    }

    async fn server() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let server = Server::new(listener);
            let on_connected = |stream, socket_addr| async move {
                accept_tcp_connection(stream, socket_addr, |_| Ok(Some(Regs)))
            };
            let _ = server.serve(&on_connected, |e| eprintln!("{e}")).await;
        });
        addr
    }

    async fn client(hub: &TrafficHub, addr: std::net::SocketAddr) -> tokio_modbus::client::Context {
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let tap = Tap::new(stream, hub.device("plc"), hub.next_conn());
        tokio_modbus::client::tcp::attach_slave(tap, Slave(1))
    }

    #[tokio::test]
    async fn records_both_directions_only_while_watched() {
        let addr = server().await;
        let hub = TrafficHub::default();
        let mut ctx = client(&hub, addr).await;

        // Not watched: polling works and nothing is kept.
        assert_eq!(
            ctx.read_holding_registers(10, 2).await.unwrap().unwrap(),
            [10, 11]
        );
        assert!(hub.device("plc").snapshot().frames.is_empty());

        hub.device("plc").watch();
        assert_eq!(
            ctx.read_holding_registers(10, 2).await.unwrap().unwrap(),
            [10, 11]
        );
        let exc = ctx.read_holding_registers(500, 1).await.unwrap();
        assert_eq!(exc, Err(ExceptionCode::IllegalDataAddress));

        let snap = hub.device("plc").snapshot();
        let dirs: Vec<Dir> = snap.frames.iter().map(|f| f.dir).collect();
        assert_eq!(dirs, [Dir::Tx, Dir::Rx, Dir::Tx, Dir::Rx]);
        let decoded = crate::traffic::decode("plc", &snap.frames, i64::MAX / 2);
        assert_eq!(decoded[1].pair_id, Some(decoded[0].seq));
        assert_eq!(
            decoded[1].summary,
            "Read Holding Registers response, 2 registers"
        );
        let err = decoded[3].error.as_ref().unwrap();
        assert_eq!(err.code, Some(2));
        assert!(decoded[2].error.is_none(), "answered, so not flagged");
    }

    /// Cost of the tap per poll (one 10-register read: a 12-byte request and
    /// a 29-byte response), watched and not. Run in release:
    /// `cargo test --release --lib modbus::tap::tests::overhead -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn overhead() {
        let hub = TrafficHub::default();
        let dev = hub.device("bench");
        let req = [0u8, 1, 0, 0, 0, 6, 1, 3, 0, 0, 0, 10];
        let mut resp = vec![0u8, 1, 0, 0, 0, 23, 1, 3, 20];
        resp.extend([0x12; 20]);
        let mut tap = Tap::new((), dev.clone(), 1);
        let n = 1_000_000;
        let run = |tap: &mut Tap<()>| {
            let t = std::time::Instant::now();
            for _ in 0..n {
                tap.observe(Dir::Tx, std::hint::black_box(&req));
                tap.observe(Dir::Rx, std::hint::black_box(&resp));
            }
            t.elapsed().as_nanos() as f64 / n as f64
        };
        let idle = run(&mut tap);
        dev.watch();
        let watched = run(&mut tap);
        let frames = dev.snapshot().frames;
        let t = std::time::Instant::now();
        let decoded = crate::traffic::decode("bench", &frames, 0);
        let decode_ns = t.elapsed().as_nanos() as f64 / decoded.len() as f64;
        println!(
            "tap per poll: {idle:.1} ns not watched, {watched:.1} ns watched; decode on read: {decode_ns:.0} ns per frame ({} frames)",
            decoded.len()
        );
    }
}
