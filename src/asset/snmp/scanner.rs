use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::{Result, bail};
use tokio::sync::Semaphore;
use tokio::task;
use tracing::{info, warn};

use super::client::snmp_query_host;
use crate::asset::models::SnmpScanResult;

// ── Scan State ──────────────────────────────────────────────────────────────

/// Thread-safe scan progress tracker shared across concurrent SNMP queries.
#[derive(Clone)]
pub struct ScanState {
    pub scanning: Arc<AtomicBool>,
    pub progress: Arc<AtomicUsize>,
    pub total: Arc<AtomicUsize>,
}

impl ScanState {
    pub fn new() -> Self {
        Self {
            scanning: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(AtomicUsize::new(0)),
            total: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns true if a scan is currently in progress.
    pub fn is_scanning(&self) -> bool {
        self.scanning.load(Ordering::SeqCst)
    }

    /// Returns (completed, total) progress counters.
    pub fn get_progress(&self) -> (usize, usize) {
        (
            self.progress.load(Ordering::SeqCst),
            self.total.load(Ordering::SeqCst),
        )
    }
}

impl Default for ScanState {
    fn default() -> Self {
        Self::new()
    }
}

/// Maximum number of concurrent SNMP queries per subnet scan.
const MAX_CONCURRENCY: usize = 50;

// ── CIDR Parsing ────────────────────────────────────────────────────────────

/// Parse a CIDR notation string like "192.168.1.0/24" into a network address
/// and prefix length.
///
/// # Errors
/// Returns an error if the format is invalid, the prefix is > 32, or the IP
/// address is malformed.
pub fn parse_cidr(cidr: &str) -> Result<(Ipv4Addr, u32)> {
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 {
        bail!("Invalid CIDR format: expected 'IP/PREFIX', got '{}'", cidr);
    }

    let ip: Ipv4Addr = parts[0]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid IPv4 address: '{}'", parts[0]))?;
    let prefix: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid prefix length: '{}'", parts[1]))?;

    if prefix > 32 {
        bail!("Prefix length must be <= 32, got {}", prefix);
    }

    Ok((ip, prefix))
}

/// Enumerate all host IPs within a CIDR range, skipping the network address
/// (all-zero host bits) and the broadcast address (all-one host bits).
///
/// For /31 (point-to-point) and /32 (single host) ranges, all addresses are
/// included since there is no distinct broadcast address.
pub fn cidr_to_hosts(ip: Ipv4Addr, prefix: u32) -> Vec<Ipv4Addr> {
    if prefix >= 31 {
        // /31 = 2 hosts (point-to-point link, RFC 3021), /32 = 1 host
        // Both cases: include all addresses (no broadcast to skip)
        if prefix == 32 {
            return vec![ip];
        }
        // /31: two addresses
        let base = u32::from(ip);
        return vec![Ipv4Addr::from(base), Ipv4Addr::from(base + 1)];
    }

    let host_bits = 32 - prefix;
    let num_hosts = 2u32.pow(host_bits); // total including network + broadcast
    let base = u32::from(ip);

    // Skip network (offset 0) and broadcast (offset num_hosts - 1)
    let mut hosts = Vec::with_capacity((num_hosts - 2) as usize);
    for i in 1..num_hosts - 1 {
        hosts.push(Ipv4Addr::from(base + i));
    }
    hosts
}

// ── Scan Runner ─────────────────────────────────────────────────────────────

/// Run an SNMP scan across a subnet, querying each host concurrently with a
/// semaphore limit of 50.
///
/// Returns an error if a scan is already in progress (per the `ScanState`).
/// On success, returns all hosts that responded to SNMP queries.
pub async fn run_scan(
    subnet: &str,
    community: &str,
    timeout_ms: u32,
    scan_state: &ScanState,
) -> Result<Vec<SnmpScanResult>> {
    // Reject concurrent scans
    if scan_state.is_scanning() {
        bail!("SNMP scan already in progress");
    }

    // Parse CIDR and enumerate hosts
    let (network, prefix) = parse_cidr(subnet)?;
    let hosts = cidr_to_hosts(network, prefix);
    let total = hosts.len();

    if total == 0 {
        return Ok(Vec::new());
    }

    info!(
        subnet = subnet,
        total_hosts = total,
        community = %community,
        "Starting SNMP scan"
    );

    // Initialise state
    scan_state.scanning.store(true, Ordering::SeqCst);
    scan_state.total.store(total, Ordering::SeqCst);
    scan_state.progress.store(0, Ordering::SeqCst);

    // Semaphore to cap concurrency at MAX_CONCURRENCY
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENCY));
    let progress = Arc::clone(&scan_state.progress);
    let community_owned = community.to_string();

    // Spawn blocking tasks for each host (snmp crate is sync)
    let mut handles = Vec::with_capacity(total);

    for ip in hosts {
        let permit = semaphore.clone().acquire_owned().await?;
        let progress = Arc::clone(&progress);
        let comm = community_owned.clone();

        let handle = task::spawn_blocking(move || {
            let result = snmp_query_host(ip, &comm, timeout_ms);
            drop(permit);
            progress.fetch_add(1, Ordering::SeqCst);
            result
        });
        handles.push(handle);
    }

    // Collect results
    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Some(scan_result)) => results.push(scan_result),
            Ok(None) => { /* host did not respond — expected for most IPs */ }
            Err(e) => {
                warn!(error = %e, "SNMP scan task panicked or was cancelled");
            }
        }
    }

    scan_state.scanning.store(false, Ordering::SeqCst);

    info!(
        subnet = subnet,
        responding = results.len(),
        total_queried = total,
        "SNMP scan complete"
    );

    Ok(results)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::snmp::client::{format_timeticks, parse_sys_descr};

    #[test]
    fn test_parse_cidr_valid() {
        let (ip, prefix) = parse_cidr("192.168.1.0/24").unwrap();
        assert_eq!(ip, Ipv4Addr::new(192, 168, 1, 0));
        assert_eq!(prefix, 24);
    }

    #[test]
    fn test_parse_cidr_invalid_format() {
        assert!(parse_cidr("192.168.1.0").is_err());
        assert!(parse_cidr("192.168.1.0/24/extra").is_err());
        assert!(parse_cidr("not-an-ip/24").is_err());
        assert!(parse_cidr("192.168.1.0/33").is_err());
    }

    #[test]
    fn test_cidr_to_hosts_small() {
        // /30 has 4 addresses: network, 2 usable hosts, broadcast
        let hosts = cidr_to_hosts(Ipv4Addr::new(192, 168, 1, 0), 30);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(hosts[1], Ipv4Addr::new(192, 168, 1, 2));
    }

    #[test]
    fn test_cidr_to_hosts_32() {
        let hosts = cidr_to_hosts(Ipv4Addr::new(10, 0, 0, 1), 32);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0], Ipv4Addr::new(10, 0, 0, 1));
    }

    #[test]
    fn test_cidr_to_hosts_31() {
        // /31 point-to-point: both addresses are usable
        let hosts = cidr_to_hosts(Ipv4Addr::new(10, 0, 0, 0), 31);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0], Ipv4Addr::new(10, 0, 0, 0));
        assert_eq!(hosts[1], Ipv4Addr::new(10, 0, 0, 1));
    }

    #[test]
    fn test_cidr_to_hosts_24() {
        let hosts = cidr_to_hosts(Ipv4Addr::new(192, 168, 1, 0), 24);
        assert_eq!(hosts.len(), 254);
        assert_eq!(hosts[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(hosts[253], Ipv4Addr::new(192, 168, 1, 254));
    }

    #[test]
    fn test_parse_sys_descr_siemens() {
        let (vendor, model, firmware) =
            parse_sys_descr("SIMATIC S7-1500, Firmware V2.8.1, Serial: XYZ");
        assert_eq!(vendor.as_deref(), Some("Siemens"));
        assert!(model.is_some());
        assert!(firmware.is_some());
    }

    #[test]
    fn test_parse_sys_descr_rockwell() {
        let (vendor, _model, _firmware) =
            parse_sys_descr("Rockwell Automation/1756-L83E/Firmware 33.11");
        assert_eq!(vendor.as_deref(), Some("Rockwell Automation"));
    }

    #[test]
    fn test_parse_sys_descr_schneider() {
        let (vendor, _model, _firmware) =
            parse_sys_descr("Schneider Electric, Modicon M580, Firmware V3.20");
        assert_eq!(vendor.as_deref(), Some("Schneider Electric"));
    }

    #[test]
    fn test_parse_sys_descr_cisco() {
        let (vendor, _model, _firmware) = parse_sys_descr("Cisco IOS Software, Version 15.2(4)M7");
        assert_eq!(vendor.as_deref(), Some("Cisco"));
    }

    #[test]
    fn test_parse_sys_descr_empty() {
        let (vendor, model, firmware) = parse_sys_descr("");
        assert!(vendor.is_none());
        assert!(model.is_none());
        assert!(firmware.is_none());
    }

    #[test]
    fn test_format_timeticks() {
        assert_eq!(format_timeticks(0), "0h 0m 0s");
        assert_eq!(format_timeticks(100), "0h 0m 1s");
        // 1 day = 8640000 hundredths of a second
        assert_eq!(format_timeticks(8640000), "1d 0h 0m 0s");
    }

    #[test]
    fn test_scan_state() {
        let state = ScanState::new();
        assert!(!state.is_scanning());
        assert_eq!(state.get_progress(), (0, 0));
    }
}
