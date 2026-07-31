//! The lifecycle contract: deterministic ids that dedupe a retry but separate an
//! incarnation, fail-closed validation of the session id, and a projection that
//! carries no free text.

use k8s_openapi::chrono::{TimeZone, Utc};

use super::super::identity::AuditIdentity;
use super::super::projection::EventLimits;
use super::super::test_support::service;
use super::*;

const SESSION: &str = "5f6a2c19-1e2b-5a7f-9c31-6b0a2f7751d4";

fn limits() -> EventLimits {
    EventLimits::new(64 * 1024)
}

fn event(action: LifecycleAction) -> SandboxLifecycleV1 {
    SandboxLifecycleV1::new(
        action,
        RuntimeBackendKind::Kubernetes,
        SESSION,
        AuditIdentity::reconciler(Some(4242)),
        service(),
    )
    .at(Utc
        .timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("instant"))
}

fn runtime(id: Option<&str>, hint: Option<&str>) -> LifecycleRuntime {
    LifecycleRuntime {
        runtime_id: id.map(str::to_string),
        created_at: None,
        incarnation_hint: hint.map(str::to_string),
    }
}

#[test]
fn the_same_effect_derives_the_same_event_id_on_every_retry() {
    // This is what makes at-least-once delivery safe: PostHog deduplicates on
    // the UUID, so a reconcile retry cannot write a second row.
    let first = event(LifecycleAction::Created).with_runtime(runtime(Some("fkst-sess-1"), None));
    let second = event(LifecycleAction::Created).with_runtime(runtime(Some("fkst-sess-1"), None));
    assert_eq!(first.event_id, second.event_id);
}

#[test]
fn a_new_runtime_incarnation_derives_a_distinct_event_id() {
    let first = event(LifecycleAction::Created).with_runtime(runtime(Some("sbx-1"), None));
    let second = event(LifecycleAction::Created).with_runtime(runtime(Some("sbx-2"), None));
    assert_ne!(
        first.event_id, second.event_id,
        "a respawned session must not collapse into the first incarnation's row"
    );
}

#[test]
fn a_created_at_stamp_separates_two_runtimes_that_reuse_one_name() {
    // Kubernetes names a session's Pod deterministically, so a killed and
    // respawned session reuses the name; the creation instant is what keeps the
    // two incarnations apart.
    let base = runtime(Some("fkst-sess-1"), None);
    let first = event(LifecycleAction::Created).with_runtime(LifecycleRuntime {
        created_at: Utc.timestamp_opt(1_700_000_000, 0).single(),
        ..base.clone()
    });
    let second = event(LifecycleAction::Created).with_runtime(LifecycleRuntime {
        created_at: Utc.timestamp_opt(1_700_009_999, 0).single(),
        ..base
    });
    assert_ne!(first.event_id, second.event_id);
}

#[test]
fn a_create_with_no_runtime_yet_keys_on_the_supplied_incarnation_hint() {
    let first = event(LifecycleAction::CreateRequested).with_runtime(runtime(None, Some("cfg-a")));
    let repeat = event(LifecycleAction::CreateRequested).with_runtime(runtime(None, Some("cfg-a")));
    let changed =
        event(LifecycleAction::CreateRequested).with_runtime(runtime(None, Some("cfg-b")));
    assert_eq!(first.event_id, repeat.event_id, "a retry dedupes");
    assert_ne!(
        first.event_id, changed.event_id,
        "a changed configuration is a new incarnation"
    );
}

#[test]
fn distinct_actions_and_backends_never_collide() {
    let created = event(LifecycleAction::Created).with_runtime(runtime(Some("r"), None));
    let deleted = event(LifecycleAction::Deleted).with_runtime(runtime(Some("r"), None));
    assert_ne!(created.event_id, deleted.event_id);

    let osb = SandboxLifecycleV1::new(
        LifecycleAction::Created,
        RuntimeBackendKind::OpenSandbox,
        SESSION,
        AuditIdentity::reconciler(Some(4242)),
        service(),
    )
    .with_runtime(runtime(Some("r"), None));
    assert_ne!(created.event_id, osb.event_id);
}

#[test]
fn an_autonomous_effect_is_a_system_actor_with_an_installation_principal() {
    let event = event(LifecycleAction::Created);
    assert_eq!(event.actor.kind.as_str(), "system");
    assert_eq!(event.actor.id, None, "no person did this");
    assert_eq!(event.principal.kind.as_str(), "github_app_installation");
    assert_eq!(event.principal.id.as_deref(), Some("4242"));
}

#[test]
fn the_projection_is_grouped_under_the_system_distinct_id_with_no_person_profile() {
    let projected = event(LifecycleAction::Created)
        .with_attribution(LifecycleAttribution {
            creator_id: Some(4242),
            creator_login: Some("alice".to_string()),
            trigger_author_id: Some(77),
            trigger_author_login: Some("octocat".to_string()),
        })
        .to_capture_event(limits())
        .expect("a well-formed record projects");
    assert_eq!(projected.event, LIFECYCLE_EVENT_NAME);
    assert_eq!(
        projected.distinct_id, "fkst:system",
        "a reconcile-driven delete must never appear under the creator's PostHog profile"
    );
    assert_eq!(projected.properties["$process_person_profile"], false);
    assert_eq!(projected.properties["creator_login"], "alice");
    assert_eq!(projected.properties["lifecycle_action"], "created");
    assert_eq!(projected.properties["schema_version"], 1);
}

#[test]
fn a_reason_code_is_a_closed_enum_string_and_absent_when_unknown() {
    let without = event(LifecycleAction::Created)
        .to_capture_event(limits())
        .expect("projects");
    assert!(without.properties["reason_code"].is_null());

    let with = event(LifecycleAction::Deleted)
        .with_reason(LifecycleReason::Idle)
        .to_capture_event(limits())
        .expect("projects");
    assert_eq!(with.properties["reason_code"], "idle");
}

#[test]
fn the_projected_properties_carry_no_free_text_field() {
    // A canary: the only strings on the wire are ids, logins, closed enums, and
    // the deployment identity. Nothing here can smuggle an upstream error, an
    // issue body, or an environment value.
    let projected = event(LifecycleAction::CreateFailed)
        .with_reason(LifecycleReason::BackendUnavailable)
        .with_correlation(LifecycleCorrelation {
            repo_full_name: Some("acme/site".to_string()),
            installation_id: Some(4242),
            trigger_issue: Some(7),
            request_id: None,
        })
        .to_capture_event(limits())
        .expect("projects");
    let allowed = [
        "schema_version",
        "event_id",
        "occurred_at",
        "lifecycle_action",
        "actor_kind",
        "actor_id",
        "actor_login",
        "principal_kind",
        "principal_id",
        "session_id",
        "backend",
        "runtime_id",
        "creator_id",
        "creator_login",
        "trigger_author_id",
        "trigger_author_login",
        "repo_full_name",
        "installation_id",
        "trigger_issue",
        "request_id",
        "created_at",
        "reason_code",
        "service_version",
        "service_environment",
        "$process_person_profile",
    ];
    for key in projected.properties.keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "{key} is not in the lifecycle allowlist"
        );
    }
    for key in allowed {
        assert!(
            projected.properties.contains_key(key),
            "{key} is missing from the projected record"
        );
    }
}

#[test]
fn a_session_id_that_is_not_the_canonical_shape_is_rejected() {
    // The session id is what a scoped read authorizes on, so free text, a path,
    // or a query string must never be able to ride it.
    for bad in [
        "",
        "../etc/passwd",
        "sess id",
        "SESS-A",
        "-leading",
        "trailing-",
        "sess?a=b",
    ] {
        let event = SandboxLifecycleV1::new(
            LifecycleAction::Created,
            RuntimeBackendKind::Kubernetes,
            bad,
            AuditIdentity::reconciler(None),
            service(),
        );
        assert!(
            validate_lifecycle(&event).is_err(),
            "{bad:?} must not validate"
        );
    }
    let good = event(LifecycleAction::Created);
    assert!(validate_lifecycle(&good).is_ok());
}

#[test]
fn an_empty_optional_string_is_rejected_rather_than_projected() {
    let mut event = event(LifecycleAction::Created);
    event.attribution.creator_login = Some(String::new());
    assert!(
        validate_lifecycle(&event).is_err(),
        "an empty value is a field that should have been absent"
    );
}

#[test]
fn an_oversized_record_fails_rather_than_being_truncated() {
    let mut event = event(LifecycleAction::Created);
    event.runtime.runtime_id = Some("r".repeat(64));
    assert!(matches!(
        event.to_capture_event(EventLimits::new(64)),
        Err(EventError::TooLarge { .. })
    ));
}

#[test]
fn an_overlong_bounded_field_is_rejected() {
    let mut event = event(LifecycleAction::Created);
    event.runtime.runtime_id = Some("r".repeat(1_000));
    assert!(matches!(
        validate_lifecycle(&event),
        Err(EventError::Invalid {
            field: "runtime_id",
            ..
        })
    ));
}

#[test]
fn a_wrong_schema_version_is_rejected() {
    let mut event = event(LifecycleAction::Created);
    event.schema_version = 2;
    assert!(matches!(
        validate_lifecycle(&event),
        Err(EventError::Invalid {
            field: "schema_version",
            ..
        })
    ));
}

#[test]
fn every_action_and_reason_string_is_stable() {
    let actions: Vec<&str> = LifecycleAction::ALL.iter().map(|a| a.as_str()).collect();
    assert_eq!(
        actions,
        vec![
            "create_requested",
            "created",
            "create_failed",
            "delete_requested",
            "deleted",
            "delete_failed",
            "identity_backfilled",
            "identity_conflict",
        ]
    );
    assert_eq!(LifecycleReason::TriggerClosed.as_str(), "trigger_closed");
    assert_eq!(
        LifecycleReason::AttributionConflict.as_str(),
        "attribution_conflict"
    );
}
