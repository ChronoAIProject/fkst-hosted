//! Unit tests for the canvas mutation safe arguments.

use std::collections::BTreeMap;

use super::*;
use crate::audit::arguments::test_support::{
    assert_no_canary, assert_policy_matches, assert_within_allowlist, properties, string,
};
use crate::disposable_environment::DisposableEnvironmentRequest;

/// Every hostile value a create-session request can carry.
const CANARIES: &[&str] = &[
    "canary-session-name",
    "canary-install-command",
    "canary-variable-key",
    "canary-variable-value",
    "canary-secret-key",
    "canary-secret-value",
    "canary-log-grantee",
    "canary-collaborator",
];

fn request() -> CreateSessionRequest {
    let mut variables = BTreeMap::new();
    variables.insert(
        "canary-variable-key".to_string(),
        "canary-variable-value".to_string(),
    );
    let mut secrets = BTreeMap::new();
    secrets.insert(
        "canary-secret-key".to_string(),
        "canary-secret-value".to_string(),
    );
    serde_json::from_value::<CreateSessionRequest>(serde_json::json!({
        "name": "canary-session-name",
        "packages": ["acme/pkgs@main:packages/devloop"],
        "manifests": ["acme/pkgs@main:manifests/default.json"],
        "work_label": "fkst:work",
        "source_branch": "main",
        "target_branch": "fkst-hosted-default",
        "auto_merge": true,
        "log_access": ["canary-log-grantee", "  "],
        "collaborators": ["canary-collaborator"],
        "output_lang": "zh-CN",
    }))
    .map(|mut request| {
        request.disposable_environment = Some(DisposableEnvironmentRequest {
            install: vec!["canary-install-command".to_string()],
            variables,
            secrets,
        });
        request
    })
    .expect("the fixture request parses")
}

fn safe(request: &CreateSessionRequest) -> SafeCanvasCreateSession {
    CreateSessionInput {
        owner: "acme",
        repo: "site",
        request,
    }
    .to_safe_audit_arguments()
}

#[test]
fn both_mutation_dtos_are_wired_to_their_declared_policies() {
    assert_policy_matches::<SafeCanvasCreateSession>();
    assert_policy_matches::<SafeCanvasCreateWorkItem>();
}

/// The whole point of the create-session projection: names, commands, keys, and
/// values become counts and flags, and the references keep their canonical form.
#[test]
fn a_create_session_record_carries_shape_and_never_content() {
    let request = request();
    let safe = safe(&request);
    assert_within_allowlist(&safe);
    assert_no_canary(&safe, CANARIES);

    let values = properties(&safe);
    assert_eq!(string(&values, "owner").as_deref(), Some("acme"));
    assert_eq!(string(&values, "repo").as_deref(), Some("site"));
    assert_eq!(
        values.get("package_refs").and_then(|v| v.as_array()),
        Some(&vec![serde_json::json!("acme/pkgs@main:packages/devloop")])
    );
    assert_eq!(
        values
            .get("package_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        values
            .get("manifest_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(string(&values, "work_label").as_deref(), Some("fkst:work"));
    assert_eq!(string(&values, "source_branch").as_deref(), Some("main"));
    assert_eq!(
        string(&values, "target_branch").as_deref(),
        Some("fkst-hosted-default")
    );
    assert_eq!(string(&values, "output_language").as_deref(), Some("zh-CN"));
    assert_eq!(
        values.get("auto_merge").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        values
            .get("disposable_environment_present")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        values
            .get("disposable_variable_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        values
            .get("disposable_secret_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    // The blank log-access entry the request carried is not a grantee.
    assert_eq!(
        values
            .get("log_access_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        values
            .get("collaborator_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert!(
        !values.contains_key("environment_name"),
        "a disposable environment has no named environment"
    );
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Parsed);
}

/// References are re-rendered from the PARSED parts, so what lands in the record
/// is a value the strict grammar accepted — never the caller's raw line.
#[test]
fn only_references_the_strict_parser_accepts_are_recorded() {
    let mut request = request();
    request.packages = vec![
        "acme/pkgs@main:packages/devloop".to_string(),
        "canary-not-a-package-reference".to_string(),
        "acme/pkgs@main:../escape".to_string(),
    ];
    let safe = safe(&request);
    let values = properties(&safe);
    assert_eq!(
        values.get("package_refs").and_then(|v| v.as_array()),
        Some(&vec![serde_json::json!("acme/pkgs@main:packages/devloop")])
    );
    assert_eq!(
        values
            .get("package_count")
            .and_then(serde_json::Value::as_u64),
        Some(3),
        "the true count still describes the request"
    );
    assert_eq!(
        values
            .get("package_refs_truncated")
            .and_then(|v| v.as_bool()),
        Some(true),
        "dropping an entry makes the rendered list partial"
    );
    assert_no_canary(&safe, &["canary-not-a-package-reference", "../escape"]);
}

/// A list over the documented cap keeps a bounded prefix, the true count, and
/// the truncation marker — and never changes the business request.
#[test]
fn an_over_long_reference_list_is_bounded_with_its_true_count() {
    let mut request = request();
    request.packages = (0..MAX_REF_ENTRIES + 5)
        .map(|i| format!("acme/pkgs@main:packages/p{i}"))
        .collect();
    let safe = safe(&request);
    let values = properties(&safe);
    assert_eq!(
        values
            .get("package_refs")
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(MAX_REF_ENTRIES)
    );
    assert_eq!(
        values
            .get("package_count")
            .and_then(serde_json::Value::as_u64),
        Some((MAX_REF_ENTRIES + 5) as u64)
    );
    assert_eq!(
        values
            .get("package_refs_truncated")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        request.packages.len(),
        MAX_REF_ENTRIES + 5,
        "audit projection must never mutate the business request"
    );
}

/// The truncation marker is emitted only when something really was clipped, so
/// a reader can never mistake a complete list for a prefix.
#[test]
fn a_complete_reference_list_emits_no_truncation_marker() {
    let values = properties(&safe(&request()));
    assert!(!values.contains_key("package_refs_truncated"));
    assert!(!values.contains_key("manifest_refs_truncated"));
}

#[test]
fn a_named_environment_is_recorded_and_a_disposable_one_is_only_counted() {
    let mut request = request();
    request.disposable_environment = None;
    request.environment = Some("node-20".to_string());
    let values = properties(&safe(&request));
    assert_eq!(
        string(&values, "environment_name").as_deref(),
        Some("node-20")
    );
    assert_eq!(
        values
            .get("disposable_environment_present")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
}

/// The work item's title and body are issue free text: only their sizes travel.
#[test]
fn a_work_item_record_carries_the_resolved_label_and_only_text_sizes() {
    let safe = CreateWorkItemInput {
        owner: "acme",
        repo: "site",
        trigger_issue: 42,
        selected_label: "fkst:work",
        title: "canary-work-item-title",
        body: "canary-work-item-body",
    }
    .to_safe_audit_arguments();
    assert_within_allowlist(&safe);
    assert_no_canary(&safe, &["canary-work-item-title", "canary-work-item-body"]);

    let values = properties(&safe);
    assert_eq!(
        string(&values, "selected_label").as_deref(),
        Some("fkst:work")
    );
    assert_eq!(
        values
            .get("trigger_issue")
            .and_then(serde_json::Value::as_i64),
        Some(42)
    );
    assert_eq!(
        values
            .get("title_bytes")
            .and_then(serde_json::Value::as_u64),
        Some("canary-work-item-title".len() as u64)
    );
    assert_eq!(
        values.get("body_present").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        values.get("body_bytes").and_then(serde_json::Value::as_u64),
        Some("canary-work-item-body".len() as u64)
    );
}

#[test]
fn a_body_less_work_item_reports_a_zero_size() {
    let values = properties(
        &CreateWorkItemInput {
            owner: "acme",
            repo: "site",
            trigger_issue: 1,
            selected_label: "fkst:work",
            title: "t",
            body: "",
        }
        .to_safe_audit_arguments(),
    );
    assert_eq!(
        values.get("body_present").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        values.get("body_bytes").and_then(serde_json::Value::as_u64),
        Some(0)
    );
}
