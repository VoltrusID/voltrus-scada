use crate::storage::Storage;
use serde::{Deserialize, Serialize};

/// A health check issue found during project audit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthIssue {
    pub severity: HealthSeverity,
    pub category: String,
    pub title: String,
    pub description: String,
    pub suggestion: String,
    pub affected_items: Vec<String>,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum HealthSeverity {
    Critical,
    Warning,
    Info,
    Pass,
}

/// Result of a full project health check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub overall_score: u32, // 0-100
    pub issues: Vec<HealthIssue>,
    pub summary: HealthSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthSummary {
    pub total_devices: u32,
    pub total_tags: u32,
    pub total_alarms: u32,
    pub total_flows: u32,
    pub total_screens: u32,
    pub critical_issues: u32,
    pub warning_issues: u32,
    pub info_issues: u32,
    pub tags_without_alarms: u32,
    pub tags_without_units: u32,
    pub unnamed_devices: u32,
    pub devices_without_tags: u32,
    pub duplicate_tag_names: u32,
    pub nonstandard_tag_names: u32,
    pub missing_dashboards: u32,
    pub disabled_devices: u32,
}

/// Run a full project health check
pub async fn run_health_check(storage: &Storage) -> Result<HealthReport, anyhow::Error> {
    let mut issues: Vec<HealthIssue> = Vec::new();

    // ── Gather data ──
    let devices = storage.list_devices().await?;
    let tags = storage.list_data_sources(None).await?;
    let alarms = storage.list_alarms(None, 99999).await?;
    let flows = storage.list_flows().await?;
    let tag_keys: std::collections::HashSet<String> = tags.iter().map(|t| t.key.clone()).collect();
    let device_ids: std::collections::HashSet<String> =
        devices.iter().map(|d| d.id.clone()).collect();

    // ── Check: Tags without alarms ──
    {
        let alarm_tag_keys: std::collections::HashSet<String> =
            alarms.iter().map(|a| a.source_key.clone()).collect();

        let unalarmed: Vec<String> = tag_keys
            .iter()
            .filter(|k| !alarm_tag_keys.contains(*k))
            .take(50)
            .cloned()
            .collect();

        let count = unalarmed.len().min(tag_keys.len() - alarm_tag_keys.len());
        if !unalarmed.is_empty() {
            issues.push(HealthIssue {
                severity: HealthSeverity::Warning,
                category: "Alarms".into(),
                title: "Tags without alarm configuration".into(),
                description: format!("{count} tags have no alarm thresholds configured. Operators can't be notified of abnormal values."),
                suggestion: "Configure alarm thresholds for critical and major tags. At minimum: level sensors, pressure transmitters, pump faults, and safety-critical measurements.".into(),
                affected_items: unalarmed,
                count: count as u32,
            });
        }
    }

    // ── Check: Tags without units ──
    {
        let unitless: Vec<String> = tags
            .iter()
            .filter(|t| t.unit.is_empty() || t.unit == "-")
            .take(30)
            .map(|t| t.key.clone())
            .collect();

        if !unitless.is_empty() {
            let n = unitless.len() as u32;
            issues.push(HealthIssue {
                severity: HealthSeverity::Info,
                category: "Data Quality".into(),
                title: "Tags missing units of measurement".into(),
                description: format!("{} tags have no unit configured. Units help operators and reports interpret values correctly.", n),
                suggestion: "Add units to all analog tags: °C, bar, m³/h, A, V, %, etc. Digital/boolean tags can use empty or 'bool'.".into(),
                affected_items: unitless,
                count: n,
            });
        }
    }

    // ── Check: Unnamed/disconnected devices ──
    {
        let unnamed: Vec<String> = devices
            .iter()
            .filter(|d| {
                d.name.is_empty() || d.name == "Unnamed Device" || d.name.contains("modbus_")
            })
            .map(|d| d.id.clone())
            .collect();

        if !unnamed.is_empty() {
            let n = unnamed.len() as u32;
            issues.push(HealthIssue {
                severity: HealthSeverity::Warning,
                category: "Device Configuration".into(),
                title: "Devices with generic or missing names".into(),
                description: format!("{} devices have generic names or auto-generated IDs. This makes them hard to identify during maintenance.", n),
                suggestion: "Rename devices to descriptive names: 'Raw Water Pump Station', 'Boiler Control PLC', 'HVAC Chiller Plant'.".into(),
                affected_items: unnamed,
                count: n,
            });
        }
    }

    // ── Check: Devices without tags ──
    {
        let device_tag_counts: std::collections::HashMap<String, usize> = {
            let mut m = std::collections::HashMap::new();
            for t in &tags {
                *m.entry(t.device_id.clone()).or_default() += 1;
            }
            m
        };

        let empty_devices: Vec<String> = device_ids
            .iter()
            .filter(|id| device_tag_counts.get(*id).copied().unwrap_or(0) == 0)
            .cloned()
            .collect();

        if !empty_devices.is_empty() {
            let n = empty_devices.len() as u32;
            issues.push(HealthIssue {
                severity: HealthSeverity::Warning,
                category: "Device Configuration".into(),
                title: "Devices with no configured tags".into(),
                description: format!("{} devices have zero data sources. They exist in the configuration but collect no data.", n),
                suggestion: "Either add tags to these devices (use the device manager or auto-discovery), or remove them if they're no longer needed.".into(),
                affected_items: empty_devices,
                count: n,
            });
        }
    }

    // ── Check: Duplicate tag names ──
    {
        let mut name_counts: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for t in &tags {
            let name = t.name.to_lowercase();
            name_counts.entry(name).or_default().push(t.key.clone());
        }

        let duplicates: Vec<String> = name_counts
            .values()
            .filter(|keys| keys.len() > 1)
            .flat_map(|keys| keys.clone())
            .take(30)
            .collect();

        if !duplicates.is_empty() {
            let n = (duplicates.len() / 2).min(50) as u32;
            issues.push(HealthIssue {
                severity: HealthSeverity::Warning,
                category: "Tag Naming".into(),
                title: "Duplicate tag names detected".into(),
                description: "Multiple tags share the same name. This can cause confusion during alarm handling and reporting.".into(),
                suggestion: "Use unique, descriptive names. Follow ISA-5.1 naming convention: equipment_prefix_measurement (e.g., PMP001_RUN, TANK_LEVEL).".into(),
                affected_items: duplicates,
                count: n,
            });
        }
    }

    // ── Check: Non-standard tag names ──
    {
        let nonstandard: Vec<String> = tags
            .iter()
            .filter(|t| {
                let name = &t.name;
                // Check against common patterns
                let has_underscore = name.contains('_');
                let all_uppercase = name
                    .chars()
                    .all(|c| c.is_uppercase() || c.is_numeric() || c == '_' || c == ':');
                let has_spaces = name.contains(' ');
                // Red flags: spaces, mixed case (not all uppercase), no underscores
                has_spaces
                    || (!all_uppercase && name.len() > 3)
                    || (!has_underscore
                        && name.len() > 5
                        && name.contains(|c: char| c.is_lowercase()))
            })
            .take(20)
            .map(|t| format!("{} ({})", t.key, t.name))
            .collect();

        if !nonstandard.is_empty() {
            let n = nonstandard.len() as u32;
            issues.push(HealthIssue {
                severity: HealthSeverity::Info,
                category: "Tag Naming".into(),
                title: "Non-standard tag names detected".into(),
                description: format!("{} tags use naming conventions that differ from ISA-5.1. Standard naming improves readability and reduces errors.", n),
                suggestion: "Adopt ISA-5.1: EQUIPMENT_MEASUREMENT (e.g., PMP001_RUN, TANK_LEVEL). Use uppercase, underscores, no spaces.".into(),
                affected_items: nonstandard,
                count: n,
            });
        }
    }

    // ── Check: Missing dashboards/screens ──
    {
        let has_any_screen = storage.count_screen_data_sources().await.unwrap_or(0) > 0;

        if !tags.is_empty() && !has_any_screen && !devices.is_empty() {
            issues.push(HealthIssue {
                severity: HealthSeverity::Warning,
                category: "Dashboards".into(),
                title: "No dashboards configured".into(),
                description: "Tags and devices exist but no dashboard screens have been created. Operators need visual dashboards to monitor the process.".into(),
                suggestion: "Use the Project Wizard to auto-generate dashboards, or manually create screens and assign data sources.".into(),
                affected_items: vec![],
                count: 1,
            });
        }
    }

    // ── Check: Disabled devices ──
    {
        let mut disabled_list: Vec<String> = Vec::new();

        // Check Modbus devices with enabled=false
        if let Ok(modbus_devs) = storage.list_modbus_devices().await {
            for d in &modbus_devs {
                if !d.enabled {
                    disabled_list.push(format!("{} ({})", d.name, d.ip));
                }
            }
        }

        // Also check devices with "disabled" in name (legacy heuristic)
        for d in &devices {
            let name_lower = d.name.to_lowercase();
            if (name_lower.contains("disabled") || name_lower.contains("inactive"))
                && !disabled_list.iter().any(|item| item.contains(&d.id))
            {
                disabled_list.push(d.id.clone());
            }
        }

        disabled_list.truncate(20);
        if !disabled_list.is_empty() {
            let n = disabled_list.len() as u32;
            issues.push(HealthIssue {
                severity: HealthSeverity::Info,
                category: "Device Status".into(),
                title: "Disabled devices detected".into(),
                description: format!("{} devices are disabled and won't collect data.", n),
                suggestion: "Review disabled devices. Either fix connectivity or remove them if decommissioned.".into(),
                affected_items: disabled_list,
                count: n,
            });
        }
    }

    // ── Calculate score ──
    let critical_count = issues
        .iter()
        .filter(|i| i.severity == HealthSeverity::Critical)
        .count() as u32;
    let warning_count = issues
        .iter()
        .filter(|i| i.severity == HealthSeverity::Warning)
        .count() as u32;
    let info_count = issues
        .iter()
        .filter(|i| i.severity == HealthSeverity::Info)
        .count() as u32;

    let score = if tags.is_empty() && devices.is_empty() {
        100 // Empty project isn't broken
    } else {
        let base = 100i32;
        let penalty =
            (critical_count as i32 * 20) + (warning_count as i32 * 5) + (info_count as i32);
        (base - penalty).max(0) as u32
    };

    // ── Summary ──
    let summary = HealthSummary {
        total_devices: devices.len() as u32,
        total_tags: tags.len() as u32,
        total_alarms: alarms.len() as u32,
        total_flows: flows.len() as u32,
        total_screens: storage.count_screen_data_sources().await.unwrap_or(0) as u32,
        critical_issues: critical_count,
        warning_issues: warning_count,
        info_issues: info_count,
        tags_without_alarms: issues
            .iter()
            .find(|i| i.title == "Tags without alarm configuration")
            .map(|i| i.count)
            .unwrap_or(0),
        tags_without_units: issues
            .iter()
            .find(|i| i.title == "Tags missing units of measurement")
            .map(|i| i.count)
            .unwrap_or(0),
        unnamed_devices: issues
            .iter()
            .find(|i| i.title == "Devices with generic or missing names")
            .map(|i| i.count)
            .unwrap_or(0),
        devices_without_tags: issues
            .iter()
            .find(|i| i.title == "Devices with no configured tags")
            .map(|i| i.count)
            .unwrap_or(0),
        duplicate_tag_names: issues
            .iter()
            .find(|i| i.title == "Duplicate tag names detected")
            .map(|i| i.count)
            .unwrap_or(0),
        nonstandard_tag_names: issues
            .iter()
            .find(|i| i.title == "Non-standard tag names detected")
            .map(|i| i.count)
            .unwrap_or(0),
        missing_dashboards: issues
            .iter()
            .find(|i| i.title == "No dashboards configured")
            .map(|i| i.count)
            .unwrap_or(0),
        disabled_devices: issues
            .iter()
            .find(|i| i.title == "Disabled devices detected")
            .map(|i| i.count)
            .unwrap_or(0),
    };

    Ok(HealthReport {
        overall_score: score,
        issues,
        summary,
    })
}
