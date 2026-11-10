//! Per-value data quality, after the OPC UA Good / Uncertain / Bad model.
//!
//! A value that is not `Good` must not drive alarms, interlocks or history:
//! drivers mark what they know (a failed item, a device exception, a lost
//! connection) and the cache marks values that stopped updating as stale.

use serde::{Deserialize, Serialize};

/// The quality of one cached value. Serialized as a snake_case string so
/// clients can test the family with a prefix (`bad*`, `uncertain*`).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    #[default]
    Good,
    /// The source says the value is usable but not trustworthy.
    Uncertain,
    /// The value was read once but has not been refreshed for several poll intervals.
    UncertainStale,
    /// The source says the value is unusable, for no reason we know more about.
    Bad,
    /// The device cannot be reached; the value is the last one read.
    BadCommFailure,
    /// The device answered this item with an error.
    BadDeviceError,
    /// The item is configured in a way the device or driver cannot serve.
    BadConfigError,
}

impl Quality {
    pub fn is_good(self) -> bool {
        self == Quality::Good
    }

    pub fn is_bad(self) -> bool {
        matches!(
            self,
            Quality::Bad
                | Quality::BadCommFailure
                | Quality::BadDeviceError
                | Quality::BadConfigError
        )
    }

    pub fn is_uncertain(self) -> bool {
        matches!(self, Quality::Uncertain | Quality::UncertainStale)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Good => "good",
            Quality::Uncertain => "uncertain",
            Quality::UncertainStale => "uncertain_stale",
            Quality::Bad => "bad",
            Quality::BadCommFailure => "bad_comm_failure",
            Quality::BadDeviceError => "bad_device_error",
            Quality::BadConfigError => "bad_config_error",
        }
    }

    /// Rank for "worst of": bad > uncertain > good.
    fn severity(self) -> u8 {
        if self.is_bad() {
            2
        } else if self.is_uncertain() {
            1
        } else {
            0
        }
    }

    /// The worse of two qualities (the first one on a tie).
    pub fn worst(self, other: Quality) -> Quality {
        if other.severity() > self.severity() {
            other
        } else {
            self
        }
    }

    /// The quality a driver implies by handing over a value without saying:
    /// NaN and infinities are how drivers have always reported "no value".
    pub fn implied_by(value: f64) -> Quality {
        if value.is_finite() {
            Quality::Good
        } else {
            Quality::Bad
        }
    }
}

/// A value is stale after this many poll intervals without a refresh.
pub const STALE_POLL_INTERVALS: i64 = 3;
/// Floor for the stale window, so a fast poller that hiccups once or a slow
/// cache reader does not flicker values to stale.
pub const MIN_STALE_MS: i64 = 5_000;

/// How long a value read every `interval_ms` stays fresh.
pub fn stale_after_ms(interval_ms: u64) -> i64 {
    (interval_ms as i64)
        .saturating_mul(STALE_POLL_INTERVALS)
        .max(MIN_STALE_MS)
}

/// The quality of a value when it is read at `now_ms`: a good value that is
/// older than the stale window becomes `UncertainStale`. Without a known poll
/// interval (report-by-exception sources such as OPC UA subscriptions, where
/// an unchanged value is not re-sent) age says nothing, so nothing goes stale.
pub fn effective(
    stored: Quality,
    ts_ms: i64,
    now_ms: i64,
    poll_interval_ms: Option<u64>,
) -> Quality {
    match poll_interval_ms {
        Some(interval) if stored.is_good() && now_ms - ts_ms > stale_after_ms(interval) => {
            Quality::UncertainStale
        }
        _ => stored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_snake_case_strings() {
        assert_eq!(
            serde_json::to_string(&Quality::BadCommFailure).unwrap(),
            "\"bad_comm_failure\""
        );
        for q in [
            Quality::Good,
            Quality::Uncertain,
            Quality::UncertainStale,
            Quality::Bad,
            Quality::BadCommFailure,
            Quality::BadDeviceError,
            Quality::BadConfigError,
        ] {
            assert_eq!(
                serde_json::to_string(&q).unwrap(),
                format!("\"{}\"", q.as_str())
            );
        }
    }

    #[test]
    fn families() {
        assert!(Quality::Good.is_good());
        assert!(Quality::BadDeviceError.is_bad());
        assert!(!Quality::BadDeviceError.is_uncertain());
        assert!(Quality::UncertainStale.is_uncertain());
        assert!(!Quality::UncertainStale.is_good());
    }

    #[test]
    fn worst_prefers_bad_then_uncertain() {
        assert_eq!(Quality::Good.worst(Quality::Uncertain), Quality::Uncertain);
        assert_eq!(Quality::Bad.worst(Quality::Uncertain), Quality::Bad);
        assert_eq!(
            Quality::UncertainStale.worst(Quality::BadCommFailure),
            Quality::BadCommFailure
        );
    }

    #[test]
    fn nan_implies_bad() {
        assert_eq!(Quality::implied_by(1.5), Quality::Good);
        assert_eq!(Quality::implied_by(f64::NAN), Quality::Bad);
        assert_eq!(Quality::implied_by(f64::INFINITY), Quality::Bad);
    }

    #[test]
    fn stale_after_three_intervals_with_a_floor() {
        assert_eq!(stale_after_ms(1000), 5_000);
        assert_eq!(stale_after_ms(10_000), 30_000);
        let now = 1_000_000;
        // 1 s poll: fresh at 5 s old, stale past it.
        assert_eq!(
            effective(Quality::Good, now - 5_000, now, Some(1000)),
            Quality::Good
        );
        assert_eq!(
            effective(Quality::Good, now - 5_001, now, Some(1000)),
            Quality::UncertainStale
        );
        // 10 s poll: 20 s old is still fresh.
        assert_eq!(
            effective(Quality::Good, now - 20_000, now, Some(10_000)),
            Quality::Good
        );
    }

    #[test]
    fn stale_never_hides_bad_and_needs_an_interval() {
        let now = 1_000_000;
        assert_eq!(
            effective(Quality::BadCommFailure, 0, now, Some(1000)),
            Quality::BadCommFailure
        );
        assert_eq!(effective(Quality::Good, 0, now, None), Quality::Good);
    }
}
