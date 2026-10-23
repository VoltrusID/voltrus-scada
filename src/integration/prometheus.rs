//! Prometheus exporter — exposes current tag values as Prometheus gauges.
//!
//! Endpoint: `GET /metrics`
//!
//! Produces standard Prometheus text exposition format.

use crate::cache::DeviceCache;
use crate::storage::Storage;

/// Render all current tag values as Prometheus text exposition format.
pub async fn render_metrics(cache: &DeviceCache, storage: &Storage) -> String {
    let mut buf = String::with_capacity(8192);

    buf.push_str("# HELP voltrus_tag_value Current value of a Voltrus tag\n");
    buf.push_str("# TYPE voltrus_tag_value gauge\n");

    let snapshots = cache.get_all().await;
    let sources = storage.list_data_sources(None).await.unwrap_or_default();
    let mut source_map = std::collections::HashMap::new();
    for src in &sources {
        let key = format!("{}:{}", src.device_id, src.key);
        source_map.insert(key, (src.name.clone(), src.unit.clone()));
    }

    for snap in &snapshots {
        for (metric, value) in &snap.metrics {
            let device = sanitize_label(&snap.device_id);
            let tag = sanitize_label(metric);
            let key = format!("{}:{}", snap.device_id, metric);
            let unit = source_map
                .get(&key)
                .map(|(_, u)| {
                    if u.is_empty() {
                        String::new()
                    } else {
                        sanitize_label(u)
                    }
                })
                .unwrap_or_default();
            let ts_ms = snap.timestamp * 1000;
            if unit.is_empty() {
                buf.push_str(&format!(
                    "voltrus_tag_value{{device=\"{}\",tag=\"{}\"}} {} {}\n",
                    device, tag, value, ts_ms
                ));
            } else {
                buf.push_str(&format!(
                    "voltrus_tag_value{{device=\"{}\",tag=\"{}\",unit=\"{}\"}} {} {}\n",
                    device, tag, unit, value, ts_ms
                ));
            }
        }
    }

    buf.push('\n');
    buf.push_str("# HELP voltrus_device_up Device online status (1=online, 0=offline)\n");
    buf.push_str("# TYPE voltrus_device_up gauge\n");
    for snap in &snapshots {
        let device = sanitize_label(&snap.device_id);
        let up = if snap.status == crate::models::DeviceStatus::Online {
            1.0
        } else {
            0.0
        };
        let ts_ms = snap.timestamp * 1000;
        buf.push_str(&format!(
            "voltrus_device_up{{device=\"{}\"}} {} {}\n",
            device, up, ts_ms
        ));
    }

    buf.push('\n');
    buf.push_str("# HELP voltrus_alarms_active Number of active alarms by severity\n");
    buf.push_str("# TYPE voltrus_alarms_active gauge\n");
    if let Ok(summary) = storage.get_alarm_counts_by_severity().await {
        for (severity, count) in &summary {
            buf.push_str(&format!(
                "voltrus_alarms_active{{severity=\"{}\"}} {}\n",
                severity, count
            ));
        }
    }

    buf
}

fn sanitize_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_sanitize_label() {
        assert_eq!(sanitize_label("plc_1"), "plc_1");
        assert_eq!(sanitize_label("val\"ue"), "val\\\"ue");
    }
}
