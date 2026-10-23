use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SimConfig {
    pub data_type: String,
    #[serde(flatten)]
    pub params: HashMap<String, serde_json::Value>,
}

pub fn generate_value(config: &SimConfig, elapsed_secs: f64) -> f64 {
    match config.data_type.as_str() {
        "sinusoidal" => sinusoidal(&config.params, elapsed_secs),
        "random_walk" => random_walk(&config.params, elapsed_secs),
        "step" => step(&config.params, elapsed_secs),
        "constant" => constant(&config.params),
        "square_wave" => square_wave(&config.params, elapsed_secs),
        _ => 0.0,
    }
}

fn get_f(params: &HashMap<String, serde_json::Value>, key: &str, default: f64) -> f64 {
    params.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn sinusoidal(params: &HashMap<String, serde_json::Value>, t: f64) -> f64 {
    let amplitude = get_f(params, "amplitude", 1.0);
    let center = get_f(params, "center", 0.0);
    let period_sec = get_f(params, "period_sec", 60.0);
    let noise_pct = get_f(params, "noise_pct", 0.0);
    let value = center + amplitude * (2.0 * std::f64::consts::PI * t / period_sec).sin();
    apply_noise(value, noise_pct)
}

fn random_walk(params: &HashMap<String, serde_json::Value>, _t: f64) -> f64 {
    let start_value = get_f(params, "start_value", 0.0);
    let step_size = get_f(params, "step_size", 0.1);
    let min = get_f(params, "min", -100.0);
    let max = get_f(params, "max", 100.0);
    let mut rng = rand::rng();
    let delta: f64 = rand::Rng::random_range(&mut rng, -step_size..step_size);
    let value = start_value + delta;
    value.clamp(min, max)
}

fn step(params: &HashMap<String, serde_json::Value>, t: f64) -> f64 {
    let low = get_f(params, "low_value", 0.0);
    let high = get_f(params, "high_value", 1.0);
    let period = get_f(params, "step_period_sec", 60.0);
    let phase = t % (period * 2.0);
    if phase < period { high } else { low }
}

fn constant(params: &HashMap<String, serde_json::Value>) -> f64 {
    get_f(params, "value", 0.0)
}

fn square_wave(params: &HashMap<String, serde_json::Value>, t: f64) -> f64 {
    let high = get_f(params, "high_value", 1.0);
    let low = get_f(params, "low_value", 0.0);
    let period = get_f(params, "period_sec", 60.0);
    let duty_pct = get_f(params, "duty_pct", 50.0);
    let phase = t % period;
    let on_time = period * (duty_pct / 100.0);
    if phase < on_time { high } else { low }
}

fn apply_noise(value: f64, noise_pct: f64) -> f64 {
    if noise_pct <= 0.0 {
        return value;
    }
    let mut rng = rand::rng();
    let noise_range = value.abs() * (noise_pct / 100.0);
    let noise: f64 = rand::Rng::random_range(&mut rng, -noise_range..noise_range);
    value + noise
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(keys: &[(&str, f64)]) -> HashMap<String, serde_json::Value> {
        keys.iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(*v)))
            .collect()
    }

    #[test]
    fn test_constant() {
        let p = params(&[("value", 42.5)]);
        let config = SimConfig {
            data_type: "constant".into(),
            params: p,
        };
        assert_eq!(generate_value(&config, 0.0), 42.5);
        assert_eq!(generate_value(&config, 100.0), 42.5);
    }

    #[test]
    fn test_sinusoidal_zero_noise() {
        let p = params(&[
            ("amplitude", 1.0),
            ("center", 0.0),
            ("period_sec", 1.0),
            ("noise_pct", 0.0),
        ]);
        let config = SimConfig {
            data_type: "sinusoidal".into(),
            params: p,
        };
        let v = generate_value(&config, 0.0);
        assert!((v - 0.0).abs() < 0.001);
        let v = generate_value(&config, 0.25);
        assert!((v - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_step() {
        let p = params(&[
            ("low_value", 0.0),
            ("high_value", 5.0),
            ("step_period_sec", 10.0),
        ]);
        let config = SimConfig {
            data_type: "step".into(),
            params: p,
        };
        assert_eq!(generate_value(&config, 5.0), 5.0);
        assert_eq!(generate_value(&config, 15.0), 0.0);
        assert_eq!(generate_value(&config, 25.0), 5.0);
    }

    #[test]
    fn test_square_wave() {
        let p = params(&[
            ("high_value", 1.0),
            ("low_value", 0.0),
            ("period_sec", 10.0),
            ("duty_pct", 50.0),
        ]);
        let config = SimConfig {
            data_type: "square_wave".into(),
            params: p,
        };
        assert_eq!(generate_value(&config, 2.0), 1.0);
        assert_eq!(generate_value(&config, 7.0), 0.0);
    }

    #[test]
    fn test_random_walk_clamped() {
        let p = params(&[
            ("start_value", 0.0),
            ("step_size", 0.1),
            ("min", -1.0),
            ("max", 1.0),
        ]);
        let config = SimConfig {
            data_type: "random_walk".into(),
            params: p,
        };
        let v = generate_value(&config, 0.0);
        assert!(v >= -1.0 && v <= 1.0);
    }
}
