//! Edge deployment guide generation.
//!
//! Generates step-by-step deployment instructions for edge hardware
//! (Raspberry Pi, Advantech IPC, Siemens IPC, generic fanless PCs).

use super::platform::PlatformInfo;

/// Generate an edge deployment guide tailored to the detected platform.
pub fn generate_edge_guide(platform: &PlatformInfo) -> String {
    let mut guide = String::new();

    guide.push_str(&format!(
        "=== Voltrus Edge Deployment Guide ===\n\
         Generated for: {} ({})\n\
         OS: {} | Arch: {} | RAM: {} MB | Storage: {}\n\
         Board: {}\n\n",
        platform.hostname,
        platform.board_type,
        platform.os,
        platform.arch,
        platform.ram_total_mb,
        platform.storage_type,
        platform.board_type,
    ));

    if platform.is_raspberry_pi {
        guide.push_str(&generate_raspberry_pi_guide(platform));
    } else if platform.is_industrial_pc {
        guide.push_str(&generate_industrial_pc_guide(platform));
    } else {
        guide.push_str(&generate_generic_guide(platform));
    }

    guide.push_str(&generate_common_post_deploy(platform));
    guide.push_str(&generate_sd_card_optimization(platform));

    guide
}

/// Generate Raspberry Pi specific deployment guide.
fn generate_raspberry_pi_guide(platform: &PlatformInfo) -> String {
    let has_nvme = platform.storage_type == "nvme";

    format!(
        "## Raspberry Pi Deployment\n\n\
         ### Step 1: Flash the SD Card\n\
         ```bash\n\
         # Download Raspberry Pi OS Lite (64-bit, Bookworm recommended)\n\
         # Flash using Raspberry Pi Imager or:\n\
         dd if=raspios-lite-arm64.img of=/dev/sdX bs=4M status=progress conv=fsync\n\
         ```\n\n\
         ### Step 2: Enable Headless Mode\n\
         ```bash\n\
         # Mount the boot partition and create these files:\n\
         # Enable SSH:\n\
         touch /boot/firmware/ssh\n\n\
         # Configure WiFi (if needed):\n\
         cat > /boot/firmware/wpa_supplicant.conf << 'EOF'\n\
         country=US\n\
         ctrl_interface=DIR=/var/run/wpa_supplicant GROUP=netdev\n\
         update_config=1\n\
         network={{\n\
             ssid=\"YOUR_WIFI_SSID\"\n\
             psk=\"YOUR_WIFI_PASSWORD\"\n\
         }}\n\
         EOF\n\
         ```\n\n\
         ### Step 3: Install Voltrus\n\
         ```bash\n\
         # SSH into your Pi:\n\
         ssh pi@raspberrypi.local\n\n\
         # Download Voltrus for ARM64:\n\
         curl -L https://voltrus.id/downloads/voltrus-arm64-latest.tar.gz -o voltrus.tar.gz\n\
         tar xzf voltrus.tar.gz\n\
         sudo mv voltrus /usr/local/bin/\n\n\
         # Create config directory:\n\
         sudo mkdir -p /etc/voltrus\n\
         sudo chown $USER:$USER /etc/voltrus\n\
         ```\n\n\
         ### Step 4: Create config.yaml\n\
         ```bash\n\
         cat > /etc/voltrus/config.yaml << 'EOF'\n\
         server_port: 8080\n\
         db_path: /var/lib/voltrus/voltrus.db\n\
         persist_interval_secs: 5\n\
         auth_secret: CHANGE_THIS_TO_A_LONG_RANDOM_STRING\n\n\
         retention:\n\
           pack_after_secs: 3600\n\
           enabled: true\n\n\
         # SD card optimization:\n\
         # Use longer persist interval to reduce writes\n\
         # persist_interval_secs: 10  # Uncomment for older SD cards\n\
         EOF\n\n\
         sudo mkdir -p /var/lib/voltrus\n\
         sudo chown $USER:$USER /var/lib/voltrus\n\
         ```\n\n\
         ### Step 5: Set Up Auto-Start (systemd)\n\
         ```bash\n\
         sudo tee /etc/systemd/system/voltrus.service << 'EOF'\n\
         [Unit]\n\
         Description=Voltrus SCADA Server\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         User=pi\n\
         WorkingDirectory=/etc/voltrus\n\
         ExecStart=/usr/local/bin/voltrus /etc/voltrus/config.yaml\n\
         Restart=always\n\
         RestartSec=5\n\n\
         # Hardening for OT environment:\n\
         NoNewPrivileges=true\n\
         ProtectSystem=strict\n\
         ReadWritePaths=/var/lib/voltrus\n\
         PrivateTmp=true\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n\
         EOF\n\n\
         sudo systemctl daemon-reload\n\
         sudo systemctl enable voltrus\n\
         sudo systemctl start voltrus\n\n\
         # Check status:\n\
         sudo systemctl status voltrus\n\
         journalctl -u voltrus -f\n\
         ```\n\n{}\
         ### Step 6: Verify\n\
         ```bash\n\
         # Check health:\n\
         curl http://localhost:8080/api/v1/health\n\n\
         # Open in browser:\n\
         # http://<PI_IP_ADDRESS>:8080\n\
         ```\n\n",
        if has_nvme {
            "### Note: NVMe HAT Detected\n\
             Your system has NVMe storage. For best results, point db_path to the NVMe mount:\n\
             ```yaml\n\
             db_path: /mnt/nvme/voltrus/voltrus.db\n\
             ```\n\n"
        } else {
            "### SD Card Warning\n\
             Your system uses an SD card for storage. For production deployments:\n\
             - Use an A2-rated MicroSD card (minimum Class 10 / U3)\n\
             - Consider a USB SSD or NVMe HAT for Raspberry Pi 5\n\
             - Enable data compression to reduce SD card writes\n\
             - Schedule regular backups (SD cards can fail without warning)\n\n"
        }
    )
}

/// Generate industrial PC deployment guide.
fn generate_industrial_pc_guide(_platform: &PlatformInfo) -> String {
    "## Industrial PC Deployment\n\n\
         ### Step 1: Prepare the OS\n\
         Install a minimal Linux distribution:\n\
         - Debian 12 (Bookworm) minimal — recommended\n\
         - Ubuntu Server 22.04 LTS\n\
         - For Windows: Windows 10/11 IoT Enterprise LTSC\n\n\
         ### Step 2: Install Voltrus\n\
         ```bash\n\
         # Download for your architecture:\n\
         # For x86_64:\n\
         curl -L https://voltrus.id/downloads/voltrus-x86_64-latest.tar.gz -o voltrus.tar.gz\n\
         # For ARM64:\n\
         curl -L https://voltrus.id/downloads/voltrus-arm64-latest.tar.gz -o voltrus.tar.gz\n\n\
         tar xzf voltrus.tar.gz\n\
         sudo mv voltrus /usr/local/bin/\n\
         ```\n\n\
         ### Step 3: Configure\n\
         ```bash\n\
         sudo mkdir -p /etc/voltrus /var/lib/voltrus\n\
         sudo tee /etc/voltrus/config.yaml << 'EOF'\n\
         server_port: 8080\n\
         db_path: /var/lib/voltrus/voltrus.db\n\
         persist_interval_secs: 5\n\
         auth_secret: CHANGE_THIS_TO_A_LONG_RANDOM_STRING\n\n\
         retention:\n\
           pack_after_secs: 3600\n\
           enabled: true\n\
         EOF\n\
         ```\n\n\
         ### Step 4: Systemd Service\n\
         ```bash\n\
         sudo tee /etc/systemd/system/voltrus.service << 'EOF'\n\
         [Unit]\n\
         Description=Voltrus SCADA Server\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         User=voltrus\n\
         WorkingDirectory=/etc/voltrus\n\
         ExecStart=/usr/local/bin/voltrus /etc/voltrus/config.yaml\n\
         Restart=always\n\
         RestartSec=5\n\
         NoNewPrivileges=true\n\
         ProtectSystem=strict\n\
         ReadWritePaths=/var/lib/voltrus\n\
         PrivateTmp=true\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n\
         EOF\n\n\
         sudo useradd -r -s /bin/false voltrus\n\
         sudo chown voltrus:voltrus /var/lib/voltrus\n\
         sudo systemctl daemon-reload\n\
         sudo systemctl enable --now voltrus\n\
         ```\n\n\
         ### Step 5: EDR/Antivirus Exclusions\n\
         If this system runs endpoint security (common in OT):\n\
         ```bash\n\
         voltrus edr report\n\
         voltrus edr exclusions\n\
         ```\n\n\
         ### Step 6: Verify\n\
         ```bash\n\
         curl http://localhost:8080/api/v1/health\n\
         # Open: http://<IPC_IP_ADDRESS>:8080\n\
         ```\n\n"
        .to_string()
}

/// Generate generic deployment guide.
fn generate_generic_guide(_platform: &PlatformInfo) -> String {
    "## Generic Deployment\n\n\
     ### Step 1: Download and Install\n\
     ```bash\n\
     # Download the latest release for your platform:\n\
     curl -L https://voltrus.id/downloads/voltrus-$(uname -m)-latest.tar.gz -o voltrus.tar.gz\n\
     tar xzf voltrus.tar.gz\n\
     sudo mv voltrus /usr/local/bin/\n\
     ```\n\n\
     ### Step 2: Create Configuration\n\
     ```bash\n\
     mkdir -p /etc/voltrus /var/lib/voltrus\n\
     cat > /etc/voltrus/config.yaml << 'EOF'\n\
     server_port: 8080\n\
     db_path: /var/lib/voltrus/voltrus.db\n\
     persist_interval_secs: 5\n\
     auth_secret: CHANGE_THIS_TO_A_LONG_RANDOM_STRING\n\n\
     retention:\n\
       pack_after_secs: 3600\n\
       enabled: true\n\
     EOF\n\
     ```\n\n\
     ### Step 3: Auto-Start (Linux systemd)\n\
     ```bash\n\
     sudo tee /etc/systemd/system/voltrus.service << 'EOF'\n\
     [Unit]\n\
     Description=Voltrus SCADA Server\n\
     After=network.target\n\n\
     [Service]\n\
     Type=simple\n\
     User=$USER\n\
     WorkingDirectory=/etc/voltrus\n\
     ExecStart=/usr/local/bin/voltrus /etc/voltrus/config.yaml\n\
     Restart=always\n\
     RestartSec=5\n\n\
     [Install]\n\
     WantedBy=multi-user.target\n\
     EOF\n\n\
     sudo systemctl daemon-reload\n\
     sudo systemctl enable voltrus\n\
     sudo systemctl start voltrus\n\
     ```\n\n\
     ### Step 4: Verify\n\
     ```bash\n\
     curl http://localhost:8080/api/v1/health\n\
     ```\n\n"
        .into()
}

/// Common post-deployment steps.
fn generate_common_post_deploy(_platform: &PlatformInfo) -> String {
    "## Post-Deployment Checklist\n\n\
     - [ ] Activate your license: `voltrus --online-activate YOUR-KEY`\n\
     - [ ] Change the default admin password on first login\n\
     - [ ] Configure your first Modbus device in the web UI\n\
     - [ ] Set up alarm notifications (Settings → Notifications)\n\
     - [ ] Enable data compression for long-term storage\n\
     - [ ] Configure a backup schedule: `voltrus backup --output /mnt/backup/voltrus-$(date +%Y%m%d).tar.gz`\n\
     - [ ] Set up EDR exclusions if running on Windows: `voltrus edr exclusions-ps1`\n\
     - [ ] Run hardware benchmark: `voltrus hw benchmark`\n\
     - [ ] Review hardware compatibility: `voltrus hw report`\n\n\
     ## Performance Tuning\n\n\
     ### Persist Interval\n\
     - Default: 5 seconds (good balance of freshness vs. write overhead)\n\
     - High-frequency SCADA: 1-2 seconds\n\
     - SD card (longevity): 10-30 seconds\n\n\
     ### Compression\n\
     - Enable compression to reduce storage by 50-100x\n\
     - Critical for SD card and eMMC storage longevity\n\
     - Compression runs automatically as a background task\n\n".into()
}

/// SD card write optimization tips.
fn generate_sd_card_optimization(platform: &PlatformInfo) -> String {
    if platform.storage_type != "sd_card" {
        return String::new();
    }

    "## SD Card Write Optimization\n\n\
     Your system uses SD card storage. Voltrus includes several optimizations:\n\n\
     1. **Batch Writes**: Telemetry is collected and written in batches (500 rows\n\
        per transaction) instead of individual writes. This reduces write\n\
        amplification by 10-50x.\n\n\
     2. **WAL Mode**: SQLite uses Write-Ahead Logging, which appends sequentially\n\
        instead of writing to random positions. Much better for SD cards.\n\n\
     3. **Deadband Filtering**: A tag with `history_deadband` in its source config\n\
        only stores values that move beyond that band (plus a heartbeat sample),\n\
        which removes most writes for steady signals. See docs/historian.md.\n\n\
     4. **Compression**: Complete hours older than an hour are packed into\n\
        compressed blocks (about 2.5 bytes per sample instead of ~64 for a row),\n\
        which also removes most of the rows the SD card would otherwise keep\n\
        rewriting.\n\n\
     5. **Reduced fsync**: WAL mode with synchronous=NORMAL minimizes forced\n\
        disk flushes while maintaining data integrity.\n\n\
     ### Additional SD Card Tips\n\
     - Use A2-rated SD cards (Samsung EVO Select, SanDisk Extreme Pro)\n\
     - Avoid cheap/no-name SD cards — they fail fast under write-heavy workloads\n\
     - Consider overlayfs (read-only root) to prevent filesystem corruption\n\
     - Schedule regular backups with `voltrus backup`\n\
     - Monitor SD card health with `voltrus hw report`\n\n"
        .into()
}

/// Generate a quick-start guide for headless Raspberry Pi deployment.
pub fn generate_rpi_quickstart() -> String {
    "=== Voltrus Raspberry Pi Quick Start ===\n\n\
     1. Flash Raspberry Pi OS Lite (64-bit) to SD card\n\
     2. Enable SSH: touch /boot/firmware/ssh\n\
     3. Boot the Pi and SSH in: ssh pi@raspberrypi.local\n\
     4. Download Voltrus:\n\
        curl -L https://voltrus.id/downloads/voltrus-arm64-latest.tar.gz | tar xz\n\
        sudo mv voltrus /usr/local/bin/\n\
     5. Create config:\n\
        mkdir -p /etc/voltrus /var/lib/voltrus\n\
        echo 'server_port: 8080' > /etc/voltrus/config.yaml\n\
        echo 'db_path: /var/lib/voltrus/voltrus.db' >> /etc/voltrus/config.yaml\n\
        echo 'auth_secret: CHANGE_ME_$(openssl rand -hex 16)' >> /etc/voltrus/config.yaml\n\
     6. Start: voltrus /etc/voltrus/config.yaml\n\
     7. Open: http://<PI_IP>:8080\n\n\
     For auto-start, watchdog, and production hardening:\n\
     Run: voltrus hw edge-deploy\n"
        .into()
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_edge_guide_raspberry_pi() {
        let platform = PlatformInfo {
            hostname: "scada-pi".into(),
            os: "Linux".into(),
            arch: "aarch64".into(),
            cpu_model: "Cortex-A72".into(),
            cpu_cores: 4,
            ram_total_mb: 4096,
            disk_total_gb: 32.0,
            disk_available_gb: 28.0,
            board_type: "Raspberry Pi 4 Model B".into(),
            storage_type: "sd_card".into(),
            is_raspberry_pi: true,
            is_industrial_pc: false,
            kernel_version: "6.1.0".into(),
        };

        let guide = generate_edge_guide(&platform);
        assert!(guide.contains("Raspberry Pi"));
        assert!(guide.contains("Flash"));
        assert!(guide.contains("systemd"));
        assert!(guide.contains("SD Card"));
        assert!(guide.contains("Post-Deployment"));
    }

    #[test]
    fn test_generate_edge_guide_industrial_pc() {
        let platform = PlatformInfo {
            hostname: "advantech-ipc".into(),
            os: "Linux".into(),
            arch: "x86_64".into(),
            cpu_model: "Intel Core i7".into(),
            cpu_cores: 4,
            ram_total_mb: 8192,
            disk_total_gb: 128.0,
            disk_available_gb: 100.0,
            board_type: "Advantech Industrial PC".into(),
            storage_type: "ssd".into(),
            is_raspberry_pi: false,
            is_industrial_pc: true,
            kernel_version: "6.1.0".into(),
        };

        let guide = generate_edge_guide(&platform);
        assert!(guide.contains("Industrial PC"));
        assert!(guide.contains("EDR"));
        assert!(guide.contains("systemd"));
    }

    #[test]
    fn test_generate_edge_guide_generic() {
        let platform = PlatformInfo {
            hostname: "server".into(),
            os: "Linux".into(),
            arch: "x86_64".into(),
            cpu_model: "Intel Xeon".into(),
            cpu_cores: 8,
            ram_total_mb: 16384,
            disk_total_gb: 500.0,
            disk_available_gb: 400.0,
            board_type: "Generic PC/Server".into(),
            storage_type: "nvme".into(),
            is_raspberry_pi: false,
            is_industrial_pc: false,
            kernel_version: "6.1.0".into(),
        };

        let guide = generate_edge_guide(&platform);
        assert!(guide.contains("Generic Deployment"));
        assert!(!guide.contains("SD Card Write Optimization"));
    }

    #[test]
    fn test_rpi_quickstart() {
        let quickstart = generate_rpi_quickstart();
        assert!(quickstart.contains("Raspberry Pi"));
        assert!(quickstart.contains("Flash"));
        assert!(quickstart.contains("8080"));
    }
}
