//! ISA-18.2 style alarm on/off delays.
//!
//! A pure state machine used by the alarm evaluation loop: a raised condition
//! must persist for `on_delay_secs` before the alarm fires, and the normal
//! condition must persist for `off_delay_secs` before the alarm resolves.
//! Time is injected by the caller so the behavior is deterministic and
//! unit-testable. Delay states live in memory only — they start empty on
//! startup and recompute from the current value, so no stale pending survives
//! a restart.
//!
//! States are tracked per source key, not per severity: the delay measures
//! "this tag is in an alarm condition" regardless of which threshold was
//! crossed. The severity recorded in the alarm is the one active when the
//! delay elapses. With both delays at 0 the caller skips this module entirely
//! and keeps the immediate historical behavior.

/// Per-source delay bookkeeping for the evaluation loop.
#[derive(Debug, Clone, Default)]
pub struct DelayState {
    /// Timestamp the raised condition was first seen (on-delay pending).
    pub pending_since: Option<i64>,
    /// Timestamp the normal condition was first seen while an alarm is
    /// active (off-delay pending).
    pub clear_pending_since: Option<i64>,
}

/// Outcome of one evaluation tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelayTransition {
    /// Keep waiting — the condition has not persisted long enough yet.
    Pending,
    /// Raised condition sustained past `on_delay_secs` — fire the alarm.
    Fire,
    /// Normal condition sustained past `off_delay_secs` — resolve the alarm.
    Resolve,
}

/// Advance the delay state machine by one evaluation tick.
///
/// `now` is the current unix timestamp in seconds (injected for tests).
/// With `on_delay_secs <= 0` a raised condition fires immediately; with
/// `off_delay_secs <= 0` a normal condition resolves immediately.
pub fn update_delay(
    state: &mut DelayState,
    condition_active: bool,
    on_delay_secs: i64,
    off_delay_secs: i64,
    now: i64,
) -> DelayTransition {
    if condition_active {
        state.clear_pending_since = None;
        if on_delay_secs <= 0 {
            state.pending_since = None;
            return DelayTransition::Fire;
        }
        match state.pending_since {
            None => {
                state.pending_since = Some(now);
                DelayTransition::Pending
            }
            Some(since) if now.saturating_sub(since) >= on_delay_secs => {
                state.pending_since = None;
                DelayTransition::Fire
            }
            Some(_) => DelayTransition::Pending,
        }
    } else {
        // The raised condition dropped: any unexpired on-delay pending is a
        // blip — drop it, the alarm never fires from it.
        state.pending_since = None;
        if off_delay_secs <= 0 {
            state.clear_pending_since = None;
            return DelayTransition::Resolve;
        }
        match state.clear_pending_since {
            None => {
                state.clear_pending_since = Some(now);
                DelayTransition::Pending
            }
            Some(since) if now.saturating_sub(since) >= off_delay_secs => {
                state.clear_pending_since = None;
                DelayTransition::Resolve
            }
            Some(_) => DelayTransition::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The evaluation loop ticks roughly every persist_interval seconds; these
    // tests replay ticks with an injected clock instead of sleeping.

    #[test]
    fn test_on_delay_fires_only_after_sustained_condition() {
        let mut state = DelayState::default();

        // Tick 1 (t=100): condition raised — pending starts, no alarm yet.
        let t = update_delay(&mut state, true, 10, 0, 100);
        assert_eq!(t, DelayTransition::Pending);
        assert_eq!(state.pending_since, Some(100));

        // Tick 2 (t=105): 5s < 10s delay — still pending.
        let t = update_delay(&mut state, true, 10, 0, 105);
        assert_eq!(t, DelayTransition::Pending);

        // Tick 3 (t=110): 10s sustained — fires and clears the pending.
        let t = update_delay(&mut state, true, 10, 0, 110);
        assert_eq!(t, DelayTransition::Fire);
        assert_eq!(state.pending_since, None);
    }

    #[test]
    fn test_blip_shorter_than_on_delay_never_fires() {
        let mut state = DelayState::default();

        // 5s blip against a 10s delay: pending starts, then the condition
        // drops before it can fire.
        assert_eq!(
            update_delay(&mut state, true, 10, 0, 100),
            DelayTransition::Pending
        );
        assert_eq!(
            update_delay(&mut state, true, 10, 0, 105),
            DelayTransition::Pending
        );
        assert_eq!(
            update_delay(&mut state, false, 10, 0, 105),
            DelayTransition::Resolve
        );
        assert_eq!(state.pending_since, None, "blip must clear the pending");

        // A later raise starts a fresh delay from the new tick — it must not
        // inherit the elapsed time of the cleared blip.
        let t = update_delay(&mut state, true, 10, 0, 112);
        assert_eq!(t, DelayTransition::Pending);
        assert_eq!(state.pending_since, Some(112));
        assert_eq!(
            update_delay(&mut state, true, 10, 0, 121),
            DelayTransition::Pending
        );
        assert_eq!(
            update_delay(&mut state, true, 10, 0, 122),
            DelayTransition::Fire
        );
    }

    #[test]
    fn test_off_delay_symmetric_to_on_delay() {
        let mut state = DelayState::default();

        // Fire first.
        assert_eq!(
            update_delay(&mut state, true, 5, 10, 100),
            DelayTransition::Pending
        );
        assert_eq!(
            update_delay(&mut state, true, 5, 10, 105),
            DelayTransition::Fire
        );

        // Condition drops: alarm stays raised until the normal condition has
        // persisted for off_delay_secs.
        assert_eq!(
            update_delay(&mut state, false, 5, 10, 200),
            DelayTransition::Pending
        );
        assert_eq!(state.clear_pending_since, Some(200));
        assert_eq!(
            update_delay(&mut state, false, 5, 10, 204),
            DelayTransition::Pending
        );
        assert_eq!(
            update_delay(&mut state, false, 5, 10, 210),
            DelayTransition::Resolve
        );
        assert_eq!(state.clear_pending_since, None);

        // A blip back into the alarm condition during the clear delay resets
        // it — the alarm stays raised.
        assert_eq!(
            update_delay(&mut state, true, 5, 10, 300),
            DelayTransition::Pending
        );
        assert_eq!(
            update_delay(&mut state, false, 5, 10, 301),
            DelayTransition::Pending
        );
        assert_eq!(
            update_delay(&mut state, true, 5, 10, 305),
            DelayTransition::Pending
        );
        assert_eq!(
            state.clear_pending_since, None,
            "raise during clear delay resets it"
        );
    }

    #[test]
    fn test_zero_delay_is_immediate_and_stateless() {
        let mut state = DelayState::default();

        // delay = 0 must behave exactly like the pre-delay behavior: fire and
        // resolve on the first tick, with no state tracked.
        assert_eq!(
            update_delay(&mut state, true, 0, 0, 100),
            DelayTransition::Fire
        );
        assert_eq!(state.pending_since, None);
        assert_eq!(
            update_delay(&mut state, false, 0, 0, 101),
            DelayTransition::Resolve
        );
        assert_eq!(state.clear_pending_since, None);
    }

    #[test]
    fn test_fresh_state_after_restart_recomputes_from_current_value() {
        // On startup states begin empty: a raised condition starts a new
        // on-delay from now (no stuck pendings from before the restart).
        let mut state = DelayState::default();
        let t = update_delay(&mut state, true, 30, 0, 1_000);
        assert_eq!(t, DelayTransition::Pending);
        assert_eq!(state.pending_since, Some(1_000));
    }
}
