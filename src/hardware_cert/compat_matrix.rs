//! Hardware compatibility matrix — minimum requirements per license tier.

use super::platform::PlatformInfo;
use super::{StorageAssessment, TierAssessment, TierCompatibility};

/// Minimum RAM (MB) per tier.
const MIN_RAM_ENGINEERING: u64 = 256;
const MIN_RAM_STARTER: u64 = 512;
const MIN_RAM_PROFESSIONAL: u64 = 1024;
const MIN_RAM_ENTERPRISE: u64 = 2048;

/// Minimum CPU cores per tier.
const MIN_CORES_ENGINEERING: u32 = 1;
const MIN_CORES_STARTER: u32 = 1;
const MIN_CORES_PROFESSIONAL: u32 = 2;
const MIN_CORES_ENTERPRISE: u32 = 4;

/// Minimum available disk (GB) per tier.
const MIN_DISK_ENGINEERING: f64 = 1.0;
const MIN_DISK_STARTER: f64 = 5.0;
const MIN_DISK_PROFESSIONAL: f64 = 20.0;
const MIN_DISK_ENTERPRISE: f64 = 50.0;

/// Default max tags for a tier given hardware specs.
fn calculate_max_tags(ram_mb: u64, cpu_cores: u32, storage_gb: f64) -> u32 {
    // Tags are limited by RAM (each tag uses ~50 bytes in cache)
    // and storage (1 year of 10K tags at 1Hz ≈ 15GB compressed)
    let ram_limited_tags = ((ram_mb as f64 * 1024.0 * 1024.0) / 50.0 * 0.3) as u32; // 30% of RAM for cache
    let storage_limited_tags = ((storage_gb / 15.0) * 10000.0) as u32; // 15GB per 10K tags/year
    let cpu_limited_tags = cpu_cores * 2500; // ~2500 tags per core at 1Hz

    ram_limited_tags
        .min(storage_limited_tags)
        .min(cpu_limited_tags)
        .max(50)
}

/// Assess compatibility for a single tier.
fn assess_tier(
    name: &str,
    platform: &PlatformInfo,
    min_ram: u64,
    min_cores: u32,
    min_disk: f64,
) -> TierAssessment {
    let mut limitations = Vec::new();
    let mut compatible = true;

    if platform.ram_total_mb < min_ram {
        compatible = false;
        limitations.push(format!(
            "RAM: {} MB (need {} MB for {})",
            platform.ram_total_mb, min_ram, name
        ));
    } else if platform.ram_total_mb < min_ram * 2 {
        limitations.push(format!(
            "RAM: {} MB (recommended {} MB for {} with headroom)",
            platform.ram_total_mb,
            min_ram * 2,
            name
        ));
    }

    if platform.cpu_cores < min_cores {
        compatible = false;
        limitations.push(format!(
            "CPU cores: {} (need {} for {})",
            platform.cpu_cores, min_cores, name
        ));
    }

    if platform.disk_available_gb < min_disk {
        compatible = false;
        limitations.push(format!(
            "Available disk: {:.1} GB (need {:.0} GB for {})",
            platform.disk_available_gb, min_disk, name
        ));
    }

    let max_tags = if compatible {
        calculate_max_tags(
            platform.ram_total_mb,
            platform.cpu_cores,
            platform.disk_available_gb,
        )
    } else {
        0
    };

    let compression_recommended =
        platform.storage_type == "sd_card" || platform.storage_type == "emmc" || max_tags > 1000;

    TierAssessment {
        compatible,
        max_tags,
        max_poll_rate: if platform.cpu_cores >= 4 { 10 } else { 1 },
        compression_recommended,
        limitations,
    }
}

/// Generate full tier compatibility assessment.
pub fn assess_compatibility(platform: &PlatformInfo) -> TierCompatibility {
    TierCompatibility {
        engineering: assess_tier(
            "Engineering",
            platform,
            MIN_RAM_ENGINEERING,
            MIN_CORES_ENGINEERING,
            MIN_DISK_ENGINEERING,
        ),
        starter: assess_tier(
            "Starter",
            platform,
            MIN_RAM_STARTER,
            MIN_CORES_STARTER,
            MIN_DISK_STARTER,
        ),
        professional: assess_tier(
            "Professional",
            platform,
            MIN_RAM_PROFESSIONAL,
            MIN_CORES_PROFESSIONAL,
            MIN_DISK_PROFESSIONAL,
        ),
        enterprise: assess_tier(
            "Enterprise",
            platform,
            MIN_RAM_ENTERPRISE,
            MIN_CORES_ENTERPRISE,
            MIN_DISK_ENTERPRISE,
        ),
    }
}

/// Generate storage assessment.
pub fn assess_storage(platform: &PlatformInfo) -> StorageAssessment {
    let write_optimization_critical =
        platform.storage_type == "sd_card" || platform.storage_type == "emmc";

    let estimated_endurance_years = match platform.storage_type.as_str() {
        "sd_card" => {
            // SD cards: typical 100-1000 TBW for A2 cards
            // At 10K tags @ 1Hz with compression: ~24MB/day writes = ~8.7GB/year
            // A2 card (300 TBW): 300,000 / 8.7 ≈ 34,000 years (theoretical)
            // In practice, write amplification + journaling = 50x overhead → ~700 years
            // But cheap cards fail at ~10-50 TBW, so be conservative
            5.0
        }
        "emmc" => 10.0,
        "ssd" => 15.0,
        "nvme" => 20.0,
        "hdd" => 50.0,
        _ => 10.0,
    };

    let mut storage_recommendations = Vec::new();

    match platform.storage_type.as_str() {
        "sd_card" => {
            storage_recommendations
                .push("Use A2-rated MicroSD card (minimum Class 10 / U3)".into());
            storage_recommendations.push("Enable data compression to reduce SD card writes".into());
            storage_recommendations
                .push("Consider NVMe HAT for Raspberry Pi 5 for production deployments".into());
            storage_recommendations
                .push("Schedule periodic backups — SD cards can fail without warning".into());
        }
        "emmc" => {
            storage_recommendations
                .push("Enable data compression to extend flash endurance".into());
            storage_recommendations.push("Monitor write amplification factor".into());
        }
        "hdd" => {
            storage_recommendations.push("Consider SSD upgrade for better write latency".into());
        }
        _ => {}
    }

    StorageAssessment {
        storage_type: platform.storage_type.clone(),
        write_optimization_critical,
        estimated_endurance_years,
        batch_writes_active: true, // We always use batch writes
        write_amplification: 1.0,  // Would need IoCounters integration for real value
        storage_recommendations,
    }
}

/// Get the official hardware compatibility matrix for documentation.
pub fn get_compatibility_matrix() -> Vec<super::HardwareCompatibilityEntry> {
    vec![
        super::HardwareCompatibilityEntry {
            platform: "Raspberry Pi 4 (4GB)".into(),
            cpu: "Broadcom BCM2711 (4-core ARM Cortex-A72 @ 1.5GHz)".into(),
            ram_mb: 4096,
            storage: "MicroSD (A2 recommended)".into(),
            supported_tiers: vec!["engineering".into(), "starter".into()],
            max_tags: 1000,
            max_poll_rate: 1,
            notes: "Suitable for small deployments. Enable compression. Use A2 SD card. Consider USB SSD for better performance.".into(),
        },
        super::HardwareCompatibilityEntry {
            platform: "Raspberry Pi 4 (8GB)".into(),
            cpu: "Broadcom BCM2711 (4-core ARM Cortex-A72 @ 1.5GHz)".into(),
            ram_mb: 8192,
            storage: "MicroSD (A2) or USB SSD".into(),
            supported_tiers: vec!["engineering".into(), "starter".into(), "professional".into()],
            max_tags: 3000,
            max_poll_rate: 1,
            notes: "Good for professional tier with USB SSD. Enable compression for long-term storage on SD card.".into(),
        },
        super::HardwareCompatibilityEntry {
            platform: "Raspberry Pi 5 (4GB)".into(),
            cpu: "Broadcom BCM2712 (4-core ARM Cortex-A76 @ 2.4GHz)".into(),
            ram_mb: 4096,
            storage: "MicroSD (A2) or NVMe HAT".into(),
            supported_tiers: vec!["engineering".into(), "starter".into(), "professional".into()],
            max_tags: 5000,
            max_poll_rate: 1,
            notes: "NVMe HAT recommended for production. 2-3x faster than Pi 4.".into(),
        },
        super::HardwareCompatibilityEntry {
            platform: "Raspberry Pi 5 (8GB)".into(),
            cpu: "Broadcom BCM2712 (4-core ARM Cortex-A76 @ 2.4GHz)".into(),
            ram_mb: 8192,
            storage: "NVMe HAT recommended, MicroSD (A2) OK for small deployments".into(),
            supported_tiers: vec!["engineering".into(), "starter".into(), "professional".into()],
            max_tags: 5000,
            max_poll_rate: 5,
            notes: "Best edge device for the price. NVMe HAT eliminates SD card bottleneck. Supports professional tier fully.".into(),
        },
        super::HardwareCompatibilityEntry {
            platform: "Advantech UNO-2484G".into(),
            cpu: "Intel Core i7-8665UE (4-core @ 1.7GHz, turbo 4.4GHz)".into(),
            ram_mb: 8192,
            storage: "M.2 SATA SSD or eMMC".into(),
            supported_tiers: vec!["engineering".into(), "starter".into(), "professional".into()],
            max_tags: 8000,
            max_poll_rate: 5,
            notes: "Industrial-grade, wide temperature range (-20°C to 60°C). Common in SCADA deployments. Excellent reliability.".into(),
        },
        super::HardwareCompatibilityEntry {
            platform: "Advantech UNO-2271G".into(),
            cpu: "Intel Celeron J41 (2-core @ 2.0GHz)".into(),
            ram_mb: 4096,
            storage: "eMMC or mSATA SSD".into(),
            supported_tiers: vec!["engineering".into(), "starter".into()],
            max_tags: 2000,
            max_poll_rate: 1,
            notes: "Entry-level industrial PC. Fanless, compact. Good for small SCADA deployments.".into(),
        },
        super::HardwareCompatibilityEntry {
            platform: "Siemens SIMATIC IPC127E".into(),
            cpu: "Intel Atom x6427FE (4-core @ 1.9GHz)".into(),
            ram_mb: 8192,
            storage: "M.2 SSD".into(),
            supported_tiers: vec!["engineering".into(), "starter".into(), "professional".into()],
            max_tags: 5000,
            max_poll_rate: 5,
            notes: "Siemens entry-level IPC. Designed for SCADA/HMI. Native S7 protocol support in Voltrus makes this a natural fit.".into(),
        },
        super::HardwareCompatibilityEntry {
            platform: "Siemens SIMATIC IPC227E".into(),
            cpu: "Intel Core i3-1115G4 (2-core @ 3.0GHz, turbo 4.1GHz)".into(),
            ram_mb: 16384,
            storage: "NVMe SSD".into(),
            supported_tiers: vec!["engineering".into(), "starter".into(), "professional".into(), "enterprise".into()],
            max_tags: 10000,
            max_poll_rate: 10,
            notes: "Enterprise-grade IPC. Full enterprise tier support. Ideal for multi-protocol deployments with S7 + OPC-UA.".into(),
        },
        super::HardwareCompatibilityEntry {
            platform: "Generic Fanless PC (Intel NUC-class)".into(),
            cpu: "Intel Core i3/i5 (2-4 cores @ 2.0-3.5GHz)".into(),
            ram_mb: 8192,
            storage: "NVMe SSD".into(),
            supported_tiers: vec!["engineering".into(), "starter".into(), "professional".into()],
            max_tags: 8000,
            max_poll_rate: 5,
            notes: "Common choice for cost-effective SCADA. Ensure industrial temperature rating for OT environments.".into(),
        },
        super::HardwareCompatibilityEntry {
            platform: "Standard Server / Desktop".into(),
            cpu: "Modern x86_64 (4+ cores @ 2.5GHz+)".into(),
            ram_mb: 16384,
            storage: "SSD or NVMe".into(),
            supported_tiers: vec!["engineering".into(), "starter".into(), "professional".into(), "enterprise".into()],
            max_tags: 10000,
            max_poll_rate: 10,
            notes: "Full feature support. Recommended for enterprise deployments with all protocols enabled.".into(),
        },
    ]
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assess_compatibility_raspberry_pi_4() {
        let platform = PlatformInfo {
            hostname: "scada-pi".into(),
            os: "Linux".into(),
            arch: "aarch64".into(),
            cpu_model: "Cortex-A72".into(),
            cpu_cores: 4,
            ram_total_mb: 4096,
            disk_total_gb: 32.0,
            disk_available_gb: 28.0,
            board_type: "Raspberry Pi 4 Model B Rev 1.5".into(),
            storage_type: "sd_card".into(),
            is_raspberry_pi: true,
            is_industrial_pc: false,
            kernel_version: "6.1.0".into(),
        };

        let compat = assess_compatibility(&platform);
        assert!(compat.engineering.compatible);
        assert!(compat.starter.compatible);
        assert!(compat.professional.compatible); // 4GB RAM is enough for professional
        assert!(!compat.enterprise.compatible); // Need 2GB RAM minimum but 4 cores only
        assert!(compat.starter.compression_recommended);
    }

    #[test]
    fn test_assess_compatibility_server() {
        let platform = PlatformInfo {
            hostname: "scada-server".into(),
            os: "Linux".into(),
            arch: "x86_64".into(),
            cpu_model: "Intel Core i7".into(),
            cpu_cores: 8,
            ram_total_mb: 16384,
            disk_total_gb: 500.0,
            disk_available_gb: 450.0,
            board_type: "Generic PC/Server".into(),
            storage_type: "nvme".into(),
            is_raspberry_pi: false,
            is_industrial_pc: false,
            kernel_version: "6.1.0".into(),
        };

        let compat = assess_compatibility(&platform);
        assert!(compat.engineering.compatible);
        assert!(compat.starter.compatible);
        assert!(compat.professional.compatible);
        assert!(compat.enterprise.compatible);
        assert!(compat.starter.max_tags > 0);
    }

    #[test]
    fn test_assess_compatibility_low_end() {
        let platform = PlatformInfo {
            hostname: "tiny-device".into(),
            os: "Linux".into(),
            arch: "armv7l".into(),
            cpu_model: "ARM Cortex-A53".into(),
            cpu_cores: 2,
            ram_total_mb: 512,
            disk_total_gb: 8.0,
            disk_available_gb: 5.0,
            board_type: "ARM SBC".into(),
            storage_type: "sd_card".into(),
            is_raspberry_pi: false,
            is_industrial_pc: false,
            kernel_version: "5.15.0".into(),
        };

        let compat = assess_compatibility(&platform);
        assert!(compat.engineering.compatible);
        assert!(compat.starter.compatible);
        assert!(!compat.professional.compatible); // Only 512MB RAM
        assert!(!compat.enterprise.compatible);
    }

    #[test]
    fn test_storage_assessment_sd_card() {
        let platform = PlatformInfo {
            storage_type: "sd_card".into(),
            ..PlatformInfo::default()
        };

        let assessment = assess_storage(&platform);
        assert!(assessment.write_optimization_critical);
        assert!(assessment.estimated_endurance_years < 10.0);
        assert!(!assessment.storage_recommendations.is_empty());
    }

    #[test]
    fn test_storage_assessment_nvme() {
        let platform = PlatformInfo {
            storage_type: "nvme".into(),
            ..PlatformInfo::default()
        };

        let assessment = assess_storage(&platform);
        assert!(!assessment.write_optimization_critical);
        assert!(assessment.estimated_endurance_years > 15.0);
    }

    #[test]
    fn test_compatibility_matrix() {
        let matrix = get_compatibility_matrix();
        assert!(!matrix.is_empty());
        assert!(matrix.iter().any(|e| e.platform.contains("Raspberry Pi")));
        assert!(matrix.iter().any(|e| e.platform.contains("Advantech")));
        assert!(matrix.iter().any(|e| e.platform.contains("Siemens")));
    }

    #[test]
    fn test_max_tags_calculation() {
        let tags = calculate_max_tags(4096, 4, 28.0);
        assert!(tags > 0);
        assert!(tags <= 50000); // Reasonable upper bound
    }
}
