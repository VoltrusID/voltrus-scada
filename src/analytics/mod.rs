pub mod custom;
pub mod histogram;
pub mod runchart;
pub mod spc;

use anyhow::Result;
use serde::Serialize;

use crate::storage::Storage;

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AggregatedMetric {
    pub metric: String,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub stddev: f64,
    pub count: u64,
    pub first_ts: i64,
    pub last_ts: i64,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ComparisonResult {
    pub metric: String,
    pub current: AggregatedMetric,
    pub previous: AggregatedMetric,
    pub change_percent: f64,
}

/// Aggregate metrics for a time range with configurable interval.
/// Returns a list of (bucket_start_timestamp, AggregatedMetric) pairs.
pub async fn aggregate_metrics(
    storage: &Storage,
    device_id: &str,
    metric: &str,
    from: i64,
    to: i64,
    interval_secs: i64,
) -> Result<Vec<(i64, AggregatedMetric)>> {
    // Clamp interval to reasonable range
    let interval = if interval_secs < 60 {
        60
    } else {
        interval_secs
    };

    // Hourly aggregates only exist for hours that already left full
    // resolution, so a long range reads them for its old part and the
    // full-resolution samples (raw rows and packed blocks) for the rest.
    let use_hourly = (to - from) > 86400 * 7 && interval >= 3600;

    let full = storage
        .aggregate_raw(device_id, metric, from, to, interval)
        .await?;
    if !use_hourly {
        return Ok(full);
    }
    let hourly = storage
        .aggregate_hourly(device_id, metric, from, to, interval)
        .await?;
    Ok(merge_buckets(hourly, full))
}

/// Combine two ascending bucket lists; a bucket present in both is merged
/// (count-weighted mean, widest min/max and time span).
fn merge_buckets(
    a: Vec<(i64, AggregatedMetric)>,
    b: Vec<(i64, AggregatedMetric)>,
) -> Vec<(i64, AggregatedMetric)> {
    let mut out: Vec<(i64, AggregatedMetric)> = Vec::with_capacity(a.len() + b.len());
    let (mut a, mut b) = (a.into_iter().peekable(), b.into_iter().peekable());
    loop {
        let next = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) if x.0 == y.0 => {
                let (bucket, x) = a.next().expect("peeked");
                let (_, y) = b.next().expect("peeked");
                let count = x.count + y.count;
                let avg = if count == 0 {
                    0.0
                } else {
                    (x.avg * x.count as f64 + y.avg * y.count as f64) / count as f64
                };
                (
                    bucket,
                    AggregatedMetric {
                        metric: x.metric,
                        min: x.min.min(y.min),
                        max: x.max.max(y.max),
                        avg,
                        stddev: 0.0,
                        count,
                        first_ts: x.first_ts.min(y.first_ts),
                        last_ts: x.last_ts.max(y.last_ts),
                    },
                )
            }
            (Some(x), Some(y)) => {
                if x.0 < y.0 {
                    a.next().expect("peeked")
                } else {
                    b.next().expect("peeked")
                }
            }
            (Some(_), None) => a.next().expect("peeked"),
            (None, Some(_)) => b.next().expect("peeked"),
            (None, None) => break,
        };
        out.push(next);
    }
    out
}

/// Compare two time periods for a given metric.
pub async fn compare_periods(
    storage: &Storage,
    device_id: &str,
    metric: &str,
    period1: (i64, i64),
    period2: (i64, i64),
) -> Result<ComparisonResult> {
    let current = storage
        .aggregate_single_period(device_id, metric, period1.0, period1.1)
        .await?;

    let previous = storage
        .aggregate_single_period(device_id, metric, period2.0, period2.1)
        .await?;

    let change_percent = if previous.avg.abs() > f64::EPSILON {
        ((current.avg - previous.avg) / previous.avg) * 100.0
    } else {
        0.0
    };

    Ok(ComparisonResult {
        metric: metric.to_string(),
        current,
        previous,
        change_percent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_change_percent_calculation() {
        let current = AggregatedMetric {
            metric: "temp".into(),
            min: 0.0,
            max: 0.0,
            avg: 110.0,
            stddev: 0.0,
            count: 1,
            first_ts: 0,
            last_ts: 0,
        };
        let previous = AggregatedMetric {
            metric: "temp".into(),
            min: 0.0,
            max: 0.0,
            avg: 100.0,
            stddev: 0.0,
            count: 1,
            first_ts: 0,
            last_ts: 0,
        };
        let change = ((current.avg - previous.avg) / previous.avg) * 100.0;
        assert!((change - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_change_percent_zero_previous() {
        let change = 0.0_f64;
        let previous_avg = 0.0_f64;
        let result = if previous_avg.abs() > f64::EPSILON {
            ((change - previous_avg) / previous_avg) * 100.0
        } else {
            0.0
        };
        assert_eq!(result, 0.0);
    }

    #[test]
    fn test_change_percent_negative() {
        let current_avg = 80.0_f64;
        let previous_avg = 100.0_f64;
        let change = ((current_avg - previous_avg) / previous_avg) * 100.0;
        assert!((change - (-20.0)).abs() < 0.001);
    }
}
