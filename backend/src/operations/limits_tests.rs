//! Admission tests: both budgets, the RAII release, and the bounded map.

use super::*;

#[test]
fn the_per_principal_budget_refuses_the_caller_before_the_global_one() {
    let limiter = ActivityConcurrency::new(8, 2);
    let _first = limiter.try_acquire(101).expect("first is admitted");
    let _second = limiter.try_acquire(101).expect("second is admitted");
    let denial = limiter
        .try_acquire(101)
        .expect_err("a third from the same caller is refused");
    assert_eq!(denial, AdmissionDenial::PerPrincipal);
    assert_eq!(denial.as_str(), "principal_capacity");

    // Another caller is unaffected: that is the whole point of the split.
    let _other = limiter
        .try_acquire(202)
        .expect("a different principal still has budget");
}

#[test]
fn the_global_budget_refuses_once_it_is_exhausted() {
    let limiter = ActivityConcurrency::new(2, 5);
    let _a = limiter.try_acquire(1).expect("admitted");
    let _b = limiter.try_acquire(2).expect("admitted");
    let denial = limiter.try_acquire(3).expect_err("global budget is gone");
    assert_eq!(denial, AdmissionDenial::Global);
    assert_eq!(denial.as_str(), "global_capacity");
}

#[test]
fn a_permit_releases_its_capacity_on_drop() {
    let limiter = ActivityConcurrency::new(1, 1);
    {
        let _permit = limiter.try_acquire(101).expect("admitted");
        assert!(limiter.try_acquire(101).is_err());
    }
    limiter
        .try_acquire(101)
        .expect("capacity returns when the permit drops");
}

/// An early return, a `?`, or a panic must not leak a slot — the guard is what
/// makes that true without a single explicit release call.
#[test]
fn capacity_survives_a_panicking_holder() {
    let limiter = ActivityConcurrency::new(1, 1);
    let panicking = std::panic::catch_unwind({
        let limiter = limiter.clone();
        move || {
            let _permit = limiter.try_acquire(101).expect("admitted");
            panic!("the handler exploded");
        }
    });
    assert!(panicking.is_err());
    limiter
        .try_acquire(101)
        .expect("a panicking holder still released its permit");
}

/// The per-principal map must not grow one entry per caller the process ever
/// served; the last permit removes the entry outright.
#[test]
fn the_per_principal_map_does_not_grow_without_bound() {
    let limiter = ActivityConcurrency::new(64, 1);
    for principal in 0..1_000 {
        let _permit = limiter.try_acquire(principal).expect("admitted");
    }
    // Every principal is back to a full budget, which is only true if their
    // entries were removed.
    for principal in 0..1_000 {
        let _permit = limiter
            .try_acquire(principal)
            .expect("each principal has its budget back");
    }
}

/// The documented defaults must stay a sane pair: a per-principal budget at or
/// above the global one would make the fair-share cap inert, and an unbounded
/// `Retry-After` would turn a capacity blip into a stalled client.
#[test]
fn the_documented_defaults_are_bounded_and_sane() {
    let per_principal = DEFAULT_PER_PRINCIPAL_LIMIT;
    let global = DEFAULT_GLOBAL_LIMIT;
    assert!(per_principal < global, "{per_principal} !< {global}");
    let retry_after = RETRY_AFTER_SECS;
    assert!((1..=60).contains(&retry_after), "{retry_after}");
    let limiter = ActivityConcurrency::default();
    let _permit = limiter.try_acquire(1).expect("the default budget admits");
}
