//! The value of a device tag and the alarm limits it is checked against,
//! shared by alarm evaluation and the live data API so both agree.

use std::collections::HashMap;

use crate::models::{DataSource, DeviceSnapshot, DeviceStatus};
use crate::quality::Quality;

/// A tag's current value, its quality and when it was read (Unix ms).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TagValue {
    pub value: f64,
    pub quality: Quality,
    pub ts_ms: i64,
}

fn source_config(ds: &DataSource) -> serde_json::Value {
    serde_json::from_str(&ds.source_config).unwrap_or(serde_json::Value::Null)
}

/// The expression a tag is computed from, if it has one.
fn expression_of(cfg: &serde_json::Value) -> Option<&str> {
    cfg.get("expression")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// The value of a device tag (`source_type` "modbus", which covers every
/// polled protocol) from the cache. `None` when the device or the metric is
/// not in the cache: nothing was ever read, so there is nothing to show or
/// to judge, and no stand-in value is made up.
pub fn resolve(ds: &DataSource, snapshots: &[DeviceSnapshot]) -> Option<TagValue> {
    let snap = snapshots.iter().find(|s| s.device_id == ds.device_id)?;
    if snap.status != DeviceStatus::Online {
        return Some(TagValue {
            value: f64::NAN,
            quality: Quality::BadCommFailure,
            ts_ms: snap.ts_ms,
        });
    }
    let cfg = source_config(ds);
    if let Some(expr) = expression_of(&cfg) {
        return Some(evaluate_expression(expr, snap));
    }
    let metric = crate::api::data::metric_name(&ds.key, &ds.device_id);
    let value = *snap.metrics.get(metric)?;
    Some(TagValue {
        value,
        quality: snap.quality_of(metric),
        ts_ms: snap.metric_ts(metric),
    })
}

/// Evaluate a tag expression over a device's metrics. The expression gets
/// the quality of its inputs: if it cannot be computed from the good values
/// alone, it leans on a value that is not good.
fn evaluate_expression(expr: &str, snap: &DeviceSnapshot) -> TagValue {
    let ts_ms = snap.ts_ms;
    let value = match crate::expression::evaluate(expr, &snap.metrics) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Expression eval error for {}: {e}", snap.device_id);
            return TagValue {
                value: f64::NAN,
                quality: Quality::BadConfigError,
                ts_ms,
            };
        }
    };
    if !value.is_finite() {
        return TagValue {
            value,
            quality: Quality::Bad,
            ts_ms,
        };
    }
    if snap.quality.is_empty() {
        return TagValue {
            value,
            quality: Quality::Good,
            ts_ms,
        };
    }
    let good: HashMap<String, f64> = snap
        .metrics
        .iter()
        .filter(|(name, _)| snap.quality_of(name).is_good())
        .map(|(name, v)| (name.clone(), *v))
        .collect();
    let quality = if crate::expression::evaluate(expr, &good).is_ok() {
        Quality::Good
    } else {
        // We do not know which input it used; the worst of the device's
        // non-good values is the honest upper bound.
        snap.quality
            .values()
            .fold(Quality::Good, |worst, q| worst.worst(*q))
    };
    TagValue {
        value,
        quality,
        ts_ms,
    }
}

/// The alarm limits of a tag. `None` is a limit that is not set; any number,
/// 0 and negative included, is a limit.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Thresholds {
    pub high_warn: Option<f64>,
    pub high_crit: Option<f64>,
    pub low_warn: Option<f64>,
    pub low_crit: Option<f64>,
    pub deadband: f64,
}

/// A high limit from its column and the optional `<name>_enabled` flag in
/// source_config. The columns are NOT NULL and have always used 0 for "not
/// set", so a tag without the flag keeps that reading (only a limit above 0
/// counts); a tag saved with the flag uses the column as is, so 0 and
/// negative limits work. No stored row changes meaning.
fn high_limit(column: f64, flag: Option<&serde_json::Value>) -> Option<f64> {
    match flag.and_then(|v| v.as_bool()) {
        Some(true) => Some(column),
        Some(false) => None,
        None => (column > 0.0).then_some(column),
    }
}

impl Thresholds {
    pub fn of(ds: &DataSource) -> Self {
        let cfg = source_config(ds);
        Self {
            high_warn: high_limit(ds.warn_val, cfg.get("warn_enabled")),
            high_crit: high_limit(ds.crit_val, cfg.get("crit_enabled")),
            low_warn: cfg.get("low_warn").and_then(|v| v.as_f64()),
            low_crit: cfg.get("low_crit").and_then(|v| v.as_f64()),
            deadband: cfg.get("deadband").and_then(|v| v.as_f64()).unwrap_or(0.0),
        }
    }

    /// The alarm condition `value` is in, if any: (severity, limit, "high" or
    /// "low"). High limits fire at value >= limit, low ones at value <= limit;
    /// critical wins over warning.
    pub fn condition(&self, value: f64) -> Option<(&'static str, f64, &'static str)> {
        if let Some(l) = self.high_crit.filter(|l| value >= *l) {
            Some(("critical", l, "high"))
        } else if let Some(l) = self.high_warn.filter(|l| value >= *l) {
            Some(("warning", l, "high"))
        } else if let Some(l) = self.low_crit.filter(|l| value <= *l) {
            Some(("critical", l, "low"))
        } else {
            self.low_warn
                .filter(|l| value <= *l)
                .map(|l| ("warning", l, "low"))
        }
    }
}

/// The value alarm evaluation may act on: only a good one. A bad or stale
/// value says nothing about the process, so the tag's alarms neither raise
/// nor clear on it.
pub fn alarmable(tag: Option<TagValue>) -> Option<f64> {
    tag.filter(|t| t.quality.is_good() && t.value.is_finite())
        .map(|t| t.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ds(key: &str, warn: f64, crit: f64, cfg: serde_json::Value) -> DataSource {
        serde_json::from_value(serde_json::json!({
            "id": 1, "key": key, "name": key, "unit": "", "color": "",
            "min_val": 0.0, "max_val": 100.0, "warn_val": warn, "crit_val": crit,
            "base_val": 50.0, "sort_order": 0, "decimals": 1,
            "source_type": "modbus", "source_config": cfg.to_string(),
            "device_id": "plc", "equipment_id": null, "chart_type": "sparkline",
            "path": null
        }))
        .unwrap()
    }

    fn snap(metrics: &[(&str, f64)], quality: &[(&str, Quality)], online: bool) -> DeviceSnapshot {
        DeviceSnapshot {
            device_id: "plc".into(),
            status: if online {
                DeviceStatus::Online
            } else {
                DeviceStatus::Offline
            },
            timestamp: 1,
            metrics: metrics.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            simulated: false,
            protocol: None,
            ts_ms: 1000,
            quality: quality.iter().map(|(k, q)| (k.to_string(), *q)).collect(),
            metric_ts_ms: HashMap::new(),
        }
    }

    #[test]
    fn legacy_zero_high_limits_stay_disabled() {
        let t = Thresholds::of(&ds("t", 0.0, 0.0, serde_json::json!({})));
        assert_eq!(t.high_warn, None);
        assert_eq!(t.high_crit, None);
        assert_eq!(t.condition(1000.0), None);
        let t = Thresholds::of(&ds("t", 80.0, 90.0, serde_json::json!({})));
        assert_eq!(t.condition(85.0), Some(("warning", 80.0, "high")));
        assert_eq!(t.condition(95.0), Some(("critical", 90.0, "high")));
    }

    #[test]
    fn enabled_zero_and_negative_high_limits_fire() {
        let cfg = serde_json::json!({"warn_enabled": true, "crit_enabled": true});
        let t = Thresholds::of(&ds("t", -10.0, 0.0, cfg));
        assert_eq!(t.condition(-20.0), None);
        assert_eq!(t.condition(-5.0), Some(("warning", -10.0, "high")));
        assert_eq!(t.condition(0.0), Some(("critical", 0.0, "high")));
    }

    #[test]
    fn a_disabled_flag_switches_a_positive_limit_off() {
        let cfg = serde_json::json!({"warn_enabled": false});
        let t = Thresholds::of(&ds("t", 80.0, 90.0, cfg));
        assert_eq!(t.condition(85.0), None);
        assert_eq!(t.condition(95.0), Some(("critical", 90.0, "high")));
    }

    #[test]
    fn zero_and_negative_low_limits_fire() {
        let cfg = serde_json::json!({"low_warn": 0.0, "low_crit": -5.0});
        let t = Thresholds::of(&ds("t", 0.0, 0.0, cfg));
        assert_eq!(t.condition(1.0), None);
        assert_eq!(t.condition(0.0), Some(("warning", 0.0, "low")));
        assert_eq!(t.condition(-5.0), Some(("critical", -5.0, "low")));
    }

    #[test]
    fn a_missing_metric_is_no_value_not_base_val() {
        let tag = ds("plc.temp", 80.0, 90.0, serde_json::json!({}));
        assert_eq!(resolve(&tag, &[snap(&[("other", 1.0)], &[], true)]), None);
        assert_eq!(resolve(&tag, &[]), None);
        assert_eq!(alarmable(resolve(&tag, &[])), None);
    }

    #[test]
    fn alarms_skip_values_that_are_not_good() {
        let tag = ds("plc.temp", 80.0, 90.0, serde_json::json!({}));
        let fresh = resolve(&tag, &[snap(&[("temp", 95.0)], &[], true)]);
        assert_eq!(alarmable(fresh), Some(95.0));

        for q in [
            Quality::BadDeviceError,
            Quality::UncertainStale,
            Quality::Uncertain,
        ] {
            let v = resolve(&tag, &[snap(&[("temp", 95.0)], &[("temp", q)], true)]);
            assert_eq!(v.unwrap().quality, q);
            assert_eq!(alarmable(v), None, "{q:?}");
        }

        let offline = resolve(&tag, &[snap(&[("temp", 95.0)], &[], false)]).unwrap();
        assert_eq!(offline.quality, Quality::BadCommFailure);
        assert!(offline.value.is_nan());
        assert_eq!(alarmable(Some(offline)), None);
    }

    #[test]
    fn an_expression_takes_the_quality_of_its_inputs() {
        let cfg = serde_json::json!({"expression": "{a} + 1"});
        let tag = ds("plc.sum", 0.0, 0.0, cfg);
        let good = resolve(
            &tag,
            &[snap(
                &[("a", 1.0), ("b", 2.0)],
                &[("b", Quality::Bad)],
                true,
            )],
        )
        .unwrap();
        assert_eq!((good.value, good.quality), (2.0, Quality::Good));

        let stale = resolve(
            &tag,
            &[snap(&[("a", 1.0)], &[("a", Quality::UncertainStale)], true)],
        )
        .unwrap();
        assert_eq!(stale.quality, Quality::UncertainStale);

        let broken = resolve(
            &ds(
                "plc.x",
                0.0,
                0.0,
                serde_json::json!({"expression": "{nope}"}),
            ),
            &[snap(&[("a", 1.0)], &[], true)],
        )
        .unwrap();
        assert_eq!(broken.quality, Quality::BadConfigError);
    }
}
