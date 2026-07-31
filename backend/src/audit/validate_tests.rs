//! Fail-closed validation: what a completed record may and may not contain.

use super::*;
use crate::audit::event::{
    Actor, ActorKind, ApiRequestCompletedV1, AuthenticationMethod, Principal, PrincipalKind,
};
use crate::audit::test_support::{
    anonymous_event, human_event, instant, system_event, webhook_event,
};

/// Assert the record is rejected and the failure names `field`.
fn rejects(event: &ApiRequestCompletedV1, field: &str) {
    match validate(event) {
        Ok(()) => panic!("expected `{field}` to be rejected"),
        Err(EventError::Invalid { field: actual, .. }) => {
            assert_eq!(actual, field, "wrong field rejected");
        }
        Err(other) => panic!("expected a field rejection, got {other}"),
    }
}

#[test]
fn the_canonical_fixtures_are_valid() {
    for event in [
        human_event(),
        anonymous_event(),
        system_event(),
        webhook_event(Some(583_231)),
        webhook_event(None),
    ] {
        validate(&event).unwrap_or_else(|e| panic!("fixture must validate: {e}"));
    }
}

#[test]
fn a_query_bearing_route_is_rejected() {
    // A `?` means the caller passed a raw URI, which would smuggle query values
    // (possibly secrets) into the analytics store.
    let mut event = human_event();
    event.route_template = "/api/v1/logs/sess-abc?token=shhh".to_string();
    rejects(&event, "route_template");

    let mut fragment = human_event();
    fragment.route_template = "/api/v1/logs#frag".to_string();
    rejects(&fragment, "route_template");
}

#[test]
fn a_relative_or_empty_route_is_rejected() {
    let mut relative = human_event();
    relative.route_template = "api/v1/logs".to_string();
    rejects(&relative, "route_template");

    let mut empty = human_event();
    empty.route_template = String::new();
    rejects(&empty, "route_template");
}

#[test]
fn a_lowercase_or_oversized_method_is_rejected() {
    let mut lower = human_event();
    lower.method = "get".to_string();
    rejects(&lower, "method");

    let mut long = human_event();
    long.method = "G".repeat(limits::METHOD + 1);
    rejects(&long, "method");
}

#[test]
fn missing_required_identifiers_are_rejected() {
    let mut no_request_id = human_event();
    no_request_id.request_id = "   ".to_string();
    rejects(&no_request_id, "request_id");

    let mut no_operation = human_event();
    no_operation.operation_id = String::new();
    rejects(&no_operation, "operation_id");

    let mut bad_version = human_event();
    bad_version.schema_version = 2;
    rejects(&bad_version, "schema_version");
}

#[test]
fn control_characters_are_rejected_anywhere_they_would_corrupt_a_log() {
    let mut event = human_event();
    event.request_id = "req\n0001".to_string();
    rejects(&event, "request_id");
}

#[test]
fn inverted_or_inconsistent_timestamps_are_rejected() {
    let mut inverted = human_event();
    inverted.completed_at = instant(1_699_999_999, 0);
    rejects(&inverted, "completed_at");

    let mut lying_duration = human_event();
    lying_duration.duration_ms = 5;
    rejects(&lying_duration, "duration_ms");
}

#[test]
fn every_invalid_status_outcome_pair_is_rejected() {
    for (status, outcome) in [
        (Some(200), AuditOutcome::ClientError),
        (Some(404), AuditOutcome::Success),
        (Some(302), AuditOutcome::Success),
        (Some(500), AuditOutcome::ClientError),
        (Some(200), AuditOutcome::Timeout),
        (Some(200), AuditOutcome::Rejected),
        // An incomplete record by definition never produced a status.
        (Some(200), AuditOutcome::Incomplete),
    ] {
        let mut event = human_event();
        event.status_code = status;
        event.outcome = outcome;
        match validate(&event) {
            Err(EventError::Invalid { field, .. }) => {
                assert!(
                    field == "outcome" || field == "status_code",
                    "{status:?}/{outcome} rejected the wrong field: {field}"
                );
            }
            other => panic!("{status:?}/{outcome} must be rejected, got {other:?}"),
        }
    }
}

#[test]
fn every_valid_status_outcome_pair_is_accepted() {
    for (status, outcome) in [
        (Some(204), AuditOutcome::Success),
        (Some(302), AuditOutcome::Redirect),
        (Some(422), AuditOutcome::ClientError),
        (Some(503), AuditOutcome::ServerError),
        (Some(504), AuditOutcome::Timeout),
        (Some(408), AuditOutcome::Timeout),
        (Some(401), AuditOutcome::Rejected),
        // The leader-readiness gate rejects with 503 before the handler runs.
        (Some(503), AuditOutcome::Rejected),
        (None, AuditOutcome::Timeout),
        (None, AuditOutcome::Incomplete),
    ] {
        let mut event = human_event();
        event.status_code = status;
        event.outcome = outcome;
        validate(&event).unwrap_or_else(|e| panic!("{status:?}/{outcome} must be accepted: {e}"));
    }
}

#[test]
fn a_missing_status_is_rejected_for_a_completed_outcome() {
    let mut event = human_event();
    event.status_code = None;
    event.outcome = AuditOutcome::Success;
    rejects(&event, "status_code");
}

#[test]
fn an_out_of_range_status_is_rejected() {
    let mut event = human_event();
    event.status_code = Some(999);
    rejects(&event, "status_code");
}

#[test]
fn free_text_in_the_error_code_is_rejected() {
    // Error TEXT is forbidden data; only stable machine codes are recorded.
    let mut event = human_event();
    event.status_code = Some(500);
    event.outcome = AuditOutcome::ServerError;
    event.error_code = Some("connection refused to 10.0.0.5:5432".to_string());
    rejects(&event, "error_code");

    let mut ok = human_event();
    ok.status_code = Some(500);
    ok.outcome = AuditOutcome::ServerError;
    ok.error_code = Some("upstream_error".to_string());
    validate(&ok).expect("a stable snake_case code is accepted");
}

#[test]
fn a_human_actor_whose_ids_disagree_is_rejected() {
    // The core authorization-supporting invariant: the canonical filter field
    // must never be able to point at a different person than the nested one.
    let mut event = human_event();
    event.actor_id = Some(999);
    rejects(&event, "actor_id");

    let mut missing_canonical = human_event();
    missing_canonical.actor_id = None;
    rejects(&missing_canonical, "actor_id");
}

#[test]
fn a_verified_github_user_without_an_id_is_rejected() {
    let mut event = human_event();
    event.actor.id = None;
    event.actor_id = None;
    rejects(&event, "actor.id");
}

#[test]
fn a_non_positive_github_id_is_rejected() {
    let mut event = human_event();
    event.actor.id = Some(0);
    event.actor_id = Some(0);
    rejects(&event, "actor.id");
}

#[test]
fn a_non_human_actor_carrying_a_github_id_is_rejected() {
    let mut event = system_event();
    event.actor_id = Some(583_231);
    rejects(&event, "actor_id");

    let mut nested = ApiRequestCompletedV1::new(
        crate::audit::test_support::identity(),
        crate::audit::test_support::timing(),
        Actor {
            kind: ActorKind::Service,
            id: None,
            login: None,
            authentication: AuthenticationMethod::Internal,
        },
        Principal::new(PrincipalKind::None, None),
        crate::audit::test_support::ok(),
        crate::audit::test_support::service(),
    );
    nested.actor.id = Some(7);
    rejects(&nested, "actor_id");
}

#[test]
fn an_anonymous_actor_carrying_a_login_is_rejected() {
    let mut event = anonymous_event();
    event.actor.login = Some("octocat".to_string());
    rejects(&event, "actor.login");
}

#[test]
fn a_session_id_that_disagrees_with_its_correlation_is_rejected() {
    let mut event = human_event();
    event.session_id = Some("sess-other".to_string());
    rejects(&event, "session_id");
}

#[test]
fn a_malformed_repository_name_is_rejected() {
    for bad in ["acme", "acme/", "/site", "acme/site/extra"] {
        let mut event = human_event();
        event.correlation.repo_full_name = Some(bad.to_string());
        rejects(&event, "correlation.repo_full_name");
    }
}

#[test]
fn oversized_bounded_strings_are_rejected() {
    let mut login = human_event();
    login.actor.login = Some("a".repeat(limits::LOGIN + 1));
    rejects(&login, "actor.login");

    let mut session = human_event();
    let long = "s".repeat(limits::SESSION_ID + 1);
    session.session_id = Some(long.clone());
    session.correlation.session_id = Some(long);
    rejects(&session, "session_id");

    let mut principal = human_event();
    principal.principal.id = Some("p".repeat(limits::PRINCIPAL_ID + 1));
    rejects(&principal, "principal.id");

    let mut delivery = human_event();
    delivery.correlation.webhook_delivery_id = Some("d".repeat(limits::WEBHOOK_DELIVERY_ID + 1));
    rejects(&delivery, "correlation.webhook_delivery_id");
}
