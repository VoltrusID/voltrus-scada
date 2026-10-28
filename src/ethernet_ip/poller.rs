#![allow(dead_code)]
use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::cache::DeviceCache;
use crate::models::{DeviceStatus, WriteResult};

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

/// Connect once, then poll in a loop until an error occurs.
async fn try_connect_and_poll(config: &EnipDeviceRuntimeConfig, cache: &DeviceCache) -> Result<()> {
    let mut client = EthernetIpClient::new(&config.url);
    client
        .connect()
        .await
        .context("EtherNet/IP connect failed")?;

    cache.set_status(&config.id, DeviceStatus::Online).await;

    loop {
        let mut metrics = HashMap::new();

        for tag in &config.tags {
            match client.read_tag(&tag.tag_name).await {
                Ok(value) => {
                    if let Some(fv) = value.to_f64() {
                        metrics.insert(tag.name.clone(), fv);
                    }
                }
                Err(e) => {
                    tracing::warn!("EtherNet/IP tag read '{}' failed: {}", tag.tag_name, e);
                }
            }
        }

        if !metrics.is_empty() {
            cache.update(&config.id, metrics, false).await;
        }

        tokio::time::sleep(Duration::from_millis(config.interval_ms)).await;
    }
}

// ── One-shot write function ──

/// Write a single f64 value to an EtherNet/IP CIP tag.
///
/// Connects to the PLC at `ip:port`, writes the value as a DINT (i32) to the
/// named tag, and returns a `WriteResult`.
pub async fn write_tag(ip: &str, port: u16, tag_name: &str, value: f64) -> WriteResult {
    if !crate::node_role::is_active() {
        return WriteResult::refused_passive();
    }
    let start = std::time::Instant::now();
    let url = format!("{}:{}", ip, port);

    // Connect with timeout
    let mut client = EthernetIpClient::new(&url);
    let connect_result =
        tokio::time::timeout(std::time::Duration::from_secs(5), client.connect()).await;

    match connect_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("EtherNet/IP connection failed: {e}")),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
        Err(_) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some("EtherNet/IP connect timeout (5s)".into()),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    }

    // Write as DINT (i32) -- most common CIP data type for integer values
    let cip_value = CipValue::Dint(value as i32);

    let write_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.write_tag(tag_name, &cip_value),
    )
    .await;

    let _ = client.disconnect().await;

    match write_result {
        Ok(Ok(())) => WriteResult {
            success: true,
            device_response: Some(format!("Wrote tag '{}' = {} (DINT)", tag_name, value)),
            error: None,
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Ok(Err(e)) => WriteResult {
            success: false,
            device_response: None,
            error: Some(format!("EtherNet/IP write failed: {e}")),
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Err(_) => WriteResult {
            success: false,
            device_response: None,
            error: Some("EtherNet/IP write timeout (5s)".into()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
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
