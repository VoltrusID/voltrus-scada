use crate::api::{self, AppState};
use crate::cache::DeviceCache;
use crate::edition::{Extension, StartContext};
use crate::models::DeviceStatus;
use crate::poller_manager::PollerManager;
use crate::storage::Storage;
use crate::{
    cli, command_queue, config, health, network, notification, oee, retention, widget_sdk,
};
use axum::serve::ListenerExt;
use std::sync::Arc;

fn run_watchdog() {
    tracing_subscriber::fmt::init();
    tracing::info!("Voltrus watchdog process started");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
    rt.block_on(async {
        let mut failures = 0u32;
        let max_failures = 3u32;
        let max_restarts = 5u32;
        let mut restarts = 0u32;
        let cooldown = std::time::Duration::from_secs(30);

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));

        loop {
            interval.tick().await;

            let healthy = matches!(
                reqwest::get("http://localhost:8080/api/v1/health").await,
                Ok(resp) if resp.status().is_success()
            );

            if healthy {
                failures = 0;
                continue;
            }

            failures += 1;
            tracing::warn!("Health check failed ({}/{})", failures, max_failures);

            if failures >= max_failures && restarts < max_restarts {
                tracing::warn!(
                    "Restarting Voltrus main process (attempt {}/{})",
                    restarts + 1,
                    max_restarts
                );

                #[cfg(unix)]
                {
                    let _ = std::process::Command::new("pkill")
                        .args(["-f", "voltrus"])
                        .output();
                }

                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                let exe = std::env::current_exe().expect("Cannot find executable");
                match std::process::Command::new(exe).spawn() {
                    Ok(child) => {
                        tracing::info!("Started Voltrus process (PID: {:?})", child.id());
                        restarts += 1;
                        failures = 0;
                        tokio::time::sleep(cooldown).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to start Voltrus: {}", e);
                    }
                }
            }
        }
    });
}

/// Start the server (or handle a CLI subcommand) with the given edition hooks.
///
/// The `voltrus` binary passes [`crate::edition::Community`]. Call this from
/// your own `main` to run the core with an add-on edition.
pub async fn run(ext: Arc<dyn Extension>) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.contains(&"--watchdog".to_string()) {
        run_watchdog();
        return Ok(());
    }

    if cli::handle_cli(&args, ext.as_ref()).await {
        return Ok(());
    }

    let config_path = args.get(1).map(|s| s.as_str()).unwrap_or("config.yaml");
    let app_config = config::load_config(config_path)?;

    tracing::info!(
        "Loaded config: {} devices in YAML, port {}",
        app_config.devices.len(),
        app_config.server_port
    );

    let cache = DeviceCache::new();
    let ws_manager = Arc::new(crate::ws_manager::WsManager::new());
    let cache = cache.with_ws_manager(ws_manager.clone());
    let storage =
        Storage::new_with_key(&app_config.db_path, app_config.db_encryption_key.as_deref())?;
    let cache = cache.with_storage(storage.clone());
    let license_status = {
        let status = ext.resolve_license(&storage).await;
        Arc::new(std::sync::RwLock::new(status))
    };
    tracing::info!("Edition: {}", ext.name());

    // FlowEngine (lazy: only spawns when enabled flows exist)
    let flow_engine = {
        let engine = crate::flow_engine::engine::FlowEngine::new(storage.clone(), cache.clone());
        let mut has_enabled_flows = false;
        if let Ok(flows) = storage.list_flows().await {
            for flow in &flows {
                if flow.enabled {
                    match (
                        serde_json::from_str::<crate::flow_engine::trigger::TriggerConfig>(
                            &flow.trigger_config,
                        ),
                        serde_json::from_str::<Vec<crate::flow_engine::node::NodeConfig>>(
                            &flow.nodes,
                        ),
                    ) {
                        (Ok(trigger), Ok(nodes)) => {
                            let engine_flow = crate::flow_engine::engine::Flow {
                                id: flow.id.clone(),
                                name: flow.name.clone(),
                                enabled: flow.enabled,
                                trigger,
                                nodes,
                            };
                            engine.add_flow(engine_flow).await;
                            has_enabled_flows = true;
                        }
                        _ => {
                            tracing::warn!(
                                "Failed to deserialize flow {} config, skipping",
                                flow.id
                            );
                        }
                    }
                }
            }
        }
        engine.spawn().await;
        tracing::info!(
            seeded_flows = has_enabled_flows,
            "FlowEngine started (runtime-added flows are picked up live)"
        );
        Some(Arc::new(engine))
    };

    // Auto-seed on startup if no admin user exists
    storage.seed_database(&app_config.devices).await?;

    // Migrate YAML devices to DB (one-time)
    if !storage.is_yaml_migrated().await? && !app_config.devices.is_empty() {
        storage.import_yaml_devices(&app_config.devices).await?;
        storage.mark_yaml_migrated().await?;
        tracing::info!(
            "Migrated {} devices from YAML to database",
            app_config.devices.len()
        );
    }

    // Load device configs from DB and start pollers
    let poller_manager = PollerManager::new(
        cache.clone(),
        storage.clone(),
        network::NetworkManager::new(app_config.networks.clone()),
    );
    #[cfg(feature = "modbus")]
    {
        let device_configs = storage.get_all_device_configs().await?;
        for config in &device_configs {
            poller_manager.start(config.clone()).await;
            tracing::info!("Started poller for device {}", config.id);
        }
    }

    // Start OPC-UA pollers
    #[cfg(feature = "opcua")]
    match storage.get_all_opcua_device_configs().await {
        Ok(opcua_configs) => {
            for config in &opcua_configs {
                poller_manager.start_opcua(config.clone()).await;
                tracing::info!("Started OPC-UA poller for device {}", config.id);
            }
            if !opcua_configs.is_empty() {
                tracing::info!("Started {} OPC-UA device(s)", opcua_configs.len());
            }
        }
        Err(e) => {
            tracing::warn!("Failed to load OPC-UA device configs: {}", e);
        }
    }

    // Build optional email notifier from SMTP config
    let email_notifier = match &app_config.smtp {
        Some(smtp_cfg) => match notification::email::EmailNotifier::new(smtp_cfg) {
            Ok(notifier) => {
                tracing::info!(
                    "Email notifier initialized (from: {})",
                    smtp_cfg.from_address
                );
                Some(notifier)
            }
            Err(e) => {
                tracing::warn!("Failed to initialize email notifier: {}", e);
                None
            }
        },
        None => None,
    };

    // Spawn persistence task
    {
        let cache = cache.clone();
        let storage = storage.clone();
        let email_notifier = email_notifier.clone();
        let alarm_ws_manager = ws_manager.clone();
        let interval = app_config.persist_interval_secs;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval));
            // ISA-18.2 on/off delay state per source key. Starts empty on
            // startup and recomputes from the current value — no stale
            // pendings survive a restart.
            let mut alarm_delays: std::collections::HashMap<
                String,
                crate::alarm_delay::DelayState,
            > = std::collections::HashMap::new();
            loop {
                ticker.tick().await;
                let snapshots = cache.get_all().await;
                if snapshots.is_empty() {
                    continue;
                }
                let mut records = Vec::new();
                for snap in &snapshots {
                    for (metric, value) in &snap.metrics {
                        records.push((
                            snap.device_id.clone(),
                            snap.timestamp,
                            metric.clone(),
                            *value,
                        ));
                    }
                }
                if let Err(e) = storage.insert_batch(&records).await {
                    tracing::error!("Persistence error: {e}");
                }

                // Evaluate alarm thresholds
                if let Ok(sources) = storage.list_data_sources(None).await {
                    let mut value_map = std::collections::HashMap::new();
                    for snap in &snapshots {
                        for (metric, value) in &snap.metrics {
                            value_map.insert(format!("{}:{}", snap.device_id, metric), *value);
                        }
                    }

                    for source in &sources {
                        let value = if source.source_type == "modbus" {
                            // Find device metrics
                            let metrics: std::collections::HashMap<String, f64> = snapshots
                                .iter()
                                .filter(|s| {
                                    s.device_id == source.device_id
                                        && s.status == DeviceStatus::Online
                                })
                                .flat_map(|s| s.metrics.clone())
                                .collect();

                            if metrics.is_empty() {
                                continue;
                            }

                            // Check for expression in source_config
                            if let Ok(cfg) =
                                serde_json::from_str::<serde_json::Value>(&source.source_config)
                            {
                                if let Some(expr_str) =
                                    cfg.get("expression").and_then(|v| v.as_str())
                                {
                                    match crate::expression::evaluate(expr_str, &metrics) {
                                        Ok(val) => val,
                                        Err(_) => continue,
                                    }
                                } else {
                                    *value_map
                                        .get(&format!(
                                            "{}:{}",
                                            source.device_id,
                                            crate::api::data::metric_name(
                                                &source.key,
                                                &source.device_id
                                            )
                                        ))
                                        .unwrap_or(&source.base_val)
                                }
                            } else {
                                *value_map
                                    .get(&format!(
                                        "{}:{}",
                                        source.device_id,
                                        crate::api::data::metric_name(
                                            &source.key,
                                            &source.device_id
                                        )
                                    ))
                                    .unwrap_or(&source.base_val)
                            }
                        } else {
                            continue;
                        };

                        // Parse alarm deadband + low-alarm thresholds from source_config.
                        // Low thresholds are Optional: absent => disabled (no low alarm),
                        // present (even 0) => enabled. High thresholds use warn_val/crit_val.
                        let (deadband, low_warn, low_crit): (f64, Option<f64>, Option<f64>) =
                            if let Ok(cfg) =
                                serde_json::from_str::<serde_json::Value>(&source.source_config)
                            {
                                (
                                    cfg.get("deadband").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                    cfg.get("low_warn").and_then(|v| v.as_f64()),
                                    cfg.get("low_crit").and_then(|v| v.as_f64()),
                                )
                            } else {
                                (0.0, None, None)
                            };

                        // Decide which alarm condition (if any) is met. High alarms fire
                        // when value >= threshold; low alarms when value <= threshold.
                        // Critical takes precedence. The None arm resolves all active alarms
                        // for this source with direction-aware deadband hysteresis.
                        let cond = if source.crit_val > 0.0 && value >= source.crit_val {
                            Some(("critical", source.crit_val, "high"))
                        } else if source.warn_val > 0.0 && value >= source.warn_val {
                            Some(("warning", source.warn_val, "high"))
                        } else if low_crit.is_some_and(|lc| value <= lc) {
                            Some(("critical", low_crit.unwrap_or(0.0), "low"))
                        } else if low_warn.is_some_and(|lw| value <= lw) {
                            Some(("warning", low_warn.unwrap_or(0.0), "low"))
                        } else {
                            None
                        };

                        // ISA-18.2 on/off delays (seconds) from the source's
                        // threshold config. 0 keeps the immediate behavior.
                        let now = crate::timeutil::now_secs();
                        let on_delay = source.on_delay_secs.max(0);
                        let off_delay = source.off_delay_secs.max(0);

                        match cond {
                            Some((severity, threshold, alarm_type)) => {
                                // Non-repeating: suppress while an unresolved alarm of this
                                // severity exists or while in cooldown.
                                let has_unresolved = storage
                                    .has_unresolved_alarm(&source.key, severity)
                                    .await
                                    .unwrap_or(false);
                                let in_cooldown = storage
                                    .is_in_cooldown(&source.key, severity)
                                    .await
                                    .unwrap_or(false);
                                let mut may_fire = !has_unresolved && !in_cooldown;
                                // ISA-18.2 on delay: the raised condition must persist
                                // for on_delay seconds before the alarm fires. While
                                // suppressed by an existing alarm or cooldown, drop any
                                // pending so a fresh delay starts after it resolves.
                                if on_delay > 0 {
                                    if may_fire {
                                        let state =
                                            alarm_delays.entry(source.key.clone()).or_default();
                                        may_fire = crate::alarm_delay::update_delay(
                                            state, true, on_delay, 0, now,
                                        ) == crate::alarm_delay::DelayTransition::Fire;
                                    } else if let Some(state) = alarm_delays.get_mut(&source.key) {
                                        state.pending_since = None;
                                    }
                                }
                                if may_fire {
                                    let msg = if alarm_type == "low" {
                                        format!(
                                            "{} dropped to {}{} ({} low: {}{})",
                                            source.name,
                                            value,
                                            source.unit,
                                            severity,
                                            threshold,
                                            source.unit
                                        )
                                    } else {
                                        format!(
                                            "{} reached {}{} ({}: {}{})",
                                            source.name,
                                            value,
                                            source.unit,
                                            severity,
                                            threshold,
                                            source.unit
                                        )
                                    };
                                    match storage
                                        .create_alarm_with_type(
                                            &source.key,
                                            severity,
                                            value,
                                            threshold,
                                            &msg,
                                            deadband,
                                            alarm_type,
                                        )
                                        .await
                                    {
                                        Ok(alarm_id) => {
                                            let notif_storage = storage.clone();
                                            let notif_notifier = email_notifier.clone();
                                            let alarm = crate::models::Alarm {
                                                id: alarm_id,
                                                source_key: source.key.clone(),
                                                source_name: source.name.clone(),
                                                severity: severity.to_string(),
                                                priority: severity.to_string(),
                                                value,
                                                threshold,
                                                message: msg.clone(),
                                                triggered_at: crate::timeutil::now_secs(),
                                                resolved_at: None,
                                                acknowledged_at: None,
                                                acknowledged_by: None,
                                                cooldown_until: None,
                                                shelved_until: None,
                                                shelved_by: None,
                                                shelved_reason: None,
                                                deadband,
                                                state: "active".to_string(),
                                                alarm_type: alarm_type.to_string(),
                                            };
                                            // Broadcast alarm via WebSocket
                                            alarm_ws_manager.broadcast_alarm(
                                                crate::ws_types::ServerMessage::Alarm {
                                                    alarm_id,
                                                    tag_id: source.key.clone(),
                                                    severity: severity.to_string(),
                                                    message: msg,
                                                    value,
                                                    ts: alarm.triggered_at,
                                                },
                                            );
                                            tokio::spawn(async move {
                                                notification::dispatch_alarm_notifications(
                                                    notif_storage,
                                                    &alarm,
                                                    notif_notifier,
                                                )
                                                .await;
                                            });
                                        }
                                        Err(e) => tracing::error!("Failed to create alarm: {}", e),
                                    }
                                }
                            }
                            None => {
                                // Blip: the raised condition dropped before the on
                                // delay elapsed — clear the pending, it never fires.
                                if on_delay > 0
                                    && let Some(state) = alarm_delays.get_mut(&source.key)
                                {
                                    state.pending_since = None;
                                }
                                // ISA-18.2 off delay: with off_delay > 0 an active alarm
                                // stays raised until the normal condition persists that
                                // long. With off_delay = 0 resolve immediately with
                                // direction-aware deadband hysteresis (unchanged).
                                let resolve_now = if off_delay > 0 {
                                    let any_active = storage
                                        .has_any_unresolved_alarm(&source.key)
                                        .await
                                        .unwrap_or(false);
                                    if any_active {
                                        let state =
                                            alarm_delays.entry(source.key.clone()).or_default();
                                        crate::alarm_delay::update_delay(
                                            state, false, 0, off_delay, now,
                                        ) == crate::alarm_delay::DelayTransition::Resolve
                                    } else {
                                        // Nothing to resolve; also drop any stale
                                        // clear pending from a restart.
                                        if let Some(state) = alarm_delays.get_mut(&source.key) {
                                            state.clear_pending_since = None;
                                        }
                                        false
                                    }
                                } else {
                                    true
                                };
                                if resolve_now
                                    && let Ok(resolved) = storage
                                        .resolve_active_alarms(&source.key, Some(value))
                                        .await
                                {
                                    alarm_ws_manager
                                        .broadcast_alarm_updates("resolved", None, &resolved);
                                }
                            }
                        }
                    }
                }

                // ── Outbound webhooks: tag_change events (v0.39.0) ──
                // Dispatch tag_change webhooks for the latest data.
                // We do this after persistence + alarm evaluation, not on every
                // tick — outbound webhooks get the latest snapshot values.
                {
                    let storage_wh = storage.clone();
                    for snap in &snapshots {
                        if snap.status != DeviceStatus::Online {
                            continue;
                        }
                        for (metric, value) in &snap.metrics {
                            let payload = serde_json::json!({
                                "event": "tag_change",
                                "device_id": snap.device_id,
                                "metric": metric,
                                "value": value,
                                "timestamp": snap.timestamp,
                                "status": "online",
                            });
                            crate::notification::dispatch_outbound_webhooks(
                                &storage_wh,
                                "tag_change",
                                &payload,
                            )
                            .await;
                        }
                    }
                }
            }
        });
    }

    // Spawn retention task
    {
        let storage = storage.clone();
        let retention_config = app_config.retention.clone();
        tokio::spawn(async move {
            retention::run_retention_task(storage, retention_config).await;
        });
        tracing::info!("Data retention/compression task started");
    }

    // Spawn alarm auto-unshelf task
    {
        let storage = storage.clone();
        let unshelf_ws_manager = ws_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                match storage.auto_unshelf_expired_alarms().await {
                    Ok(unshelved) => {
                        unshelf_ws_manager.broadcast_alarm_updates("unshelved", None, &unshelved);
                    }
                    Err(e) => tracing::warn!("Auto-unshelf task failed: {}", e),
                }
            }
        });
    }

    // Spawn egress scheduler
    {
        let egress_storage = storage.clone();
        let (_egress_shutdown_tx, egress_shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
        tokio::spawn(async move {
            crate::integration::egress::run_egress_scheduler(egress_storage, egress_shutdown_rx)
                .await;
        });
        tracing::info!("Data egress scheduler started");
    }

    // Spawn OEE monitor
    {
        let oee_cache = cache.clone();
        let oee_storage = storage.clone();
        tokio::spawn(async move {
            oee::run_oee_monitor(oee_cache, oee_storage).await;
        });
        tracing::info!("OEE monitor started");
    }

    // Shared Modbus connection pool (used by both command queue and direct writes)
    #[cfg(feature = "modbus")]
    let modbus_pool = std::sync::Arc::new(crate::modbus::ModbusPool::new());

    // Spawn command queue processor
    #[cfg(feature = "modbus")]
    let queue =
        command_queue::CommandQueue::new(storage.clone(), cache.clone(), modbus_pool.clone())
            .with_ws_manager(ws_manager.clone());
    #[cfg(not(feature = "modbus"))]
    let queue = command_queue::CommandQueue::new(storage.clone(), cache.clone())
        .with_ws_manager(ws_manager.clone());
    // Optional handle so the command queue can publish MQTT writes once the
    // embedded broker starts (broker init runs after the queue is spawned).
    #[cfg(feature = "mqtt-broker")]
    let queue_broker_handle = queue.mqtt_broker_handle();
    tokio::spawn(async move {
        queue.run().await;
    });
    tracing::info!("Command queue processor started");

    let auth_secret = match &app_config.auth_secret {
        Some(secret) => {
            anyhow::ensure!(
                secret.len() >= 16,
                "auth_secret must be at least 16 characters"
            );
            secret.clone()
        }
        None => {
            use rand::Rng;
            let secret: String = rand::rng()
                .sample_iter(&rand::distr::Alphanumeric)
                .take(32)
                .map(char::from)
                .collect();
            tracing::warn!(
                "auth_secret not set in config — generated a random secret. Set auth_secret in config.yaml for persistent sessions."
            );
            secret
        }
    };

    let login_rate_limiter = crate::security::LoginRateLimiter::new();
    let global_rate_limiter = crate::security::GlobalRateLimiter::new();
    let intrusion_detector = crate::security::IntrusionDetector::new();

    // Periodic cleanup of rate limiter entries to prevent unbounded memory growth
    {
        let login_rl = login_rate_limiter.clone();
        let global_rl = global_rate_limiter.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(120));
            loop {
                interval.tick().await;
                login_rl.cleanup();
                global_rl.cleanup();
            }
        });
    }

    let (audit_logger, mut audit_rx) = crate::security::AuditLogger::new();

    // Spawn audit log consumer
    {
        let storage = storage.clone();
        tokio::spawn(async move {
            while let Some(event) = audit_rx.recv().await {
                if let Err(e) = storage
                    .log_audit_event(
                        &event.action,
                        &event.username,
                        &event.resource,
                        event.details.as_deref(),
                    )
                    .await
                {
                    tracing::warn!("Failed to write audit log: {}", e);
                }
            }
        });
    }

    // Initialize MQTT broker
    #[cfg(feature = "mqtt-broker")]
    let mqtt_broker_state = {
        {
            let broker_state = crate::mqtt_broker::MqttBrokerState::new();
            let mqtt_cfg = &app_config.mqtt_broker;

            if mqtt_cfg.enabled {
                {
                    let mut cfg = broker_state.config.write().await;
                    *cfg = Some(mqtt_cfg.clone());
                }

                match crate::mqtt_broker::start_broker(broker_state.clone(), mqtt_cfg.clone()).await
                {
                    Ok(()) => {
                        tracing::info!(
                            "Embedded MQTT broker auto-started on port {}",
                            mqtt_cfg.port
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to auto-start MQTT broker: {}", e);
                    }
                }
            }

            Some(broker_state)
        }
    };
    // Hand the running broker to the command queue so MQTT command writes can publish.
    #[cfg(feature = "mqtt-broker")]
    if let Some(broker) = mqtt_broker_state.as_ref() {
        *queue_broker_handle.write().await = Some(broker.clone());
        tracing::info!("Command queue connected to MQTT broker for writes");
    }

    #[cfg(not(feature = "mqtt-broker"))]
    let mqtt_broker_state: Option<()> = None;

    // Feed the embedded MQTT broker's message stream into the flow engine so
    // flows with MQTT triggers fire on broker messages (BLE bridge / gateway /
    // sensors publishing to the broker). No-op without the `mqtt-broker` feature.
    #[cfg(feature = "mqtt-broker")]
    if let (Some(engine), Some(broker)) = (flow_engine.as_ref(), mqtt_broker_state.as_ref()) {
        engine.set_broker_tracker(broker.topic_tracker.clone());
        tracing::info!("FlowEngine subscribed to MQTT broker event stream");
    }

    // Initialize health monitor and HA state
    let health_monitor = Arc::new(health::watchdog::HealthMonitor::new(
        app_config.watchdog.clone(),
    ));
    {
        let storage_check = {
            let storage = storage.clone();
            move || match storage.conn_health() {
                true => Ok("ok".to_string()),
                false => Err("connection unhealthy".to_string()),
            }
        };
        health_monitor
            .register("storage", Box::new(storage_check))
            .await;
    }
    if app_config.watchdog.enabled {
        health_monitor.start();
        tracing::info!(
            "Health monitor started (interval: {}s)",
            app_config.watchdog.health_check_interval_secs
        );
    }

    // A catalog URL in the config file is the device template catalog setting.
    if let Some(url) = app_config
        .device_templates
        .catalog_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        match crate::device_templates::catalog::validate_url(url) {
            Ok(()) => {
                storage
                    .upsert_setting(crate::device_templates::catalog::SETTING_CATALOG_URL, url)
                    .await?;
            }
            Err(e) => tracing::warn!("Ignoring device_templates.catalog_url: {e}"),
        }
    }

    // Let the add-on edition start its own background work before the server listens.
    ext.start(StartContext {
        config: app_config.clone(),
        config_path: config_path.to_string(),
        storage: storage.clone(),
        cache: cache.clone(),
        ws_manager: ws_manager.clone(),
        poller_manager: poller_manager.clone(),
        license_status: license_status.clone(),
        email_notifier: email_notifier.clone(),
    })
    .await?;

    let state = AppState {
        cache: cache.clone(),
        storage: storage.clone(),
        poller_manager,
        started_at: std::time::Instant::now(),
        auth_secret,
        cors_origin: app_config.cors_origin.clone(),
        cookie_secure: app_config.cookie_secure,
        retention_config: app_config.retention.clone(),
        flow_engine,
        login_rate_limiter,
        global_rate_limiter,
        audit_logger,
        intrusion_detector,
        license_status,
        ext: ext.clone(),
        smtp_config: Arc::new(tokio::sync::RwLock::new(app_config.smtp.clone())),
        config_path: config_path.to_string(),
        ws_manager: ws_manager.clone(),
        mqtt_broker_state,
        network_manager: network::NetworkManager::new(app_config.networks.clone()),
        health_monitor,
        widget_registry: widget_sdk::WidgetRegistry::new(std::path::PathBuf::from(
            "voltrus/widgets",
        )),
        #[cfg(feature = "modbus")]
        modbus_pool,
    };

    api::stream::spawn_data_broadcaster(cache.clone(), storage.clone(), Some(ws_manager.clone()));
    let app = api::create_router(state);
    api::ui::log_source().await;

    let addr = format!("{}:{}", app_config.server_host, app_config.server_port);
    tracing::info!("Server listening on {addr}");

    let listener = bind_listener(&addr).await?;

    let shutdown = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
        tracing::info!("Received shutdown signal, draining connections...");

        // Give connections 3s to drain, then force quit
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        tracing::info!("Drain timeout, forcing shutdown");
        std::process::exit(0);
    };

    // `serve(listener, router)` runs `Router::clone().with_state(())` for every accepted
    // connection, which deep-copies the whole route table (every route's layer stack
    // and its `AppState`): ~14 MB held for as long as the connection stays open.
    // The make-service form hands each connection a cheap `Arc` clone of the router,
    // and records the peer address as `ConnectInfo` for `security::client_ip`.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await?;

    tracing::info!("Server shutdown complete");
    Ok(())
}

/// Listen on `addr`, with TCP_NODELAY on every accepted socket: live-data frames are
/// small and sent right after a request or response, so Nagle's algorithm plus the
/// browser's delayed ACK would hold them back by tens of milliseconds.
async fn bind_listener(
    addr: &str,
) -> anyhow::Result<axum::serve::TapIo<tokio::net::TcpListener, fn(&mut tokio::net::TcpStream)>> {
    Ok(tokio::net::TcpListener::bind(addr)
        .await?
        .tap_io(set_nodelay as fn(&mut tokio::net::TcpStream)))
}

fn set_nodelay(tcp_stream: &mut tokio::net::TcpStream) {
    if let Err(err) = tcp_stream.set_nodelay(true) {
        tracing::debug!("failed to set TCP_NODELAY on accepted socket: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::serve::Listener;

    #[tokio::test]
    async fn accepted_sockets_have_tcp_nodelay() {
        let mut listener = bind_listener("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client =
            tokio::spawn(async move { tokio::net::TcpStream::connect(addr).await.unwrap() });
        let (accepted, _) = listener.accept().await;
        assert!(
            accepted.nodelay().unwrap(),
            "the server side of the connection must not use Nagle"
        );
        client.await.unwrap();
    }
}
