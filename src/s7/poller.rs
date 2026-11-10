use std::collections::HashMap;

use crate::cache::DeviceCache;
use crate::models::{DeviceStatus, WriteResult};
use crate::quality::Quality;

use super::client::S7Client;
use super::types::*;

// ── Poller ──

/// Runtime config for an S7 device, used by the poller.
#[derive(Debug, Clone)]
pub struct S7DeviceConfig {
    pub id: String,
    pub url: String,
    pub rack: u8,
    pub slot: u8,
    pub interval_ms: u64,
    pub tags: Vec<S7Tag>,
}

/// Poll an S7 device in a loop. Intended to be spawned as a tokio task.
pub async fn poll_s7_device(config: S7DeviceConfig, cache: DeviceCache) {
    let device_id = config.id.clone();
    let mut backoff = crate::modbus::types::Backoff::new();

    loop {
        let mut client = S7Client::new(&config.url, config.rack, config.slot);

        match client.connect().await {
            Ok(()) => {
                tracing::info!("S7 device {} connected", device_id);
                backoff.reset();
                cache.set_status(&device_id, DeviceStatus::Online).await;

                loop {
                    match client.read_tags(&config.tags).await {
                        Ok(items) => {
                            let (metrics, quality) = items_to_metrics(&config.tags, items);
                            cache
                                .update_with_quality(&device_id, metrics, quality, false)
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!("S7 device {} read error: {}", device_id, e);
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
                    "S7 device {} connection failed: {}, retrying in {}ms",
                    device_id,
                    e,
                    delay
                );
                cache.set_status(&device_id, DeviceStatus::Offline).await;
            }
        }

        let _ = client.disconnect().await;
        let delay = backoff.next();
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
}

/// The cache name of a tag: its configured name, or area and offset.
fn tag_name(tag: &S7Tag) -> String {
    tag.name
        .clone()
        .unwrap_or_else(|| format!("{}_{}", tag.area.to_string().to_lowercase(), tag.offset))
}

/// Metrics and their quality from one Read Var answer. A refused item is
/// NaN and bad, never a 0 that would read like a real value.
pub(super) fn items_to_metrics(
    tags: &[S7Tag],
    items: Vec<super::client::S7ReadItem>,
) -> (HashMap<String, f64>, HashMap<String, Quality>) {
    let mut metrics = HashMap::new();
    let mut quality = HashMap::new();
    for (tag, item) in tags.iter().zip(items) {
        let name = tag_name(tag);
        match item {
            Ok(value) => {
                metrics.insert(name, value.to_f64());
            }
            Err(code) => {
                quality.insert(name.clone(), super::client::item_error_quality(code));
                metrics.insert(name, f64::NAN);
            }
        }
    }
    (metrics, quality)
}

// ── One-shot write function ──

/// Write an engineering value to one S7 tag, encoded in the tag's own data
/// type (see [`S7Tag::resolve`]), over a short-lived connection to `url`
/// (`ip:port`) with the device's rack and slot.
pub async fn write_tag(url: &str, rack: u8, slot: u8, tag: &S7Tag, value: f64) -> WriteResult {
    if !crate::node_role::is_active() {
        return WriteResult::refused_passive();
    }
    let start = std::time::Instant::now();
    let fail = |error: String| WriteResult {
        success: false,
        device_response: None,
        error: Some(error),
        latency_ms: start.elapsed().as_millis() as u64,
    };

    let s7_value = match tag.value_for(value) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };

    let mut client = S7Client::new(url, rack, slot);
    match tokio::time::timeout(std::time::Duration::from_secs(5), client.connect()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return fail(format!("S7 connection failed: {e}")),
        Err(_) => return fail("S7 connect timeout (5s)".into()),
    }

    let write_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.write_tag(tag, &s7_value),
    )
    .await;

    let _ = client.disconnect().await;

    match write_result {
        Ok(Ok(())) => WriteResult {
            success: true,
            device_response: Some(format!(
                "Wrote {} DB{} offset {}.{} = {} ({:?})",
                tag.area, tag.db_number, tag.offset, tag.bit_offset, value, tag.data_type
            )),
            error: None,
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Ok(Err(e)) => fail(format!("S7 write failed: {e}")),
        Err(_) => fail("S7 write timeout (5s)".into()),
    }
}
