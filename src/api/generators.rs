use crate::models::DataSource;

/// Generate a pseudo-random value from a seed. Equivalent to the Hono worker's
/// `pseudoRandom` function: fract(sin(x * 12.9898 + 78.233) * 43758.5453)
pub fn pseudo_random(seed: f64) -> f64 {
    let x = (seed * 12.9898 + 78.233).sin() * 43758.5453;
    x - x.floor()
}

/// Generate a data source value based on its `source_type` and configuration.
/// `now` is a time factor (seconds since epoch as f64).
pub fn generate_value(data_source: &DataSource, now: f64) -> f64 {
    match data_source.source_type.as_str() {
        "constant" => data_source.base_val,
        "none" | "api" => data_source.base_val,
        "noise" => {
            let noise = pseudo_random(now + data_source.id as f64) * 2.0 - 1.0;
            let range = data_source.max_val - data_source.min_val;
            (data_source.base_val + noise * range * 0.05)
                .clamp(data_source.min_val, data_source.max_val)
        }
        "sine" => {
            let config: serde_json::Value =
                serde_json::from_str(&data_source.source_config).unwrap_or(serde_json::json!({}));
            let period = config["period"].as_f64().unwrap_or(60.0);
            let amplitude = config["amplitude"]
                .as_f64()
                .unwrap_or(data_source.max_val - data_source.min_val);
            let phase = config["phase"].as_f64().unwrap_or(0.0);
            let range = data_source.max_val - data_source.min_val;
            let amp = amplitude.min(range / 2.0);
            let val = data_source.base_val
                + ((now / period) * std::f64::consts::PI * 2.0 + phase).sin() * amp * 0.5;
            val.clamp(data_source.min_val, data_source.max_val)
        }
        "step" => {
            // Occasional jumps using pseudo-random trigger
            let step_seed = (now / 30.0).floor();
            let should_jump = pseudo_random(step_seed + data_source.id as f64) > 0.7;
            if should_jump {
                let jump_target = pseudo_random(step_seed + data_source.id as f64 * 3.7);
                let range = data_source.max_val - data_source.min_val;
                (data_source.min_val + jump_target * range)
                    .clamp(data_source.min_val, data_source.max_val)
            } else {
                data_source.base_val
            }
        }
        "sawtooth" => {
            let config: serde_json::Value =
                serde_json::from_str(&data_source.source_config).unwrap_or(serde_json::json!({}));
            let period = config["period"].as_f64().unwrap_or(120.0);
            let range = data_source.max_val - data_source.min_val;
            let phase = (now % period) / period;
            (data_source.min_val + phase * range).clamp(data_source.min_val, data_source.max_val)
        }
        _ => random_walk(data_source, now), // default: random_walk
    }
}

fn random_walk(data_source: &DataSource, now: f64) -> f64 {
    let range = data_source.max_val - data_source.min_val;
    let noise = (now * 0.5 + data_source.id as f64 * 1.7).sin() * 0.3
        + (now * 1.3 + data_source.id as f64 * 2.1).sin() * 0.15;
    (data_source.base_val + noise * range * 0.08).clamp(data_source.min_val, data_source.max_val)
}

/// Aggregate values within a time bucket by sampling at 1s intervals.
/// Returns (avg, min, max) — all rounded to 2 decimals.
pub fn aggregate_bucket(
    data_source: &DataSource,
    bucket_start: f64,
    sub_samples: usize,
) -> (f64, f64, f64) {
    let n = sub_samples.max(1);
    let mut sum = 0.0;
    let mut mn = f64::MAX;
    let mut mx = f64::MIN;
    for j in 0..n {
        let t = bucket_start + j as f64;
        let v = generate_value(data_source, t);
        sum += v;
        if v < mn {
            mn = v;
        }
        if v < mx {
        } else {
            mx = v;
        }
    }
    let avg = (sum / n as f64 * 100.0).round() / 100.0;
    let min_v = (mn * 100.0).round() / 100.0;
    let max_v = (mx * 100.0).round() / 100.0;
    (avg, min_v, max_v)
}

/// Parse a range string like "30m", "1h", "6h", "24h" or "startMs-endMs" into (start_timestamp, end_timestamp) as seconds since epoch.
pub fn parse_range(range: &str) -> (i64, i64) {
    // Custom range: "startMs-endMs" (milliseconds)
    if let Some((s, e)) = range.split_once('-')
        && let (Ok(start_ms), Ok(end_ms)) = (s.parse::<i64>(), e.parse::<i64>())
        && start_ms > 0
        && end_ms > start_ms
    {
        return (start_ms / 1000, end_ms / 1000);
    }
    let now = crate::timeutil::now_secs();
    let secs = match range {
        "30m" => 30 * 60,
        "1h" => 60 * 60,
        "6h" => 6 * 60 * 60,
        "24h" => 24 * 60 * 60,
        "7d" => 7 * 24 * 60 * 60,
        "30d" => 30 * 24 * 60 * 60,
        _ => 30 * 60,
    };
    (now - secs, now)
}

/// Get the interval in milliseconds for a given range string and detail mode.
/// `detail`: "full" = 1s interval (all points), anything else = auto-aggregated.
pub fn interval_for_range(range: &str, detail: Option<&str>) -> u64 {
    let is_full = detail == Some("full");

    match range {
        "30m" | "1h" => 10_000, // short ranges already fine-grained
        "6h" if is_full => 1_000,
        "6h" => 120_000,
        "24h" if is_full => 1_000,
        "24h" => 480_000,
        "7d" if is_full => 1_000,
        "7d" => 3_360_000,
        "30d" if is_full => 1_000,
        "30d" => 14_400_000,
        _ => {
            if is_full {
                1_000 // full detail: 1 point per second
            } else {
                let (start, end) = parse_range(range);
                let duration_ms = (end - start).max(1) as u64 * 1000;
                (duration_ms / 300).clamp(1_000, 3_600_000)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data_source(
        id: i64,
        source_type: &str,
        base_val: f64,
        min_val: f64,
        max_val: f64,
    ) -> DataSource {
        DataSource {
            id,
            key: "test".to_string(),
            name: "Test".to_string(),
            unit: "V".to_string(),
            color: "#fff".to_string(),
            min_val,
            max_val,
            warn_val: 0.0,
            crit_val: 0.0,
            base_val,
            sort_order: 0,
            decimals: 1,
            source_type: source_type.to_string(),
            source_config: "{}".to_string(),
            device_id: "test".to_string(),
            equipment_id: None,
            chart_type: "sparkline".to_string(),
            path: None,
            on_delay_secs: 0,
            off_delay_secs: 0,
        }
    }

    #[test]
    fn test_constant() {
        let s = make_data_source(1, "constant", 42.0, 0.0, 100.0);
        assert_eq!(generate_value(&s, 100.0), 42.0);
        assert_eq!(generate_value(&s, 200.0), 42.0);
    }

    #[test]
    fn test_noise_within_range() {
        let s = make_data_source(1, "noise", 50.0, 0.0, 100.0);
        for t in 0..100 {
            let v = generate_value(&s, t as f64);
            assert!(v >= 0.0 && v <= 100.0, "value {v} out of range at t={t}");
        }
    }

    #[test]
    fn test_sine_within_range() {
        let s = make_data_source(1, "sine", 50.0, 0.0, 100.0);
        for t in 0..100 {
            let v = generate_value(&s, t as f64);
            assert!(v >= 0.0 && v <= 100.0, "value {v} out of range at t={t}");
        }
    }

    #[test]
    fn test_random_walk_within_range() {
        let s = make_data_source(1, "random_walk", 50.0, 0.0, 100.0);
        for t in 0..100 {
            let v = generate_value(&s, t as f64);
            assert!(v >= 0.0 && v <= 100.0, "value {v} out of range at t={t}");
        }
    }

    #[test]
    fn test_sawtooth_within_range() {
        let s = make_data_source(1, "sawtooth", 50.0, 0.0, 100.0);
        for t in 0..100 {
            let v = generate_value(&s, t as f64);
            assert!(v >= 0.0 && v <= 100.0, "value {v} out of range at t={t}");
        }
    }

    #[test]
    fn test_step_within_range() {
        let s = make_data_source(1, "step", 50.0, 0.0, 100.0);
        for t in 0..100 {
            let v = generate_value(&s, t as f64);
            assert!(v >= 0.0 && v <= 100.0, "value {v} out of range at t={t}");
        }
    }

    #[test]
    fn test_pseudo_random() {
        let v = pseudo_random(1.0);
        assert!(v >= 0.0 && v < 1.0);
        // Deterministic
        assert_eq!(v, pseudo_random(1.0));
    }

    #[test]
    fn test_parse_range() {
        let (start, end) = parse_range("30m");
        assert_eq!(end - start, 30 * 60);

        let (start, end) = parse_range("1h");
        assert_eq!(end - start, 3600);

        let (start, end) = parse_range("6h");
        assert_eq!(end - start, 6 * 3600);

        let (start, end) = parse_range("24h");
        assert_eq!(end - start, 24 * 3600);
    }

    #[test]
    fn test_interval_for_range() {
        assert_eq!(interval_for_range("30m", None), 10_000);
        assert_eq!(interval_for_range("1h", None), 10_000);
        assert_eq!(interval_for_range("6h", None), 120_000);
        assert_eq!(interval_for_range("6h", Some("full")), 1_000);
        // Unknown ranges auto-compute: defaults to 30m = 1800s, interval = 1800000/300 = 6000
        assert_eq!(interval_for_range("unknown", None), 6_000);
        // Unknown range with full detail: 1s per point
        assert_eq!(interval_for_range("unknown", Some("full")), 1_000);
    }

    #[test]
    fn test_custom_range_parsing() {
        // Custom range: "startMs-endMs"
        let (start, end) = parse_range("1714300800000-1714311600000");
        assert_eq!(start, 1714300800);
        assert_eq!(end, 1714311600);
        assert_eq!(end - start, 10800); // 3 hours

        // Custom range interval: 3h = 10800000ms / 300 = 36000
        assert_eq!(
            interval_for_range("1714300800000-1714311600000", None),
            36_000
        );
        // Custom range with full detail: 1s per point = 10800 points for 3h
        assert_eq!(
            interval_for_range("1714300800000-1714311600000", Some("full")),
            1_000
        );
    }
}
