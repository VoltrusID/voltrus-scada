use super::*;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::cache::DeviceCache;
#[cfg(feature = "modbus")]
use crate::modbus::ModbusPool;
use crate::storage::Storage;
use crate::ws_manager::WsManager;

#[cfg(feature = "modbus")]
impl CommandQueue {
    pub fn new(storage: Storage, cache: DeviceCache, modbus_pool: Arc<ModbusPool>) -> Self {
        Self {
            storage,
            cache,
            in_flight: Arc::new(RwLock::new(HashSet::new())),
            ws_manager: None,
            readback_trackers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            modbus_pool,
            #[cfg(feature = "mqtt-broker")]
            mqtt_broker: Arc::new(RwLock::new(None)),
        }
    }
}

#[cfg(not(feature = "modbus"))]
impl CommandQueue {
    pub fn new(storage: Storage, cache: DeviceCache) -> Self {
        Self {
            storage,
            cache,
            in_flight: Arc::new(RwLock::new(HashSet::new())),
            ws_manager: None,
            readback_trackers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            #[cfg(feature = "mqtt-broker")]
            mqtt_broker: Arc::new(RwLock::new(None)),
        }
    }
}

impl CommandQueue {
    /// Attach a WsManager so that command status changes are broadcast over WebSocket.
    pub fn with_ws_manager(mut self, ws_manager: Arc<WsManager>) -> Self {
        self.ws_manager = Some(ws_manager);
        self
    }

    /// Returns a shared handle to the embedded MQTT broker slot so main.rs can
    /// populate it after the broker starts (the queue task is already spawned).
    #[cfg(feature = "mqtt-broker")]
    pub fn mqtt_broker_handle(&self) -> Arc<RwLock<Option<crate::mqtt_broker::MqttBrokerState>>> {
        self.mqtt_broker.clone()
    }

    /// Main loop: process the queue every 500 ms.
    pub async fn run(&self) {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(POLL_INTERVAL_MS));
        loop {
            interval.tick().await;
            // Queued commands wait for the node that owns the devices.
            if !crate::node_role::is_active() {
                continue;
            }
            if let Err(e) = self.process_queue().await {
                tracing::warn!("Command queue processor error: {}", e);
            }
        }
    }

    async fn process_queue(&self) -> anyhow::Result<()> {
        let commands = self.storage.get_queued_commands().await?;

        for cmd in &commands {
            let now = crate::timeutil::now_secs();
            tracing::info!(
                "Processing command: id={}, ds={}, device={}, protocol={}, status={}, value={}",
                cmd.id,
                cmd.data_source_id,
                cmd.device_id,
                cmd.protocol,
                cmd.status,
                cmd.value
            );

            // -- Expiry check --
            if cmd.status == "queued" && (cmd.created_at + COMMAND_EXPIRY_SECS) < now {
                self.publish_status_event(&cmd.id, "expired", None);
                self.storage
                    .update_command_status(
                        &cmd.id,
                        "expired",
                        Some("Command expired (30 min timeout)"),
                        "system",
                        Some("auto-expired"),
                    )
                    .await?;
                continue;
            }

            // -- Scheduled check --
            if cmd.policy == "scheduled"
                && let Some(scheduled_at) = cmd.scheduled_at
                && scheduled_at > now
            {
                continue; // Not yet time
            }

            // -- Retry backoff: don't hammer a failing device. --
            // Exponential backoff off retry_count (2^n s, capped at 60s). sent_at
            // records the last send and survives the queued→sent→queued retry
            // cycle, so skip until the backoff window elapses.
            if cmd.retry_count > 0
                && let Some(last_sent) = cmd.sent_at
            {
                let backoff_secs = 2_i64.pow(cmd.retry_count.min(6) as u32).min(60);
                if last_sent + backoff_secs > now {
                    continue;
                }
            }

            // -- Concurrency check: only one command per device in-flight --
            {
                let in_flight = self.in_flight.read().await;
                if in_flight.contains(&cmd.device_id) {
                    continue;
                }
            }

            // -- Resolve the text device ID from the data source for cache lookups --
            let ds = self
                .storage
                .get_data_source_by_id(cmd.data_source_id)
                .await
                .ok()
                .flatten();
            let cache_device_id = ds.as_ref().map(|d| d.device_id.clone()).unwrap_or_else(|| {
                if cmd.device_ref.is_empty() {
                    cmd.device_id.to_string()
                } else {
                    cmd.device_ref.clone()
                }
            });

            // -- Device online check --
            if !self.cache.is_device_online(&cache_device_id).await {
                tracing::warn!(
                    "Skipping command {}: device '{}' (cache key '{}') is offline or not in cache",
                    cmd.id,
                    cmd.device_id,
                    cache_device_id
                );
                continue;
            }

            // Mark as in-flight
            {
                let mut in_flight = self.in_flight.write().await;
                in_flight.insert(cmd.device_id);
            }

            // -- Execute --
            self.publish_status_event(&cmd.id, "sent", None);
            self.storage
                .update_command_status(&cmd.id, "sent", None, "system", None)
                .await?;

            let result = self.execute_write(cmd).await;

            // -- Handle result --
            match result.success {
                true => {
                    tracing::info!(
                        "Command acked: id={}, response={}, latency={}ms",
                        cmd.id,
                        result.device_response.as_deref().unwrap_or("OK"),
                        result.latency_ms
                    );
                    let response = result
                        .device_response
                        .clone()
                        .unwrap_or_else(|| "OK".to_string());
                    self.publish_ack_event(&cmd.id, &response, result.latency_ms);
                    self.storage
                        .update_command_status(&cmd.id, "acked", None, "system", None)
                        .await?;

                    // Update the cache so the new value is visible immediately
                    if let Some(ds) = ds
                        && let Ok(value) = cmd.value.parse::<f64>()
                    {
                        self.cache
                            .set_metric_value(&cache_device_id, &ds.key, value)
                            .await;
                    }
                }
                false => {
                    let error = result.error.as_deref().unwrap_or("Unknown error");
                    tracing::warn!(
                        "Command failed: id={}, error={}, retry={}/{}",
                        cmd.id,
                        error,
                        cmd.retry_count,
                        cmd.max_retries
                    );
                    if cmd.retry_count < cmd.max_retries {
                        self.publish_status_event(&cmd.id, "queued", Some(error));
                        self.storage
                            .update_command_status(
                                &cmd.id,
                                "queued",
                                Some(error),
                                "system",
                                Some("retry"),
                            )
                            .await?;
                    } else {
                        self.publish_status_event(&cmd.id, "failed", Some(error));
                        self.storage
                            .update_command_status(&cmd.id, "failed", Some(error), "system", None)
                            .await?;
                    }
                }
            }

            // Remove from in-flight
            {
                let mut in_flight = self.in_flight.write().await;
                in_flight.remove(&cmd.device_id);
            }
        }

        Ok(())
    }
}
