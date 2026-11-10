#![allow(dead_code)]
use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::cache::DeviceCache;
use crate::models::{DeviceStatus, WriteResult};
use crate::quality::Quality;

use super::client::EthernetIpClient;
use super::types::*;

// ── Poller ──

/// Runtime config for an EtherNet/IP device poller.
#[derive(Debug, Clone)]
pub struct EnipDeviceRuntimeConfig {
    pub id: String,
    pub url: String,
    pub slot: u8,
    pub interval_ms: u64,
    pub tags: Vec<EnipTagConfig>,
}

/// Tag configuration for the poller.
#[derive(Debug, Clone, Default)]
pub struct EnipTagConfig {
    pub tag_name: String,
    pub name: String,
    pub data_type: String,
    pub assembly_offset: u32,
    pub assembly_length: u32,
}

/// Main entry point for EtherNet/IP device polling.
///
/// Spawned as `tokio::spawn(ethernet_ip::poll_enip_device(config, cache))`.
/// Runs an infinite loop with exponential backoff on connection failure.
pub async fn poll_enip_device(config: EnipDeviceRuntimeConfig, cache: DeviceCache) {
    let device_id = config.id.clone();
    let mut backoff = Backoff::new();

    loop {
        match try_connect_and_poll(&config, &cache).await {
            Ok(()) => {
                backoff.reset();
            }
            Err(e) => {
                tracing::warn!("EtherNet/IP device {} error: {e}", device_id);
                cache.set_status(&device_id, DeviceStatus::Offline).await;
                let delay = backoff.next();
                tracing::info!("EtherNet/IP device {} retrying in {delay}ms", device_id);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                continue;
            }
        }

        // Wait for the configured interval
        tokio::time::sleep(Duration::from_millis(config.interval_ms)).await;
    }
}

/// Polling cycles in a row in which every tag fails before the device is
/// marked offline and reconnected. A lost connection does so at once.
const MAX_FAILED_CYCLES: u32 = 3;

/// Connect once, then poll in a loop until an error occurs.
pub(super) async fn try_connect_and_poll(
    config: &EnipDeviceRuntimeConfig,
    cache: &DeviceCache,
) -> Result<()> {
    let mut client = EthernetIpClient::new(&config.url);
    client
        .connect()
        .await
        .context("EtherNet/IP connect failed")?;

    cache.set_status(&config.id, DeviceStatus::Online).await;

    let mut failed_cycles = 0;
    loop {
        let mut metrics = HashMap::new();
        let mut quality = HashMap::new();
        let mut failures = 0;

        for tag in &config.tags {
            let result = client.read_tag(&tag.tag_name).await;
            if let Err(e) = &result {
                if !client.is_connected() {
                    anyhow::bail!("EtherNet/IP tag read '{}' failed: {e}", tag.tag_name);
                }
                tracing::warn!("EtherNet/IP tag read '{}' failed: {}", tag.tag_name, e);
                failures += 1;
            }
            record_read(&mut metrics, &mut quality, &tag.name, result);
        }

        if !config.tags.is_empty() && failures == config.tags.len() {
            failed_cycles += 1;
            if failed_cycles >= MAX_FAILED_CYCLES {
                anyhow::bail!("every tag read failed {failed_cycles} cycles in a row");
            }
        } else {
            failed_cycles = 0;
        }

        if !metrics.is_empty() {
            cache
                .update_with_quality(&config.id, metrics, quality, false)
                .await;
        }

        tokio::time::sleep(Duration::from_millis(config.interval_ms)).await;
    }
}

/// Record one tag read. A failed read is NaN and bad rather than missing, so
/// the tag does not vanish from the device; a value that is not a number
/// (a string, a structure) is left out as before.
fn record_read(
    metrics: &mut HashMap<String, f64>,
    quality: &mut HashMap<String, Quality>,
    name: &str,
    result: Result<CipValue>,
) {
    match result {
        Ok(value) => {
            if let Some(v) = value.to_f64() {
                metrics.insert(name.to_string(), v);
            }
        }
        Err(_) => {
            metrics.insert(name.to_string(), f64::NAN);
            quality.insert(name.to_string(), Quality::BadDeviceError);
        }
    }
}

// ── One-shot write function ──

/// Write an engineering value to an EtherNet/IP CIP tag at `url` (`ip:port`).
///
/// The value is encoded as `data_type` (a CIP type name such as `REAL`), since
/// a Logix controller refuses a write whose type differs from the tag's. With
/// no usable configured type, the tag is read first and written back in the
/// type the controller reports.
pub async fn write_tag(url: &str, tag_name: &str, data_type: &str, value: f64) -> WriteResult {
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

    let configured = if data_type.trim().is_empty() {
        None
    } else {
        match CipDataType::from_name(data_type) {
            Some(t) => Some(t),
            None => {
                return fail(format!(
                    "Unknown EtherNet/IP data type '{data_type}' for tag '{tag_name}'"
                ));
            }
        }
    };

    let timeout = std::time::Duration::from_secs(5);
    let mut client = EthernetIpClient::new(url);
    match tokio::time::timeout(timeout, client.connect()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return fail(format!("EtherNet/IP connection failed: {e}")),
        Err(_) => return fail("EtherNet/IP connect timeout (5s)".into()),
    }

    let result = async {
        let cip_type = match configured {
            Some(t) => t,
            None => {
                let current = tokio::time::timeout(timeout, client.read_tag(tag_name))
                    .await
                    .map_err(|_| "EtherNet/IP read timeout (5s)".to_string())?
                    .map_err(|e| format!("EtherNet/IP read of '{tag_name}' failed: {e}"))?;
                CipDataType::of_value(&current).ok_or_else(|| {
                    format!(
                        "Tag '{tag_name}' has no data type configured and the PLC's type \
                         cannot be told from its value; set the tag's data type"
                    )
                })?
            }
        };
        let cip_value = cip_type.value_for(value)?;
        tokio::time::timeout(timeout, client.write_tag(tag_name, &cip_value))
            .await
            .map_err(|_| "EtherNet/IP write timeout (5s)".to_string())?
            .map_err(|e| format!("EtherNet/IP write failed: {e}"))?;
        Ok::<_, String>(cip_type)
    }
    .await;

    let _ = client.disconnect().await;

    match result {
        Ok(cip_type) => WriteResult {
            success: true,
            device_response: Some(format!(
                "Wrote tag '{}' = {} ({})",
                tag_name,
                value,
                cip_type.as_str()
            )),
            error: None,
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Err(e) => fail(e),
    }
}

// ── Backoff helper ──

pub(crate) struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub(crate) fn new() -> Self {
        Self { attempt: 0 }
    }

    pub(crate) fn reset(&mut self) {
        self.attempt = 0;
    }

    pub(crate) fn next(&mut self) -> u64 {
        let delay = 1000 * 2u64.pow(self.attempt.min(6));
        self.attempt += 1;
        delay.min(60_000)
    }
}

// ── Tag name validation ──

/// Validate a CIP tag name. Valid tag names contain alphanumeric characters,
/// underscores, and may use dotted paths for structure members (e.g., "MyTag.Member").
pub fn validate_tag_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Tag name cannot be empty");
    }
    if name.len() > 255 {
        anyhow::bail!("Tag name too long (max 255 chars)");
    }

    for (i, ch) in name.chars().enumerate() {
        if i == 0 && ch.is_ascii_digit() {
            anyhow::bail!("Tag name cannot start with a digit");
        }
        if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '.' && ch != '[' && ch != ']' {
            anyhow::bail!(
                "Tag name contains invalid character '{}' at position {}",
                ch,
                i
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod quality_tests {
    use super::*;

    #[test]
    fn a_failed_tag_read_is_nan_and_bad() {
        let mut metrics = HashMap::new();
        let mut quality = HashMap::new();
        record_read(&mut metrics, &mut quality, "ok", Ok(CipValue::Dint(4)));
        record_read(
            &mut metrics,
            &mut quality,
            "broken",
            Err(anyhow::anyhow!("path destination unknown")),
        );
        assert_eq!(metrics["ok"], 4.0);
        assert!(!quality.contains_key("ok"));
        assert!(metrics["broken"].is_nan());
        assert_eq!(quality["broken"], Quality::BadDeviceError);
    }
}
