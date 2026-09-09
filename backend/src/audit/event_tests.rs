//! Construction invariants of the versioned event contract.

use super::*;
use crate::audit::test_support::{human_event, identity, instant, ok, service, timing};

#[test]
fn the_constructor_stamps_the_schema_version_and_derives_the_duration() {
    let event = human_event();
    assert_eq!(event.schema_version, SCHEMA_VERSION);
    assert_eq!(event.duration_ms, 250);
    assert_eq!(event.started_at_rfc3339(), "2023-11-14T22:13:20.000Z");
    assert_eq!(event.completed_at_rfc3339(), "2023-11-14T22:13:20.250Z");
}

#[test]
fn the_canonical_actor_id_mirrors_a_human_actors_nested_id() {
    let event = human_event();
    assert_eq!(event.actor_id, Some(583_231));
    assert_eq!(event.actor.id, Some(583_231));
}

#[test]
fn a_non_human_actor_never_receives_a_canonical_actor_id() {
    // Even if a caller hands the constructor an id on a system actor, it is not
    // promoted: unattributed records must stay unattributed.
    let mut actor = Actor::system();
    actor.id = Some(1);
    let event = ApiRequestCompletedV1::new(
        identity(),
        timing(),
        actor,
        Principal::none(),
        ok(),
        service(),
    );
    assert_eq!(event.actor_id, None);
}

#[test]
fn correlation_keeps_the_canonical_session_id_in_step() {
    let event = human_event();
    assert_eq!(event.session_id.as_deref(), Some("sess-abc"));
    assert_eq!(event.session_id, event.correlation.session_id);

    let plain = ApiRequestCompletedV1::new(
        identity(),
        timing(),
        Actor::anonymous(),
        Principal::none(),
        ok(),
        service(),
    );
    assert_eq!(plain.session_id, None);
    assert_eq!(plain.correlation, Correlation::default());
}

#[test]
fn arguments_default_to_empty_and_not_applicable() {
    let event = human_event();
    assert!(event.arguments.is_empty());
    assert_eq!(
        event.arguments_parse_status,
        ArgumentsParseStatus::NotApplicable
    );

    let mut arguments = serde_json::Map::new();
    arguments.insert("session_id".to_string(), serde_json::json!("sess-abc"));
    let with_arguments = human_event().with_arguments(arguments, ArgumentsParseStatus::Parsed);
    assert_eq!(with_arguments.arguments.len(), 1);
    assert_eq!(
        with_arguments.arguments_parse_status,
        ArgumentsParseStatus::Parsed
    );
}

#[test]
fn the_event_id_is_deterministic_across_constructions() {
    // Stable ids are what make at-least-once retries deduplicate in PostHog:
    // rebuilding the same terminal record must yield the same uuid.
    assert_eq!(human_event().event_id, human_event().event_id);
    assert_eq!(
        derive_event_id(&identity(), timing().started_at),
        human_event().event_id
    );
}

#[test]
fn the_event_id_changes_with_any_identity_or_start_instant() {
    let base = derive_event_id(&identity(), timing().started_at);

    let mut other_request = identity();
    other_request.request_id = "req-0002".to_string();
    assert_ne!(derive_event_id(&other_request, timing().started_at), base);

    let mut other_route = identity();
    other_route.route_template = "/api/v1/repos/{owner}/{name}".to_string();
    assert_ne!(derive_event_id(&other_route, timing().started_at), base);

    // A client that reuses one X-Request-Id across calls still gets distinct
    // events, because the start instant is part of the key.
    assert_ne!(
        derive_event_id(&identity(), instant(1_700_000_001, 0)),
        base
    );
}

#[test]
fn a_completion_before_the_start_clamps_the_duration_to_zero() {
    // The constructor never produces a negative duration; validation separately
    // rejects the inverted timestamps themselves.
    let event = ApiRequestCompletedV1::new(
        identity(),
        RequestTiming {
            started_at: instant(1_700_000_010, 0),
            completed_at: instant(1_700_000_000, 0),
        },
        Actor::anonymous(),
        Principal::none(),
        ok(),
        service(),
    );
    assert_eq!(event.duration_ms, 0);
}

#[test]
fn every_closed_enum_renders_its_stable_wire_string() {
    assert_eq!(ActorKind::GithubUser.as_str(), "github_user");
    assert_eq!(
        ActorKind::GithubWebhookSender.as_str(),
        "github_webhook_sender"
    );
    assert_eq!(ActorKind::Anonymous.as_str(), "anonymous");
    assert_eq!(ActorKind::Service.as_str(), "service");
    assert_eq!(ActorKind::System.as_str(), "system");

    assert_eq!(AuthenticationMethod::Bearer.as_str(), "bearer");
    assert_eq!(AuthenticationMethod::WebhookHmac.as_str(), "webhook_hmac");
    assert_eq!(AuthenticationMethod::Internal.as_str(), "internal");

    assert_eq!(PrincipalKind::GithubUserToken.as_str(), "github_user_token");
    assert_eq!(
        PrincipalKind::GithubAppInstallation.as_str(),
        "github_app_installation"
    );
    assert_eq!(PrincipalKind::Reconciler.as_str(), "reconciler");

    assert_eq!(AuditOutcome::Success.as_str(), "success");
    assert_eq!(AuditOutcome::ClientError.as_str(), "client_error");
    assert_eq!(AuditOutcome::ServerError.as_str(), "server_error");
    assert_eq!(AuditOutcome::Incomplete.as_str(), "incomplete");
    // Display and as_str never diverge.
    assert_eq!(AuditOutcome::Rejected.to_string(), "rejected");

    assert_eq!(
        ArgumentsParseStatus::NotApplicable.as_str(),
        "not_applicable"
    );
    assert_eq!(ArgumentsParseStatus::Unavailable.as_str(), "unavailable");
}

#[test]
fn only_github_backed_actor_kinds_are_human() {
    assert!(ActorKind::GithubUser.is_human());
    assert!(ActorKind::GithubWebhookSender.is_human());
    assert!(!ActorKind::Anonymous.is_human());
    assert!(!ActorKind::Service.is_human());
    assert!(!ActorKind::System.is_human());
}
