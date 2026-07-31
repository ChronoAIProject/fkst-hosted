//! The derived-timing rules: unlimited is null, missing is null, negative is
//! clamped-and-announced, and idle matches the reconciler exactly.

use k8s_openapi::chrono::TimeDelta;

use super::*;

fn ts(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .expect("rfc3339")
        .with_timezone(&Utc)
}

/// Idle grace 300s, minimum lifetime 120s, maximum lifetime `max`.
fn policy(max: u64) -> RuntimeLifetimePolicy {
    RuntimeLifetimePolicy {
        max_lifetime_seconds: max,
        minimum_lifetime_seconds: 120,
        idle_grace_seconds: 300,
        max_items: 5000,
    }
}

#[test]
fn a_bounded_lifetime_yields_expiry_and_remaining() {
    let observed = ts("2026-07-01T12:00:00Z");
    let created = ts("2026-07-01T11:00:00Z");
    let (timing, codes) = compute(observed, Some(created), None, &policy(7200));
    assert!(codes.is_empty(), "{codes:?}");
    assert_eq!(timing.age_seconds, Some(3600));
    assert_eq!(timing.max_lifetime_seconds, Some(7200));
    assert_eq!(timing.expires_at, Some(ts("2026-07-01T13:00:00Z")));
    assert_eq!(timing.remaining_seconds, Some(3600));
    // Minimum-lifetime shield already elapsed (3600s alive vs a 120s shield).
    assert_eq!(timing.minimum_lifetime_remaining_seconds, Some(0));
    assert_eq!(timing.minimum_lifetime_seconds, 120);
    assert_eq!(timing.idle_grace_seconds, 300);
}

#[test]
fn an_unlimited_lifetime_is_null_never_zero_remaining() {
    // `FKST_POD_SESSION_MAX_LIFETIME_SECS=0` means "runs until idle or trigger
    // close"; reporting 0 remaining would read as "about to be killed".
    let observed = ts("2026-07-01T12:00:00Z");
    let (timing, codes) = compute(observed, Some(ts("2026-07-01T11:00:00Z")), None, &policy(0));
    assert!(codes.is_empty(), "{codes:?}");
    assert_eq!(timing.max_lifetime_seconds, None);
    assert_eq!(timing.expires_at, None);
    assert_eq!(timing.remaining_seconds, None);
    // Age is still knowable and still reported.
    assert_eq!(timing.age_seconds, Some(3600));
}

#[test]
fn an_expired_runtime_reports_zero_remaining_not_a_negative_countdown() {
    let observed = ts("2026-07-01T15:00:00Z");
    let (timing, _) = compute(
        observed,
        Some(ts("2026-07-01T11:00:00Z")),
        None,
        &policy(3600),
    );
    assert_eq!(timing.expires_at, Some(ts("2026-07-01T12:00:00Z")));
    assert_eq!(timing.remaining_seconds, Some(0));
}

#[test]
fn a_missing_creation_time_yields_null_derivations_and_a_warning() {
    // Substituting `now` here would render an ancient orphan as brand new.
    let (timing, codes) = compute(ts("2026-07-01T12:00:00Z"), None, None, &policy(3600));
    assert_eq!(timing.age_seconds, None);
    assert_eq!(timing.expires_at, None);
    assert_eq!(timing.remaining_seconds, None);
    assert_eq!(timing.minimum_lifetime_remaining_seconds, None);
    assert_eq!(timing.idle_for_seconds, None);
    assert_eq!(codes, vec![InventoryWarningCode::MissingCreatedAt]);
    // The configured maximum is still displayed — the policy is knowable even
    // when this runtime's position within it is not.
    assert_eq!(timing.max_lifetime_seconds, Some(3600));
}

#[test]
fn a_future_creation_time_clamps_to_zero_and_warns() {
    let observed = ts("2026-07-01T12:00:00Z");
    let (timing, codes) = compute(observed, Some(ts("2026-07-01T12:05:00Z")), None, &policy(0));
    assert_eq!(timing.age_seconds, Some(0));
    assert_eq!(timing.idle_for_seconds, Some(0));
    assert!(
        codes.contains(&InventoryWarningCode::ClockSkew),
        "{codes:?}"
    );
    // One skew code per row even though both the age and the idle derivation saw it.
    assert_eq!(
        codes
            .iter()
            .filter(|c| **c == InventoryWarningCode::ClockSkew)
            .count(),
        1
    );
}

#[test]
fn idle_starts_at_last_pending_when_present() {
    let observed = ts("2026-07-01T12:00:00Z");
    let (timing, _) = compute(
        observed,
        Some(ts("2026-07-01T09:00:00Z")),
        Some(ts("2026-07-01T11:30:00Z")),
        &policy(0),
    );
    assert_eq!(timing.age_seconds, Some(10_800));
    assert_eq!(timing.idle_for_seconds, Some(1800));
}

#[test]
fn idle_falls_back_to_creation_exactly_as_the_reconciler_does() {
    // `desired::idle_kill_due` uses `last_pending_at.unwrap_or(created_at)`;
    // diverging here would show an operator a different idle age than the one the
    // reconciler is about to act on.
    let observed = ts("2026-07-01T12:00:00Z");
    let (timing, _) = compute(observed, Some(ts("2026-07-01T11:00:00Z")), None, &policy(0));
    assert_eq!(timing.idle_for_seconds, timing.age_seconds);
}

#[test]
fn a_last_pending_without_a_creation_time_still_yields_idle() {
    let observed = ts("2026-07-01T12:00:00Z");
    let (timing, codes) = compute(observed, None, Some(ts("2026-07-01T11:00:00Z")), &policy(0));
    assert_eq!(timing.idle_for_seconds, Some(3600));
    assert_eq!(timing.age_seconds, None);
    assert_eq!(codes, vec![InventoryWarningCode::MissingCreatedAt]);
}

#[test]
fn the_minimum_lifetime_shield_counts_down_from_creation() {
    let observed = ts("2026-07-01T12:00:30Z");
    let (timing, _) = compute(observed, Some(ts("2026-07-01T12:00:00Z")), None, &policy(0));
    assert_eq!(timing.age_seconds, Some(30));
    assert_eq!(timing.minimum_lifetime_remaining_seconds, Some(90));
}

#[test]
fn an_overflowing_lifetime_nulls_expiry_and_warns_but_keeps_the_policy() {
    let created = DateTime::<Utc>::MAX_UTC - TimeDelta::try_seconds(10).expect("delta");
    let (timing, codes) = compute(
        DateTime::<Utc>::MAX_UTC,
        Some(created),
        None,
        &policy(u64::MAX),
    );
    assert_eq!(timing.max_lifetime_seconds, Some(u64::MAX));
    assert_eq!(timing.expires_at, None);
    assert_eq!(timing.remaining_seconds, None);
    assert!(
        codes.contains(&InventoryWarningCode::LifetimeOverflow),
        "{codes:?}"
    );
}

#[test]
fn an_extreme_but_representable_configuration_does_not_panic() {
    // i64-representable seconds that still push the sum past the chrono range.
    let created = ts("2026-07-01T12:00:00Z");
    let (timing, codes) = compute(created, Some(created), None, &policy(i64::MAX as u64));
    assert_eq!(timing.expires_at, None);
    assert!(codes.contains(&InventoryWarningCode::LifetimeOverflow));
}
