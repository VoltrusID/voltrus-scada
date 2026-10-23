use super::*;

/// Import configuration from YAML files into the database.
/// This is transactional — all or nothing.
pub fn import_config(storage: &Storage, input_dir: &str) -> Result<()> {
    let rt = tokio::runtime::Runtime::new().context("Failed to create tokio runtime")?;

    rt.block_on(async {
        let base = Path::new(input_dir);

        // ── Import devices ──
        let devices_dir = base.join("devices");
        if devices_dir.exists() {
            for entry in std::fs::read_dir(&devices_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
                    continue;
                }
                let content =
                    std::fs::read_to_string(&path).context(format!("Failed to read {:?}", path))?;
                let device: DeviceExport = serde_yaml_neo::from_str(&content)
                    .context(format!("Failed to parse {:?}", path))?;

                // Validate required fields
                if device.id.is_empty() {
                    anyhow::bail!("Device in {:?} has empty id", path);
                }
                if device.ip.is_empty() {
                    anyhow::bail!("Device '{}' has empty ip", device.id);
                }

                let registers: Vec<crate::models::ModbusRegisterInput> = device
                    .registers
                    .iter()
                    .map(|r| crate::models::ModbusRegisterInput {
                        name: r.name.clone(),
                        address: Some(r.address),
                        offset: None,
                        zero_based: r.zero_based,
                        register_type: r.register_type.clone(),
                        register_area: r.register_area.clone(),
                        scale: r.scale,
                    })
                    .collect();

                // Upsert: delete existing and re-create
                let _ = storage.delete_modbus_device(&device.id).await;
                storage
                    .create_modbus_device(
                        &device.id,
                        &device.name,
                        &device.ip,
                        device.port,
                        device.unit_id,
                        device.interval_ms,
                        &device.endianness,
                        &registers,
                    )
                    .await
                    .context(format!("Failed to import device '{}'", device.id))?;

                // Restore enabled state
                if !device.enabled {
                    storage
                        .update_modbus_device(&device.id, &serde_json::json!({"enabled": false}))
                        .await?;
                }
            }
        }

        // ── Import flows ──
        let flows_dir = base.join("flows");
        if flows_dir.exists() {
            for entry in std::fs::read_dir(&flows_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
                    continue;
                }
                let content =
                    std::fs::read_to_string(&path).context(format!("Failed to read {:?}", path))?;
                let flow: FlowExport = serde_yaml_neo::from_str(&content)
                    .context(format!("Failed to parse {:?}", path))?;

                if flow.id.is_empty() {
                    anyhow::bail!("Flow in {:?} has empty id", path);
                }

                let trigger_config = serde_json::to_string(&flow.trigger_config)?;
                let nodes = serde_json::to_string(&flow.nodes)?;

                // Delete existing and recreate
                let _ = storage.delete_flow(&flow.id).await;
                storage
                    .create_flow(&flow.id, &flow.name, flow.enabled, &trigger_config, &nodes)
                    .await
                    .context(format!("Failed to import flow '{}'", flow.id))?;
            }
        }

        // ── Import notifications ──
        let notif_path = base.join("notifications.yaml");
        if notif_path.exists() {
            let content = std::fs::read_to_string(&notif_path)?;
            let notifications: Vec<NotificationExport> =
                serde_yaml_neo::from_str(&content).context("Failed to parse notifications.yaml")?;
            for n in &notifications {
                let config = serde_json::to_string(&n.config)?;
                storage
                    .create_notification_setting(&n.source_key, &n.channel_type, &config, n.enabled)
                    .await?;
            }
        }

        anyhow::Ok(())
    })?;

    Ok(())
}
