//! Unit tests for terminal outcome derivation.

use super::*;
use crate::audit::validate;

fn outcome(status: u16) -> AuditOutcome {
    derive_outcome(
        StatusCode::from_u16(status).expect("valid status"),
        false,
        false,
    )
}

#[test]
fn maps_each_status_class_to_its_outcome() {
    for (status, expected) in [
        (200, AuditOutcome::Success),
        (201, AuditOutcome::Success),
        (204, AuditOutcome::Success),
        (302, AuditOutcome::Redirect),
        (304, AuditOutcome::Redirect),
        (400, AuditOutcome::ClientError),
        (401, AuditOutcome::ClientError),
        (403, AuditOutcome::ClientError),
        (404, AuditOutcome::ClientError),
        (409, AuditOutcome::ClientError),
        (422, AuditOutcome::ClientError),
        (429, AuditOutcome::ClientError),
        (500, AuditOutcome::ServerError),
        (502, AuditOutcome::ServerError),
        (503, AuditOutcome::ServerError),
    ] {
        assert_eq!(outcome(status), expected, "status {status}");
    }
}

#[test]
fn a_408_is_a_timeout_even_without_a_marker() {
    assert_eq!(outcome(408), AuditOutcome::Timeout);
}

#[test]
fn an_explicit_timeout_marker_wins_over_the_status_class() {
    assert_eq!(
        derive_outcome(StatusCode::GATEWAY_TIMEOUT, false, true),
        AuditOutcome::Timeout
    );
    // …and over a rejection marker, because a deadline that expired is what
    // actually happened.
    assert_eq!(
        derive_outcome(StatusCode::REQUEST_TIMEOUT, true, true),
        AuditOutcome::Timeout
    );
}

#[test]
fn a_policy_short_circuit_is_rejected_with_its_real_status() {
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        assert_eq!(
            derive_outcome(status, true, false),
            AuditOutcome::Rejected,
            "{status}"
        );
    }
}

/// A misplaced marker is a call-site bug; honouring it would build a record the
/// contract rejects, replacing one bug with a missing audit row.
#[test]
fn a_rejection_marker_on_an_impossible_status_is_ignored() {
    assert_eq!(
        derive_outcome(StatusCode::OK, true, false),
        AuditOutcome::Success
    );
    assert_eq!(
        derive_outcome(StatusCode::FOUND, true, false),
        AuditOutcome::Redirect
    );
    assert_eq!(
        derive_outcome(StatusCode::INTERNAL_SERVER_ERROR, true, false),
        AuditOutcome::ServerError
    );
}

/// Every derivable pair must satisfy the event contract's status/outcome matrix,
/// or the middleware would build records the sink then silently drops.
#[test]
fn every_derived_pair_satisfies_the_event_contract() {
    for status in 200u16..=599 {
        let Ok(status) = StatusCode::from_u16(status) else {
            continue;
        };
        for rejected in [false, true] {
            for timed_out in [false, true] {
                let outcome = derive_outcome(status, rejected, timed_out);
                // A timeout marker is only ever attached to the two statuses the
                // contract admits for it; anything else is a caller bug, not a
                // derivation bug, so it is excluded from this sweep.
                if outcome == AuditOutcome::Timeout && !matches!(status.as_u16(), 408 | 504) {
                    continue;
                }
                let event = crate::audit::test_support::event_with(status.as_u16(), outcome);
                validate::validate(&event).unwrap_or_else(|error| {
                    panic!("{status} rejected={rejected} timed_out={timed_out}: {error}")
                });
            }
        }
    }
}

#[test]
fn framework_codes_cover_only_the_responses_no_call_site_can_tag() {
    assert_eq!(
        framework_error_code(StatusCode::REQUEST_TIMEOUT, true),
        Some(codes::REQUEST_TIMEOUT)
    );
    assert_eq!(
        framework_error_code(StatusCode::NOT_FOUND, false),
        Some(codes::ROUTE_NOT_FOUND)
    );
    // A handler's own 404 already carries `not_found`; the framework must not
    // relabel it as an unrouted path.
    assert_eq!(framework_error_code(StatusCode::NOT_FOUND, true), None);
    assert_eq!(
        framework_error_code(StatusCode::METHOD_NOT_ALLOWED, true),
        Some(codes::METHOD_NOT_ALLOWED)
    );
    assert_eq!(framework_error_code(StatusCode::OK, true), None);
    assert_eq!(
        framework_error_code(StatusCode::INTERNAL_SERVER_ERROR, true),
        None
    );
}
