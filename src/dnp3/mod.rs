pub mod application;
pub mod channel;
pub mod commands;
pub mod datalink;
pub mod discovery;
pub mod master;
pub mod objects;
pub mod outstation;
pub mod transport;

#[cfg(test)]
mod integration_tests;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use tokio::time::Instant;

use crate::models::{DeviceStatus, Dnp3DeviceConfig};
use crate::quality::Quality;

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
    points_to_readings(config, points).0
}

/// Values and quality of the configured tags among received points. A point
/// whose flags are not good keeps its value but is bad: comm failure when
/// the outstation flags COMM_LOST, otherwise plain bad.
pub fn points_to_readings(
    config: &Dnp3DeviceConfig,
    points: &[objects::Dnp3PointValue],
) -> (HashMap<String, f64>, HashMap<String, Quality>) {
    let mut latest: HashMap<(u8, u16), (f64, u8)> = HashMap::new();
    for p in points {
        latest.insert((reading_group(p.group), p.index), (p.value, p.flags));
    }
    let mut metrics = HashMap::new();
    let mut quality = HashMap::new();
    for point in &config.points {
        if let Some(&(v, point_flags)) =
            latest.get(&(reading_group(point.group_num), point.point_index))
        {
            metrics.insert(point.name.clone(), v * point.scale);
            if let Some(q) = flags_quality(point_flags) {
                quality.insert(point.name.clone(), q);
            }
        }
    }
    (metrics, quality)
}

/// The quality DNP3 point flags imply, or `None` when they are good.
fn flags_quality(point_flags: u8) -> Option<Quality> {
    if dnp3_flags_to_good(point_flags) {
        None
    } else if point_flags & objects::flags::COMM_LOST != 0 {
        Some(Quality::BadCommFailure)
    } else {
        Some(Quality::Bad)
    }
}

/// Whether a set of points is the whole picture (an integrity poll) or only
/// what changed (events, unsolicited responses).
#[derive(Clone, Copy)]
enum Publish {
    Integrity,
    Changes,
}

async fn publish(
    config: &Dnp3DeviceConfig,
    cache: &crate::cache::DeviceCache,
    points: &[objects::Dnp3PointValue],
    kind: Publish,
) {
    let (metrics, quality) = points_to_readings(config, points);
    if metrics.is_empty() {
        return;
    }
    match kind {
        Publish::Integrity => {
            cache
                .update_with_quality(&config.id, metrics, quality, false)
                .await
        }
        // Events carry only the points that changed: merge, so the others
        // keep their value.
        Publish::Changes => cache.merge_with_quality(&config.id, metrics, quality).await,
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
    publish(config, cache, &points, Publish::Integrity).await;

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
        publish(config, cache, &unsolicited, Publish::Changes).await;

        let now = Instant::now();
        // IIN class bits mean the outstation has events waiting: fetch them
        // now rather than at the next scan.
        if now >= next_scan || session.iin.has_events() {
            let points = session.event_poll().await?;
            publish(config, cache, &points, Publish::Changes).await;
            next_scan = Instant::now() + scan;
        }
        if session.iin.device_restart()
            || (!integrity_every.is_zero() && Instant::now() >= next_integrity)
        {
            let mut points = session.housekeeping().await?;
            if points.is_empty() {
                points = session.integrity_poll().await?;
            }
            publish(config, cache, &points, Publish::Integrity).await;
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

#[cfg(test)]
mod quality_tests {
    use super::objects::{Dnp3PointValue, flags};
    use super::*;
    use crate::models::Dnp3PointConfig;

    fn config() -> Dnp3DeviceConfig {
        let points = ["good", "offline", "comm_lost", "restart"]
            .iter()
            .zip(0u16..)
            .map(|(name, i)| Dnp3PointConfig {
                group_num: 30,
                variation: 1,
                point_index: i,
                name: (*name).into(),
                scale: 1.0,
            })
            .collect();
        Dnp3DeviceConfig {
            id: "dnp3_q".into(),
            ip: "127.0.0.1".into(),
            port: 20000,
            master_address: 1,
            outstation_address: 10,
            event_scan_interval_ms: None,
            enable_unsolicited: false,
            integrity_poll_interval_secs: 0,
            points,
        }
    }

    fn analog(index: u16, value: f64, point_flags: u8) -> Dnp3PointValue {
        Dnp3PointValue {
            group: 30,
            variation: 1,
            index,
            value,
            flags: point_flags,
            time_ms: None,
        }
    }

    #[test]
    fn point_flags_set_quality() {
        let (metrics, quality) = points_to_readings(
            &config(),
            &[
                analog(0, 1.0, flags::ONLINE),
                analog(1, 2.0, 0),
                analog(2, 3.0, flags::ONLINE | flags::COMM_LOST),
                analog(3, 4.0, flags::ONLINE | flags::RESTART),
            ],
        );
        assert_eq!(metrics["good"], 1.0);
        assert_eq!(metrics["comm_lost"], 3.0);
        assert!(!quality.contains_key("good"));
        assert_eq!(quality["offline"], Quality::Bad);
        assert_eq!(quality["comm_lost"], Quality::BadCommFailure);
        assert_eq!(quality["restart"], Quality::Bad);
    }

    #[tokio::test]
    async fn events_merge_and_keep_bad_flags_in_the_cache() {
        let cfg = config();
        let cache = crate::cache::DeviceCache::new();
        publish(
            &cfg,
            &cache,
            &[analog(0, 1.0, flags::ONLINE), analog(1, 2.0, flags::ONLINE)],
            Publish::Integrity,
        )
        .await;
        publish(
            &cfg,
            &cache,
            &[analog(1, 5.0, flags::ONLINE | flags::COMM_LOST)],
            Publish::Changes,
        )
        .await;
        let good = cache.get_reading("dnp3_q", "good").await.unwrap();
        let offline = cache.get_reading("dnp3_q", "offline").await.unwrap();
        assert_eq!(good.value, 1.0);
        assert_eq!(good.quality, Quality::Good);
        assert_eq!(offline.value, 5.0);
        assert_eq!(offline.quality, Quality::BadCommFailure);
    }
}
