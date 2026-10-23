// src/bin/dnp3-sim.rs

#[path = "../dnp3/datalink.rs"]
mod datalink;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// DNP3 outstation simulator for testing.
#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20000);
    let outstation_addr: u16 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await.expect("Failed to bind");
    tracing_subscriber::fmt::init();
    tracing::info!(
        "DNP3 outstation simulator listening on {}, address={}",
        addr,
        outstation_addr
    );

    loop {
        let (mut stream, peer) = listener.accept().await.expect("Accept failed");
        tracing::info!("Connection from {}", peer);

        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) => {
                        tracing::info!("Client disconnected");
                        return;
                    }
                    Ok(n) => {
                        tracing::debug!("Received {} bytes", n);
                        if let Ok(frame) = datalink::Dnp3Frame::decode(&buf[..n]) {
                            tracing::info!(
                                "Frame: master={}, dest={}, src={}, data_len={}",
                                frame.master,
                                frame.destination,
                                frame.source,
                                frame.data.len()
                            );

                            let response = datalink::Dnp3Frame {
                                master: false,
                                fcb: false,
                                fcv: false,
                                sequence: frame.sequence,
                                destination: frame.source,
                                source: frame.destination,
                                data: vec![0x81, 0x80, 0x00, 0x00],
                            };
                            let response_bytes = match response.encode() {
                                Ok(b) => b,
                                Err(e) => {
                                    tracing::error!("Encode error: {}", e);
                                    return;
                                }
                            };
                            if stream.write_all(&response_bytes).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Read error: {}", e);
                        return;
                    }
                }
            }
        });
    }
}
