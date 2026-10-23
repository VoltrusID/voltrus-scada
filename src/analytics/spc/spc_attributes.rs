//! SPC attribute charts — P-chart and NP-chart.
//!
//! Provides proportion-defective (P) and number-defective (NP) chart functions
//! using the crate's `SpcResult`, `SpcPoint`, and `ControlLimits` types.

use crate::models::{ControlLimits, SpcPoint, SpcResult};

/// P-chart (proportion defective).
///
/// Each entry in `defectives` is the count of defective items in the
/// corresponding sample whose size is given by `sample_sizes[i]`.
pub fn p_chart(
    defectives: &[u32],
    sample_sizes: &[u32],
    timestamps: &[i64],
) -> Result<SpcResult, String> {
    if defectives.is_empty() {
        return Err("defectives must not be empty".to_string());
    }
    if defectives.len() != sample_sizes.len() {
        return Err("defectives and sample_sizes must have the same length".to_string());
    }
    if sample_sizes.contains(&0) {
        return Err("sample sizes must be > 0".to_string());
    }

    let total_defective: u64 = defectives.iter().map(|&d| d as u64).sum();
    let total_inspected: u64 = sample_sizes.iter().map(|&n| n as u64).sum();
    let p_bar = total_defective as f64 / total_inspected as f64;

    let n = defectives.len();
    let mut points: Vec<SpcPoint> = Vec::with_capacity(n);
    let mut violations: Vec<usize> = Vec::new();

    for (i, (&d, &sz)) in defectives.iter().zip(sample_sizes.iter()).enumerate() {
        let p = d as f64 / sz as f64;
        let se = (p_bar * (1.0 - p_bar) / sz as f64).sqrt();
        let ucl = p_bar + 3.0 * se;
        let lcl = (p_bar - 3.0 * se).max(0.0);

        let ts = if i < timestamps.len() {
            timestamps[i]
        } else {
            0
        };

        points.push(SpcPoint {
            index: i,
            value: p,
            timestamp: ts,
            subgroup_values: None,
        });

        if p > ucl || p < lcl {
            violations.push(i);
        }
    }

    // Use overall limits for the result's control_limits
    let avg_n = total_inspected as f64 / n as f64;
    let overall_se = (p_bar * (1.0 - p_bar) / avg_n).sqrt();

    let in_control = (1.0 - (violations.len() as f64 / n as f64)) * 100.0;

    Ok(SpcResult {
        chart_type: "p".to_string(),
        data_points: points,
        control_limits: ControlLimits {
            ucl: p_bar + 3.0 * overall_se,
            cl: p_bar,
            lcl: (p_bar - 3.0 * overall_se).max(0.0),
        },
        violations,
        in_control_percent: in_control,
    })
}

/// NP-chart (number defective).
///
/// Fixed `sample_size` across all samples. `defectives[i]` is the count of
/// defective items in sample `i`.
pub fn np_chart(
    defectives: &[u32],
    sample_size: u32,
    timestamps: &[i64],
) -> Result<SpcResult, String> {
    if defectives.is_empty() {
        return Err("defectives must not be empty".to_string());
    }
    if sample_size == 0 {
        return Err("sample_size must be > 0".to_string());
    }

    let n = defectives.len();
    let total_defective: u64 = defectives.iter().map(|&d| d as u64).sum();
    let total_inspected = sample_size as u64 * n as u64;
    let p_bar = total_defective as f64 / total_inspected as f64;
    let np_bar = sample_size as f64 * p_bar;

    let se = (np_bar * (1.0 - p_bar)).sqrt();
    let ucl = np_bar + 3.0 * se;
    let lcl = (np_bar - 3.0 * se).max(0.0);

    let mut points: Vec<SpcPoint> = Vec::with_capacity(n);
    let mut violations: Vec<usize> = Vec::new();

    for (i, &d) in defectives.iter().enumerate() {
        let ts = if i < timestamps.len() {
            timestamps[i]
        } else {
            0
        };

        let d_f64 = d as f64;
        points.push(SpcPoint {
            index: i,
            value: d_f64,
            timestamp: ts,
            subgroup_values: None,
        });

        if d_f64 > ucl || d_f64 < lcl {
            violations.push(i);
        }
    }

    let in_control = (1.0 - (violations.len() as f64 / n as f64)) * 100.0;

    Ok(SpcResult {
        chart_type: "np".to_string(),
        data_points: points,
        control_limits: ControlLimits {
            ucl,
            cl: np_bar,
            lcl,
        },
        violations,
        in_control_percent: in_control,
    })
}
