//! Derived lifetime / idle timings, computed against ONE snapshot instant.
//!
//! Every duration an inventory item reports is derived here, from the snapshot's
//! single `observed_at` — never from a fresh `Utc::now()` per field, which would
//! let two fields of the same row disagree about the present.
//!
//! Three rules, each protecting against a specific way an operations view lies:
//!
//! - **Unlimited is null, not zero.** `FKST_POD_SESSION_MAX_LIFETIME_SECS=0` means
//!   a session runs until it idles or its trigger closes. Reporting
//!   `remaining_seconds = 0` for it would read as "about to be killed", which is
//!   the exact opposite of the truth.
//! - **A missing timestamp yields null, never `now`.** Defaulting an absent
//!   creation time to the observation instant would render an ancient orphan as a
//!   zero-second-old runtime.
//! - **Negative is clamped and announced.** Clocks skew; a runtime "created in the
//!   future" reports zero age with a [`InventoryWarningCode::ClockSkew`] warning,
//!   never a wrapped-around duration.
//!
//! Idle semantics mirror [`crate::reconcile::desired`]'s `idle_kill_due` EXACTLY:
//! the idle clock starts at `last_pending_at` and falls back to `created_at`. A
//! divergence here would show an operator a different idle age than the one the
//! reconciler is about to act on.

use k8s_openapi::chrono::{DateTime, TimeDelta, Utc};

use super::warning::InventoryWarningCode;
use super::RuntimeLifetimePolicy;

/// The derived timing block of one inventory item.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeTiming {
    pub age_seconds: Option<u64>,
    pub max_lifetime_seconds: Option<u64>,
    pub expires_at: Option<DateTime<Utc>>,
    pub remaining_seconds: Option<u64>,
    pub minimum_lifetime_seconds: u64,
    pub minimum_lifetime_remaining_seconds: Option<u64>,
    pub idle_grace_seconds: u64,
    pub idle_for_seconds: Option<u64>,
}

/// Compute one runtime's timing block, returning the warning codes the derivation
/// produced. Pure: the caller attaches runtime/session correlation to the codes.
pub fn compute(
    observed_at: DateTime<Utc>,
    created_at: Option<DateTime<Utc>>,
    last_pending_at: Option<DateTime<Utc>>,
    policy: &RuntimeLifetimePolicy,
) -> (RuntimeTiming, Vec<InventoryWarningCode>) {
    let mut codes = Vec::new();
    let mut timing = RuntimeTiming {
        minimum_lifetime_seconds: policy.minimum_lifetime_seconds,
        idle_grace_seconds: policy.idle_grace_seconds,
        // Zero means unlimited; every derived lifetime field then stays null.
        max_lifetime_seconds: (policy.max_lifetime_seconds > 0)
            .then_some(policy.max_lifetime_seconds),
        ..RuntimeTiming::default()
    };

    if let Some(created_at) = created_at {
        let age = elapsed_seconds(created_at, observed_at, &mut codes);
        timing.age_seconds = Some(age);
        timing.minimum_lifetime_remaining_seconds =
            Some(policy.minimum_lifetime_seconds.saturating_sub(age));
        if let Some(max) = timing.max_lifetime_seconds {
            match expiry_of(created_at, max) {
                Some(expires_at) => {
                    timing.expires_at = Some(expires_at);
                    // A past expiry is 0 remaining, not a negative countdown; the
                    // reconciler has simply not swept it yet.
                    timing.remaining_seconds =
                        Some(elapsed_seconds(observed_at, expires_at, &mut Vec::new()));
                }
                None => codes.push(InventoryWarningCode::LifetimeOverflow),
            }
        }
    } else {
        codes.push(InventoryWarningCode::MissingCreatedAt);
    }

    // The idle clock: last-pending, falling back to creation — the reconciler's
    // own rule. With neither, idle age is genuinely unknowable.
    if let Some(idle_since) = last_pending_at.or(created_at) {
        timing.idle_for_seconds = Some(elapsed_seconds(idle_since, observed_at, &mut codes));
    }

    (timing, codes)
}

/// Whole seconds between two instants, clamped at zero.
///
/// Computed on the two epoch-second values with `checked_sub` rather than on a
/// `TimeDelta`, so an extreme timestamp pair can never wrap or panic; an
/// unrepresentable difference degrades to zero with a skew warning.
fn elapsed_seconds(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    codes: &mut Vec<InventoryWarningCode>,
) -> u64 {
    let Some(delta) = to.timestamp().checked_sub(from.timestamp()) else {
        push_once(codes, InventoryWarningCode::ClockSkew);
        return 0;
    };
    match u64::try_from(delta) {
        Ok(seconds) => seconds,
        // Negative: the "from" instant is in the future relative to "to".
        Err(_) => {
            push_once(codes, InventoryWarningCode::ClockSkew);
            0
        }
    }
}

/// `created_at + max_lifetime`, or `None` when the sum is not representable.
fn expiry_of(created_at: DateTime<Utc>, max_lifetime_seconds: u64) -> Option<DateTime<Utc>> {
    let seconds = i64::try_from(max_lifetime_seconds).ok()?;
    let delta = TimeDelta::try_seconds(seconds)?;
    created_at.checked_add_signed(delta)
}

/// One code per snapshot row is enough; a skewed clock affecting both the age and
/// the idle derivation should not report itself twice.
fn push_once(codes: &mut Vec<InventoryWarningCode>, code: InventoryWarningCode) {
    if !codes.contains(&code) {
        codes.push(code);
    }
}

#[cfg(test)]
#[path = "timing_tests.rs"]
mod tests;
