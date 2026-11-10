// src/dnp3/discovery.rs

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::dnp3::datalink::{ControlByte, LinkFrame, ParseResult, func, parse_frame};

/// Outstation addresses tried by the probe. Outstations ignore frames for
/// other addresses and never answer broadcasts, so the common defaults are
/// asked in one write and whichever answers reveals its address.
const PROBE_ADDRESSES: [u16; 4] = [10, 1, 0, 1024];
const PROBE_MASTER: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveredDnp3Device {
    pub ip: String,
    pub port: u16,
    pub outstation_address: u16,
}

/// Scan a subnet for DNP3 outstations.
pub async fn scan_network(
    cidr: &str,
    port: u16,
    timeout_ms: u64,
) -> Result<Vec<DiscoveredDnp3Device>> {
    let addresses = crate::discovery::parse_cidr(cidr)?;
    let concurrency_limit = 50;
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency_limit));

    let mut handles = Vec::new();
    for ip in addresses {
        let permit = semaphore.clone().acquire_owned().await?;
        let ip_str = ip.to_string();
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            probe_outstation(&ip_str, port, timeout).await
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(Some(device)) = handle.await {
            results.push(device);
        }
    }

    results.sort_by(|a, b| {
        let a_parts: Vec<u16> = a.ip.split('.').filter_map(|s| s.parse().ok()).collect();
        let b_parts: Vec<u16> = b.ip.split('.').filter_map(|s| s.parse().ok()).collect();
        a_parts.cmp(&b_parts)
    });

    Ok(results)
}

/// Probe a single IP:port for DNP3 outstation.
async fn probe_outstation(
    ip: &str,
    port: u16,
    timeout: std::time::Duration,
) -> Option<DiscoveredDnp3Device> {
    let addr = format!("{}:{}", ip, port);
    let socket_addr: std::net::SocketAddr = addr.parse().ok()?;

    let connect_result = tokio::time::timeout(timeout, TcpStream::connect(socket_addr)).await;
    let mut stream = match connect_result {
        Ok(Ok(s)) => s,
        _ => return None,
    };

    let mut bytes = Vec::new();
    for dest in PROBE_ADDRESSES {
        let control = ControlByte {
            dir: true,
            prm: true,
            fcb: false,
            fcv: false,
            func: func::PRI_REQUEST_LINK_STATUS,
        };
        bytes.extend(
            LinkFrame::new(control, dest, PROBE_MASTER, vec![])
                .encode()
                .ok()?,
        );
    }
    if stream.write_all(&bytes).await.is_err() {
        return None;
    }

    let deadline = tokio::time::Instant::now() + timeout;
    let mut rx = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = match tokio::time::timeout_at(deadline, stream.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => n,
            _ => return None,
        };
        rx.extend_from_slice(&buf[..n]);
        loop {
            match parse_frame(&rx) {
                Ok(ParseResult::Frame(f, _)) if f.destination == PROBE_MASTER => {
                    return Some(DiscoveredDnp3Device {
                        ip: ip.to_string(),
                        port,
                        outstation_address: f.source,
                    });
                }
                Ok(ParseResult::Frame(_, used)) => {
                    rx.drain(..used);
                }
                Ok(ParseResult::Incomplete) => break,
                Err(_) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cidr_dnp3() {
        let addrs = crate::discovery::parse_cidr("192.168.1.0/30").unwrap();
        assert_eq!(addrs.len(), 2);
    }

    #[tokio::test]
    async fn test_probe_nonexistent_device() {
        let result =
            probe_outstation("192.0.2.1", 20000, std::time::Duration::from_millis(100)).await;
        assert!(result.is_none());
    }

    #[test]
    fn test_discovered_device_serialization() {
        let device = DiscoveredDnp3Device {
            ip: "192.168.1.100".to_string(),
            port: 20000,
            outstation_address: 10,
        };
        let json = serde_json::to_string(&device).unwrap();
        assert!(json.contains("192.168.1.100"));
        assert!(json.contains("20000"));
    }
}
