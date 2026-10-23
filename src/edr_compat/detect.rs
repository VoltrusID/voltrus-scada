//! EDR/antivirus product detection
//!
//! Detects running EDR and antivirus products on the current system.
//! On Windows, checks running processes and services. On other platforms,
//! returns an empty list (EDR is primarily a Windows concern in OT).

use crate::edr_compat::EdrProduct;

/// Known EDR/antivirus products to detect.
/// Each entry contains process/service names to check and metadata.
struct KnownProduct {
    name: &'static str,
    vendor: &'static str,
    category: &'static str,
    /// Process names to look for (case-insensitive)
    process_names: &'static [&'static str],
    /// Notes about compatibility
    notes: Option<&'static str>,
}

/// Catalog of known OT-relevant EDR products.
/// Endpoint agents pushed by Active Directory can use significant CPU on SCADA servers.
fn known_products() -> Vec<KnownProduct> {
    vec![
        KnownProduct {
            name: "Microsoft Defender Antivirus",
            vendor: "Microsoft",
            category: "antivirus",
            process_names: &["MsMpEng", "MsSense", "SenseIR"],
            notes: Some(
                "Built-in on Windows. Add exclusions for Voltrus directories and processes. Consider Defender for OT sensors for network-level monitoring.",
            ),
        },
        KnownProduct {
            name: "Microsoft Defender for Endpoint",
            vendor: "Microsoft",
            category: "edr",
            process_names: &["MsSense", "SenseIR", "SenseCncProxy", "SenseSampleUploader"],
            notes: Some(
                "Enterprise EDR. May be eligible for free Defender for OT licenses. Ensure Voltrus process and data directories are excluded from real-time scanning.",
            ),
        },
        KnownProduct {
            name: "Trellix ENS (formerly McAfee)",
            vendor: "Trellix",
            category: "antivirus",
            process_names: &[
                "McAfeeValidationTrustProtection",
                "mfemms",
                "macompatsvc",
                "McShield",
            ],
            notes: Some(
                "Common in OT environments. Known for high CPU during real-time scanning of telemetry databases. Exclusions strongly recommended.",
            ),
        },
        KnownProduct {
            name: "Trellix EDR (formerly FireEye)",
            vendor: "Trellix",
            category: "edr",
            process_names: &["xagt", "xagtnotif", "FireEyeMAR"],
            notes: Some(
                "xagt.exe known to waste CPU monitoring SCADA communications. Exclude Voltrus binary and data directories.",
            ),
        },
        KnownProduct {
            name: "CrowdStrike Falcon",
            vendor: "CrowdStrike",
            category: "edr",
            process_names: &["CSFalconService", "CSFalconContainer", "CSFalconUI"],
            notes: Some(
                "Generally well-behaved with SCADA software. Add process exclusion for Voltrus executable.",
            ),
        },
        KnownProduct {
            name: "SentinelOne",
            vendor: "SentinelOne",
            category: "edr",
            process_names: &[
                "SentinelAgent",
                "SentinelServiceHost",
                "SentinelBrowserNativeHost",
            ],
            notes: Some(
                "May flag Voltrus binary due to unsigned executable. Add path exclusion and submit for whitelisting.",
            ),
        },
        KnownProduct {
            name: "Carbon Black",
            vendor: "Broadcom (VMware)",
            category: "edr",
            process_names: &["cb", "RepMgr", "RepUtils", "RepWSS"],
            notes: Some(
                "May slow down database writes due to real-time monitoring. Exclude Voltrus data directory.",
            ),
        },
        KnownProduct {
            name: "Cisco Secure Endpoint (formerly AMP)",
            vendor: "Cisco",
            category: "edr",
            process_names: &["Immunet", "CiscoAMP"],
            notes: None,
        },
        KnownProduct {
            name: "CyberArk EPM",
            vendor: "CyberArk",
            category: "edr",
            process_names: &["CyberArkEPM", "EpmAgent"],
            notes: Some(
                "Privilege management may block Voltrus from binding to network ports. Elevate Voltrus binary.",
            ),
        },
        KnownProduct {
            name: "Fortinet FortiClient",
            vendor: "Fortinet",
            category: "antivirus",
            process_names: &["FortiClient", "FortiShield", "FortiProxy"],
            notes: Some(
                "Fortinet offers OT-specific protocol filtering. Voltrus data should be excluded from scan.",
            ),
        },
        KnownProduct {
            name: "Nozomi Networks Guardian",
            vendor: "Nozomi",
            category: "ot_security",
            process_names: &["n2os", "nozomi"],
            notes: Some(
                "Passive OT network monitoring. No interaction with Voltrus — purely network-based.",
            ),
        },
        KnownProduct {
            name: "Claroty xDome",
            vendor: "Claroty",
            category: "ot_security",
            process_names: &["claroty", "ClarotyEdge"],
            notes: Some("Passive OT monitoring platform. No direct interaction with Voltrus host."),
        },
        KnownProduct {
            name: "Trend Micro Apex One",
            vendor: "Trend Micro",
            category: "antivirus",
            process_names: &["PccNT", "TmListen", "NTRtScan", "TmCCSF"],
            notes: Some(
                "Common in APAC OT environments. Exclude Voltrus directories from real-time scan.",
            ),
        },
    ]
}

/// Detect running EDR/antivirus products on this system.
/// Returns a list of detected products with their status.
pub fn detect_edr_products() -> Vec<EdrProduct> {
    let mut detected = Vec::new();

    // Check running processes for known EDR products
    if let Ok(processes) = list_running_processes() {
        let process_names_lower: Vec<String> = processes.iter().map(|p| p.to_lowercase()).collect();

        for product in known_products() {
            let matching_processes: Vec<&String> = process_names_lower
                .iter()
                .filter(|proc| {
                    product
                        .process_names
                        .iter()
                        .any(|known| proc.contains(&known.to_lowercase()))
                })
                .collect();

            if !matching_processes.is_empty() {
                detected.push(EdrProduct {
                    name: product.name.to_string(),
                    vendor: product.vendor.to_string(),
                    active: true,
                    notes: product.notes.map(|s| s.to_string()),
                    category: product.category.to_string(),
                });
            }
        }
    }

    // On Windows, also check Windows Defender service status
    #[cfg(target_os = "windows")]
    {
        if is_defender_active() {
            // Don't add duplicate if already detected via process scan
            if !detected
                .iter()
                .any(|p| p.name == "Microsoft Defender Antivirus")
            {
                detected.push(EdrProduct {
                    name: "Microsoft Defender Antivirus".to_string(),
                    vendor: "Microsoft".to_string(),
                    active: true,
                    notes: Some(
                        "Built-in on Windows. Add exclusions for Voltrus directories and processes."
                            .to_string(),
                    ),
                    category: "antivirus".to_string(),
                });
            }
        }
    }

    detected
}

/// List currently running process names on the system.
fn list_running_processes() -> Result<Vec<String>, String> {
    #[cfg(target_os = "windows")]
    {
        list_windows_processes()
    }

    #[cfg(not(target_os = "windows"))]
    {
        list_unix_processes()
    }
}

/// List running processes on Windows using `tasklist`.
#[cfg(target_os = "windows")]
fn list_windows_processes() -> Result<Vec<String>, String> {
    let output = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
        .map_err(|e| format!("Failed to run tasklist: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();

    for line in stdout.lines() {
        // CSV format: "name.exe","PID","Session Name","Session#","Mem Usage"
        if let Some(name) = line.split(',').next() {
            let name = name.trim_matches('"').trim();
            // Strip .exe extension for matching
            let name = name.strip_suffix(".exe").unwrap_or(name).to_string();
            if !name.is_empty() {
                processes.push(name);
            }
        }
    }

    Ok(processes)
}

/// List running processes on Unix using `ps`.
#[cfg(not(target_os = "windows"))]
fn list_unix_processes() -> Result<Vec<String>, String> {
    let output = std::process::Command::new("ps")
        .args(["-axco", "comm"])
        .output()
        .map_err(|e| format!("Failed to run ps: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let processes: Vec<String> = stdout
        .lines()
        .skip(1) // Skip header
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    Ok(processes)
}

/// Check if Windows Defender real-time protection is active.
#[cfg(target_os = "windows")]
fn is_defender_active() -> bool {
    // Check if the Windows Defender service is running
    let output = std::process::Command::new("sc")
        .args(["query", "WinDefend"])
        .output();

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
        return stdout.contains("running");
    }
    false
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_edr_products_returns_vec() {
        // This test just verifies the function doesn't panic
        let products = detect_edr_products();
        // On a dev machine, we may or may not detect products
        assert!(products.len() <= known_products().len());
    }

    #[test]
    fn test_known_products_catalog() {
        let products = known_products();
        // Should have at least the top products mentioned in the roadmap
        assert!(products.len() >= 10);
        let names: Vec<&str> = products.iter().map(|p| p.name).collect();
        assert!(names.contains(&"Trellix EDR (formerly FireEye)"));
        assert!(names.contains(&"Microsoft Defender Antivirus"));
    }

    #[test]
    fn test_list_running_processes() {
        // Should succeed on any platform
        let processes = list_running_processes();
        assert!(processes.is_ok());
        let procs = processes.unwrap();
        // Any running system should have at least a few processes
        assert!(!procs.is_empty());
    }

    #[test]
    fn test_product_categories() {
        let products = known_products();
        for product in &products {
            assert!(
                ["antivirus", "edr", "xdr", "ot_security"].contains(&product.category),
                "Unknown category: {}",
                product.category
            );
        }
    }
}
