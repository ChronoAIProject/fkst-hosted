//! Unit tests for [`super::ClaimMap`] (split out to keep `claims.rs` under the
//! 500-line file budget). Included via `#[path] mod tests;` from `claims.rs`, so
//! `super::*` resolves to the `claims` module's items.

use super::*;

fn uuid() -> bson::Uuid {
    bson::Uuid::new()
}

#[test]
fn claim_then_conflict_for_different_session() {
    let m = ClaimMap::new();
    let s1 = uuid();
    m.claim("pkg", s1, None, "w1").unwrap();
    let err = m.claim("pkg", uuid(), None, "w1").unwrap_err();
    assert_eq!(err, ClaimError::AlreadyClaimed("pkg".to_string()));
}

#[test]
fn claim_idempotent_for_same_session() {
    let m = ClaimMap::new();
    let s1 = uuid();
    let first = m.claim("pkg", s1, None, "w1").unwrap();
    let again = m.claim("pkg", s1, None, "w1").unwrap();
    assert_eq!(first, again);
    assert_eq!(first.fencing_id, again.fencing_id, "no new fence on replay");
}

#[test]
fn claim_succeeds_after_release() {
    let m = ClaimMap::new();
    let s1 = uuid();
    let e1 = m.claim("pkg", s1, None, "w1").unwrap();
    assert!(m.release("pkg", e1.fencing_id));
    // A different session can now claim the freed key.
    assert!(m.claim("pkg", uuid(), None, "w1").is_ok());
}

#[test]
fn fencing_id_is_strictly_monotonic() {
    let m = ClaimMap::new();
    let mut last = 0;
    for i in 0..5 {
        let e = m.claim(&format!("pkg-{i}"), uuid(), None, "w1").unwrap();
        assert!(e.fencing_id > last, "fence must strictly increase");
        last = e.fencing_id;
    }
    let r = m.reassign("pkg-0", "w2").unwrap();
    assert!(r.fencing_id > last);
}

#[test]
fn set_status_guard_miss_on_stale_fence() {
    let m = ClaimMap::new();
    let e = m.claim("pkg", uuid(), None, "w1").unwrap();
    let stale_fence = e.fencing_id;
    m.reassign("pkg", "w2").unwrap(); // bumps the fence
                                      // A superseded worker writing with the pre-reassign fence is rejected.
    assert!(!m.set_status(
        "pkg",
        stale_fence,
        &[ClaimStatus::Pending],
        ClaimStatus::Running
    ));
}

#[test]
fn set_status_respects_from_set() {
    let m = ClaimMap::new();
    let e = m.claim("pkg", uuid(), None, "w1").unwrap();
    // Wrong `from` set -> miss.
    assert!(!m.set_status(
        "pkg",
        e.fencing_id,
        &[ClaimStatus::Running],
        ClaimStatus::Stopping
    ));
    // Correct `from` -> apply.
    assert!(m.set_status(
        "pkg",
        e.fencing_id,
        &[ClaimStatus::Pending],
        ClaimStatus::Validating
    ));
    assert_eq!(m.get("pkg").unwrap().status, ClaimStatus::Validating);
}

#[test]
fn release_is_equality_pinned() {
    let m = ClaimMap::new();
    let e = m.claim("pkg", uuid(), None, "w1").unwrap();
    assert!(
        !m.release("pkg", e.fencing_id + 999),
        "stale fence removes nothing"
    );
    assert!(m.get("pkg").is_some());
}

#[test]
fn reassign_bumps_fence_changes_owner_resets_status() {
    let m = ClaimMap::new();
    let e = m.claim("pkg", uuid(), None, "w1").unwrap();
    m.set_status(
        "pkg",
        e.fencing_id,
        &[ClaimStatus::Pending],
        ClaimStatus::Running,
    );
    let r = m.reassign("pkg", "w2").unwrap();
    assert!(r.fencing_id > e.fencing_id);
    assert_eq!(r.owner_worker, "w2");
    assert_eq!(r.status, ClaimStatus::Pending);
}

#[test]
fn owned_by_lists_only_active_entries_for_worker() {
    let m = ClaimMap::new();
    let a = m.claim("a", uuid(), None, "w1").unwrap();
    m.claim("b", uuid(), None, "w1").unwrap();
    m.claim("c", uuid(), None, "w2").unwrap();
    // Make `a` terminal.
    m.set_status(
        "a",
        a.fencing_id,
        &[ClaimStatus::Pending],
        ClaimStatus::Stopped,
    );
    let owned: Vec<String> = m.owned_by("w1").into_iter().map(|e| e.lease_key).collect();
    assert_eq!(owned, vec!["b".to_string()]);
    assert_eq!(m.active_load("w1"), 1);
    assert_eq!(m.active_load("w2"), 1);
}

#[test]
fn fence_ok_for_session_matches_active_claim_on_current_fence() {
    let m = ClaimMap::new();
    let s = uuid();
    let e = m.claim("pkg", s, None, "w1").unwrap();
    assert!(
        m.fence_ok_for_session(s, e.fencing_id),
        "active claim on its own fence must pass"
    );
}

#[test]
fn fence_ok_for_session_rejects_stale_fence() {
    let m = ClaimMap::new();
    let s = uuid();
    let e = m.claim("pkg", s, None, "w1").unwrap();
    let stale = e.fencing_id;
    m.reassign("pkg", "w2").unwrap(); // bumps the fence; same session id
    assert!(
        !m.fence_ok_for_session(s, stale),
        "a superseded worker's stale fence must be refused"
    );
}

#[test]
fn fence_ok_for_session_rejects_unknown_session() {
    let m = ClaimMap::new();
    m.claim("pkg", uuid(), None, "w1").unwrap();
    assert!(
        !m.fence_ok_for_session(uuid(), 1),
        "an unknown session has no claim and must be refused"
    );
}

#[test]
fn fence_ok_for_session_rejects_terminal_claim() {
    let m = ClaimMap::new();
    let s = uuid();
    let e = m.claim("pkg", s, None, "w1").unwrap();
    m.set_status(
        "pkg",
        e.fencing_id,
        &[ClaimStatus::Pending],
        ClaimStatus::Stopped,
    );
    assert!(
        !m.fence_ok_for_session(s, e.fencing_id),
        "a terminal claim is not active and must be refused"
    );
}

#[test]
fn set_status_for_session_applies_on_matching_fence() {
    let m = ClaimMap::new();
    let s = uuid();
    let e = m.claim("pkg", s, None, "w1").unwrap();
    assert!(m.set_status_for_session(
        s,
        e.fencing_id,
        &[ClaimStatus::Pending],
        ClaimStatus::Running,
    ));
    assert_eq!(m.get("pkg").unwrap().status, ClaimStatus::Running);
}

#[test]
fn set_status_for_session_no_op_on_stale_fence() {
    let m = ClaimMap::new();
    let s = uuid();
    let e = m.claim("pkg", s, None, "w1").unwrap();
    let stale = e.fencing_id;
    m.reassign("pkg", "w2").unwrap(); // bumps the fence; status back to Pending
    assert!(
        !m.set_status_for_session(s, stale, &[ClaimStatus::Pending], ClaimStatus::Running),
        "stale fence must not mutate"
    );
    assert_eq!(
        m.get("pkg").unwrap().status,
        ClaimStatus::Pending,
        "status unchanged after a fenced-off write"
    );
}

#[test]
fn set_status_for_session_no_op_on_unknown_session() {
    let m = ClaimMap::new();
    m.claim("pkg", uuid(), None, "w1").unwrap();
    assert!(
        !m.set_status_for_session(uuid(), 1, &[ClaimStatus::Pending], ClaimStatus::Running),
        "an unknown session must not mutate any claim"
    );
}

#[test]
fn seed_fencing_raises_floor_for_post_restart_redo() {
    let m = ClaimMap::new();
    m.seed_fencing(100);
    let e = m.claim("pkg", uuid(), None, "w1").unwrap();
    assert!(
        e.fencing_id > 100,
        "next fence must exceed the seeded floor"
    );
}

#[test]
fn lease_key_for_session_resolves_the_bound_entry() {
    let m = ClaimMap::new();
    let s = uuid();
    m.claim("pkg", s, None, "w1").unwrap();
    assert_eq!(m.lease_key_for_session(s).as_deref(), Some("pkg"));
    // An unknown session resolves to nothing.
    assert_eq!(m.lease_key_for_session(uuid()), None);
}

#[test]
fn pending_count_counts_only_pending_claims() {
    let m = ClaimMap::new();
    // Fresh claims start `Pending`.
    let a = m.claim("a", uuid(), None, "w1").unwrap();
    m.claim("b", uuid(), None, "w1").unwrap();
    assert_eq!(m.pending_count(), 2, "both fresh claims are pending");

    // Advancing one out of Pending drops the count.
    assert!(m.set_status(
        "a",
        a.fencing_id,
        &[ClaimStatus::Pending],
        ClaimStatus::Running
    ));
    assert_eq!(m.pending_count(), 1, "only the still-pending claim counts");
}

#[test]
fn snapshot_returns_every_entry() {
    let m = ClaimMap::new();
    m.claim("a", uuid(), None, "w1").unwrap();
    m.claim("b", uuid(), None, "w2").unwrap();
    let snap = m.snapshot();
    assert_eq!(snap.len(), 2);
    // The snapshot is the same data `list` returns (an alias for the consumers).
    assert_eq!(snap.len(), m.list().len());
}
