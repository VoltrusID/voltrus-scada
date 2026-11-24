use super::*;

/// Evaluate all data sources against current cache, producing DataSourceData[].
pub(crate) fn evaluate_data_sources(
    data_sources: &[DataSource],
    cache_data: &[DeviceSnapshot],
    now: f64,
) -> Vec<DataSourceData> {
    data_sources
        .iter()
        .map(|ds| {
            let tag = if ds.source_type == "modbus" {
                // Never read (device or metric not in the cache) reads as a
                // communication failure: there is no value to show.
                crate::alarm_eval::resolve(ds, cache_data).unwrap_or(crate::alarm_eval::TagValue {
                    value: f64::NAN,
                    quality: crate::quality::Quality::BadCommFailure,
                    ts_ms: 0,
                })
            } else {
                // Synthetic tags: a value pushed for the device wins over the generator.
                let metric = metric_name(&ds.key, &ds.device_id);
                let cached = cache_data
                    .iter()
                    .find(|s| s.device_id == ds.device_id && s.status == DeviceStatus::Online)
                    .and_then(|s| {
                        let value = *s.metrics.get(metric)?;
                        Some(crate::alarm_eval::TagValue {
                            value,
                            quality: s.quality_of(metric),
                            ts_ms: s.metric_ts(metric),
                        })
                    });
                cached.unwrap_or_else(|| crate::alarm_eval::TagValue {
                    value: generators::generate_value(ds, now),
                    quality: crate::quality::Quality::Good,
                    ts_ms: 0,
                })
            };
            let value = tag.value;

            let status = if value.is_nan() {
                "offline"
            } else {
                match crate::alarm_eval::Thresholds::of(ds).condition(value) {
                    Some(("critical", _, _)) => "critical",
                    Some(_) => "warning",
                    None => "normal",
                }
            }
            .to_string();

            DataSourceData {
                id: ds.id,
                key: ds.key.clone(),
                value,
                warn: ds.warn_val,
                crit: ds.crit_val,
                source_type: ds.source_type.clone(),
                status,
                quality: tag.quality,
                ts_ms: (tag.ts_ms > 0).then_some(tag.ts_ms),
            }
        })
        .collect()
}
