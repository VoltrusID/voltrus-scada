//! Platform detection — identify CPU, RAM, storage type, and board model.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Detected platform information.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PlatformInfo {
    /// Hostname
    pub hostname: String,
    /// Operating system name
    pub os: String,
    /// CPU architecture (x86_64, aarch64, armv7l)
    pub arch: String,
    /// CPU model name
    pub cpu_model: String,
    /// Number of CPU cores
    pub cpu_cores: u32,
    /// Total RAM in MB
    pub ram_total_mb: u64,
    /// Total disk space in GB (database volume)
    pub disk_total_gb: f64,
    /// Available disk space in GB
    pub disk_available_gb: f64,
    /// Detected board/system type
    pub board_type: String,
    /// Storage type (sd_card, ssd, hdd, emmc, nvme, unknown)
    pub storage_type: String,
    /// Whether the system appears to be a Raspberry Pi
    pub is_raspberry_pi: bool,
    /// Whether the system appears to be an industrial PC
    pub is_industrial_pc: bool,
    /// Kernel version (Linux) or Windows version
    pub kernel_version: String,
}

impl Default for PlatformInfo {
    fn default() -> Self {
        Self {
            hostname: "unknown".into(),
            os: "unknown".into(),
            arch: std::env::consts::ARCH.into(),
            cpu_model: "unknown".into(),
            cpu_cores: 1,
            ram_total_mb: 0,
            disk_total_gb: 0.0,
            disk_available_gb: 0.0,
            board_type: "unknown".into(),
            storage_type: "unknown".into(),
            is_raspberry_pi: false,
            is_industrial_pc: false,
            kernel_version: "unknown".into(),
        }
    }
}

/// Detect the current platform's hardware information.
pub fn detect_platform(db_path: &str) -> PlatformInfo {
    use sysinfo::{Disks, System};

    let mut sys = System::new_all();
    sys.refresh_all();

    let hostname = System::host_name().unwrap_or_else(|| "unknown".into());
    let os = System::name().unwrap_or_else(|| "unknown".into());
    let arch = std::env::consts::ARCH.to_string();
    let kernel_version = System::kernel_version().unwrap_or_else(|| "unknown".into());

    // CPU info
    let cpus = sys.cpus();
    let cpu_model = cpus
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_else(|| "unknown".into());
    let cpu_cores = cpus.len() as u32;

    // RAM
    let ram_total_mb = sys.total_memory() / (1024 * 1024);

    // Disk
    let disks = Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .find(|d| {
            let mount = d.mount_point().to_str().unwrap_or("");
            // Try to find the disk that contains the DB path
            mount == "/" || mount == "C:\\" || db_path.starts_with(mount)
        })
        .or_else(|| disks.list().first());

    let (disk_total_gb, disk_available_gb) = if let Some(d) = disk {
        let total = d.total_space() as f64 / (1024.0 * 1024.0 * 1024.0);
        let available = d.available_space() as f64 / (1024.0 * 1024.0 * 1024.0);
        (total, available)
    } else {
        (0.0, 0.0)
    };

    // Detect board type
    let (board_type, is_raspberry_pi, is_industrial_pc) = detect_board_type(&cpu_model, &arch);

    // Detect storage type
    let storage_type = detect_storage_type(is_raspberry_pi, db_path);

    PlatformInfo {
        hostname,
        os,
        arch,
        cpu_model,
        cpu_cores,
        ram_total_mb,
        disk_total_gb,
        disk_available_gb,
        board_type,
        storage_type,
        is_raspberry_pi,
        is_industrial_pc,
        kernel_version,
    }
}

/// Detect the board/system type from CPU model and architecture.
fn detect_board_type(cpu_model: &str, arch: &str) -> (String, bool, bool) {
    // Check for Raspberry Pi
    if let Some(model) = read_raspberry_pi_model() {
        return (model, true, false);
    }

    // Check for known industrial PCs
    let cpu_lower = cpu_model.to_lowercase();

    // Advantech systems often have specific DMI data
    if is_advantech_system() {
        return ("Advantech Industrial PC".into(), false, true);
    }

    // Siemens SIMATIC IPC
    if is_siemens_ipc() {
        return ("Siemens SIMATIC IPC".into(), false, true);
    }

    // Generic ARM SBC detection
    if (arch == "aarch64" || arch == "armv7l") && cpu_lower.contains("cortex") {
        return ("ARM Single Board Computer".into(), false, false);
    }

    // Generic x86 industrial detection (fanless, embedded)
    if is_fanless_pc() {
        return ("Fanless Industrial PC".into(), false, true);
    }

    // Default
    ("Generic PC/Server".into(), false, false)
}

/// Read Raspberry Pi model from /proc/device-tree/model or /proc/cpuinfo.
fn read_raspberry_pi_model() -> Option<String> {
    // Try device tree model first (most reliable on Pi)
    if let Ok(model) = std::fs::read_to_string("/proc/device-tree/model") {
        let model = model.trim_end_matches('\0').trim();
        if model.contains("Raspberry Pi") {
            return Some(model.to_string());
        }
    }

    // Fallback: check cpuinfo for Pi-specific hardware
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if line.starts_with("Model")
                && line.contains("Raspberry Pi")
                && let Some(model) = line.split(':').nth(1)
            {
                return Some(model.trim().to_string());
            }
            if line.starts_with("Hardware") && line.contains("BCM") {
                // Broadcom chip = likely Raspberry Pi
                return Some("Raspberry Pi (Broadcom)".to_string());
            }
        }
    }

    None
}

/// Check if the system is an Advantech industrial PC.
fn is_advantech_system() -> bool {
    // Check DMI product name for Advantech
    if let Ok(product) = std::fs::read_to_string("/sys/class/dmi/id/product_name")
        && (product.to_lowercase().contains("advantech")
            || product.to_lowercase().contains("uno")
            || product.to_lowercase().contains("ipc"))
    {
        return true;
    }

    // Check DMI vendor
    if let Ok(vendor) = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")
        && vendor.to_lowercase().contains("advantech")
    {
        return true;
    }

    false
}

/// Check if the system is a Siemens SIMATIC IPC.
fn is_siemens_ipc() -> bool {
    if let Ok(product) = std::fs::read_to_string("/sys/class/dmi/id/product_name")
        && (product.to_lowercase().contains("simatic")
            || product.to_lowercase().contains("siemens"))
    {
        return true;
    }

    if let Ok(vendor) = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")
        && vendor.to_lowercase().contains("siemens")
    {
        return true;
    }

    false
}

/// Heuristic: detect fanless industrial PCs by checking for common indicators.
fn is_fanless_pc() -> bool {
    // Fanless PCs often have specific chassis types in DMI
    if let Ok(chassis) = std::fs::read_to_string("/sys/class/dmi/id/chassis_type") {
        // Type 3 = Desktop, Type 13 = All-in-One, Type 35 = Mini PC
        // These are common for fanless industrial PCs
        if let Ok(t) = chassis.trim().parse::<u32>()
            && (t == 35 || t == 13)
        {
            return true;
        }
    }

    // Check for common industrial PC vendors in DMI
    if let Ok(vendor) = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor") {
        let v = vendor.to_lowercase();
        if v.contains("intel") && v.contains("nuc")
            || v.contains("onlogic")
            || v.contains("logic supply")
            || v.contains("necter")
            || v.contains("trustedge")
            || v.contains("beckhoff")
            || v.contains("kontron")
            || v.contains("portwell")
        {
            return true;
        }
    }

    false
}

/// Detect storage type based on platform and mount options.
fn detect_storage_type(is_raspberry_pi: bool, db_path: &str) -> String {
    // Raspberry Pi: almost always SD card unless NVMe HAT is present
    if is_raspberry_pi {
        // Check if root filesystem is on SD card (mmcblk)
        if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
            for line in mounts.lines() {
                if (line.starts_with("/dev/mmcblk") || line.starts_with("/dev/sd"))
                    && let Some(_fs_type) = line.split_whitespace().nth(2)
                    && db_path.starts_with("/")
                    && line.split_whitespace().nth(1) == Some("/")
                    && line.contains("mmcblk")
                {
                    return "sd_card".into();
                }
                // NVMe HAT on Pi
                if line.starts_with("/dev/nvme") {
                    return "nvme".into();
                }
            }
        }
        return "sd_card".into(); // Default for Pi
    }

    // Check for NVMe
    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            if line.starts_with("/dev/nvme") {
                return "nvme".into();
            }
        }
        // eMMC (embedded flash, common on industrial PCs)
        for line in mounts.lines() {
            if line.contains("mmcblk") {
                return "emmc".into();
            }
        }
    }

    // Check rotational flag for SSD vs HDD
    if let Ok(block_devs) = std::fs::read_dir("/sys/block/") {
        for entry in block_devs.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if (name.starts_with("sd") || name.starts_with("vd") || name.starts_with("sata"))
                && let Ok(rotational) =
                    std::fs::read_to_string(format!("/sys/block/{}/queue/rotational", name))
            {
                if rotational.trim() == "0" {
                    return "ssd".into();
                } else if rotational.trim() == "1" {
                    return "hdd".into();
                }
            }
        }
    }

    "unknown".into()
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_info_default() {
        let info = PlatformInfo::default();
        assert_eq!(info.hostname, "unknown");
        assert_eq!(info.cpu_cores, 1);
        assert!(!info.is_raspberry_pi);
        assert_eq!(info.storage_type, "unknown");
    }

    #[test]
    fn test_detect_platform() {
        // This test runs on the actual host, so we just verify it doesn't crash
        let info = detect_platform("test.db");
        assert!(!info.hostname.is_empty() || info.hostname == "unknown");
        assert!(!info.arch.is_empty());
        assert!(info.cpu_cores >= 1);
    }

    #[test]
    fn test_storage_type_inference() {
        // On non-Pi systems, we can't predict exact storage,
        // but the function should return a valid type
        let storage = detect_storage_type(false, "test.db");
        assert!(matches!(
            storage.as_str(),
            "sd_card" | "ssd" | "hdd" | "emmc" | "nvme" | "unknown"
        ));
    }
}
