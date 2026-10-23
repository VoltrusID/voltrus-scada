use std::net::{IpAddr, Ipv4Addr};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;

// ── Public types ──

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveredDevice {
    pub ip: String,
    pub port: u16,
    pub unit_id: u8,
    pub responsive_registers: Vec<DiscoveredRegister>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveredRegister {
    pub address: u16,
    pub value: f64,
    pub raw: Vec<u16>,
}

// ── CIDR parsing ──

/// Parse a CIDR notation string (e.g., "192.168.1.0/24") into a list of IPv4 addresses.
pub fn parse_cidr(cidr: &str) -> Result<Vec<IpAddr>> {
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid CIDR format: expected 'addr/mask', got '{}'", cidr);
    }

    let ip: Ipv4Addr = parts[0]
        .parse()
        .context(format!("Invalid IP address: {}", parts[0]))?;
    let mask: u32 = parts[1]
        .parse()
        .context(format!("Invalid subnet mask: {}", parts[1]))?;

    if mask > 32 {
        anyhow::bail!("Subnet mask must be 0-32, got {}", mask);
    }

    let ip_u32 = u32::from(ip);

    if mask == 32 {
        return Ok(vec![IpAddr::V4(ip)]);
    }

    let host_bits = 32 - mask;
    let network_mask = if mask == 0 { 0 } else { !0u32 << host_bits };
    let network = ip_u32 & network_mask;
    let broadcast = network | !network_mask;
    let host_count = broadcast - network - 1;

    // Limit to /24 at most (254 hosts) to prevent accidental large scans
    if host_count > 254 {
        anyhow::bail!(
            "Subnet /{} has {} hosts — maximum supported is /24 (254 hosts). Use a smaller subnet range.",
            mask,
            host_count
        );
    }

    let mut addresses = Vec::new();
    for i in 1..=host_count {
        let addr_u32 = network + i;
        let addr = Ipv4Addr::from(addr_u32);
        addresses.push(IpAddr::V4(addr));
    }

    Ok(addresses)
}

// ── Network scanning ──

/// Scan a subnet for Modbus TCP devices.
/// Returns a list of responsive devices with their register values.
pub async fn scan_network(cidr: &str, port: u16, timeout_ms: u64) -> Result<Vec<DiscoveredDevice>> {
    let addresses = parse_cidr(cidr)?;

    let concurrency_limit = 50;
    let timeout = std::time::Duration::from_millis(timeout_ms);

    // Use a semaphore to limit concurrency
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency_limit));

    let mut handles = Vec::new();

    for ip in addresses {
        let permit = semaphore.clone().acquire_owned().await?;
        let ip_str = ip.to_string();

        handles.push(tokio::spawn(async move {
            let _permit = permit;
            probe_device(&ip_str, port, timeout).await
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(Some(device)) = handle.await {
            results.push(device);
        }
    }

    // Sort by IP for consistent output
    results.sort_by(|a, b| {
        let a_parts: Vec<u16> = a.ip.split('.').filter_map(|s| s.parse().ok()).collect();
        let b_parts: Vec<u16> = b.ip.split('.').filter_map(|s| s.parse().ok()).collect();
        a_parts.cmp(&b_parts)
    });

    Ok(results)
}

/// Probe a single IP:port for Modbus TCP responsiveness.
/// Tries common unit IDs and reads holding registers at addresses 0-9.
async fn probe_device(
    ip: &str,
    port: u16,
    timeout: std::time::Duration,
) -> Option<DiscoveredDevice> {
    let addr = format!("{}:{}", ip, port);
    let socket_addr: std::net::SocketAddr = addr.parse().ok()?;

    // Try unit IDs 1, 2, 3 (most common)
    for unit_id in [1u8, 2, 3] {
        let result = try_read_registers(socket_addr, unit_id, timeout).await;
        if let Some(registers) = result {
            return Some(DiscoveredDevice {
                ip: ip.to_string(),
                port,
                unit_id,
                responsive_registers: registers,
            });
        }
    }

    None
}

/// Attempt to read holding registers 0-9 from a Modbus TCP device.
async fn try_read_registers(
    socket_addr: std::net::SocketAddr,
    unit_id: u8,
    timeout: std::time::Duration,
) -> Option<Vec<DiscoveredRegister>> {
    let connect_result =
        tokio::time::timeout(timeout, tcp::connect_slave(socket_addr, Slave(unit_id))).await;

    let mut ctx = match connect_result {
        Ok(Ok(ctx)) => ctx,
        _ => return None,
    };

    // Try reading first 10 holding registers (addresses 0-9)
    let read_result = tokio::time::timeout(timeout, ctx.read_holding_registers(0, 10)).await;

    match read_result {
        Ok(Ok(result)) => match result {
            Ok(regs) => {
                let mut discovered = Vec::new();
                let words: Vec<u16> = regs.to_vec();
                for (i, &word) in words.iter().enumerate() {
                    discovered.push(DiscoveredRegister {
                        address: i as u16,
                        value: word as f64,
                        raw: vec![word],
                    });
                }
                Some(discovered)
            }
            Err(_) => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cidr_single_host() {
        let addrs = parse_cidr("192.168.1.1/32").unwrap();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "192.168.1.1");
    }

    #[test]
    fn test_parse_cidr_small_subnet() {
        let addrs = parse_cidr("192.168.1.0/30").unwrap();
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0].to_string(), "192.168.1.1");
        assert_eq!(addrs[1].to_string(), "192.168.1.2");
    }

    #[test]
    fn test_parse_cidr_class_c() {
        let addrs = parse_cidr("10.0.0.0/24").unwrap();
        assert_eq!(addrs.len(), 254);
        assert_eq!(addrs[0].to_string(), "10.0.0.1");
        assert_eq!(addrs[253].to_string(), "10.0.0.254");
    }

    #[test]
    fn test_parse_cidr_class_b_rejected() {
        // /16 has 65534 hosts — should be rejected
        let result = parse_cidr("10.0.0.0/16");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("maximum supported")
        );
    }

    #[test]
    fn test_parse_cidr_invalid_format() {
        assert!(parse_cidr("not-a-cidr").is_err());
        assert!(parse_cidr("192.168.1.0").is_err());
        assert!(parse_cidr("192.168.1.0/33").is_err());
        assert!(parse_cidr("192.168.1.0/abc").is_err());
        assert!(parse_cidr("999.999.999.999/24").is_err());
    }

    #[test]
    fn test_parse_cidr_zero_mask() {
        // /0 has 2^32 - 2 hosts — should be rejected
        let result = parse_cidr("0.0.0.0/0");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cidr_edge_cases() {
        // /31 — point-to-point link, 0 usable hosts (RFC 3021 allows 2)
        let addrs = parse_cidr("192.168.1.0/31").unwrap();
        assert_eq!(addrs.len(), 0);

        // /25 — 126 hosts
        let addrs = parse_cidr("10.0.0.0/25").unwrap();
        assert_eq!(addrs.len(), 126);
    }

    #[test]
    fn test_discovered_device_serialization() {
        let device = DiscoveredDevice {
            ip: "192.168.1.10".to_string(),
            port: 502,
            unit_id: 1,
            responsive_registers: vec![DiscoveredRegister {
                address: 0,
                value: 220.0,
                raw: vec![220],
            }],
        };
        let json = serde_json::to_string(&device).unwrap();
        assert!(json.contains("192.168.1.10"));
        assert!(json.contains("\"port\":502"));
        assert!(json.contains("\"unit_id\":1"));
    }

    #[test]
    fn test_discovered_register_equality() {
        let r1 = DiscoveredRegister {
            address: 0,
            value: 100.0,
            raw: vec![100],
        };
        let r2 = DiscoveredRegister {
            address: 0,
            value: 100.0,
            raw: vec![100],
        };
        assert_eq!(r1, r2);
    }

    #[tokio::test]
    async fn test_probe_device_timeout() {
        // Probe a non-existent device — should return None quickly
        let result = probe_device("192.0.2.1", 502, std::time::Duration::from_millis(100)).await;
        assert!(result.is_none());
    }
}
