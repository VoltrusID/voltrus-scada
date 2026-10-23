use std::collections::HashMap;

use crate::cache::DeviceCache;
use crate::models::{DeviceStatus, WriteResult};

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
                        Ok(values) => {
                            let mut metrics = HashMap::new();
                            for (i, value) in values.iter().enumerate() {
                                let tag_name = if i < config.tags.len() {
                                    config.tags[i].name.clone().unwrap_or_else(|| {
                                        format!(
                                            "{}_{}",
                                            config.tags[i].area.to_string().to_lowercase(),
                                            config.tags[i].offset
                                        )
                                    })
                                } else {
                                    format!("tag_{}", i)
                                };
                                metrics.insert(tag_name, value.to_f64());
                            }
                            cache.update(&device_id, metrics, false).await;
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

// ── One-shot write function ──

/// Write a single f64 value to an S7 PLC tag.
///
/// Connects to the PLC at `ip:port`, resolves the area string to an S7 area,
/// writes the value using the specified data type (defaults to REAL), and returns a `WriteResult`.
///
/// The `area` parameter accepts strings like "DB", "Input", "Output", "Marker",
/// "Timer", or "Counter".
///
/// The `data_type` parameter accepts strings like "bool", "byte", "word", "dword",
/// "real", "string". Defaults to "real" when None.
pub async fn write_tag(
    ip: &str,
    port: u16,
    area: &str,
    db_number: u32,
    offset: u16,
    value: f64,
    data_type: Option<&str>,
) -> WriteResult {
    let start = std::time::Instant::now();
    let url = format!("{}:{}", ip, port);

    // Resolve area string
    let s7_area = match parse_area_string(area) {
        Some(a) => a,
        None => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("Unknown S7 area: '{}'", area)),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    // Resolve data type (defaults to Real for backward compatibility)
    let s7_data_type = data_type
        .and_then(S7DataType::from_str_name)
        .unwrap_or(S7DataType::Real);

    let tag = S7Tag {
        area: s7_area,
        db_number: db_number as u16,
        offset,
        bit_offset: 0,
        data_type: s7_data_type.clone(),
        length: if s7_data_type == S7DataType::String {
            254
        } else {
            1
        },
        name: None,
    };

    let s7_value = match s7_data_type {
        S7DataType::Bool => S7Value::Bool(value != 0.0),
        S7DataType::Byte => S7Value::Byte(value as u8),
        S7DataType::Word => S7Value::Word(value as u16),
        S7DataType::DWord => S7Value::DWord(value as u32),
        S7DataType::Real => S7Value::Real(value as f32),
        S7DataType::String => S7Value::String(format!("{}", value)),
    };

    // Connect with timeout
    let mut client = S7Client::new(&url, 0, 1);
    let connect_result =
        tokio::time::timeout(std::time::Duration::from_secs(5), client.connect()).await;

    match connect_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("S7 connection failed: {e}")),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
        Err(_) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some("S7 connect timeout (5s)".into()),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    }

    // Write with timeout
    let write_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.write_tag(&tag, &s7_value),
    )
    .await;

    let _ = client.disconnect().await;

    match write_result {
        Ok(Ok(())) => WriteResult {
            success: true,
            device_response: Some(format!(
                "Wrote {} {} offset {} = {} ({:?})",
                area, db_number, offset, value, s7_data_type
            )),
            error: None,
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Ok(Err(e)) => WriteResult {
            success: false,
            device_response: None,
            error: Some(format!("S7 write failed: {e}")),
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Err(_) => WriteResult {
            success: false,
            device_response: None,
            error: Some("S7 write timeout (5s)".into()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// Parse an area string (case-insensitive) to an `S7Area`.
fn parse_area_string(s: &str) -> Option<S7Area> {
    match s.to_lowercase().as_str() {
        "db" | "data" => Some(S7Area::DB),
        "input" | "i" => Some(S7Area::Input),
        "output" | "q" | "o" => Some(S7Area::Output),
        "marker" | "m" => Some(S7Area::Marker),
        "timer" | "t" => Some(S7Area::Timer),
        "counter" | "c" => Some(S7Area::Counter),
        _ => None,
    }
}
