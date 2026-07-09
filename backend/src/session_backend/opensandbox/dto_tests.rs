//! Pure serde unit tests for the OpenSandbox wire DTOs (sibling `#[path]` module,
//! mirrors the repo's `*_tests.rs` convention). These exercise serialization/
//! deserialization edges with no network — the wiremock transport tests live in
//! `lifecycle_tests.rs`.

use std::collections::BTreeMap;

use super::*;

fn sample_request(timeout: Option<i64>) -> CreateSandboxRequest {
    CreateSandboxRequest {
        image: ImageSpec {
            uri: "python:3.11".to_string(),
            auth: None,
        },
        entrypoint: vec!["python".to_string(), "/app/main.py".to_string()],
        env: BTreeMap::from([("DEBUG".to_string(), "true".to_string())]),
        resource_limits: ResourceLimits(BTreeMap::from([
            ("cpu".to_string(), "500m".to_string()),
            ("memory".to_string(), "512Mi".to_string()),
        ])),
        timeout,
        metadata: BTreeMap::from([("name".to_string(), "demo".to_string())]),
        extensions: BTreeMap::new(),
    }
}

#[test]
fn create_request_serializes_timeout_none_as_literal_null() {
    // The cheapest proof of the literal-null contract: `None` must render as JSON
    // `null`, not be omitted (the API reads null as "no auto-expiry").
    let value = serde_json::to_value(sample_request(None)).expect("serialize");
    assert_eq!(value["timeout"], serde_json::Value::Null);
    // And the field is actually present (not skipped).
    assert!(value.as_object().unwrap().contains_key("timeout"));
}

#[test]
fn create_request_serializes_timeout_some_as_number() {
    let value = serde_json::to_value(sample_request(Some(900))).expect("serialize");
    assert_eq!(value["timeout"], serde_json::json!(900));
}

#[test]
fn create_request_uses_camel_case_and_free_form_resource_map() {
    let value = serde_json::to_value(sample_request(None)).expect("serialize");
    // camelCase rename of the struct field.
    assert_eq!(
        value["resourceLimits"],
        serde_json::json!({ "cpu": "500m", "memory": "512Mi" })
    );
    // Empty maps still render as `{}` (no skip).
    assert_eq!(value["extensions"], serde_json::json!({}));
    assert_eq!(value["env"], serde_json::json!({ "DEBUG": "true" }));
}

#[test]
fn resource_limits_round_trips_arbitrary_keys() {
    // Proves the free-form map is faithful: an extra key like `gpu` is preserved
    // (a fixed cpu/memory struct would drop it).
    let limits = ResourceLimits(BTreeMap::from([
        ("cpu".to_string(), "250m".to_string()),
        ("gpu".to_string(), "1".to_string()),
    ]));
    let value = serde_json::to_value(limits).expect("serialize");
    assert_eq!(value, serde_json::json!({ "cpu": "250m", "gpu": "1" }));
}

#[test]
fn image_spec_omits_auth_when_absent_and_includes_it_when_present() {
    let without = serde_json::to_value(ImageSpec {
        uri: "ubuntu:22.04".to_string(),
        auth: None,
    })
    .expect("serialize");
    assert!(without.as_object().unwrap().get("auth").is_none());

    let with = serde_json::to_value(ImageSpec {
        uri: "private.example.com/app:1".to_string(),
        auth: Some(RegistryAuth {
            username: "svc".to_string(),
            password: "pat".to_string(),
        }),
    })
    .expect("serialize");
    assert_eq!(
        with["auth"],
        serde_json::json!({ "username": "svc", "password": "pat" })
    );
}

#[test]
fn sandbox_state_maps_known_values() {
    for (wire, expected) in [
        ("Pending", SandboxState::Pending),
        ("Running", SandboxState::Running),
        ("Pausing", SandboxState::Pausing),
        ("Paused", SandboxState::Paused),
        ("Resuming", SandboxState::Resuming),
        ("Stopping", SandboxState::Stopping),
        ("Terminated", SandboxState::Terminated),
        ("Failed", SandboxState::Failed),
    ] {
        let got: SandboxState =
            serde_json::from_value(serde_json::json!(wire)).expect("deserialize state");
        assert_eq!(got, expected, "wire {wire}");
    }
}

#[test]
fn sandbox_state_unknown_value_is_captured_not_rejected() {
    let got: SandboxState =
        serde_json::from_value(serde_json::json!("SomeFutureState")).expect("deserialize");
    assert_eq!(got, SandboxState::Unknown("SomeFutureState".to_string()));
}

#[test]
fn sandbox_view_lifts_nested_status_to_flat_fields() {
    let wire = serde_json::json!({
        "id": "sbx-1",
        "status": { "state": "Running", "reason": "user_create", "message": "ready" },
        "metadata": { "name": "demo", "team": "ml" }
    });
    let view: SandboxView = serde_json::from_value(wire).expect("deserialize view");
    assert_eq!(view.id, "sbx-1");
    assert_eq!(view.state, SandboxState::Running);
    assert_eq!(view.reason.as_deref(), Some("user_create"));
    assert_eq!(view.message.as_deref(), Some("ready"));
    assert_eq!(view.metadata.get("team").map(String::as_str), Some("ml"));
    assert!(view.extensions.is_empty());
}

#[test]
fn sandbox_view_ignores_unknown_fields_and_defaults_optionals() {
    // A full `Sandbox` GET body carries fields this view does not model (image,
    // platform, entrypoint, createdAt, lastTransitionAt); they must be ignored, and
    // an absent reason/message/metadata must default rather than fail.
    let wire = serde_json::json!({
        "id": "sbx-2",
        "image": { "uri": "python:3.11" },
        "platform": null,
        "status": { "state": "Pending", "lastTransitionAt": "2026-07-09T00:00:00Z" },
        "entrypoint": ["python"],
        "createdAt": "2026-07-09T00:00:00Z"
    });
    let view: SandboxView = serde_json::from_value(wire).expect("deserialize view");
    assert_eq!(view.id, "sbx-2");
    assert_eq!(view.state, SandboxState::Pending);
    assert_eq!(view.reason, None);
    assert_eq!(view.message, None);
    assert!(view.metadata.is_empty());
    assert!(view.extensions.is_empty());
}
