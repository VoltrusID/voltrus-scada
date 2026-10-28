use std::collections::HashMap;

use crate::cache::DeviceCache;
use crate::models::DeviceStatus;

use super::client::PlcRuntimeClient;
use super::types::VoltrusPlcDeviceConfig;

/// Poll a Voltrus PLC Runtime in a loop. Intended to be spawned as a tokio task.
///
/// Follows the same pattern as poll_s7_device / poll_device:
/// - Connect with backoff/retry
/// - Read variables each cycle
/// - Update the device cache for WebSocket broadcast to SCADA clients
pub async fn poll_voltrus_plc_device(config: VoltrusPlcDeviceConfig, cache: DeviceCache) {
    let device_id = config.id.clone();
    let mut backoff = Backoff::new();

    loop {
        let client = PlcRuntimeClient::new(&config.url, &config.transport);

        match client.connect().await {
            Ok(()) => {
                tracing::info!("Voltrus PLC {} connected ({})", device_id, config.transport);
                backoff.reset();
                cache.set_status(&device_id, DeviceStatus::Online).await;

                loop {
                    match client.read_variables().await {
                        Ok(variables) => {
                            // Convert variable values to metrics map for cache
                            let mut metrics = HashMap::new();
                            for tag in &config.tags {
                                let key = if tag.display_name.is_empty() {
                                    tag.variable_name.clone()
                                } else {
                                    tag.display_name.clone()
                                };
                                if let Some(value) = variables.get(&tag.variable_name) {
                                    metrics.insert(key, *value);
                                }
                            }
                            cache.update(&device_id, metrics, false).await;
                        }
                        Err(e) => {
                            tracing::warn!("Voltrus PLC {} read error: {}", device_id, e);
                            cache.set_status(&device_id, DeviceStatus::Offline).await;
                            break; // Reconnect
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_millis(config.interval_ms)).await;
                }
            }
            Err(e) => {
                let delay = backoff.next();
                tracing::warn!(
                    "Voltrus PLC {} connection failed: {}, retrying in {}ms",
                    device_id,
                    e,
                    delay
                );
                cache.set_status(&device_id, DeviceStatus::Offline).await;
            }
        }

        let delay = backoff.next();
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
}

/// Write a value to a variable on a Voltrus PLC Runtime.
pub async fn write_tag(
    url: &str,
    transport: &str,
    variable_name: &str,
    value: f64,
) -> crate::models::WriteResult {
    if !crate::node_role::is_active() {
        return crate::models::WriteResult::refused_passive();
    }
    let start = std::time::Instant::now();
    let client = PlcRuntimeClient::new(url, transport);

    match client.write_variable(variable_name, value).await {
        Ok(result) => result,
        Err(e) => crate::models::WriteResult {
            success: false,
            device_response: None,
            error: Some(e.to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// Simple exponential backoff (mirrors the modbus Backoff pattern).
struct Backoff {
    attempt: u32,
    max_delay_ms: u64,
}

impl Backoff {
    fn new() -> Self {
        Self {
            attempt: 0,
            max_delay_ms: 30_000,
        }
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }

    fn next(&mut self) -> u64 {
        let delay = 1000 * 2u64.pow(self.attempt);
        self.attempt += 1;
        delay.min(self.max_delay_ms)
    }
}
