use super::*;

/// Export all configuration from the database to YAML files in the given directory.
pub fn export_config(storage: &Storage, output_dir: &str) -> Result<()> {
    // Use tokio runtime for async storage calls
    let rt = tokio::runtime::Runtime::new().context("Failed to create tokio runtime")?;

    rt.block_on(async {
        let base = Path::new(output_dir);

        // Create directory structure
        std::fs::create_dir_all(base.join("devices")).context("Failed to create devices dir")?;
        std::fs::create_dir_all(base.join("screens")).context("Failed to create screens dir")?;
        std::fs::create_dir_all(base.join("flows")).context("Failed to create flows dir")?;

        // ── Export Modbus devices ──
        let devices = storage.list_modbus_devices().await?;
        for device in &devices {
            let registers = storage.list_modbus_registers(&device.id).await?;
            let export = DeviceExport {
                id: device.id.clone(),
                name: device.name.clone(),
                ip: device.ip.clone(),
                port: device.port,
                unit_id: device.unit_id,
                interval_ms: device.interval_ms,
                endianness: device.endianness.clone(),
                enabled: device.enabled,
                registers: registers
                    .iter()
                    .map(|r| RegisterExport {
                        name: r.name.clone(),
                        address: r.address,
                        register_type: r.register_type.clone(),
                        register_area: r.register_area.clone(),
                        scale: r.scale,
                        sort_order: r.sort_order,
                        zero_based: r.zero_based,
                    })
                    .collect(),
            };
            let yaml = serde_yaml_neo::to_string(&export).context("Failed to serialize device")?;
            let filename = sanitize_filename(&device.id);
            let path = base.join("devices").join(format!("{}.yaml", filename));
            std::fs::write(&path, yaml).context(format!("Failed to write {:?}", path))?;
        }

        // ── Export Screens ──
        let screens = storage.list_devices().await?;
        for screen in &screens {
            let _data_sources = storage.list_data_sources(Some(&screen.id)).await?;

            // Get layouts for this screen
            let layouts = storage.get_layouts(&screen.id).await.unwrap_or_default();
            let layout_exports: Vec<ScreenLayoutExport> = layouts
                .iter()
                .map(|l| ScreenLayoutExport {
                    breakpoint: l.breakpoint.clone(),
                    layout: serde_json::from_str(&l.layout).unwrap_or(serde_json::json!(null)),
                })
                .collect();

            // Get screen data source associations
            let ds_exports: Vec<ScreenDataSourceExport> = storage
                .get_screen_data_sources(&screen.id)
                .await
                .unwrap_or_default()
                .iter()
                .map(|s| ScreenDataSourceExport {
                    data_source_id: s.data_source_id,
                    sort_order: s.sort_order,
                })
                .collect();

            let export = ScreenExport {
                id: screen.id.clone(),
                name: screen.name.clone(),
                device_type: screen.device_type.clone(),
                sort_order: screen.sort_order,
                data_sources: ds_exports,
                layouts: layout_exports,
            };
            let yaml = serde_yaml_neo::to_string(&export).context("Failed to serialize screen")?;
            let filename = sanitize_filename(&screen.id);
            let path = base.join("screens").join(format!("{}.yaml", filename));
            std::fs::write(&path, yaml).context(format!("Failed to write {:?}", path))?;
        }

        // ── Export Flows ──
        let flows = storage.list_flows().await?;
        for flow in &flows {
            let trigger_config: serde_json::Value =
                serde_json::from_str(&flow.trigger_config).unwrap_or(serde_json::json!(null));
            let nodes: serde_json::Value =
                serde_json::from_str(&flow.nodes).unwrap_or(serde_json::json!(null));
            let export = FlowExport {
                id: flow.id.clone(),
                name: flow.name.clone(),
                enabled: flow.enabled,
                trigger_config,
                nodes,
            };
            let yaml = serde_yaml_neo::to_string(&export).context("Failed to serialize flow")?;
            let filename = sanitize_filename(&flow.id);
            let path = base.join("flows").join(format!("{}.yaml", filename));
            std::fs::write(&path, yaml).context(format!("Failed to write {:?}", path))?;
        }

        // ── Export alarm thresholds (from data sources) ──
        let all_sources = storage.list_data_sources(None).await?;
        let alarm_exports: Vec<AlarmExport> = all_sources
            .iter()
            .filter(|s| s.warn_val > 0.0 || s.crit_val > 0.0)
            .map(|s| AlarmExport {
                source_key: s.key.clone(),
                source_name: s.name.clone(),
                severity: "threshold".to_string(),
                value: 0.0,
                threshold: s.crit_val,
                message: format!(
                    "{} threshold: warn={} crit={} {}",
                    s.name, s.warn_val, s.crit_val, s.unit
                ),
            })
            .collect();
        let alarm_yaml =
            serde_yaml_neo::to_string(&alarm_exports).context("Failed to serialize alarms")?;
        std::fs::write(base.join("alarms.yaml"), alarm_yaml)
            .context("Failed to write alarms.yaml")?;

        // ── Export notification settings ──
        let notifications = storage.get_all_notification_settings().await?;
        let notif_exports: Vec<NotificationExport> = notifications
            .iter()
            .map(|n| NotificationExport {
                source_key: n.source_key.clone(),
                channel_type: n.channel_type.clone(),
                config: serde_json::from_str(&n.config).unwrap_or(serde_json::json!({})),
                enabled: n.enabled,
            })
            .collect();
        let notif_yaml = serde_yaml_neo::to_string(&notif_exports)
            .context("Failed to serialize notifications")?;
        std::fs::write(base.join("notifications.yaml"), notif_yaml)
            .context("Failed to write notifications.yaml")?;

        anyhow::Ok(())
    })?;

    Ok(())
}
