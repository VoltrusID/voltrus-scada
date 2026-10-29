use std::collections::{BTreeMap, HashMap, HashSet};

use crate::expression;
use crate::models::{DataPoint, DataSource};
use crate::storage::SeriesBucket;

/// Most points per tag that `agg=sampling` (the default) answers with. A
/// window with fewer samples is answered with every one of them; a longer one
/// with the newest sample of each of 2000 equal slices of the window.
pub(crate) const SAMPLING_MAX_POINTS: i64 = 2_000;

/// Most points per tag that `detail=full` answers with. The same rule: every
/// sample when the window holds fewer, an even thinning of the whole window
/// when it holds more. The answer never stops short of the end of the window.
pub(crate) const FULL_MAX_POINTS: i64 = 10_000;

/// What a history series holds per point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// A real sample: the newest of each bucket.
    Sampling,
    /// The mean of each bucket.
    Avg,
    /// The mean of each bucket with its lowest and highest sample.
    MinMax,
}

/// How a request's window is cut into buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Plan {
    pub mode: Mode,
    pub bucket_secs: i64,
}

/// Choose the aggregation and bucket width for a request.
///
/// `interval_ms` is the nominal point spacing of the range (see
/// `generators::interval_for_range`). Averages use it, widened if the window
/// would otherwise exceed `FULL_MAX_POINTS`. Sampling ignores it: the width is
/// 1 s (every sample) unless the window holds more than the point cap.
pub(crate) fn plan(agg: &str, full: bool, start: i64, end: i64, interval_ms: u64) -> Plan {
    let mode = match agg {
        "avg" => Mode::Avg,
        "minmax" => Mode::MinMax,
        _ => Mode::Sampling,
    };
    let window = (end - start).max(1);
    let fit = |cap: i64| (window + cap - 1) / cap;
    let bucket_secs = match mode {
        Mode::Sampling => fit(if full {
            FULL_MAX_POINTS
        } else {
            SAMPLING_MAX_POINTS
        }),
        Mode::Avg | Mode::MinMax => ((interval_ms / 1000) as i64).max(fit(FULL_MAX_POINTS)),
    }
    .max(1);
    Plan { mode, bucket_secs }
}

/// The points of one tag's buckets for `mode`: real samples for `Sampling`
/// (at the time they were taken), bucket means rounded to two decimals as the
/// generated series are, with the min/max band for `MinMax`.
pub(crate) fn tag_points(series: &[SeriesBucket], mode: Mode) -> (Vec<DataPoint>, Option<Band>) {
    match mode {
        Mode::Sampling => (
            series
                .iter()
                .map(|b| DataPoint {
                    timestamp: b.last_ts,
                    value: b.last,
                })
                .collect(),
            None,
        ),
        Mode::Avg | Mode::MinMax => series_from_buckets(
            series
                .iter()
                .map(|b| Bucket {
                    timestamp: b.start,
                    avg: b.avg,
                    min: b.min,
                    max: b.max,
                })
                .collect(),
            mode == Mode::MinMax,
        ),
    }
}

/// The points of a tag defined by an expression over other tags. The
/// expression is evaluated on each bucket's values of the tags it names (their
/// means, newest samples, or lows and highs, by `mode`), so an expression that
/// is not monotonic in its inputs gets a band that is approximate. A bucket
/// where an input has no data is left out.
pub(crate) fn expression_points(
    expr: &str,
    series: &BTreeMap<String, Vec<SeriesBucket>>,
    mode: Mode,
) -> (Vec<DataPoint>, Option<Band>) {
    let mut by_start: BTreeMap<i64, Vec<(&str, &SeriesBucket)>> = BTreeMap::new();
    for (metric, buckets) in series {
        for b in buckets {
            by_start.entry(b.start).or_default().push((metric, b));
        }
    }
    let eval = |at: &[(&str, &SeriesBucket)], pick: fn(&SeriesBucket) -> f64| {
        let values: HashMap<String, f64> =
            at.iter().map(|(m, b)| (m.to_string(), pick(b))).collect();
        expression::evaluate(expr, &values)
            .ok()
            .filter(|v| v.is_finite())
    };

    let mut buckets = Vec::new();
    let mut points = Vec::new();
    for (start, at) in &by_start {
        match mode {
            Mode::Sampling => {
                if let Some(value) = eval(at, |b| b.last) {
                    let timestamp = at.iter().map(|(_, b)| b.last_ts).max().unwrap_or(*start);
                    points.push(DataPoint { timestamp, value });
                }
            }
            Mode::Avg | Mode::MinMax => {
                if let Some(avg) = eval(at, |b| b.avg) {
                    let (lo, hi) = (eval(at, |b| b.min), eval(at, |b| b.max));
                    let edges = [Some(avg), lo, hi].into_iter().flatten();
                    let (min, max) = edges
                        .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), v| {
                            (mn.min(v), mx.max(v))
                        });
                    buckets.push(Bucket {
                        timestamp: *start,
                        avg,
                        min,
                        max,
                    });
                }
            }
        }
    }
    match mode {
        Mode::Sampling => (points, None),
        Mode::Avg | Mode::MinMax => series_from_buckets(buckets, mode == Mode::MinMax),
    }
}

/// One aggregation bucket of a history series.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bucket {
    /// Start of the bucket in Unix seconds.
    pub timestamp: i64,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
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

    fn bucket(start: i64, avg: f64, min: f64, max: f64, last: f64, last_ts: i64) -> SeriesBucket {
        SeriesBucket {
            start,
            avg,
            min,
            max,
            last,
            last_ts,
            count: 3,
        }
    }

    #[test]
    fn sampling_plan_keeps_every_sample_until_the_cap() {
        // 30 min: one bucket per second, so every sample is its own point.
        assert_eq!(plan("sampling", false, 0, 1800, 10_000).bucket_secs, 1);
        // 24 h: 86400 s over 2000 points.
        assert_eq!(plan("sampling", false, 0, 86_400, 480_000).bucket_secs, 44);
        // Full detail allows 10000 points: a day fits in 9 s buckets.
        assert_eq!(plan("sampling", true, 0, 86_400, 1_000).bucket_secs, 9);
        assert_eq!(plan("sampling", true, 0, 3_600, 1_000).bucket_secs, 1);
        // An empty or reversed window still gives a usable width.
        assert_eq!(plan("sampling", false, 10, 10, 1_000).bucket_secs, 1);
        assert_eq!(
            plan("anything else", false, 0, 60, 1_000).mode,
            Mode::Sampling
        );
    }

    #[test]
    fn average_plan_follows_the_range_interval() {
        let p = plan("avg", false, 0, 86_400, 480_000);
        assert_eq!((p.mode, p.bucket_secs), (Mode::Avg, 480));
        let p = plan("minmax", false, 0, 1_800, 10_000);
        assert_eq!((p.mode, p.bucket_secs), (Mode::MinMax, 10));
        // Full detail asks for 1 s buckets, but a long window is widened so the
        // answer stays within the point cap.
        assert_eq!(plan("avg", true, 0, 86_400, 1_000).bucket_secs, 9);
    }

    #[test]
    fn tag_points_by_mode() {
        let series = [
            bucket(100, 20.0, 10.0, 30.0, 25.0, 109),
            bucket(110, 5.0, 5.0, 5.0, 5.0, 110),
        ];
        let (pts, band) = tag_points(&series, Mode::Sampling);
        assert_eq!(
            pts.iter()
                .map(|p| (p.timestamp, p.value))
                .collect::<Vec<_>>(),
            vec![(109, 25.0), (110, 5.0)]
        );
        assert!(band.is_none());
        let (pts, band) = tag_points(&series, Mode::MinMax);
        assert_eq!(
            pts.iter().map(|p| p.timestamp).collect::<Vec<_>>(),
            vec![100, 110]
        );
        assert_eq!(band, Some((vec![10.0, 5.0], vec![30.0, 5.0])));
        assert!(tag_points(&series, Mode::Avg).1.is_none());
    }

    #[test]
    fn expression_points_need_every_input() {
        let mut series = BTreeMap::new();
        series.insert(
            "a".to_string(),
            vec![
                bucket(0, 2.0, 1.0, 3.0, 3.0, 9),
                bucket(10, 4.0, 4.0, 4.0, 4.0, 19),
            ],
        );
        series.insert("b".to_string(), vec![bucket(0, 10.0, 10.0, 10.0, 10.0, 8)]);
        let (pts, _) = expression_points("{a} + {b}", &series, Mode::Avg);
        // Only the first bucket has both inputs.
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0].value, 12.0);
        let (pts, band) = expression_points("{a} + {b}", &series, Mode::MinMax);
        assert_eq!(pts.len(), 1);
        assert_eq!(band, Some((vec![11.0], vec![13.0])));
        let (pts, _) = expression_points("{a} + {b}", &series, Mode::Sampling);
        assert_eq!((pts[0].timestamp, pts[0].value), (9, 13.0));
    }
}
