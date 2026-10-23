#![allow(dead_code)]
use std::time::{Duration, Instant};

/// Per-command readback verification state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadbackState {
    Pending,
    Sent,
    Polling,
    Verified,
    Timeout,
    Mismatch,
    Rejected,
}

/// Tracks readback verification for a single command.
pub struct ReadbackTracker {
    pub state: ReadbackState,
    pub expected_value: f64,
    pub actual_value: Option<f64>,
    pub attempts: u32,
    pub max_attempts: u32,
    pub poll_interval: Duration,
    pub timeout: Duration,
    pub started_at: Option<Instant>,
    pub last_poll: Option<Instant>,
    pub verified_at: Option<Instant>,
}

impl ReadbackTracker {
    pub fn new(expected_value: f64, config: &crate::models::ReadbackConfig) -> Self {
        Self {
            state: ReadbackState::Pending,
            expected_value,
            actual_value: None,
            attempts: 0,
            max_attempts: config.retry_count,
            poll_interval: Duration::from_millis(config.poll_interval_ms),
            timeout: Duration::from_millis(config.timeout_ms),
            started_at: None,
            last_poll: None,
            verified_at: None,
        }
    }

    /// Transition to Sent state (write transmitted).
    pub fn mark_sent(&mut self) {
        self.state = ReadbackState::Sent;
        self.started_at = Some(Instant::now());
    }

    /// Transition to Polling state.
    pub fn start_polling(&mut self) {
        self.state = ReadbackState::Polling;
        self.last_poll = Some(Instant::now());
        self.attempts += 1;
    }

    /// Check if we should poll now (interval elapsed).
    pub fn should_poll(&self) -> bool {
        match self.last_poll {
            Some(last) => last.elapsed() >= self.poll_interval,
            None => true,
        }
    }

    /// Check if readback has timed out.
    pub fn is_timed_out(&self) -> bool {
        match self.started_at {
            Some(started) => started.elapsed() >= self.timeout,
            None => false,
        }
    }

    /// Verify a readback value against expected.
    /// Returns true if verified, false if mismatch (and retries remain).
    pub fn verify(&mut self, actual: f64) -> bool {
        self.actual_value = Some(actual);

        let tolerance = (self.expected_value.abs() * 0.001).max(0.001);
        if (actual - self.expected_value).abs() <= tolerance {
            self.state = ReadbackState::Verified;
            self.verified_at = Some(Instant::now());
            true
        } else if self.attempts >= self.max_attempts {
            self.state = ReadbackState::Mismatch;
            false
        } else {
            false
        }
    }

    /// Mark as timed out.
    pub fn mark_timeout(&mut self) {
        self.state = ReadbackState::Timeout;
    }

    /// Mark as rejected by device.
    pub fn mark_rejected(&mut self) {
        self.state = ReadbackState::Rejected;
    }

    /// Whether this tracker is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            ReadbackState::Verified
                | ReadbackState::Timeout
                | ReadbackState::Mismatch
                | ReadbackState::Rejected
        )
    }

    /// Time since readback started, in milliseconds.
    pub fn elapsed_ms(&self) -> Option<u64> {
        self.started_at.map(|t| t.elapsed().as_millis() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ReadbackConfig;

    fn test_config() -> ReadbackConfig {
        ReadbackConfig {
            enabled: true,
            timeout_ms: 5000,
            retry_count: 2,
            poll_interval_ms: 100,
        }
    }

    #[test]
    fn test_verified_on_match() {
        let mut tracker = ReadbackTracker::new(1.0, &test_config());
        tracker.mark_sent();
        tracker.start_polling();
        assert!(tracker.verify(1.0));
        assert_eq!(tracker.state, ReadbackState::Verified);
    }

    #[test]
    fn test_mismatch_after_retries() {
        let mut tracker = ReadbackTracker::new(1.0, &test_config());
        tracker.mark_sent();
        tracker.start_polling();
        assert!(!tracker.verify(0.5));
        tracker.start_polling();
        assert!(!tracker.verify(0.5));
        assert_eq!(tracker.state, ReadbackState::Mismatch);
    }

    #[test]
    fn test_timeout() {
        let mut tracker = ReadbackTracker::new(
            1.0,
            &ReadbackConfig {
                timeout_ms: 1,
                ..test_config()
            },
        );
        tracker.mark_sent();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(tracker.is_timed_out());
        tracker.mark_timeout();
        assert_eq!(tracker.state, ReadbackState::Timeout);
    }
}
