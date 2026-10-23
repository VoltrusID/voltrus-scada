pub mod application;
pub mod datalink;
pub mod discovery;
pub mod master;
pub mod objects;
pub mod transport;

use anyhow::Result;

use crate::models::{DeviceStatus, Dnp3DeviceConfig};

pub async fn poll_dnp3_device(config: Dnp3DeviceConfig, cache: crate::cache::DeviceCache) {
    let device_id = config.id.clone();
    let mut backoff = Backoff::new();

    loop {
        match try_poll(&config, &cache).await {
            Ok(()) => backoff.reset(),
            Err(e) => {
                tracing::warn!("DNP3 device {device_id} error: {e}");
                cache.set_status(&device_id, DeviceStatus::Offline).await;
                let delay = backoff.next();
                tracing::info!("DNP3 device {device_id} retrying in {delay}ms");
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }
    }
}

async fn try_poll(config: &Dnp3DeviceConfig, cache: &crate::cache::DeviceCache) -> Result<()> {
    let mut session = master::Dnp3MasterSession::connect(
        &config.ip,
        config.port,
        config.master_address,
        config.outstation_address,
    )
    .await?;

    tracing::info!("DNP3 device {} connected", config.id);
    cache.set_status(&config.id, DeviceStatus::Online).await;

    let values = session.integrity_poll().await?;
    let mut metrics = std::collections::HashMap::new();
    for point in &config.points {
        if let Some(val) = values.get(&(point.group_num, point.variation, point.point_index)) {
            metrics.insert(point.name.clone(), val.value * point.scale);
        }
    }
    cache.update(&config.id, metrics, false).await;

    let scan_interval = config.event_scan_interval_ms.unwrap_or(1000);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(scan_interval)).await;
        let events = session.event_scan(1).await?;
        let mut metrics = std::collections::HashMap::new();
        for point in &config.points {
            if let Some(val) = events.get(&(point.group_num, point.variation, point.point_index)) {
                metrics.insert(point.name.clone(), val.value * point.scale);
            }
        }
        if !metrics.is_empty() {
            cache.update(&config.id, metrics, false).await;
        }
    }
}

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
        let attempt = self.attempt.min(30);
        let delay = 1000 * 2u64.pow(attempt);
        self.attempt = self.attempt.saturating_add(1);
        delay.min(self.max_delay_ms)
    }
}
