use super::*;

/// Validate config files without importing anything.
/// Returns a list of messages describing what would happen.
pub fn validate_config(input_dir: &str) -> Result<Vec<String>> {
    let mut messages = Vec::new();
    let base = Path::new(input_dir);

    // Validate devices
    let devices_dir = base.join("devices");
    if devices_dir.exists() {
        for entry in std::fs::read_dir(&devices_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            match serde_yaml_neo::from_str::<DeviceExport>(&content) {
                Ok(device) => {
                    let mut issues = Vec::new();
                    if device.id.is_empty() {
                        issues.push("empty id");
                    }
                    if device.ip.is_empty() {
                        issues.push("empty ip");
                    }
                    if device.port == 0 {
                        issues.push("port is 0");
                    }
                    if issues.is_empty() {
                        messages.push(format!(
                            "OK: devices/{:?} — {} ({} registers)",
                            path.file_name().unwrap_or_default(),
                            device.id,
                            device.registers.len()
                        ));
                    } else {
                        messages.push(format!(
                            "ERROR: devices/{:?} — {}",
                            path.file_name().unwrap_or_default(),
                            issues.join(", ")
                        ));
                    }
                }
                Err(e) => {
                    messages.push(format!(
                        "ERROR: devices/{:?} — parse error: {}",
                        path.file_name().unwrap_or_default(),
                        e
                    ));
                }
            }
        }
    } else {
        messages.push("SKIP: devices/ directory not found".to_string());
    }

    // Validate screens
    let screens_dir = base.join("screens");
    if screens_dir.exists() {
        for entry in std::fs::read_dir(&screens_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            match serde_yaml_neo::from_str::<ScreenExport>(&content) {
                Ok(screen) => {
                    messages.push(format!(
                        "OK: screens/{:?} — {} ({} data sources, {} layouts)",
                        path.file_name().unwrap_or_default(),
                        screen.id,
                        screen.data_sources.len(),
                        screen.layouts.len()
                    ));
                }
                Err(e) => {
                    messages.push(format!(
                        "ERROR: screens/{:?} — parse error: {}",
                        path.file_name().unwrap_or_default(),
                        e
                    ));
                }
            }
        }
    } else {
        messages.push("SKIP: screens/ directory not found".to_string());
    }

    // Validate flows
    let flows_dir = base.join("flows");
    if flows_dir.exists() {
        for entry in std::fs::read_dir(&flows_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            match serde_yaml_neo::from_str::<FlowExport>(&content) {
                Ok(flow) => {
                    messages.push(format!(
                        "OK: flows/{:?} — {} (enabled={})",
                        path.file_name().unwrap_or_default(),
                        flow.id,
                        flow.enabled
                    ));
                }
                Err(e) => {
                    messages.push(format!(
                        "ERROR: flows/{:?} — parse error: {}",
                        path.file_name().unwrap_or_default(),
                        e
                    ));
                }
            }
        }
    } else {
        messages.push("SKIP: flows/ directory not found".to_string());
    }

    // Validate alarms
    let alarms_path = base.join("alarms.yaml");
    if alarms_path.exists() {
        let content = std::fs::read_to_string(&alarms_path)?;
        match serde_yaml_neo::from_str::<Vec<AlarmExport>>(&content) {
            Ok(alarms) => {
                messages.push(format!(
                    "OK: alarms.yaml — {} alarm thresholds",
                    alarms.len()
                ));
            }
            Err(e) => {
                messages.push(format!("ERROR: alarms.yaml — parse error: {}", e));
            }
        }
    } else {
        messages.push("SKIP: alarms.yaml not found".to_string());
    }

    // Validate notifications
    let notif_path = base.join("notifications.yaml");
    if notif_path.exists() {
        let content = std::fs::read_to_string(&notif_path)?;
        match serde_yaml_neo::from_str::<Vec<NotificationExport>>(&content) {
            Ok(notifs) => {
                messages.push(format!(
                    "OK: notifications.yaml — {} notification settings",
                    notifs.len()
                ));
            }
            Err(e) => {
                messages.push(format!("ERROR: notifications.yaml — parse error: {}", e));
            }
        }
    } else {
        messages.push("SKIP: notifications.yaml not found".to_string());
    }

    Ok(messages)
}
