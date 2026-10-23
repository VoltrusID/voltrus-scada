//! Statistical Process Control (SPC) calculations.
//!
//! Provides X-bar & R, CUSUM, EWMA, P-chart, and NP-chart functions
//! using the crate's `SpcResult`, `SpcPoint`, and `ControlLimits` types.

pub mod spc_attributes;
pub mod spc_calculations;

// Re-export all public items so that `analytics::spc::xxx` still works.
#[allow(unused_imports)]
pub use spc_attributes::*;
#[allow(unused_imports)]
pub use spc_calculations::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xbar_r_basic() {
        // 20 values, subgroup_size=4 => 5 subgroups
        let values: Vec<f64> = vec![
            10.0, 11.0, 12.0, 13.0, // subgroup 0: xbar=11.5, r=3.0
            14.0, 15.0, 16.0, 17.0, // subgroup 1
            10.0, 12.0, 14.0, 16.0, // subgroup 2
            11.0, 13.0, 15.0, 17.0, // subgroup 3
            12.0, 14.0, 16.0, 18.0, // subgroup 4
        ];
        let timestamps: Vec<i64> = (0..20).collect();

        let (xbar_res, r_res) = xbar_r(&values, 4, &timestamps).unwrap();

        // 5 subgroups
        assert_eq!(xbar_res.data_points.len(), 5);
        assert_eq!(r_res.data_points.len(), 5);

        // xbar[0] = (10+11+12+13)/4 = 11.5
        assert!((xbar_res.data_points[0].value - 11.5).abs() < 1e-9);

        // r[0] = 13 - 10 = 3.0
        assert!((r_res.data_points[0].value - 3.0).abs() < 1e-9);

        // chart types
        assert_eq!(xbar_res.chart_type, "xbar");
        assert_eq!(r_res.chart_type, "r");
    }

    #[test]
    fn test_xbar_r_invalid_subgroup() {
        let values = vec![1.0, 2.0, 3.0];
        let ts: Vec<i64> = vec![0, 1, 2];

        // Size 0 should error
        assert!(xbar_r(&values, 0, &ts).is_err());

        // Size 11 should error
        assert!(xbar_r(&values, 11, &ts).is_err());
    }

    #[test]
    fn test_cusum_basic() {
        // Data centered on target=10 — should produce no violations
        let values: Vec<f64> = vec![
            10.1, 9.9, 10.0, 9.8, 10.2, 10.0, 9.9, 10.1, 10.0, 10.0, 9.9, 10.1, 10.0, 9.8, 10.2,
            10.0, 10.1, 9.9, 10.0, 10.0,
        ];
        let timestamps: Vec<i64> = (0..20).collect();

        let result = cusum(&values, 10.0, 0.5, 5.0, &timestamps).unwrap();
        assert_eq!(result.chart_type, "cusum");
        assert_eq!(result.data_points.len(), 20);
        assert!(
            result.violations.is_empty(),
            "stable data around target should have no CUSUM violations, got {:?}",
            result.violations
        );
        assert!((result.control_limits.ucl - 5.0).abs() < 1e-9);
        assert!((result.control_limits.lcl - (-5.0)).abs() < 1e-9);
        assert!((result.control_limits.cl - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_ewma_basic() {
        let values: Vec<f64> = vec![10.0, 11.0, 9.5, 10.5, 10.0, 9.8, 10.2, 10.1];
        let timestamps: Vec<i64> = (0..8).collect();

        let result = ewma(&values, 10.0, 0.2, &timestamps).unwrap();

        assert_eq!(result.chart_type, "ewma");
        assert_eq!(result.data_points.len(), 8);

        // First point: z = 0.2*10.0 + 0.8*10.0 = 10.0
        assert!((result.data_points[0].value - 10.0).abs() < 1e-9);

        // Second point: z = 0.2*11.0 + 0.8*10.0 = 10.2
        assert!((result.data_points[1].value - 10.2).abs() < 1e-9);
    }

    #[test]
    fn test_p_chart_basic() {
        // 8 samples, each of size 50
        let defectives: Vec<u32> = vec![3, 5, 2, 4, 6, 3, 5, 4];
        let sample_sizes: Vec<u32> = vec![50; 8];
        let timestamps: Vec<i64> = (0..8).collect();

        let result = p_chart(&defectives, &sample_sizes, &timestamps).unwrap();

        assert_eq!(result.chart_type, "p");
        assert_eq!(result.data_points.len(), 8);
        assert!(result.control_limits.cl > 0.0, "cl (p_bar) should be > 0");

        // p_bar = 32/400 = 0.08
        let expected_p_bar: f64 = 32.0 / 400.0;
        assert!((result.control_limits.cl - expected_p_bar).abs() < 1e-9);
    }

    #[test]
    fn test_np_chart_basic() {
        let defectives: Vec<u32> = vec![3, 5, 2, 4, 6, 3, 5, 4];
        let timestamps: Vec<i64> = (0..8).collect();

        let result = np_chart(&defectives, 50, &timestamps).unwrap();

        assert_eq!(result.chart_type, "np");
        assert_eq!(result.data_points.len(), 8);

        // np_bar = 50 * (32 / 400) = 4.0
        assert!((result.control_limits.cl - 4.0).abs() < 1e-9);
        assert!(result.control_limits.ucl > result.control_limits.cl);
        assert!(result.control_limits.lcl < result.control_limits.cl);
    }
}
