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

/// Whether tags of this `source_type` are read from a device by a poller,
/// as opposed to simulated or pushed through the API.
pub fn is_device_source_type(source_type: &str) -> bool {
    matches!(
        source_type,
        "modbus" | "s7" | "opcua" | "enip" | "ethernet_ip"
    )
}

/// The cache prefix of a protocol whose poller fills the cache under
/// `<prefix>_<device id>`.
fn protocol_prefix(source_type: &str) -> Option<&'static str> {
    match source_type {
        "s7" => Some("s7"),
        "opcua" => Some("opcua"),
        "enip" | "ethernet_ip" => Some("enip"),
        _ => None,
    }
}

/// Where a device tag's value lives in the cache: the cache's device id and
/// the metric name. `None` for a tag that is not read from a device.
///
/// Modbus tags name the cache device directly and key their metric as
/// `<device>.<metric>` (or just `<metric>`). The S7, OPC UA and EtherNet/IP
/// pollers fill the cache under `<protocol>_<id>` with the tag's name as the
/// metric. Their data sources carry the numeric device id; the tag name is
/// `source_config.tag` (written when the tag is created, backfilled at start),
/// else what follows the first `:` of the key `[s7_]<device name>:<tag name>`,
/// else the whole key (a tag made through the generic API).
pub fn cache_location(ds: &DataSource) -> Option<(String, String)> {
    if ds.source_type == "modbus" {
        return Some((
            ds.device_id.clone(),
            crate::api::data::metric_name(&ds.key, &ds.device_id).to_string(),
        ));
    }
    let prefix = format!("{}_", protocol_prefix(&ds.source_type)?);
    let device = if ds.device_id.starts_with(&prefix) {
        ds.device_id.clone()
    } else {
        format!("{prefix}{}", ds.device_id)
    };
    let metric = source_config(ds)
        .get("tag")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match ds.key.split_once(':') {
            Some((_, name)) => name.to_string(),
            None => ds.key.clone(),
        });
    Some((device, metric))
}

/// The cache device id and metric a tag is read under: [`cache_location`]
/// for a device tag, otherwise the tag's own device id and key, as every
/// other tag kind has always been looked up.
pub fn cache_ref(ds: &DataSource) -> (String, String) {
    cache_location(ds).unwrap_or_else(|| {
        (
            ds.device_id.clone(),
            crate::api::data::metric_name(&ds.key, &ds.device_id).to_string(),
        )
    })
}

/// The value of a device tag (see [`is_device_source_type`]) from the
/// cache. `None` when the device or the metric is not in the cache: nothing
/// was ever read, so there is nothing to show or to judge, and no stand-in
/// value is made up.
pub fn resolve(ds: &DataSource, snapshots: &[DeviceSnapshot]) -> Option<TagValue> {
    let (device, metric) = cache_location(ds)?;
    let snap = snapshots.iter().find(|s| s.device_id == device)?;
    let metric = metric.as_str();
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

/// Most decimals a reading or limit is shown with: more is float noise.
const MAX_DECIMALS: usize = 6;

/// The decimals a tag's values are shown with: its configured `decimals`,
/// kept to 0..=6 so a bad setting neither panics nor prints float noise.
fn shown_decimals(decimals: i64) -> usize {
    decimals.clamp(0, MAX_DECIMALS as i64) as usize
}

/// The decimals an alarm limit is shown with: the tag's decimals, or more
/// when the limit needs them, so a limit of 0.05 on a one-decimal tag reads
/// `0.05`, not `0.1`.
fn limit_decimals(threshold: f64, decimals: i64) -> usize {
    let shown = shown_decimals(decimals);
    if !threshold.is_finite() {
        return shown;
    }
    (shown..=MAX_DECIMALS)
        .find(|&d| format!("{threshold:.d$}").parse::<f64>() == Ok(threshold))
        .unwrap_or(MAX_DECIMALS)
}

/// `value` with `decimals` decimals, rounded the way the web client's
/// `toFixed` rounds: to the nearest, halves away from zero, on the exact
/// value of the float (Rust's `{:.N}` rounds exact halves to even, so
/// `0.125` would print `0.12` here and `0.13` in the browser). Never `-0.00`.
/// `NaN` and infinities print as `NaN`, `inf` and `-inf`.
pub fn format_number(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    // The exact decimal expansion, far past any digit that could matter:
    // a tie at digit `decimals + 1` is then a 5 followed only by zeros.
    let long = format!("{:.*}", decimals + 40, value.abs());
    let (int_part, frac_part) = long.split_once('.').unwrap_or((&long, ""));
    let mut digits: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes().take(decimals))
        .collect();
    if frac_part
        .as_bytes()
        .get(decimals)
        .is_some_and(|&d| d >= b'5')
    {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, b'1');
                break;
            }
            i -= 1;
            if digits[i] == b'9' {
                digits[i] = b'0';
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let int_len = digits.len() - decimals;
    let mut out = String::with_capacity(digits.len() + 2);
    if value < 0.0 && digits.iter().any(|&d| d != b'0') {
        out.push('-');
    }
    out.push_str(std::str::from_utf8(&digits[..int_len]).unwrap_or("0"));
    if decimals > 0 {
        out.push('.');
        out.push_str(std::str::from_utf8(&digits[int_len..]).unwrap_or(""));
    }
    out
}

/// `value` rounded to the tag's decimals, for channels that carry a number
/// (webhooks, chat). The same rounding as [`format_reading`], so the number
/// and the text never disagree.
pub fn round_to_decimals(value: f64, decimals: i64) -> f64 {
    format_number(value, shown_decimals(decimals))
        .parse()
        .unwrap_or(value)
}

/// [`round_to_decimals`] for an alarm limit: with the decimals
/// [`format_limit`] shows it with.
pub fn round_limit(threshold: f64, decimals: i64) -> f64 {
    format_number(threshold, limit_decimals(threshold, decimals))
        .parse()
        .unwrap_or(threshold)
}

fn with_unit(number: String, unit: &str) -> String {
    let unit = unit.trim();
    if unit.is_empty() {
        number
    } else if unit.starts_with('%') || unit.starts_with('\u{b0}') {
        format!("{number}{unit}")
    } else {
        format!("{number} {unit}")
    }
}

/// `value` with the tag's decimals and unit, e.g. `2.60 m` or `85.0%`.
pub fn format_reading(value: f64, decimals: i64, unit: &str) -> String {
    with_unit(format_number(value, shown_decimals(decimals)), unit)
}

/// An alarm limit with its unit, with at least the tag's decimals and as
/// many more as the limit needs (up to 6).
pub fn format_limit(threshold: f64, decimals: i64, unit: &str) -> String {
    with_unit(
        format_number(threshold, limit_decimals(threshold, decimals)),
        unit,
    )
}

/// The text of an alarm raised on `ds`: its value and the limit it crossed,
/// both with the tag's decimals and unit.
pub fn alarm_message(
    ds: &DataSource,
    severity: &str,
    alarm_type: &str,
    value: f64,
    threshold: f64,
) -> String {
    let reading = format_reading(value, ds.decimals, &ds.unit);
    let limit = format_limit(threshold, ds.decimals, &ds.unit);
    if alarm_type == "low" {
        format!("{} dropped to {reading} ({severity} low: {limit})", ds.name)
    } else {
        format!("{} reached {reading} ({severity}: {limit})", ds.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alarm_messages_use_the_tag_decimals_and_unit() {
        let mut tag = ds("wtp.filter1_headloss", 2.2, 2.6, serde_json::json!({}));
        tag.name = "Filter F-301 Headloss".into();
        tag.unit = "m".into();
        tag.decimals = 2;
        assert_eq!(
            alarm_message(&tag, "critical", "high", 2.602250099182129, 2.6),
            "Filter F-301 Headloss reached 2.60 m (critical: 2.60 m)"
        );
        tag.unit = "%".into();
        tag.decimals = 1;
        assert_eq!(
            alarm_message(&tag, "warning", "low", 19.94999, 20.0),
            "Filter F-301 Headloss dropped to 19.9% (warning low: 20.0%)"
        );
        tag.unit = String::new();
        tag.decimals = 0;
        assert_eq!(
            alarm_message(&tag, "critical", "high", 1.0, 1.0),
            "Filter F-301 Headloss reached 1 (critical: 1)"
        );
    }

    #[test]
    fn readings_round_to_sane_decimals() {
        assert_eq!(
            format_reading(2.602250099182129, 99, "mg/L"),
            "2.602250 mg/L"
        );
        assert_eq!(format_reading(2.6, -3, "\u{b0}C"), "3\u{b0}C");
        assert_eq!(round_to_decimals(2.602250099182129, 2), 2.6);
        assert_eq!(round_to_decimals(1234.5678, 1), 1234.6);
        assert_eq!(round_to_decimals(f64::MAX, 6), f64::MAX);
    }

    #[test]
    fn halves_round_away_from_zero_like_the_browser() {
        // Exact binary halves: `{:.N}` would give 2, 0.12, -2 (half to even).
        assert_eq!(format_number(2.5, 0), "3");
        assert_eq!(format_number(0.125, 2), "0.13");
        assert_eq!(format_number(-2.5, 0), "-3");
        assert_eq!(format_number(9.995, 2), "9.99"); // 9.99499999... in binary
        assert_eq!(format_number(99.95, 1), "100.0"); // 99.950000000000003
        assert_eq!(format_number(999.5, 0), "1000");
        assert_eq!(round_to_decimals(0.125, 2), 0.13);
    }

    #[test]
    fn negative_readings_and_negative_zero() {
        assert_eq!(format_reading(-12.345, 1, "\u{b0}C"), "-12.3\u{b0}C");
        assert_eq!(format_reading(-0.001, 2, "bar"), "0.00 bar");
        assert_eq!(format_reading(-0.0, 1, ""), "0.0");
        assert_eq!(format_number(-0.4, 0), "0");
        assert_eq!(round_to_decimals(-0.001, 2).to_string(), "0");
        let mut tag = ds("t", -5.0, -10.0, serde_json::json!({}));
        tag.unit = "\u{b0}C".into();
        tag.decimals = 1;
        assert_eq!(
            alarm_message(&tag, "critical", "low", -10.04, -10.0),
            "t dropped to -10.0\u{b0}C (critical low: -10.0\u{b0}C)"
        );
    }

    #[test]
    fn non_finite_readings_do_not_panic() {
        assert_eq!(format_reading(f64::NAN, 2, "m"), "NaN m");
        assert_eq!(format_reading(f64::INFINITY, 2, ""), "inf");
        assert_eq!(format_limit(f64::NEG_INFINITY, 2, "%"), "-inf%");
        assert!(round_to_decimals(f64::NAN, 2).is_nan());
        assert_eq!(round_limit(f64::INFINITY, 1), f64::INFINITY);
        let mut tag = ds("t", 1.0, 2.0, serde_json::json!({}));
        tag.decimals = 1;
        assert_eq!(
            alarm_message(&tag, "warning", "high", f64::NAN, 1.0),
            "t reached NaN (warning: 1.0)"
        );
    }

    #[test]
    fn limits_finer_than_the_tag_decimals_keep_their_digits() {
        let mut tag = ds("ph", 0.5, 0.05, serde_json::json!({}));
        tag.decimals = 0;
        assert_eq!(
            alarm_message(&tag, "warning", "high", 0.7, 0.5),
            "ph reached 1 (warning: 0.5)"
        );
        tag.decimals = 1;
        assert_eq!(
            alarm_message(&tag, "critical", "low", 0.04, 0.05),
            "ph dropped to 0.0 (critical low: 0.05)"
        );
        // A limit never prints more than six decimals, nor fewer than the tag's.
        assert_eq!(format_limit(0.1 + 0.2, 1, ""), "0.300000");
        assert_eq!(format_limit(2.0, 2, "m"), "2.00 m");
        assert_eq!(round_limit(0.05, 1), 0.05);
    }

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

    /// A tag as the S7, OPC UA and EtherNet/IP managers create it: the
    /// numeric device id and `[s7_]<device name>:<tag name>` as the key.
    fn protocol_tag(source_type: &str, key: &str, device_id: &str, warn: f64) -> DataSource {
        let mut tag = ds(key, warn, 0.0, serde_json::json!({}));
        tag.source_type = source_type.into();
        tag.device_id = device_id.into();
        tag
    }

    /// A cache snapshot as a protocol poller writes it: `<protocol>_<id>`
    /// as the device and the bare tag name as the metric.
    fn poller_snap(device_id: &str, metrics: &[(&str, f64)], online: bool) -> DeviceSnapshot {
        let mut s = snap(metrics, &[], online);
        s.device_id = device_id.into();
        s
    }

    #[test]
    fn s7_opcua_and_enip_tags_read_their_poller_cache() {
        for (source_type, key, cache_device) in [
            ("s7", "s7_Press1:Temp", "s7_3"),
            ("opcua", "Line1:Temp", "opcua_3"),
            ("enip", "Line1:Temp", "enip_3"),
        ] {
            let tag = protocol_tag(source_type, key, "3", 0.0);
            assert_eq!(
                cache_location(&tag),
                Some((cache_device.to_string(), "Temp".to_string())),
                "{source_type}"
            );
            let value = resolve(&tag, &[poller_snap(cache_device, &[("Temp", 42.0)], true)]);
            assert_eq!(
                value.map(|v| (v.value, v.quality)),
                Some((42.0, Quality::Good)),
                "{source_type}"
            );
        }
    }

    #[test]
    fn a_protocol_tag_on_an_offline_device_reads_comm_failure() {
        let tag = protocol_tag("s7", "s7_Press1:Temp", "3", 0.0);
        let value = resolve(&tag, &[poller_snap("s7_3", &[("Temp", 42.0)], false)]).unwrap();
        assert_eq!(value.quality, Quality::BadCommFailure);
    }

    #[test]
    fn a_protocol_tag_over_its_limit_is_in_alarm() {
        let tag = protocol_tag("opcua", "Line1:Temp", "3", 40.0);
        let value = alarmable(resolve(
            &tag,
            &[poller_snap("opcua_3", &[("Temp", 42.0)], true)],
        ))
        .unwrap();
        assert!(is_device_source_type(&tag.source_type));
        assert_eq!(
            Thresholds::of(&tag).condition(value),
            Some(("warning", 40.0, "high"))
        );
    }

    #[test]
    fn simulated_tags_have_no_cache_location() {
        let mut tag = ds("plc.x", 0.0, 0.0, serde_json::json!({}));
        tag.source_type = "random_walk".into();
        assert_eq!(cache_location(&tag), None);
        assert!(!is_device_source_type("random_walk"));
        assert_eq!(cache_ref(&tag), ("plc".to_string(), "x".to_string()));
    }

    #[test]
    fn the_recorded_tag_name_wins_over_the_key() {
        // A device named after its address: the key has two ':'.
        let mut tag = protocol_tag("s7", "s7_10.0.0.5:102:temp", "3", 0.0);
        assert_eq!(
            cache_location(&tag),
            Some(("s7_3".to_string(), "102:temp".to_string()))
        );
        tag.source_config = serde_json::json!({"tag": "temp"}).to_string();
        assert_eq!(
            cache_location(&tag),
            Some(("s7_3".to_string(), "temp".to_string()))
        );
    }

    #[test]
    fn a_protocol_tag_without_a_colon_uses_its_key_on_its_protocol_device() {
        let tag = protocol_tag("s7", "temp", "3", 0.0);
        assert_eq!(
            cache_location(&tag),
            Some(("s7_3".to_string(), "temp".to_string()))
        );
        assert_eq!(cache_ref(&tag), ("s7_3".to_string(), "temp".to_string()));
    }

    #[test]
    fn ethernet_ip_is_the_enip_poller() {
        let tag = protocol_tag("ethernet_ip", "Line1:Count", "4", 0.0);
        assert!(is_device_source_type("ethernet_ip"));
        assert_eq!(
            cache_location(&tag),
            Some(("enip_4".to_string(), "Count".to_string()))
        );
    }
}
