//! Run chart calculations with signal detection.
//!
//! Computes median, builds run chart points, and detects runs (8+ consecutive
//! points on one side of median) and trends (6+ consecutive increasing or
//! decreasing values).

use crate::models::{RunChartPoint, RunChartResult, Signal};

/// Compute a run chart from raw values and timestamps.
///
/// Returns a `RunChartResult` containing the chart points, median value,
/// detected signals, and total run count.
pub fn compute_run_chart(values: &[f64], timestamps: &[i64]) -> Result<RunChartResult, String> {
    if values.is_empty() {
        return Err("values must not be empty".to_string());
    }
    if timestamps.len() != values.len() {
        return Err("timestamps must have same length as values".to_string());
    }

    // 1. Compute median
    let median = compute_median(values);

    // 2. Build RunChartPoints
    let points: Vec<RunChartPoint> = values
        .iter()
        .zip(timestamps.iter())
        .map(|(&v, &ts)| RunChartPoint {
            timestamp: ts,
            value: v,
            above_median: v > median,
        })
        .collect();

    // 3. Detect signals
    let mut signals = Vec::new();

    detect_runs(&points, &mut signals);
    detect_trends(values, &mut signals);

    // 4. Count runs
    let runs_count = count_runs(&points);

    Ok(RunChartResult {
        points,
        median,
        signals,
        runs_count,
    })
}

/// Compute the median of a slice of f64 values.
fn compute_median(values: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let len = sorted.len();
    if len.is_multiple_of(2) {
        (sorted[len / 2 - 1] + sorted[len / 2]) / 2.0
    } else {
        sorted[len / 2]
    }
}

/// Detect runs: 8+ consecutive points on one side of the median.
fn detect_runs(points: &[RunChartPoint], signals: &mut Vec<Signal>) {
    if points.is_empty() {
        return;
    }

    let mut run_start = 0;
    let mut run_above = points[0].above_median;

    for i in 1..=points.len() {
        let side_changed = i == points.len() || points[i].above_median != run_above;

        if side_changed {
            let run_len = i - run_start;
            if run_len >= 8 {
                let side = if run_above { "above" } else { "below" };
                signals.push(Signal {
                    rule: "run".to_string(),
                    start_idx: run_start,
                    end_idx: i - 1,
                    description: format!(
                        "{} consecutive points {} median (indices {}-{})",
                        run_len,
                        side,
                        run_start,
                        i - 1
                    ),
                });
            }

            if i < points.len() {
                run_start = i;
                run_above = points[i].above_median;
            }
        }
    }
}

/// Detect trends: 6+ consecutive strictly increasing or decreasing values.
fn detect_trends(values: &[f64], signals: &mut Vec<Signal>) {
    if values.len() < 2 {
        return;
    }

    // Direction: 1 = increasing, -1 = decreasing, 0 = equal
    let mut trend_start = 0;
    let mut trend_dir: i8 = 0; // direction of the current trend

    for i in 1..values.len() {
        let cmp = values[i]
            .partial_cmp(&values[i - 1])
            .unwrap_or(std::cmp::Ordering::Equal);

        let dir: i8 = match cmp {
            std::cmp::Ordering::Greater => 1,
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
        };

        if dir == 0 {
            // Equal values break any trend; finalize if needed
            check_trend_signal(values, trend_start, i, trend_dir, signals);
            trend_start = i;
            trend_dir = 0;
        } else if dir == trend_dir {
            // Continue current trend — nothing to do
        } else {
            // Direction changed; finalize previous trend if long enough
            check_trend_signal(values, trend_start, i, trend_dir, signals);
            trend_start = i - 1;
            trend_dir = dir;
        }
    }

    // Finalize any trend at the end
    check_trend_signal(values, trend_start, values.len(), trend_dir, signals);
}

/// Check if a completed trend qualifies as a signal (6+ points).
fn check_trend_signal(
    _values: &[f64],
    start: usize,
    end: usize,
    dir: i8,
    signals: &mut Vec<Signal>,
) {
    let len = end - start;
    if len >= 6 && dir != 0 {
        let direction = if dir > 0 { "increasing" } else { "decreasing" };
        signals.push(Signal {
            rule: "trend".to_string(),
            start_idx: start,
            end_idx: end - 1,
            description: format!(
                "{} consecutive {} values (indices {}-{})",
                len,
                direction,
                start,
                end - 1
            ),
        });
    }
}

/// Count the number of runs (groups of consecutive above/below median points).
fn count_runs(points: &[RunChartPoint]) -> u32 {
    if points.is_empty() {
        return 0;
    }

    let mut count = 1;
    for i in 1..points.len() {
        if points[i].above_median != points[i - 1].above_median {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_timestamps(n: usize) -> Vec<i64> {
        (0..n).map(|i| 1000 + i as i64 * 60).collect()
    }

    #[test]
    fn test_run_chart_basic() {
        let values = vec![3.0, 7.0, 2.0, 9.0, 5.0, 1.0, 8.0, 4.0, 6.0, 10.0];
        let timestamps = make_timestamps(10);
        let result = compute_run_chart(&values, &timestamps).unwrap();

        assert_eq!(result.points.len(), 10);
        assert!(result.median > 0.0);
        assert!(result.runs_count >= 1);
    }

    #[test]
    fn test_run_chart_detects_run() {
        // 10 identical values: median = 5.0, none are > median (all false),
        // so all 10 consecutive points are on the "below" side → run signal.
        let values = vec![5.0; 10];
        let timestamps = make_timestamps(10);
        let result = compute_run_chart(&values, &timestamps).unwrap();

        assert_eq!(result.median, 5.0);
        assert_eq!(result.points.len(), 10);
        // All points should have above_median == false
        for p in &result.points {
            assert!(!p.above_median);
        }
        let has_run = result.signals.iter().any(|s| s.rule == "run");
        assert!(
            has_run,
            "expected a run signal but got signals: {:?}",
            result.signals
        );
    }

    #[test]
    fn test_run_chart_detects_trend() {
        // 10 strictly increasing values
        let values: Vec<f64> = (1..=10).map(|v| v as f64).collect();
        let timestamps = make_timestamps(10);
        let result = compute_run_chart(&values, &timestamps).unwrap();

        let has_trend = result.signals.iter().any(|s| s.rule == "trend");
        assert!(
            has_trend,
            "expected a trend signal but got signals: {:?}",
            result.signals
        );
    }

    #[test]
    fn test_run_chart_empty() {
        let values: Vec<f64> = vec![];
        let timestamps: Vec<i64> = vec![];
        let result = compute_run_chart(&values, &timestamps);

        assert!(result.is_err());
    }
}
