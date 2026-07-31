//! The PostHog wire projection: exact property names, distinct-id rules, person
//! profiles, and the bounded size gate.

use super::*;
use crate::audit::event::{ArgumentsParseStatus, AuditOutcome};
use crate::audit::test_support::{
    anonymous_event, human_event, service_event, system_event, webhook_event,
};

/// A generous cap, so size never interferes with a content assertion.
fn limits() -> EventLimits {
    EventLimits::new(65_536)
}

fn sorted_keys(event: &CaptureEvent) -> Vec<String> {
    let mut keys: Vec<String> = event.properties.keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn a_human_record_projects_every_contract_field_under_its_exact_name() {
    let event = human_event().with_arguments(
        serde_json::json!({ "session_id": "sess-abc" })
            .as_object()
            .cloned()
            .expect("object"),
        ArgumentsParseStatus::Parsed,
    );
    let captured = event
        .to_capture_event(limits())
        .expect("the canonical record projects");

    assert_eq!(captured.event, "fkst api request completed");
    assert_eq!(captured.distinct_id, "github:583231");
    assert_eq!(captured.uuid, event.event_id.to_string());
    assert_eq!(captured.timestamp, "2023-11-14T22:13:20.250Z");

    // The exact property surface. A new key must be a deliberate contract change,
    // because HogQL, the relay SQL, and the UI all read these names.
    assert_eq!(
        sorted_keys(&captured),
        vec![
            "actor",
            "actor_id",
            "actor_kind",
            "actor_login",
            "arguments",
            "arguments_parse_status",
            "completed_at",
            "correlation",
            "duration_ms",
            "error_code",
            "event_id",
            "installation_id",
            "method",
            "operation_id",
            "outcome",
            "principal",
            "principal_kind",
            "repo_full_name",
            "request_id",
            "route_template",
            "schema_version",
            "service_environment",
            "service_version",
            "session_id",
            "started_at",
            "status_code",
            "trigger_issue",
            "webhook_delivery_id",
        ]
    );

    let p = &captured.properties;
    assert_eq!(p["schema_version"], serde_json::json!(1));
    assert_eq!(p["event_id"], serde_json::json!(event.event_id.to_string()));
    assert_eq!(p["request_id"], serde_json::json!("req-0001"));
    assert_eq!(p["method"], serde_json::json!("GET"));
    assert_eq!(
        p["route_template"],
        serde_json::json!("/api/v1/logs/{session_id}")
    );
    assert_eq!(p["operation_id"], serde_json::json!("logs_download"));
    assert_eq!(
        p["started_at"],
        serde_json::json!("2023-11-14T22:13:20.000Z")
    );
    assert_eq!(
        p["completed_at"],
        serde_json::json!("2023-11-14T22:13:20.250Z")
    );
    assert_eq!(p["duration_ms"], serde_json::json!(250));
    assert_eq!(p["status_code"], serde_json::json!(200));
    assert_eq!(p["outcome"], serde_json::json!("success"));
    assert_eq!(p["error_code"], serde_json::Value::Null);

    // The authorization-supporting canonical filter fields.
    assert_eq!(p["actor_kind"], serde_json::json!("github_user"));
    assert_eq!(p["actor_id"], serde_json::json!(583_231));
    assert_eq!(p["actor_login"], serde_json::json!("octocat"));
    assert_eq!(p["principal_kind"], serde_json::json!("github_user_token"));
    assert_eq!(p["session_id"], serde_json::json!("sess-abc"));
    assert_eq!(p["repo_full_name"], serde_json::json!("acme/site"));
    assert_eq!(p["trigger_issue"], serde_json::json!(77));
    assert_eq!(p["installation_id"], serde_json::json!(4242));

    assert_eq!(p["service_version"], serde_json::json!("9.9.9"));
    assert_eq!(p["service_environment"], serde_json::json!("test"));
    assert_eq!(p["arguments_parse_status"], serde_json::json!("parsed"));
    assert_eq!(
        p["arguments"],
        serde_json::json!({"session_id": "sess-abc"})
    );

    // The structured objects preserved for display / forward compatibility.
    assert_eq!(
        p["actor"],
        serde_json::json!({
            "kind": "github_user",
            "id": 583_231,
            "login": "octocat",
            "authentication": "bearer",
        })
    );
    assert_eq!(
        p["principal"],
        serde_json::json!({ "kind": "github_user_token", "id": "github_user_token" })
    );
    assert_eq!(
        p["correlation"],
        serde_json::json!({
            "session_id": "sess-abc",
            "repo_full_name": "acme/site",
            "installation_id": 4242,
            "trigger_issue": 77,
            "webhook_delivery_id": "11111111-2222-3333-4444-555555555555",
        })
    );
}

#[test]
fn a_human_record_gets_a_person_profile_and_no_set_properties() {
    let captured = human_event().to_capture_event(limits()).expect("projects");
    assert!(!captured.properties.contains_key("$process_person_profile"));
    // Person properties would be a second, mutable copy of identity state; the
    // contract never emits them.
    assert!(!captured.properties.contains_key("$set"));
    assert!(!captured.properties.contains_key("$set_once"));
}

#[test]
fn an_anonymous_record_uses_the_stable_non_human_distinct_id() {
    let captured = anonymous_event()
        .to_capture_event(limits())
        .expect("projects");
    assert_eq!(captured.distinct_id, "fkst:anonymous");
    assert_eq!(
        captured.properties["$process_person_profile"],
        serde_json::json!(false)
    );
    // Unattributed rows carry no actor id at all; they are global-admin-only.
    assert_eq!(captured.properties["actor_id"], serde_json::Value::Null);
    assert_eq!(captured.properties["actor_login"], serde_json::Value::Null);
    assert_eq!(
        captured.properties["outcome"],
        serde_json::json!("rejected")
    );
    assert_eq!(
        captured.properties["error_code"],
        serde_json::json!("unauthorized")
    );
}

#[test]
fn a_system_record_uses_the_system_distinct_id() {
    let captured = system_event().to_capture_event(limits()).expect("projects");
    assert_eq!(captured.distinct_id, "fkst:system");
    assert_eq!(
        captured.properties["$process_person_profile"],
        serde_json::json!(false)
    );
    assert_eq!(
        captured.properties["actor_kind"],
        serde_json::json!("system")
    );
}

#[test]
fn a_service_record_uses_the_service_distinct_id() {
    let captured = service_event()
        .to_capture_event(limits())
        .expect("projects");
    assert_eq!(captured.distinct_id, "fkst:service");
    // A machine caller must never create a PostHog person profile.
    assert_eq!(
        captured.properties["$process_person_profile"],
        serde_json::json!(false)
    );
    assert_eq!(
        captured.properties["actor_kind"],
        serde_json::json!("service")
    );
    // Its label is displayed but is not an identity: no actor id, and the label
    // never leaks into the distinct id.
    assert_eq!(captured.properties["actor_id"], serde_json::Value::Null);
    assert_eq!(
        captured.properties["actor_login"],
        serde_json::json!("fkst-probe")
    );
    assert!(!captured.distinct_id.contains("fkst-probe"));
}

#[test]
fn a_webhook_record_follows_its_senders_resolvability() {
    // Resolvable sender: a real GitHub person, so the human distinct id applies.
    let known = webhook_event(Some(583_231))
        .to_capture_event(limits())
        .expect("projects");
    assert_eq!(known.distinct_id, "github:583231");
    assert!(!known.properties.contains_key("$process_person_profile"));
    assert_eq!(
        known.properties["actor_kind"],
        serde_json::json!("github_webhook_sender")
    );
    assert_eq!(known.properties["actor_id"], serde_json::json!(583_231));

    // Unresolvable sender: unattributed, and no person profile is created.
    let unknown = webhook_event(None)
        .to_capture_event(limits())
        .expect("projects");
    assert_eq!(unknown.distinct_id, "fkst:webhook");
    assert_eq!(
        unknown.properties["$process_person_profile"],
        serde_json::json!(false)
    );
    assert_eq!(unknown.properties["actor_id"], serde_json::Value::Null);
}

#[test]
fn the_login_is_never_the_distinct_id() {
    // GitHub logins can be renamed and reassigned, so they must never key a
    // person — the immutable numeric id does.
    let captured = human_event().to_capture_event(limits()).expect("projects");
    assert!(!captured.distinct_id.contains("octocat"));
    assert!(captured.distinct_id.starts_with("github:"));
}

#[test]
fn an_invalid_record_is_never_projected() {
    let mut event = human_event();
    event.route_template = "/api/v1/logs?token=shhh".to_string();
    match event.to_capture_event(limits()) {
        Err(EventError::Invalid { field, .. }) => assert_eq!(field, "route_template"),
        other => panic!("a query-bearing route must not project: {other:?}"),
    }
}

#[test]
fn an_oversized_record_is_rejected_rather_than_truncated() {
    // Arbitrary JSON cannot be shortened without corrupting the record, so
    // overflow becomes a delivery error (and a metric), never a silent trim.
    let mut arguments = serde_json::Map::new();
    arguments.insert("packages".to_string(), serde_json::json!("x".repeat(4_096)));
    let event = human_event().with_arguments(arguments, ArgumentsParseStatus::Parsed);

    match event.to_capture_event(EventLimits::new(4_096)) {
        Err(EventError::TooLarge { actual, limit }) => {
            assert_eq!(limit, 4_096);
            assert!(actual > limit, "{actual} must exceed {limit}");
        }
        other => panic!("an oversized record must be rejected: {other:?}"),
    }
    // The same record fits under the default 64 KiB cap.
    event
        .to_capture_event(limits())
        .expect("the default cap accommodates it");
}

#[test]
fn an_incomplete_record_projects_a_null_status() {
    // No system fabricates a status for a request that never answered.
    let mut event = human_event();
    event.status_code = None;
    event.outcome = AuditOutcome::Incomplete;
    let captured = event.to_capture_event(limits()).expect("projects");
    assert_eq!(captured.properties["status_code"], serde_json::Value::Null);
    assert_eq!(
        captured.properties["outcome"],
        serde_json::json!("incomplete")
    );
}
