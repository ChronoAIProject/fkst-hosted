//! The error envelope: stable codes, correct statuses, and no upstream text.

use super::*;

#[test]
fn every_variant_has_a_stable_code_and_status() {
    let cases = [
        (RelayError::Unauthorized, "unauthorized", 401),
        (RelayError::Invalid("field"), "invalid_request", 400),
        (RelayError::Conflict, "event_id_conflict", 409),
        (RelayError::NoStart, "no_registered_start", 409),
        (RelayError::Capacity, "relay_at_capacity", 503),
        (RelayError::Unavailable, "relay_unavailable", 503),
    ];
    for (error, code, status) in cases {
        assert_eq!(error.code(), code);
        assert_eq!(error.status().as_u16(), status);
    }
}

#[test]
fn a_storage_failure_never_names_its_internal_cause_on_the_wire() {
    // Busy, corrupt, and internal all map onto one client-visible answer: the
    // caller retries idempotently either way, and naming the difference would
    // describe storage internals to whoever can reach the relay.
    for storage in [
        DbError::Busy,
        DbError::Unavailable("corrupt"),
        DbError::Unavailable("disk_full"),
        DbError::Internal("query"),
    ] {
        assert_eq!(RelayError::from(storage), RelayError::Unavailable);
    }
    assert_eq!(RelayError::from(DbError::Conflict), RelayError::Conflict);
    assert_eq!(RelayError::from(DbError::NoStart), RelayError::NoStart);
    assert_eq!(RelayError::from(DbError::Capacity), RelayError::Capacity);
}

#[test]
fn the_telemetry_label_matches_the_failure_class() {
    assert_eq!(
        RelayError::Unauthorized.ingress_result(),
        IngressResult::Unauthorized
    );
    assert_eq!(
        RelayError::Invalid("x").ingress_result(),
        IngressResult::Rejected
    );
    assert_eq!(
        RelayError::Conflict.ingress_result(),
        IngressResult::Conflict
    );
    assert_eq!(
        RelayError::Unavailable.ingress_result(),
        IngressResult::Unavailable
    );
}
