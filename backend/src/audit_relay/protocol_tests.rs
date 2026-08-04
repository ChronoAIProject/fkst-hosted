//! Wire-contract tests: round-trips, and every rejection that must NOT be
//! coerced into a default.

use super::*;
use crate::audit::event::{ActorKind, AuditOutcome, PrincipalKind};
use crate::audit_relay::test_support::{completion, lifecycle, now, start};

#[test]
fn a_start_yields_its_immutable_identity() {
    let identity = start("11111111-1111-4111-8111-111111111111")
        .to_identity()
        .expect("valid start");
    assert_eq!(
        identity.request_id,
        "req-11111111-1111-4111-8111-111111111111"
    );
    assert_eq!(identity.method, "GET");
    assert_eq!(identity.operation_id, "canvas_overview");
    assert_eq!(identity.started_at, now());
}

#[test]
fn a_start_with_a_non_uuid_event_id_is_refused() {
    let mut start = start("11111111-1111-4111-8111-111111111111");
    start.event_id = "not-a-uuid".to_string();
    assert!(matches!(
        start.to_identity(),
        Err(ProtocolError::Invalid {
            field: "event_id",
            ..
        })
    ));
}

#[test]
fn a_deadline_before_the_start_is_refused() {
    let mut start = start("11111111-1111-4111-8111-111111111111");
    start.completion_deadline_at =
        format_instant(now() - k8s_openapi::chrono::Duration::seconds(1));
    assert!(matches!(
        start.to_identity(),
        Err(ProtocolError::Invalid {
            field: "completion_deadline_at",
            ..
        })
    ));
}

#[test]
fn an_unsupported_schema_version_is_refused_rather_than_upgraded() {
    let mut start = start("11111111-1111-4111-8111-111111111111");
    start.schema_version = 99;
    assert_eq!(
        start.to_identity().expect_err("refused"),
        ProtocolError::UnsupportedSchema(99)
    );
}

#[test]
fn a_completion_round_trips_through_the_domain_type() {
    let wire = completion("22222222-2222-4222-8222-222222222222", Some(101));
    let domain = wire.to_domain().expect("valid completion");
    assert_eq!(domain.actor_id, Some(101));
    assert_eq!(domain.actor.kind, ActorKind::GithubUser);
    assert_eq!(domain.principal.kind, PrincipalKind::GithubUserToken);
    assert_eq!(domain.outcome, AuditOutcome::Success);
    assert_eq!(domain.status_code, Some(200));
    // The projection back onto the wire must be byte-identical, which is what
    // makes the relay's idempotent-replay comparison sound.
    let reprojected = RequestCompletionV1::from_domain(&domain);
    assert_eq!(
        serde_json::to_vec(&reprojected).expect("encodes"),
        serde_json::to_vec(&wire).expect("encodes")
    );
}

#[test]
fn an_unknown_enum_spelling_is_refused_not_defaulted() {
    let mut wire = completion("33333333-3333-4333-8333-333333333333", Some(101));
    wire.outcome = "mostly_fine".to_string();
    assert!(matches!(
        wire.to_domain(),
        Err(ProtocolError::Invalid {
            field: "outcome",
            ..
        })
    ));

    let mut wire = completion("33333333-3333-4333-8333-333333333333", Some(101));
    wire.actor.kind = "wizard".to_string();
    assert!(matches!(
        wire.to_domain(),
        Err(ProtocolError::Invalid {
            field: "actor.kind",
            ..
        })
    ));
}

#[test]
fn a_lifecycle_event_round_trips_through_the_domain_type() {
    let wire = lifecycle("44444444-4444-4444-8444-444444444444", "sess-1");
    let domain = wire.to_domain().expect("valid lifecycle event");
    assert_eq!(domain.session_id, "sess-1");
    assert_eq!(domain.attribution.creator_id, Some(101));
    assert_eq!(
        domain.correlation.repo_full_name.as_deref(),
        Some("acme/site")
    );
    let reprojected = LifecycleEventV1::from_domain(&domain);
    assert_eq!(
        serde_json::to_vec(&reprojected).expect("encodes"),
        serde_json::to_vec(&wire).expect("encodes")
    );
}

#[test]
fn an_unknown_lifecycle_action_is_refused() {
    let mut wire = lifecycle("55555555-5555-4555-8555-555555555555", "sess-1");
    wire.lifecycle_action = "vibed".to_string();
    assert!(matches!(
        wire.to_domain(),
        Err(ProtocolError::Invalid {
            field: "lifecycle_action",
            ..
        })
    ));
}

#[test]
fn the_wire_carries_no_field_for_forbidden_data() {
    // A structural guard: the serialized start/completion key sets are fixed and
    // contain no place a raw body, URI, header, or credential could ride.
    let start =
        serde_json::to_value(start("66666666-6666-4666-8666-666666666666")).expect("start encodes");
    let keys: Vec<&str> = start
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    for forbidden in ["uri", "url", "query", "headers", "body", "authorization"] {
        assert!(
            !keys.contains(&forbidden),
            "the start wire must not carry `{forbidden}`"
        );
    }
}
