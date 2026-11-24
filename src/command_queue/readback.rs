//! Write read-back: an acknowledged write is only known to have landed when
//! a later poll reads the commanded value back from the device.
//!
//! The tracker is pure (timestamps are passed in) so the outcome logic is
//! tested without a device or a clock.

use crate::cache::Reading;

/// Fresh polls that must disagree before a write is called a mismatch. The
/// first poll after the ack may have been read before the device applied
/// the write, so one disagreeing poll is not proof yet.
pub const MISMATCH_POLLS: u32 = 2;
/// The shortest wait for a read-back, whatever the poll interval.
pub const MIN_TIMEOUT_MS: i64 = 10_000;
/// Poll intervals to wait for a read-back.
pub const TIMEOUT_POLLS: i64 = 3;

/// How long to wait for a read-back of a device polled every `interval_ms`
/// (unknown for report-by-exception sources).
pub fn timeout_ms(interval_ms: Option<u64>) -> i64 {
    interval_ms
        .map(|ms| (ms as i64).saturating_mul(TIMEOUT_POLLS))
        .unwrap_or(0)
        .max(MIN_TIMEOUT_MS)
}

/// How far a read-back may be from the commanded value and still match.
/// The tag's write deadband when it has one; otherwise integers (and
/// booleans) must match up to float noise from scaling, and fractions up
/// to what a 32-bit float register keeps.
pub fn tolerance(expected: f64, write_deadband: Option<f64>) -> f64 {
    match write_deadband {
        Some(db) if db > 0.0 => db,
        _ if expected.fract() == 0.0 => 1e-6,
        _ => (expected.abs() * 1e-5).max(1e-9),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Confirmed { value: f64, ts_ms: i64 },
    Mismatch { value: f64, ts_ms: i64 },
    Timeout,
}

impl Outcome {
    /// The value of the `readback_status` column.
    pub fn status(&self) -> &'static str {
        match self {
            Outcome::Confirmed { .. } => "confirmed",
            Outcome::Mismatch { .. } => "mismatch",
            Outcome::Timeout => "timeout",
        }
    }
}

/// One acknowledged write waiting to be read back.
#[derive(Debug, Clone)]
pub struct PendingReadback {
    pub command_id: String,
    /// Device and metric names as the cache knows them.
    pub device_id: String,
    pub metric: String,
    pub expected: f64,
    pub tolerance: f64,
    pub acked_at_ms: i64,
    pub deadline_ms: i64,
    /// Fresh good polls compared so far.
    pub attempts: u32,
    last_ts_ms: i64,
    last_mismatch: Option<(f64, i64)>,
}

impl PendingReadback {
    pub fn new(
        command_id: String,
        device_id: String,
        metric: String,
        expected: f64,
        tolerance: f64,
        acked_at_ms: i64,
        timeout_ms: i64,
    ) -> Self {
        Self {
            command_id,
            device_id,
            metric,
            expected,
            tolerance,
            acked_at_ms,
            deadline_ms: acked_at_ms + timeout_ms,
            attempts: 0,
            last_ts_ms: acked_at_ms,
            last_mismatch: None,
        }
    }

    /// Look at the tag as it reads at `now_ms`; `Some` once the outcome is known.
    pub fn check(&mut self, reading: Option<Reading>, now_ms: i64) -> Option<Outcome> {
        if let Some(r) = reading
            && r.quality.is_good()
            && r.ts_ms > self.last_ts_ms
        {
            // Only a poll newer than the ack (and than the last one compared)
            // says anything about the write.
            self.last_ts_ms = r.ts_ms;
            self.attempts += 1;
            if (r.value - self.expected).abs() <= self.tolerance {
                return Some(Outcome::Confirmed {
                    value: r.value,
                    ts_ms: r.ts_ms,
                });
            }
            self.last_mismatch = Some((r.value, r.ts_ms));
            if self.attempts >= MISMATCH_POLLS {
                return Some(Outcome::Mismatch {
                    value: r.value,
                    ts_ms: r.ts_ms,
                });
            }
        }
        if now_ms >= self.deadline_ms {
            // A poll that disagreed is evidence; no poll at all is a timeout.
            return Some(match self.last_mismatch {
                Some((value, ts_ms)) => Outcome::Mismatch { value, ts_ms },
                None => Outcome::Timeout,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::Quality;

    const ACK: i64 = 1_000_000;

    fn pending(expected: f64) -> PendingReadback {
        PendingReadback::new(
            "c1".into(),
            "plc".into(),
            "sp".into(),
            expected,
            tolerance(expected, None),
            ACK,
            timeout_ms(Some(1000)),
        )
    }

    fn good(value: f64, ts_ms: i64) -> Option<Reading> {
        Some(Reading {
            value,
            quality: Quality::Good,
            ts_ms,
        })
    }

    #[test]
    fn timeout_is_three_polls_or_ten_seconds() {
        assert_eq!(timeout_ms(Some(1000)), 10_000);
        assert_eq!(timeout_ms(Some(5000)), 15_000);
        assert_eq!(timeout_ms(None), 10_000);
    }

    #[test]
    fn tolerance_follows_the_value() {
        assert_eq!(tolerance(42.0, Some(0.5)), 0.5);
        assert_eq!(tolerance(42.0, Some(0.0)), 1e-6);
        assert_eq!(tolerance(1.0, None), 1e-6);
        assert!((tolerance(3.3, None) - 3.3e-5).abs() < 1e-12);
    }

    #[test]
    fn confirmed_by_the_first_fresh_good_poll() {
        let mut p = pending(42.0);
        // The value polled before the ack does not count, even if it matches.
        assert_eq!(p.check(good(42.0, ACK - 1), ACK + 100), None);
        assert_eq!(p.attempts, 0);
        // Scaling noise still matches an integer.
        assert_eq!(
            p.check(good(42.000_000_000_000_01, ACK + 500), ACK + 600),
            Some(Outcome::Confirmed {
                value: 42.000_000_000_000_01,
                ts_ms: ACK + 500
            })
        );
        assert_eq!(p.attempts, 1);
    }

    #[test]
    fn a_float_register_matches_within_its_precision() {
        let mut p = pending(3.3);
        let read_back = 3.3_f32 as f64;
        assert!(matches!(
            p.check(good(read_back, ACK + 1), ACK + 1),
            Some(Outcome::Confirmed { .. })
        ));
    }

    #[test]
    fn mismatch_after_two_fresh_polls_disagree() {
        let mut p = pending(42.0);
        assert_eq!(p.check(good(41.0, ACK + 500), ACK + 600), None);
        // The same poll seen again is not a second attempt.
        assert_eq!(p.check(good(41.0, ACK + 500), ACK + 1100), None);
        assert_eq!(p.attempts, 1);
        assert_eq!(
            p.check(good(40.0, ACK + 1500), ACK + 1600),
            Some(Outcome::Mismatch {
                value: 40.0,
                ts_ms: ACK + 1500
            })
        );
        assert_eq!(p.attempts, 2);
    }

    #[test]
    fn a_late_apply_is_still_confirmed() {
        let mut p = pending(1.0);
        assert_eq!(p.check(good(0.0, ACK + 200), ACK + 300), None);
        assert!(matches!(
            p.check(good(1.0, ACK + 1200), ACK + 1300),
            Some(Outcome::Confirmed { .. })
        ));
    }

    #[test]
    fn timeout_without_a_fresh_good_poll() {
        let mut p = pending(42.0);
        let bad = Some(Reading {
            value: f64::NAN,
            quality: Quality::BadCommFailure,
            ts_ms: ACK + 500,
        });
        assert_eq!(p.check(bad, ACK + 9_999), None);
        assert_eq!(p.check(None, ACK + 10_000), Some(Outcome::Timeout));
        assert_eq!(p.attempts, 0);
    }

    #[test]
    fn one_disagreeing_poll_then_silence_is_a_mismatch() {
        let mut p = pending(42.0);
        assert_eq!(p.check(good(7.0, ACK + 500), ACK + 600), None);
        assert_eq!(
            p.check(good(7.0, ACK + 500), ACK + 10_000),
            Some(Outcome::Mismatch {
                value: 7.0,
                ts_ms: ACK + 500
            })
        );
    }
}
