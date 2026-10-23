//! Histogram calculations with normal curve overlay and capability indices.
//!
//! Provides `compute_histogram` which bins numerical data, computes distribution
//! statistics (mean, stddev, skewness, kurtosis), generates a normal curve overlay,
//! and optionally calculates process capability indices (Cp, Cpk, Pp, Ppk).

use crate::models::{
    CapabilityIndices, CurvePoint, DistributionStats, HistogramBin, HistogramResult,
};

use std::f64::consts::PI;

/// Compute a histogram from a slice of numerical values.
///
/// - `values`: the data points to bin
/// - `num_bins`: number of equal-width bins
/// - `lsl`: optional Lower Specification Limit (enables capability indices)
/// - `usl`: optional Upper Specification Limit (enables capability indices)
///
/// Returns `HistogramResult` containing bins, normal curve overlay, distribution
/// stats, and optionally capability indices when both LSL and USL are provided.
pub fn compute_histogram(
    values: &[f64],
    num_bins: u32,
    lsl: Option<f64>,
    usl: Option<f64>,
) -> Result<HistogramResult, String> {
    if values.is_empty() {
        return Err("values must not be empty".to_string());
    }
    if num_bins == 0 {
        return Err("num_bins must be > 0".to_string());
    }

    let n = values.len() as u64;

    // ── Distribution stats ──
    let stats = compute_distribution_stats(values);

    // Edge case: all values identical → stddev is 0, skip normal curve
    if stats.stddev < f64::EPSILON {
        let min_val = stats.mean;
        let _max_val = stats.mean;
        let bin_width = if num_bins > 1 { 1.0 } else { 0.0 };

        let mut bins: Vec<HistogramBin> = Vec::with_capacity(num_bins as usize);
        for i in 0..num_bins as usize {
            let lower = min_val + i as f64 * bin_width;
            let upper = lower + bin_width;
            let count = if i == 0 { n } else { 0 };
            bins.push(HistogramBin {
                lower,
                upper,
                count,
                frequency: count as f64 / n as f64,
            });
        }

        return Ok(HistogramResult {
            bins,
            normal_curve: vec![CurvePoint {
                x: stats.mean,
                y: 0.0,
            }],
            stats,
            capability: None,
        });
    }

    // ── Bins ──
    let min_val = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_val = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    let range = max_val - min_val;
    let bin_width = if range < f64::EPSILON {
        1.0
    } else {
        range / num_bins as f64
    };

    let mut bin_counts: Vec<u64> = vec![0; num_bins as usize];
    let num_bins_usize = num_bins as usize;

    for &v in values {
        let idx = if bin_width < f64::EPSILON {
            0
        } else {
            let raw = ((v - min_val) / bin_width).floor() as usize;
            // Clamp: max value lands in the last bin
            if raw >= num_bins_usize {
                num_bins_usize - 1
            } else {
                raw
            }
        };
        bin_counts[idx] += 1;
    }

    let bins: Vec<HistogramBin> = (0..num_bins_usize)
        .map(|i| {
            let lower = min_val + i as f64 * bin_width;
            let upper = lower + bin_width;
            let count = bin_counts[i];
            HistogramBin {
                lower,
                upper,
                count,
                frequency: count as f64 / n as f64,
            }
        })
        .collect();

    // ── Normal curve overlay (100 points from min to max) ──
    let num_curve_points: usize = 100;
    let normal_curve = if num_curve_points > 1 {
        let step = (max_val - min_val) / (num_curve_points - 1) as f64;
        (0..num_curve_points)
            .map(|i| {
                let x = min_val + i as f64 * step;
                let z = (x - stats.mean) / stats.stddev;
                let y = (-0.5 * z * z).exp() / (stats.stddev * (2.0 * PI).sqrt());
                CurvePoint { x, y }
            })
            .collect()
    } else {
        vec![CurvePoint {
            x: stats.mean,
            y: (-0.5 * 0.0_f64).exp() / (stats.stddev * (2.0 * PI).sqrt()),
        }]
    };

    // ── Capability indices (only if both LSL and USL are provided) ──
    let capability = match (lsl, usl) {
        (Some(l), Some(u)) => {
            let sigma = stats.stddev;
            if sigma < f64::EPSILON {
                None
            } else {
                let cp = (u - l) / (6.0 * sigma);
                let cpu = (u - stats.mean) / (3.0 * sigma);
                let cpl = (stats.mean - l) / (3.0 * sigma);
                let cpk = cpu.min(cpl);
                // Pp = Cp, Ppk = Cpk (same since we don't separate within/overall)
                let pp = cp;
                let ppk = cpk;
                Some(CapabilityIndices { cp, cpk, pp, ppk })
            }
        }
        _ => None,
    };

    Ok(HistogramResult {
        bins,
        normal_curve,
        stats,
        capability,
    })
}

/// Compute distribution statistics: mean, stddev, skewness, excess kurtosis, count.
fn compute_distribution_stats(values: &[f64]) -> DistributionStats {
    let n = values.len() as f64;
    let count = values.len() as u64;

    // Mean
    let sum: f64 = values.iter().sum();
    let mean = sum / n;

    // Variance (population)
    let variance: f64 = values.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / n;
    let stddev = variance.sqrt();

    // Skewness and kurtosis require non-zero stddev
    let (skewness, kurtosis) = if stddev < f64::EPSILON {
        (0.0, 0.0)
    } else {
        // M3 = sum(((x - mean) / stddev)^3) / n
        let m3: f64 = values
            .iter()
            .map(|&x| ((x - mean) / stddev).powi(3))
            .sum::<f64>()
            / n;
        // M4 = sum(((x - mean) / stddev)^4) / n
        let m4: f64 = values
            .iter()
            .map(|&x| ((x - mean) / stddev).powi(4))
            .sum::<f64>()
            / n;
        let skew = m3;
        let kurt = m4 - 3.0; // excess kurtosis
        (skew, kurt)
    };

    DistributionStats {
        mean,
        stddev,
        skewness,
        kurtosis,
        count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histogram_basic() {
        // 100 values from 0..99, 10 bins
        let values: Vec<f64> = (0..100).map(|v| v as f64).collect();
        let result = compute_histogram(&values, 10, None, None).unwrap();

        assert_eq!(result.bins.len(), 10);
        let total_count: u64 = result.bins.iter().map(|b| b.count).sum();
        assert_eq!(total_count, 100);

        // Capability should be None (no spec limits)
        assert!(result.capability.is_none());

        // Normal curve should have 100 points
        assert_eq!(result.normal_curve.len(), 100);
    }

    #[test]
    fn test_histogram_with_spec_limits() {
        // 10 values, 5 bins, LSL=9, USL=13
        let values: Vec<f64> = vec![10.0, 10.5, 11.0, 11.5, 12.0, 10.2, 10.8, 11.3, 11.7, 12.5];
        let result = compute_histogram(&values, 5, Some(9.0), Some(13.0)).unwrap();

        assert_eq!(result.bins.len(), 5);
        let total_count: u64 = result.bins.iter().map(|b| b.count).sum();
        assert_eq!(total_count, 10);

        // Capability should be Some with cp > 0
        let cap = result.capability.expect("capability should be Some");
        assert!(cap.cp > 0.0, "cp should be > 0, got {}", cap.cp);
        assert!(cap.cpk > 0.0, "cpk should be > 0, got {}", cap.cpk);
        assert!((cap.pp - cap.cp).abs() < f64::EPSILON);
        assert!((cap.ppk - cap.cpk).abs() < f64::EPSILON);
    }

    #[test]
    fn test_histogram_empty() {
        let result = compute_histogram(&[], 10, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn test_distribution_stats() {
        // Known values [2, 4, 4, 4, 5, 5, 7, 9] => mean = 40/8 = 5.0
        let values: Vec<f64> = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let stats = compute_distribution_stats(&values);

        assert!(
            (stats.mean - 5.0).abs() < 1e-9,
            "expected mean ≈ 5.0, got {}",
            stats.mean
        );
        assert_eq!(stats.count, 8);
        assert!(stats.stddev > 0.0, "stddev should be > 0");

        // Verify stddev manually:
        // variance = ((2-5)^2 + 3*(4-5)^2 + 2*(5-5)^2 + (7-5)^2 + (9-5)^2) / 8
        //          = (9 + 3*1 + 0 + 4 + 16) / 8 = 32/8 = 4.0
        // stddev = 2.0
        assert!(
            (stats.stddev - 2.0).abs() < 1e-9,
            "expected stddev ≈ 2.0, got {}",
            stats.stddev
        );
    }
}
