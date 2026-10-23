//! EDR exclusion path recommendations
//!
//! Generates recommended exclusion paths for the current Voltrus installation.
//! These paths should be added to the EDR/antivirus exclusion list to prevent
//! scanning overhead on telemetry databases and real-time data files.

use crate::edr_compat::EdrExclusion;
use std::path::PathBuf;

/// Generate recommended exclusions for the current Voltrus installation.
/// Uses the executable path and current working directory to determine
/// appropriate exclusion paths.
pub fn generate_exclusions(db_path: &str) -> Vec<EdrExclusion> {
    let mut exclusions = Vec::new();

    // 1. Voltrus binary / executable exclusion
    if let Ok(exe_path) = std::env::current_exe() {
        exclusions.push(EdrExclusion {
            path: exe_path.to_string_lossy().to_string(),
            exclusion_type: "process".to_string(),
            reason: "Voltrus main executable — prevents real-time scanning overhead on the SCADA process".to_string(),
            priority: "critical".to_string(),
        });

        // Parent directory of executable
        if let Some(parent) = exe_path.parent() {
            exclusions.push(EdrExclusion {
                path: parent.to_string_lossy().to_string(),
                exclusion_type: "directory".to_string(),
                reason: "Voltrus installation directory — covers executable, libraries, and static assets".to_string(),
                priority: "critical".to_string(),
            });
        }
    }

    // 2. Database file and related files
    exclusions.push(EdrExclusion {
        path: db_path.to_string(),
        exclusion_type: "file".to_string(),
        reason:
            "SQLite telemetry database — frequent writes trigger EDR scanning, causing CPU spikes"
                .to_string(),
        priority: "critical".to_string(),
    });

    // WAL and SHM files
    exclusions.push(EdrExclusion {
        path: format!("{}-wal", db_path),
        exclusion_type: "file".to_string(),
        reason: "SQLite Write-Ahead Log — high-frequency sequential writes, scanning this causes significant overhead".to_string(),
        priority: "critical".to_string(),
    });

    exclusions.push(EdrExclusion {
        path: format!("{}-shm", db_path),
        exclusion_type: "file".to_string(),
        reason: "SQLite Shared Memory file — used for WAL mode concurrent access".to_string(),
        priority: "critical".to_string(),
    });

    // 3. Config directory
    let config_dir = find_config_directory();
    exclusions.push(EdrExclusion {
        path: config_dir.to_string_lossy().to_string(),
        exclusion_type: "directory".to_string(),
        reason: "Configuration directory — YAML configs and exported settings".to_string(),
        priority: "recommended".to_string(),
    });

    // 4. Static assets directory
    exclusions.push(EdrExclusion {
        path: "static".to_string(),
        exclusion_type: "directory".to_string(),
        reason: "Static web assets (HTML, JS, CSS) — read-only after startup, low priority"
            .to_string(),
        priority: "optional".to_string(),
    });

    // 5. Backup directory (if it exists)
    exclusions.push(EdrExclusion {
        path: "backups".to_string(),
        exclusion_type: "directory".to_string(),
        reason: "Backup archives — large tar.gz files can trigger full-file scans".to_string(),
        priority: "recommended".to_string(),
    });

    // 6. Log files (if configured)
    exclusions.push(EdrExclusion {
        path: "voltrus.log".to_string(),
        exclusion_type: "file".to_string(),
        reason: "Application log file — frequent append writes".to_string(),
        priority: "recommended".to_string(),
    });

    // 7. File extension exclusions
    exclusions.push(EdrExclusion {
        path: "*.db".to_string(),
        exclusion_type: "extension".to_string(),
        reason: "SQLite database files — all Voltrus databases use this extension".to_string(),
        priority: "recommended".to_string(),
    });

    exclusions.push(EdrExclusion {
        path: "*.db-wal".to_string(),
        exclusion_type: "extension".to_string(),
        reason: "SQLite WAL files — high-frequency write files".to_string(),
        priority: "recommended".to_string(),
    });

    exclusions.push(EdrExclusion {
        path: "*.db-shm".to_string(),
        exclusion_type: "extension".to_string(),
        reason: "SQLite shared memory files".to_string(),
        priority: "recommended".to_string(),
    });

    exclusions
}

/// Find the configuration directory path.
fn find_config_directory() -> PathBuf {
    // Check common locations
    let candidates = vec![
        PathBuf::from("config"),
        PathBuf::from("/etc/voltrus"),
        PathBuf::from("C:\\Program Files\\Voltrus\\config"),
        PathBuf::from("C:\\ProgramData\\Voltrus\\config"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return candidate.clone();
        }
    }

    // Default to current directory config/
    PathBuf::from("config")
}

/// Generate a PowerShell script to apply Windows Defender exclusions.
/// This can be run by IT teams to configure Defender for Voltrus.
pub fn generate_defender_ps1(exclusions: &[EdrExclusion]) -> String {
    let mut script = String::new();

    script.push_str("# Voltrus — Windows Defender Exclusion Script\n");
    script.push_str("# Generated by voltrus edr exclusions\n");
    script.push_str("# Run as Administrator\n");
    script.push_str("#\n");
    script.push_str("# This script adds recommended exclusions for Microsoft Defender\n");
    script.push_str("# to ensure Voltrus SCADA runs without performance degradation.\n");
    script.push_str("#\n");
    script.push_str("# Source: v0.36.0 — Windows Defender OT Compatibility\n");
    script.push_str("# Reference: https://voltrus.id/docs/ot-deployment-guide\n\n");
    script.push_str("Write-Host \"Adding Voltrus exclusions to Windows Defender...\" -ForegroundColor Green\n\n");

    for exclusion in exclusions {
        if exclusion.exclusion_type == "directory" {
            script.push_str(&format!("# {}\n", exclusion.reason));
            script.push_str(&format!(
                "Add-MpPreference -ExclusionPath \"{}\"\n",
                exclusion.path
            ));
            script.push_str(&format!(
                "Write-Host \"  Added directory exclusion: {}\" -ForegroundColor Cyan\n\n",
                exclusion.path
            ));
        } else if exclusion.exclusion_type == "process" {
            script.push_str(&format!("# {}\n", exclusion.reason));
            script.push_str(&format!(
                "Add-MpPreference -ExclusionProcess \"{}\"\n",
                exclusion.path
            ));
            script.push_str(&format!(
                "Write-Host \"  Added process exclusion: {}\" -ForegroundColor Cyan\n\n",
                exclusion.path
            ));
        } else if exclusion.exclusion_type == "extension" {
            script.push_str(&format!("# {}\n", exclusion.reason));
            script.push_str(&format!(
                "Add-MpPreference -ExclusionExtension \"{}\"\n",
                exclusion.path.trim_start_matches('*')
            ));
            script.push_str(&format!(
                "Write-Host \"  Added extension exclusion: {}\" -ForegroundColor Cyan\n\n",
                exclusion.path
            ));
        }
    }

    script.push_str("Write-Host \"\"\n");
    script.push_str(
        "Write-Host \"Voltrus exclusions applied successfully.\" -ForegroundColor Green\n",
    );
    script.push_str("Write-Host \"Verify with: Get-MpPreference | Select-Object -Property Exclusion*\" -ForegroundColor Yellow\n");

    script
}

/// Generate a Group Policy Object (GPO) configuration snippet.
/// For IT teams deploying via Active Directory Group Policy.
pub fn generate_gpo_guidance() -> String {
    let mut guide = String::new();

    guide.push_str("Voltrus — Windows Defender GPO Deployment Guide\n");
    guide.push_str("================================================\n\n");

    guide.push_str("For IT teams deploying via Active Directory Group Policy:\n\n");

    guide.push_str("1. Open Group Policy Management Console (gpmc.msc)\n");
    guide.push_str("2. Navigate to:\n");
    guide
        .push_str("   Computer Configuration > Administrative Templates > Windows Components > \n");
    guide.push_str("   Microsoft Defender Antivirus > Exclusions\n\n");

    guide.push_str("3. Configure the following policies:\n\n");

    guide.push_str("   Path Exclusions:\n");
    guide.push_str("   - Enable: 'Path Exclusions'\n");
    guide.push_str(
        "   - Add: Voltrus installation directory (e.g., C:\\Program Files\\Voltrus\\)\n",
    );
    guide.push_str("   - Add: Voltrus data directory (e.g., C:\\ProgramData\\Voltrus\\)\n\n");

    guide.push_str("   Process Exclusions:\n");
    guide.push_str("   - Enable: 'Process Exclusions'\n");
    guide.push_str("   - Add: voltrus.exe\n\n");

    guide.push_str("   Extension Exclusions:\n");
    guide.push_str("   - Enable: 'Extension Exclusions'\n");
    guide.push_str("   - Add: db, db-wal, db-shm\n\n");

    guide.push_str("4. Link the GPO to the OU containing SCADA servers\n");
    guide.push_str("5. Run 'gpupdate /force' on target machines\n\n");

    guide.push_str("IMPORTANT: Do NOT exclude the entire C:\\ drive or system directories.\n");
    guide.push_str("Only exclude Voltrus-specific paths to maintain security posture.\n\n");

    guide.push_str("Reference: https://voltrus.id/docs/ot-deployment-guide\n");

    guide
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_exclusions() {
        let exclusions = generate_exclusions("voltrus.db");
        assert!(!exclusions.is_empty());
        // Should have at least the database and WAL files
        assert!(exclusions.iter().any(|e| e.path == "voltrus.db"));
        assert!(exclusions.iter().any(|e| e.path == "voltrus.db-wal"));
        assert!(exclusions.iter().any(|e| e.path == "voltrus.db-shm"));
    }

    #[test]
    fn test_exclusion_priorities() {
        let exclusions = generate_exclusions("test.db");
        for e in &exclusions {
            assert!(
                ["critical", "recommended", "optional"].contains(&e.priority.as_str()),
                "Invalid priority: {}",
                e.priority
            );
        }
    }

    #[test]
    fn test_exclusion_types() {
        let exclusions = generate_exclusions("test.db");
        for e in &exclusions {
            assert!(
                ["directory", "file", "process", "extension"].contains(&e.exclusion_type.as_str()),
                "Invalid exclusion type: {}",
                e.exclusion_type
            );
        }
    }

    #[test]
    fn test_generate_defender_ps1() {
        let exclusions = generate_exclusions("voltrus.db");
        let script = generate_defender_ps1(&exclusions);
        assert!(script.contains("Add-MpPreference"));
        assert!(script.contains("Administrator"));
        // Should contain at least directory and process exclusions
        assert!(script.contains("ExclusionPath") || script.contains("ExclusionProcess"));
    }

    #[test]
    fn test_generate_gpo_guidance() {
        let guide = generate_gpo_guidance();
        assert!(guide.contains("Group Policy"));
        assert!(guide.contains("gpupdate"));
        assert!(guide.contains("Exclusions"));
    }
}
