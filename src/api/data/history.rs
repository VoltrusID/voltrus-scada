use std::collections::HashSet;

use crate::models::{DataPoint, DataSource};

/// One aggregation bucket of a history series.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bucket {
    /// Start of the bucket in Unix seconds.
    pub timestamp: i64,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
}

/// Group `(unix seconds, value)` samples into buckets of `bucket_secs`, in
/// time order. Buckets sit on multiples of `bucket_secs`, so they do not shift
/// between two requests for a moving range. Buckets without samples are left
/// out, and so are samples that are not finite numbers.
pub(crate) fn bucket_samples(samples: &[(i64, f64)], bucket_secs: i64) -> Vec<Bucket> {
    let bucket_secs = bucket_secs.max(1);
    let mut buckets: Vec<Bucket> = Vec::new();
    let mut count = 0usize;
    let mut sum = 0.0;
    for &(ts, value) in samples {
        if !value.is_finite() {
            continue;
        }
        let start = ts.div_euclid(bucket_secs) * bucket_secs;
        match buckets.last_mut() {
            Some(last) if last.timestamp == start => {
                count += 1;
                sum += value;
                last.min = last.min.min(value);
                last.max = last.max.max(value);
                last.avg = sum / count as f64;
            }
            _ => {
                count = 1;
                sum = value;
                buckets.push(Bucket {
                    timestamp: start,
                    avg: value,
                    min: value,
                    max: value,
                });
            }
        }
    }
    buckets
}

/// The low and high value of every point of a series.
pub(crate) type Band = (Vec<f64>, Vec<f64>);

/// The averages of `buckets` as a series of points, rounded to two decimals as
/// the generated series are, with their min/max band when `with_band` is set.
pub(crate) fn series_from_buckets(
    buckets: Vec<Bucket>,
    with_band: bool,
) -> (Vec<DataPoint>, Option<Band>) {
    let round = |v: f64| (v * 100.0).round() / 100.0;
    let points = buckets
        .iter()
        .map(|b| DataPoint {
            timestamp: b.timestamp,
            value: round(b.avg),
        })
        .collect();
    let band = with_band.then(|| {
        (
            buckets.iter().map(|b| round(b.min)).collect(),
            buckets.iter().map(|b| round(b.max)).collect(),
        )
    });
    (points, band)
}

/// Narrow the data sources a history request is about.
///
/// `sensor` is one tag key or a comma-separated list of keys; `device` is a
/// device id. Both are optional, empty means no filter, and both must match
/// when given.
pub(crate) fn filter_sources(
    sources: Vec<DataSource>,
    sensor: Option<&str>,
    device: Option<&str>,
) -> Vec<DataSource> {
    let keys: Option<HashSet<&str>> = sensor.filter(|s| !s.is_empty()).map(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .collect()
    });
    let device = device.filter(|d| !d.is_empty());
    sources
        .into_iter()
        .filter(|ds| {
            keys.as_ref()
                .is_none_or(|keys| keys.contains(ds.key.as_str()))
        })
        .filter(|ds| device.is_none_or(|d| ds.device_id == d))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_carry_avg_min_max() {
        let samples = [(101, 10.0), (102, 30.0), (103, 20.0), (111, 100.0)];
        let buckets = bucket_samples(&samples, 10);
        assert_eq!(
            buckets,
            vec![
                Bucket {
                    timestamp: 100,
                    avg: 20.0,
                    min: 10.0,
                    max: 30.0
                },
                Bucket {
                    timestamp: 110,
                    avg: 100.0,
                    min: 100.0,
                    max: 100.0
                },
            ]
        );
    }

    #[test]
    fn empty_buckets_and_non_finite_samples_are_left_out() {
        let samples = [(5, 1.0), (6, f64::NAN), (47, 3.0)];
        let buckets = bucket_samples(&samples, 10);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].timestamp, 0);
        assert_eq!(buckets[1].timestamp, 40);
        assert!(bucket_samples(&[], 10).is_empty());
    }
}
