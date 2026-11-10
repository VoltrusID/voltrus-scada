pub mod application;
pub mod channel;
pub mod commands;
pub mod datalink;
pub mod discovery;
pub mod master;
pub mod objects;
pub mod outstation;
pub mod transport;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use tokio::time::Instant;

use crate::models::{DeviceStatus, Dnp3DeviceConfig};

pub use objects::dnp3_flags_to_good;

pub async fn poll_dnp3_device(config: Dnp3DeviceConfig, cache: crate::cache::DeviceCache) {
    let device_id = config.id.clone();
    let mut backoff = Backoff::new();

    loop {
        match try_poll(&config, &cache, &mut backoff).await {
            Ok(()) => backoff.reset(),
            Err(e) => {
                tracing::warn!("DNP3 device {device_id} error: {e}");
                cache.set_status(&device_id, DeviceStatus::Offline).await;
                let delay = backoff.next();
                tracing::info!("DNP3 device {device_id} retrying in {delay}ms");
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }
}

/// The group a configured point is read from: events report on their static
/// group, and command groups (g12, g41) on the matching output status.
fn reading_group(group: u8) -> u8 {
    match group {
        12 => 10,
        41 => 40,
        g => objects::static_group(g),
    }
}

/// Map received points onto the configured tags. Points arrive in order, so
/// the latest event for a point wins.
pub fn points_to_metrics(
    config: &Dnp3DeviceConfig,
    points: &[objects::Dnp3PointValue],
) -> HashMap<String, f64> {
    let mut latest: HashMap<(u8, u16), f64> = HashMap::new();
    for p in points {
        latest.insert((reading_group(p.group), p.index), p.value);
    }
    let mut metrics = HashMap::new();
    for point in &config.points {
        if let Some(v) = latest.get(&(reading_group(point.group_num), point.point_index)) {
            metrics.insert(point.name.clone(), v * point.scale);
        }
    }
    metrics
}

async fn publish(
    config: &Dnp3DeviceConfig,
    cache: &crate::cache::DeviceCache,
    points: &[objects::Dnp3PointValue],
) {
    let metrics = points_to_metrics(config, points);
    if !metrics.is_empty() {
        cache.update(&config.id, metrics, false).await;
    }
}

async fn try_poll(
    config: &Dnp3DeviceConfig,
    cache: &crate::cache::DeviceCache,
    backoff: &mut Backoff,
) -> Result<()> {
    let mut session = master::Dnp3MasterSession::connect(
        &config.ip,
        config.port,
        config.master_address,
        config.outstation_address,
    )
    .await?;
    let (_registration, mut commands) = commands::register(&config.id);

    let points = session.startup(config.enable_unsolicited).await?;
    tracing::info!("DNP3 device {} connected", config.id);
    cache.set_status(&config.id, DeviceStatus::Online).await;
    backoff.reset();
    publish(config, cache, &points).await;

    let scan = Duration::from_millis(config.event_scan_interval_ms.unwrap_or(1000).max(50));
    let integrity_every = Duration::from_secs(config.integrity_poll_interval_secs);
    let mut next_scan = Instant::now() + scan;
    let mut next_integrity = Instant::now() + integrity_every;

    loop {
        if let Some(cmd) = session.idle_until(next_scan, &mut commands).await? {
            let result = session.operate(cmd.control, cmd.mode).await;
            match result {
                Ok(r) => {
                    let _ = cmd.reply.send(r);
                }
                Err(e) => {
                    let _ = cmd.reply.send(crate::models::WriteResult {
                        success: false,
                        device_response: None,
                        error: Some(format!("DNP3 link failure: {e}")),
                        latency_ms: 0,
                    });
                    return Err(e);
                }
            }
        }
        let unsolicited = session.take_unsolicited();
        publish(config, cache, &unsolicited).await;

        let now = Instant::now();
        // IIN class bits mean the outstation has events waiting: fetch them
        // now rather than at the next scan.
        if now >= next_scan || session.iin.has_events() {
            let points = session.event_poll().await?;
            publish(config, cache, &points).await;
            next_scan = Instant::now() + scan;
        }
        if session.iin.device_restart()
            || (!integrity_every.is_zero() && Instant::now() >= next_integrity)
        {
            let mut points = session.housekeeping().await?;
            if points.is_empty() {
                points = session.integrity_poll().await?;
            }
            publish(config, cache, &points).await;
            next_integrity = Instant::now() + integrity_every;
        } else if session.iin.need_time() {
            session.housekeeping().await?;
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
