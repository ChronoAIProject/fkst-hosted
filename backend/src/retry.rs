//! Bounded-retry primitives shared by the reconciler's long-lived loops.
//!
//! The full-resync coordinator ([`crate::reconcile::loops`]) and the session
//! token-rotation sweep ([`crate::k8s::token_rotation`]) drive the same shape:
//! attempt now, and on a failed or PARTIAL pass retry with exponential backoff plus
//! jitter instead of falling through to the ordinary periodic cadence — which, for a
//! cadence measured in tens of minutes, is what turns one transient error into a long
//! outage. These are the two pieces of that shape, in one place so the loops cannot
//! drift apart.

use std::time::Duration;

use rand::Rng;

/// Exponential backoff between `initial_secs` and `max_secs`: each
/// [`Self::next_delay`] doubles the previous delay up to the ceiling, and
/// [`Self::reset`] returns to the floor after a successful pass.
///
/// Callers should pass their own periodic interval as `max_secs`, so a persistent
/// failure degrades to exactly the un-retried cadence and never to a slower one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RetryBackoff {
    initial_secs: u64,
    max_secs: u64,
    next_secs: u64,
}

impl RetryBackoff {
    pub(crate) fn new(initial_secs: u64, max_secs: u64) -> Self {
        let initial_secs = initial_secs.max(1);
        let max_secs = max_secs.max(initial_secs);
        Self {
            initial_secs,
            max_secs,
            next_secs: initial_secs,
        }
    }

    pub(crate) fn next_delay(&mut self) -> Duration {
        let delay = self.next_secs;
        self.next_secs = self.next_secs.saturating_mul(2).min(self.max_secs);
        Duration::from_secs(delay)
    }

    pub(crate) fn reset(&mut self) {
        self.next_secs = self.initial_secs;
    }
}

/// Spread `base` uniformly over ±`jitter_percent` so a fleet of workers retrying the
/// same dependency does not re-converge on one instant.
pub(crate) fn jittered_delay(base: Duration, jitter_percent: u64) -> Duration {
    if jitter_percent == 0 || base.is_zero() {
        return base;
    }
    let base_ms = u64::try_from(base.as_millis()).unwrap_or(u64::MAX);
    let spread_ms = base_ms.saturating_mul(jitter_percent.min(100)) / 100;
    let lower = base_ms.saturating_sub(spread_ms);
    let upper = base_ms.saturating_add(spread_ms);
    Duration::from_millis(rand::thread_rng().gen_range(lower..=upper))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_the_ceiling_and_resets() {
        let mut backoff = RetryBackoff::new(15, 120);
        let delays: Vec<u64> = (0..6).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(delays, vec![15, 30, 60, 120, 120, 120]);
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(15));
    }

    #[test]
    fn backoff_never_exceeds_the_caller_supplied_ceiling() {
        // The ceiling is how a caller guarantees "retrying is never SLOWER than not
        // retrying": pass the periodic interval and a persistent failure degrades to
        // exactly that cadence.
        let ceiling = 300;
        let mut backoff = RetryBackoff::new(15, ceiling);
        for _ in 0..20 {
            assert!(backoff.next_delay().as_secs() <= ceiling);
        }
    }

    #[test]
    fn a_ceiling_below_the_floor_is_raised_to_it() {
        // A caller whose periodic interval is shorter than the retry floor must still
        // get a usable, non-zero delay rather than a degenerate one.
        let mut backoff = RetryBackoff::new(15, 5);
        assert_eq!(backoff.next_delay(), Duration::from_secs(15));
        assert_eq!(backoff.next_delay(), Duration::from_secs(15));
    }

    #[test]
    fn zero_initial_is_clamped_so_a_retry_loop_cannot_spin() {
        let mut backoff = RetryBackoff::new(0, 60);
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    }

    #[test]
    fn jitter_stays_inside_the_requested_band() {
        let base = Duration::from_secs(60);
        for _ in 0..256 {
            let delay = jittered_delay(base, 20);
            assert!(
                delay >= Duration::from_secs(48) && delay <= Duration::from_secs(72),
                "±20% of 60s, got {delay:?}"
            );
        }
    }

    #[test]
    fn zero_jitter_and_zero_base_are_returned_unchanged() {
        assert_eq!(
            jittered_delay(Duration::from_secs(60), 0),
            Duration::from_secs(60)
        );
        assert_eq!(jittered_delay(Duration::ZERO, 20), Duration::ZERO);
    }
}
