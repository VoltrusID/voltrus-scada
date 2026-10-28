//! Write-path filtering for the historian.
//!
//! The persistence loop hands every cached sample to [`HistoryFilter`] before
//! it reaches the database. A tag opts in with keys in its `source_config`:
//!
//! ```json
//! { "history_deadband": 0.5, "history_deadband_type": "absolute",
//!   "history_max_interval_secs": 300 }
//! ```
//!
//! * `history_deadband` stores a new sample only when it differs from the last
//!   *stored* sample by more than this much (`0` stores every change).
//! * `history_deadband_type` is `"absolute"` (engineering units, the default)
//!   or `"percent"` (percent of the last stored value).
//! * `history_max_interval_secs` stores a sample at least this often even when
//!   nothing changed, so a flat line still has points and a gap in the trend
//!   always means "no data" rather than "no change".
//!
//! Tags without `history_deadband` are stored as before, with one exception
//! that applies to every tag: a sample identical to the last stored one in
//! both timestamp and value (the persistence loop re-reads the cache of a
//! device that stopped answering) is dropped.

use std::collections::HashMap;

/// `(device_id, timestamp, metric, value)`, the row shape of `Storage::insert_batch`.
pub type Record = (String, i64, String, f64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeadbandKind {
    Absolute,
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryRule {
    pub kind: DeadbandKind,
    pub deadband: f64,
    pub max_interval_secs: Option<i64>,
}

impl HistoryRule {
    /// Read the rule from a tag's `source_config`. `None` when the tag has no
    /// `history_deadband` (or one that is negative or not a number).
    pub fn from_source_config(config: &serde_json::Value) -> Option<Self> {
        let deadband = config.get("history_deadband")?.as_f64()?;
        if !deadband.is_finite() || deadband < 0.0 {
            return None;
        }
        let kind = match config
            .get("history_deadband_type")
            .and_then(|v| v.as_str())
            .unwrap_or("absolute")
        {
            "percent" => DeadbandKind::Percent,
            _ => DeadbandKind::Absolute,
        };
        let max_interval_secs = config
            .get("history_max_interval_secs")
            .and_then(|v| v.as_i64())
            .filter(|&s| s > 0);
        Some(Self {
            kind,
            deadband,
            max_interval_secs,
        })
    }

    fn threshold(&self, last_stored: f64) -> f64 {
        match self.kind {
            DeadbandKind::Absolute => self.deadband,
            DeadbandKind::Percent => self.deadband / 100.0 * last_stored.abs(),
        }
    }
}

#[derive(Default)]
pub struct HistoryFilter {
    rules: HashMap<(String, String), HistoryRule>,
    /// Timestamp and value of the last sample let through, per tag.
    last: HashMap<(String, String), (i64, f64)>,
}

impl HistoryFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the rule set (called every cycle from the current tag list).
    /// Tags that lost their rule are forgotten.
    pub fn set_rules(&mut self, rules: HashMap<(String, String), HistoryRule>) {
        self.rules = rules;
    }

    /// Forget what was last stored. Called when a batch failed to reach the
    /// database, so the next cycle compares against nothing instead of
    /// against samples that were never saved.
    pub fn reset(&mut self) {
        self.last.clear();
    }

    /// Keep the records that should be stored, in order.
    pub fn filter(&mut self, records: Vec<Record>) -> Vec<Record> {
        let mut kept = Vec::with_capacity(records.len());
        for record in records {
            let key = (record.0.clone(), record.2.clone());
            let (ts, value) = (record.1, record.3);
            let store = match (self.last.get(&key), self.rules.get(&key)) {
                (None, _) => true,
                (Some(&(last_ts, last_value)), rule) => {
                    let repeat = ts == last_ts && value.to_bits() == last_value.to_bits();
                    if repeat {
                        false
                    } else if let Some(rule) = rule {
                        value.is_nan()
                            || last_value.is_nan()
                            || (value - last_value).abs() > rule.threshold(last_value)
                            || rule
                                .max_interval_secs
                                .is_some_and(|max| ts.saturating_sub(last_ts) >= max)
                    } else {
                        true
                    }
                }
            };
            if store {
                self.last.insert(key, (ts, value));
                kept.push(record);
            }
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ts: i64, v: f64) -> Record {
        ("dev".into(), ts, "temp".into(), v)
    }

    fn filter_with(rule: HistoryRule) -> HistoryFilter {
        let mut f = HistoryFilter::new();
        f.set_rules(HashMap::from([(("dev".into(), "temp".into()), rule)]));
        f
    }

    fn absolute(deadband: f64, max_interval_secs: Option<i64>) -> HistoryRule {
        HistoryRule {
            kind: DeadbandKind::Absolute,
            deadband,
            max_interval_secs,
        }
    }

    #[test]
    fn tag_without_rule_stores_everything_but_exact_repeats() {
        let mut f = HistoryFilter::new();
        let kept = f.filter(vec![rec(1, 5.0), rec(2, 5.0), rec(2, 5.0), rec(3, 5.0)]);
        assert_eq!(kept, vec![rec(1, 5.0), rec(2, 5.0), rec(3, 5.0)]);
        // The same timestamp with another value is a correction, not a repeat.
        assert_eq!(f.filter(vec![rec(3, 6.0)]), vec![rec(3, 6.0)]);
    }

    #[test]
    fn deadband_is_measured_from_the_last_stored_sample() {
        let mut f = filter_with(absolute(1.0, None));
        // 10.0 stored; 10.6 and 10.9 are inside the band; 11.2 is 1.2 from 10.0.
        let kept = f.filter(vec![rec(1, 10.0), rec(2, 10.6), rec(3, 10.9), rec(4, 11.2)]);
        assert_eq!(kept, vec![rec(1, 10.0), rec(4, 11.2)]);
        // A slow drift cannot slip through unseen: 11.2 -> 12.0 is only 0.8,
        // but 12.3 is 1.1 from the stored 11.2.
        assert_eq!(
            f.filter(vec![rec(5, 12.0), rec(6, 12.3)]),
            vec![rec(6, 12.3)]
        );
    }

    #[test]
    fn deadband_edge_is_exclusive() {
        let mut f = filter_with(absolute(1.0, None));
        assert_eq!(f.filter(vec![rec(1, 10.0), rec(2, 11.0)]).len(), 1);
    }

    #[test]
    fn zero_deadband_stores_every_change() {
        let mut f = filter_with(absolute(0.0, None));
        let kept = f.filter(vec![
            rec(1, 3.0),
            rec(2, 3.0),
            rec(3, 3.0000001),
            rec(4, 3.0),
        ]);
        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn percent_deadband_scales_with_the_stored_value() {
        let mut f = filter_with(HistoryRule {
            kind: DeadbandKind::Percent,
            deadband: 1.0,
            max_interval_secs: None,
        });
        // 1% of 200 is 2.
        let kept = f.filter(vec![rec(1, 200.0), rec(2, 201.5), rec(3, 202.5)]);
        assert_eq!(kept, vec![rec(1, 200.0), rec(3, 202.5)]);
    }

    #[test]
    fn heartbeat_stores_a_flat_signal_every_max_interval() {
        let mut f = filter_with(absolute(5.0, Some(60)));
        let kept = f.filter(vec![
            rec(0, 1.0),
            rec(30, 1.0),
            rec(59, 1.0),
            rec(60, 1.0),
            rec(100, 1.0),
            rec(120, 1.0),
        ]);
        assert_eq!(kept, vec![rec(0, 1.0), rec(60, 1.0), rec(120, 1.0)]);
    }

    #[test]
    fn rules_are_per_tag() {
        let mut f = filter_with(absolute(10.0, None));
        let other = |ts, v| ("dev".to_string(), ts, "other".to_string(), v);
        let kept = f.filter(vec![rec(1, 0.0), other(1, 0.0), rec(2, 1.0), other(2, 1.0)]);
        // `temp` has a band of 10, `other` has none.
        assert_eq!(kept, vec![rec(1, 0.0), other(1, 0.0), other(2, 1.0)]);
    }

    #[test]
    fn source_config_parsing() {
        let cfg = serde_json::json!({
            "history_deadband": 0.5, "history_deadband_type": "percent",
            "history_max_interval_secs": 300, "deadband": 9.0
        });
        assert_eq!(
            HistoryRule::from_source_config(&cfg),
            Some(HistoryRule {
                kind: DeadbandKind::Percent,
                deadband: 0.5,
                max_interval_secs: Some(300)
            })
        );
        // The alarm hysteresis key is a different setting and must not enable this.
        assert_eq!(
            HistoryRule::from_source_config(&serde_json::json!({"deadband": 1.0})),
            None
        );
        assert_eq!(
            HistoryRule::from_source_config(&serde_json::json!({"history_deadband": -1.0})),
            None
        );
        assert_eq!(
            HistoryRule::from_source_config(&serde_json::json!({"history_deadband": 2})),
            Some(absolute(2.0, None))
        );
    }
}
