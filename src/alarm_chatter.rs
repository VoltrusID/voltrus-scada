//! Alarm chatter: how often a tag's alarm is raised per hour, and the on-delay
//! (and deadband) that would keep a noisy analog signal from chattering.
//!
//! A value hovering at a limit with noise larger than the deadband raises and
//! clears its alarm again and again (Rapid SCADA in our lab: 5 times in 9 s on
//! ±0.3 NTU noise with a 0.5 deadband and no on-delay). The suggestion is a
//! starting value for the engineer, shown and pre-filled in the tag editor; the
//! server never applies it on its own.

use serde::Serialize;

/// Raised alarms per hour from which a tag counts as chattering.
pub const CHATTER_PER_HOUR: i64 = 5;

/// On-delay suggested for any analog tag without enough history to judge.
pub const DEFAULT_ANALOG_ON_DELAY_SECS: i64 = 2;
/// On-delay suggested for a noisy analog tag (noise above a third of the deadband).
pub const NOISY_ON_DELAY_SECS: i64 = 5;
/// On-delay suggested for a tag whose alarm already chatters.
pub const CHATTERING_ON_DELAY_SECS: i64 = 10;

/// Fewest samples the noise estimate needs.
const MIN_SAMPLES: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct AlarmSuggestion {
    /// Suggested `on_delay_secs` (0: none).
    pub on_delay_secs: i64,
    /// Suggested alarm deadband (three times the noise), when noise was measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadband: Option<f64>,
    /// Why, in one sentence.
    pub reason: String,
    pub analog: bool,
    /// Estimated noise (standard deviation) of the value over the last 30 minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise: Option<f64>,
    /// Samples the noise estimate used.
    pub samples: usize,
    /// Alarms raised for the tag in the last hour.
    pub alarms_last_hour: i64,
    /// Most alarms raised in one clock hour over the last 24 hours.
    pub max_alarms_per_hour: i64,
    /// `max_alarms_per_hour` reached [`CHATTER_PER_HOUR`].
    pub chattering: bool,
}

/// Noise of a series: the standard deviation of successive differences over
/// √2, which ignores slow trends that the plain standard deviation would count.
pub fn noise_sigma(values: &[f64]) -> Option<f64> {
    let v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.len() < MIN_SAMPLES {
        return None;
    }
    let diffs: Vec<f64> = v.windows(2).map(|w| w[1] - w[0]).collect();
    let n = diffs.len() as f64;
    let mean = diffs.iter().sum::<f64>() / n;
    let var = diffs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n;
    Some(var.sqrt() / std::f64::consts::SQRT_2)
}

/// Most entries in one clock hour, from alarm trigger times (unix seconds).
pub fn max_per_hour(triggered_at: &[i64]) -> i64 {
    let mut counts = std::collections::HashMap::<i64, i64>::new();
    for t in triggered_at {
        *counts.entry(t.div_euclid(3600)).or_default() += 1;
    }
    counts.into_values().max().unwrap_or(0)
}

/// Inputs of [`suggest`].
pub struct TagFacts<'a> {
    pub min_val: f64,
    pub max_val: f64,
    pub decimals: i64,
    /// Current alarm deadband (`source_config.deadband`, 0 when unset).
    pub deadband: f64,
    /// Recent values, oldest first.
    pub values: &'a [f64],
    pub alarms_last_hour: i64,
    pub max_alarms_per_hour: i64,
}

/// A tag is digital when its range is 0..1 with no decimals, or when every
/// recent value is 0 or 1.
fn is_analog(f: &TagFacts<'_>) -> bool {
    let range_digital = f.decimals == 0 && f.min_val == 0.0 && f.max_val <= 1.0;
    let values_digital =
        f.values.len() >= MIN_SAMPLES && f.values.iter().all(|v| *v == 0.0 || *v == 1.0);
    !(range_digital || values_digital)
}

pub fn suggest(f: &TagFacts<'_>) -> AlarmSuggestion {
    let analog = is_analog(f);
    let noise = if analog { noise_sigma(f.values) } else { None };
    let chattering = f.max_alarms_per_hour >= CHATTER_PER_HOUR;
    let fmt = |x: f64| {
        let digits = (f.decimals.clamp(0, 6) + 1) as usize;
        format!("{x:.digits$}")
    };
    let (on_delay_secs, reason) = if !analog {
        (
            0,
            "Digital tag: a state change is the event, no delay suggested.".to_string(),
        )
    } else if chattering {
        (
            CHATTERING_ON_DELAY_SECS,
            format!(
                "The alarm was raised {} times in one hour (chattering from {CHATTER_PER_HOUR}/h).",
                f.max_alarms_per_hour
            ),
        )
    } else if let Some(sigma) = noise.filter(|s| *s > 0.0 && f.deadband < 3.0 * s) {
        (
            NOISY_ON_DELAY_SECS,
            format!(
                "The value moves by about {} between samples, more than a third of the deadband ({}).",
                fmt(sigma),
                fmt(f.deadband)
            ),
        )
    } else {
        (
            DEFAULT_ANALOG_ON_DELAY_SECS,
            match noise {
                Some(_) => {
                    "Analog tag with little noise: a short delay filters single spikes.".to_string()
                }
                None => {
                    "Analog tag without enough recent history: default delay for analog limits."
                        .to_string()
                }
            },
        )
    };
    AlarmSuggestion {
        on_delay_secs,
        deadband: noise.filter(|s| *s > 0.0).map(|s| {
            // Three sigma, rounded to the tag's display precision (at least one step).
            let step = 10f64.powi(-(f.decimals.clamp(0, 6) as i32));
            ((3.0 * s) / step).ceil() * step
        }),
        reason,
        analog,
        noise,
        samples: f.values.len(),
        alarms_last_hour: f.alarms_last_hour,
        max_alarms_per_hour: f.max_alarms_per_hour,
        chattering,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(values: &[f64], deadband: f64, max_per_hour: i64) -> TagFacts<'_> {
        TagFacts {
            min_val: 0.0,
            max_val: 50.0,
            decimals: 1,
            deadband,
            values,
            alarms_last_hour: 0,
            max_alarms_per_hour: max_per_hour,
        }
    }

    /// ±0.3 around 18 NTU, as the turbidity in the Rapid SCADA lab.
    fn noisy() -> Vec<f64> {
        (0..60)
            .map(|i| 18.0 + if i % 2 == 0 { 0.3 } else { -0.3 })
            .collect()
    }

    #[test]
    fn noise_ignores_a_slow_ramp() {
        let ramp: Vec<f64> = (0..50).map(|i| i as f64).collect();
        assert!(noise_sigma(&ramp).unwrap() < 1e-9);
        let n = noise_sigma(&noisy()).unwrap();
        assert!(
            (n - 0.3 * 2.0 / std::f64::consts::SQRT_2).abs() < 0.01,
            "{n}"
        );
        assert_eq!(noise_sigma(&[1.0, 2.0]), None);
    }

    #[test]
    fn noisy_analog_gets_five_seconds_and_a_three_sigma_deadband() {
        let v = noisy();
        let s = suggest(&facts(&v, 0.5, 0));
        assert_eq!(s.on_delay_secs, NOISY_ON_DELAY_SECS);
        assert!(s.analog && !s.chattering);
        assert!(s.reason.contains("deadband (0.50)"), "{}", s.reason);
        // sigma ≈ 0.42 → 3 sigma ≈ 1.27 → rounded up to 1.3 at one decimal.
        assert_eq!(s.deadband, Some(1.3));
    }

    #[test]
    fn chattering_wins_and_quiet_or_unknown_analogs_get_the_default() {
        let v = noisy();
        let s = suggest(&facts(&v, 0.5, 7));
        assert_eq!(s.on_delay_secs, CHATTERING_ON_DELAY_SECS);
        assert!(s.chattering && s.reason.contains("7 times"));

        let flat = vec![20.0; 30];
        assert_eq!(
            suggest(&facts(&flat, 0.0, 0)).on_delay_secs,
            DEFAULT_ANALOG_ON_DELAY_SECS
        );
        assert_eq!(
            suggest(&facts(&[], 0.0, 0)).on_delay_secs,
            DEFAULT_ANALOG_ON_DELAY_SECS
        );
    }

    #[test]
    fn digital_tags_get_no_delay() {
        let states: Vec<f64> = (0..20).map(|i| (i % 2) as f64).collect();
        let s = suggest(&facts(&states, 0.0, 9));
        assert_eq!(s.on_delay_secs, 0);
        assert!(!s.analog);
        let range_digital = TagFacts {
            min_val: 0.0,
            max_val: 1.0,
            decimals: 0,
            ..facts(&[], 0.0, 0)
        };
        assert!(!suggest(&range_digital).analog);
    }

    #[test]
    fn max_per_hour_counts_clock_hours() {
        assert_eq!(max_per_hour(&[]), 0);
        assert_eq!(max_per_hour(&[0, 10, 3599, 3600, 7300]), 3);
    }
}
