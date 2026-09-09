//! The bounded suppression gate: what stops one unfixable runtime from
//! producing a warning and a lifecycle event on every 30-second sweep.

use std::time::Duration;

use super::*;

#[test]
fn a_fresh_gate_allows_every_session() {
    let gate = IdentityGate::new();
    assert!(gate.allow("sess-a"));
    assert!(gate.allow("sess-b"));
    assert!(gate.is_empty());
}

#[test]
fn a_suppressed_session_is_declined_until_its_cooldown_lapses() {
    let gate = IdentityGate::new();
    gate.suppress("sess-a", PERMANENT_COOLDOWN);
    assert!(!gate.allow("sess-a"));
    assert!(
        gate.allow("sess-b"),
        "suppression is per session, never a global stop"
    );
    assert_eq!(gate.len(), 1);
}

#[test]
fn an_elapsed_cooldown_releases_the_session() {
    let gate = IdentityGate::new();
    // A zero-length cooldown is already in the past by the time it is read.
    gate.suppress("sess-a", Duration::from_nanos(1));
    std::thread::sleep(Duration::from_millis(2));
    assert!(gate.allow("sess-a"));
    assert!(
        gate.is_empty(),
        "expired entries are pruned, not accumulated"
    );
}

#[test]
fn a_longer_cooldown_is_never_shortened_by_a_later_shorter_one() {
    let gate = IdentityGate::new();
    gate.suppress("sess-a", PERMANENT_COOLDOWN);
    // A settle after a conflict must not release the conflict early: the
    // conflict is the decision that needs a human.
    gate.suppress("sess-a", SETTLE_COOLDOWN);
    assert!(!gate.allow("sess-a"));
}

#[test]
fn a_shorter_cooldown_is_extended_by_a_later_longer_one() {
    let gate = IdentityGate::new();
    gate.suppress("sess-a", SETTLE_COOLDOWN);
    gate.suppress("sess-a", PERMANENT_COOLDOWN);
    assert!(!gate.allow("sess-a"));
    assert_eq!(gate.len(), 1);
}

#[test]
fn the_cooldowns_are_ordered_so_a_settle_never_outlasts_a_permanent_failure() {
    assert!(SETTLE_COOLDOWN < PERMANENT_COOLDOWN);
}

#[test]
fn the_debug_rendering_carries_a_count_and_no_session_id() {
    let gate = IdentityGate::new();
    gate.suppress("sess-secret", PERMANENT_COOLDOWN);
    let rendered = format!("{gate:?}");
    assert!(rendered.contains("suppressed"), "{rendered}");
    assert!(
        !rendered.contains("sess-secret"),
        "a `{{:?}}` of the reconcile context must not dump session ids: {rendered}"
    );
}
