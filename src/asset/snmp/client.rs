use std::net::Ipv4Addr;
use std::time::Duration;

use snmp::{SyncSession, Value};
use tracing::debug;

use crate::asset::models::SnmpScanResult;

// ── OID Constants ───────────────────────────────────────────────────────────

/// sysDescr.0 — system description string
pub const OID_SYS_DESCR: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 1, 0];
/// sysName.0 — administratively-assigned name
pub const OID_SYS_NAME: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 5, 0];
/// sysUpTime.0 — time since network management portion was last re-initialized
pub const OID_SYS_UP_TIME: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 3, 0];

// ── SNMP Host Query ─────────────────────────────────────────────────────────

/// Query a single host via SNMPv2c for sysDescr, sysName, and sysUpTime.
///
/// Returns `Some(SnmpScanResult)` on success, `None` if the host does not
/// respond or returns an error.
///
/// This function performs blocking I/O (the `snmp` crate is synchronous) and
/// is designed to be called from a `tokio::task::spawn_blocking` context.
pub fn snmp_query_host(ip: Ipv4Addr, community: &str, timeout_ms: u32) -> Option<SnmpScanResult> {
    let addr = format!("{}:161", ip);
    let timeout = Duration::from_millis(timeout_ms as u64);

    // Build the SNMP session
    let mut sess = match SyncSession::new(&*addr, community.as_bytes(), Some(timeout), 0) {
        Ok(s) => s,
        Err(e) => {
            debug!(%ip, error = %e, "SNMP session creation failed");
            return None;
        }
    };

    // Query sysDescr
    let sys_descr = match sess.get(OID_SYS_DESCR) {
        Ok(mut resp) => resp.varbinds.next().and_then(|(_oid, val)| match val {
            Value::OctetString(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        }),
        Err(e) => {
            debug!(%ip, error = ?e, "SNMP get sysDescr failed");
            return None;
        }
    };

    // Query sysName
    let sys_name = sess.get(OID_SYS_NAME).ok().and_then(|mut resp| {
        resp.varbinds.next().and_then(|(_oid, val)| match val {
            Value::OctetString(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        })
    });

    // Query sysUpTime (Timeticks value in hundredths of a second)
    let sys_uptime = sess.get(OID_SYS_UP_TIME).ok().and_then(|mut resp| {
        resp.varbinds.next().and_then(|(_oid, val)| match val {
            Value::Timeticks(ticks) => Some(format_timeticks(ticks)),
            Value::Integer(n) => Some(format_timeticks(n as u32)),
            _ => None,
        })
    });

    let sys_descr_str = sys_descr.unwrap_or_default();
    let (vendor, model, firmware) = parse_sys_descr(&sys_descr_str);

    Some(SnmpScanResult {
        ip: ip.to_string(),
        sys_descr: if sys_descr_str.is_empty() {
            None
        } else {
            Some(sys_descr_str)
        },
        sys_name,
        sys_uptime,
        vendor,
        model,
        firmware,
        mac_address: None, // MAC not available via standard SNMP OIDs without ifTable walk
        scanned_at: chrono_free_timestamp(),
    })
}

/// Format timeticks (hundredths of a second) into a human-readable uptime
/// string like "3d 14h 22m 11s".
pub fn format_timeticks(ticks: u32) -> String {
    let total_secs = ticks as u64 / 100;
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    parts.push(format!("{}h", hours));
    parts.push(format!("{}m", minutes));
    parts.push(format!("{}s", seconds));
    parts.join(" ")
}

/// Generate an ISO-8601-ish timestamp without pulling in `chrono`.
pub fn chrono_free_timestamp() -> String {
    // Use `time` crate which is already a dependency
    use time::OffsetDateTime;
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

// ── sysDescr Parsing ────────────────────────────────────────────────────────

/// Parse a sysDescr string to extract vendor, model, and firmware version.
///
/// Recognises common OT device patterns from Siemens, Rockwell/Allen-Bradley,
/// Schneider/Modicon, Cisco, and falls back to generic extraction.
///
/// Returns `(vendor, model, firmware)` — each may be `None` if not detected.
pub fn parse_sys_descr(sys_descr: &str) -> (Option<String>, Option<String>, Option<String>) {
    let descr = sys_descr.trim();
    if descr.is_empty() {
        return (None, None, None);
    }

    let lower = descr.to_lowercase();

    // ── Siemens ──
    if lower.contains("siemens") || lower.contains("s7-") || lower.contains("simatic") {
        return parse_siemens(descr);
    }

    // ── Rockwell / Allen-Bradley ──
    if lower.contains("rockwell")
        || lower.contains("allen-bradley")
        || lower.contains("controllogix")
        || lower.contains("compactlogix")
        || lower.contains("micrologix")
        || lower.contains("slc")
        || lower.contains("plc-5")
        || lower.contains("factorytalk")
    {
        return parse_rockwell(descr);
    }

    // ── Schneider / Modicon ──
    if lower.contains("schneider")
        || lower.contains("modicon")
        || lower.contains("quantum")
        || lower.contains("premium")
        || lower.contains("m340")
        || lower.contains("m580")
    {
        return parse_schneider(descr);
    }

    // ── Cisco ──
    if lower.contains("cisco") || lower.contains("ios ") {
        return parse_cisco(descr);
    }

    // ── Generic fallback ──
    parse_generic(descr)
}

fn parse_siemens(descr: &str) -> (Option<String>, Option<String>, Option<String>) {
    let vendor = Some("Siemens".to_string());

    // Patterns: "SIMATIC S7-1500, Firmware V2.8" or "Siemens, S7-300, HW..."
    let model = extract_word_after(descr, &["SIMATIC ", "S7-", "CPU "]);
    let firmware = extract_version(descr);

    (vendor, model, firmware)
}

fn parse_rockwell(descr: &str) -> (Option<String>, Option<String>, Option<String>) {
    let vendor = Some("Rockwell Automation".to_string());

    let model = extract_word_after(
        descr,
        &[
            "ControlLogix",
            "CompactLogix",
            "MicroLogix",
            "1756-",
            "1769-",
            "1766-",
            "1768-",
        ],
    );
    let firmware = extract_version(descr);

    (vendor, model, firmware)
}

fn parse_schneider(descr: &str) -> (Option<String>, Option<String>, Option<String>) {
    let vendor = Some("Schneider Electric".to_string());

    let model = extract_word_after(descr, &["Modicon ", "Quantum", "Premium", "M340", "M580"]);
    let firmware = extract_version(descr);

    (vendor, model, firmware)
}

fn parse_cisco(descr: &str) -> (Option<String>, Option<String>, Option<String>) {
    let vendor = Some("Cisco".to_string());

    let model = extract_word_after(descr, &["Cisco ", "IOS "]);
    let firmware = extract_version(descr);

    (vendor, model, firmware)
}

fn parse_generic(descr: &str) -> (Option<String>, Option<String>, Option<String>) {
    // Try to find a vendor-like word (capitalised, early in the string)
    let vendor = descr
        .split([',', '\n', ';'])
        .next()
        .and_then(|s| s.split_whitespace().next())
        .filter(|w| w.len() > 1 && w.chars().next().is_some_and(|c| c.is_uppercase()))
        .map(|w| w.trim_end_matches(',').to_string());

    let firmware = extract_version(descr);

    (vendor, None, firmware)
}

/// Extract the first word found immediately after one of the given markers.
fn extract_word_after(text: &str, markers: &[&str]) -> Option<String> {
    let lower = text.to_lowercase();
    for marker in markers {
        if let Some(idx) = lower.find(&marker.to_lowercase()) {
            let start = idx + marker.len();
            let rest = text[start..].trim_start();
            // Take the first word (may include hyphens for model numbers)
            let word: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '.')
                .collect();
            if !word.is_empty() {
                return Some(word.trim_end_matches('-').trim_end_matches('.').to_string());
            }
        }
    }
    None
}

/// Try to extract a version string from the description.
fn extract_version(text: &str) -> Option<String> {
    let lower = text.to_lowercase();

    // Look for common version patterns
    let patterns = [
        "firmware v",
        "firmware version ",
        "version ",
        "v",
        "sw version ",
        "software version ",
        "hw: ",
        "fw: ",
    ];

    for pat in &patterns {
        if let Some(idx) = lower.find(pat) {
            let start = idx + pat.len();
            let rest = text[start..].trim_start();
            let version: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
                .collect();
            // Validate it looks like a version (starts with a digit)
            if version.starts_with(|c: char| c.is_ascii_digit()) && version.len() >= 2 {
                return Some(version);
            }
        }
    }

    None
}
