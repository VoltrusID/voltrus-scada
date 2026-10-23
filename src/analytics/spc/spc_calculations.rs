//! Core SPC chart calculations — measurement (continuous) charts.
//!
//! Provides X-bar & R, CUSUM, and EWMA functions using the crate's
//! `SpcResult`, `SpcPoint`, and `ControlLimits` types.

use crate::models::{ControlLimits, SpcPoint, SpcResult};

// ── SPC constants (subgroup_size 2..=10, indexed by size - 2) ──

pub(crate) const A2: [f64; 9] = [
    1.880, 1.023, 0.729, 0.577, 0.483, 0.419, 0.373, 0.337, 0.308,
];
pub(crate) const D3: [f64; 9] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.076, 0.136, 0.184, 0.223];
pub(crate) const D4: [f64; 9] = [
    3.267, 2.575, 2.282, 2.115, 2.004, 1.924, 1.864, 1.816, 1.777,
];

/// X-bar & R chart calculation.
///
/// Divides `values` into subgroups of `subgroup_size` (2-10), computes the
/// mean (X-bar) and range (R) for each subgroup, and derives control limits
/// using standard SPC constants A2, D3, D4.
///
/// Returns `((xbar_result, r_result))` on success.
pub fn xbar_r(
    values: &[f64],
    subgroup_size: usize,
    timestamps: &[i64],
) -> Result<(SpcResult, SpcResult), String> {
    if !(2..=10).contains(&subgroup_size) {
        return Err(format!(
            "subgroup_size must be between 2 and 10, got {}",
            subgroup_size
        ));
    }

    let idx = subgroup_size - 2;

    // Divide into subgroups, skipping incomplete final subgroup
    let num_subgroups = values.len() / subgroup_size;
    if num_subgroups == 0 {
        return Err("not enough values for even one subgroup".to_string());
    }

    let mut xbars: Vec<f64> = Vec::with_capacity(num_subgroups);
    let mut ranges: Vec<f64> = Vec::with_capacity(num_subgroups);
    let mut xbar_points: Vec<SpcPoint> = Vec::with_capacity(num_subgroups);
    let mut r_points: Vec<SpcPoint> = Vec::with_capacity(num_subgroups);

    for i in 0..num_subgroups {
        let start = i * subgroup_size;
        let end = start + subgroup_size;
        let subgroup = &values[start..end];

        let mean: f64 = subgroup.iter().sum::<f64>() / subgroup_size as f64;
        let min_val = subgroup.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_val = subgroup.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let range = max_val - min_val;

        xbars.push(mean);
        ranges.push(range);

        // Use the timestamp of the first element in the subgroup
        let ts = if i < timestamps.len() {
            timestamps[start]
        } else {
            0
        };

        xbar_points.push(SpcPoint {
            index: i,
            value: mean,
            timestamp: ts,
            subgroup_values: Some(subgroup.to_vec()),
        });

        r_points.push(SpcPoint {
            index: i,
            value: range,
            timestamp: ts,
            subgroup_values: Some(subgroup.to_vec()),
        });
    }

    let xbar_avg: f64 = xbars.iter().sum::<f64>() / num_subgroups as f64;
    let r_avg: f64 = ranges.iter().sum::<f64>() / num_subgroups as f64;

    // X-bar control limits
    let xbar_ucl = xbar_avg + A2[idx] * r_avg;
    let xbar_lcl = xbar_avg - A2[idx] * r_avg;

    // R control limits
    let r_ucl = D4[idx] * r_avg;
    let r_lcl = D3[idx] * r_avg;

    // Violations
    let xbar_violations: Vec<usize> = xbars
        .iter()
        .enumerate()
        .filter(|&(_, &v)| v > xbar_ucl || v < xbar_lcl)
        .map(|(i, _)| i)
        .collect();

    let r_violations: Vec<usize> = ranges
        .iter()
        .enumerate()
        .filter(|&(_, &v)| v > r_ucl || v < r_lcl)
        .map(|(i, _)| i)
        .collect();

    let xbar_in_control = if num_subgroups > 0 {
        (1.0 - (xbar_violations.len() as f64 / num_subgroups as f64)) * 100.0
    } else {
        100.0
    };

    let r_in_control = if num_subgroups > 0 {
        (1.0 - (r_violations.len() as f64 / num_subgroups as f64)) * 100.0
    } else {
        100.0
    };

    let xbar_result = SpcResult {
        chart_type: "xbar".to_string(),
        data_points: xbar_points,
        control_limits: ControlLimits {
            ucl: xbar_ucl,
            cl: xbar_avg,
            lcl: xbar_lcl,
        },
        violations: xbar_violations,
        in_control_percent: xbar_in_control,
    };

    let r_result = SpcResult {
        chart_type: "r".to_string(),
        data_points: r_points,
        control_limits: ControlLimits {
            ucl: r_ucl,
            cl: r_avg,
            lcl: r_lcl,
        },
        violations: r_violations,
        in_control_percent: r_in_control,
    };

    Ok((xbar_result, r_result))
}

/// CUSUM (Cumulative Sum) control chart.
///
/// Tracks cumulative deviation from `target` with slack `k` and decision
/// interval `h`. A point violates when `|S| > h`.
pub fn cusum(
    values: &[f64],
    target: f64,
    k: f64,
    h: f64,
    timestamps: &[i64],
) -> Result<SpcResult, String> {
    if values.is_empty() {
        return Err("values must not be empty".to_string());
    }
    if h <= 0.0 {
        return Err("h must be positive".to_string());
    }

    let n = values.len();
    let mut cusum_pos = 0.0_f64;
    let mut cusum_neg = 0.0_f64;
    let mut points: Vec<SpcPoint> = Vec::with_capacity(n);
    let mut violations: Vec<usize> = Vec::new();

    for (i, &x) in values.iter().enumerate() {
        cusum_pos = (cusum_pos + (x - target) - k).max(0.0);
        cusum_neg = (cusum_neg + (x - target) + k).min(0.0);

        let value = cusum_pos.max(cusum_neg.abs());

        let ts = if i < timestamps.len() {
            timestamps[i]
        } else {
            0
        };

        points.push(SpcPoint {
            index: i,
            value,
            timestamp: ts,
            subgroup_values: None,
        });

        if value > h {
            violations.push(i);
        }
    }

    let in_control = (1.0 - (violations.len() as f64 / n as f64)) * 100.0;

    Ok(SpcResult {
        chart_type: "cusum".to_string(),
        data_points: points,
        control_limits: ControlLimits {
            ucl: h,
            cl: 0.0,
            lcl: -h,
        },
        violations,
        in_control_percent: in_control,
    })
}

/// EWMA (Exponentially Weighted Moving Average) control chart.
///
/// Uses smoothing parameter `lambda` (0 < lambda <= 1) and L=3 sigma limits.
/// The EWMA statistic starts at `target` and converges with each observation.
pub fn ewma(
    values: &[f64],
    target: f64,
    lambda: f64,
    timestamps: &[i64],
) -> Result<SpcResult, String> {
    if values.is_empty() {
        return Err("values must not be empty".to_string());
    }
    if lambda <= 0.0 || lambda > 1.0 {
        return Err("lambda must be in (0, 1]".to_string());
    }

    let n = values.len();

    // Estimate process sigma from data
    let mean = values.iter().sum::<f64>() / n as f64;
    let variance = values.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / n as f64;
    let sigma = variance.sqrt();
    if sigma < f64::EPSILON {
        return Err("sigma is zero — data has no variation".to_string());
    }

    let l = 3.0; // 3-sigma
    let mut z = target;
    let mut points: Vec<SpcPoint> = Vec::with_capacity(n);
    let mut violations: Vec<usize> = Vec::new();

    // Steady-state variance for final displayed limits
    let steady_var = sigma * (lambda / (2.0 - lambda)).sqrt();
    let steady_ucl = target + l * steady_var;
    let steady_lcl = target - l * steady_var;

    for (i, &x) in values.iter().enumerate() {
        z = lambda * x + (1.0 - lambda) * z;

        // Exact EWMA variance at point i
        let ewma_var = sigma
            * (lambda / (2.0 - lambda) * (1.0 - (1.0 - lambda).powi(2 * ((i + 1) as i32)))).sqrt();

        let point_ucl = target + l * ewma_var;
        let point_lcl = target - l * ewma_var;

        let ts = if i < timestamps.len() {
            timestamps[i]
        } else {
            0
        };

        points.push(SpcPoint {
            index: i,
            value: z,
            timestamp: ts,
            subgroup_values: None,
        });

        if z > point_ucl || z < point_lcl {
            violations.push(i);
        }
    }

    let in_control = (1.0 - (violations.len() as f64 / n as f64)) * 100.0;

    Ok(SpcResult {
        chart_type: "ewma".to_string(),
        data_points: points,
        control_limits: ControlLimits {
            ucl: steady_ucl,
            cl: target,
            lcl: steady_lcl,
        },
        violations,
        in_control_percent: in_control,
    })
}
