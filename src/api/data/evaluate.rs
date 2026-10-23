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
            // Cache metrics are stored under the bare register name.
            let metric = metric_name(&ds.key, &ds.device_id);
            let value = if ds.source_type == "modbus" {
                let metrics = cache_data
                    .iter()
                    .find(|s| s.device_id == ds.device_id && s.status == DeviceStatus::Online)
                    .map(|s| &s.metrics);

                match metrics {
                    None => f64::NAN,
                    Some(m) => {
                        if let Ok(cfg) =
                            serde_json::from_str::<serde_json::Value>(&ds.source_config)
                        {
                            if let Some(expr_str) = cfg.get("expression").and_then(|v| v.as_str()) {
                                if expr_str.is_empty() {
                                    m.get(metric).copied().unwrap_or(f64::NAN)
                                } else {
                                    match expression::evaluate(expr_str, m) {
                                        Ok(val) => val,
                                        Err(e) => {
                                            tracing::warn!(
                                                "Expression eval error for {}: {e}",
                                                ds.key
                                            );
                                            f64::NAN
                                        }
                                    }
                                }
                            } else {
                                m.get(metric).copied().unwrap_or(f64::NAN)
                            }
                        } else {
                            m.get(metric).copied().unwrap_or(f64::NAN)
                        }
                    }
                }
            } else {
                let cached = cache_data
                    .iter()
                    .find(|s| s.device_id == ds.device_id && s.status == DeviceStatus::Online)
                    .and_then(|s| s.metrics.get(metric).copied());
                cached.unwrap_or_else(|| generators::generate_value(ds, now))
            };

            // Low-alarm thresholds live in source_config (absent = disabled).
            let (low_warn, low_crit) = serde_json::from_str::<serde_json::Value>(&ds.source_config)
                .ok()
                .map(|cfg| {
                    (
                        cfg.get("low_warn").and_then(|v| v.as_f64()),
                        cfg.get("low_crit").and_then(|v| v.as_f64()),
                    )
                })
                .unwrap_or((None, None));
            let status = if value.is_nan() {
                "offline".to_string()
            } else if (ds.crit_val > 0.0 && value >= ds.crit_val)
                || low_crit.is_some_and(|lc| value <= lc)
            {
                "critical".to_string()
            } else if (ds.warn_val > 0.0 && value >= ds.warn_val)
                || low_warn.is_some_and(|lw| value <= lw)
            {
                "warning".to_string()
            } else {
                "normal".to_string()
            };

            DataSourceData {
                id: ds.id,
                key: ds.key.clone(),
                value,
                warn: ds.warn_val,
                crit: ds.crit_val,
                source_type: ds.source_type.clone(),
                status,
            }
        })
        .collect()
}
