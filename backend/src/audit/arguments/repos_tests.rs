//! Unit tests for the repository/installation safe arguments.

use super::*;
use crate::audit::arguments::test_support::{
    assert_no_canary, assert_policy_matches, assert_within_allowlist, properties, string,
};

const DESCRIPTION_CANARY: &str = "canary-repository-description-text";

fn create_repo(description: Option<&str>) -> SafeCreateRepo {
    CreateRepoInput {
        owner: "acme",
        name: "site",
        private: true,
        description,
    }
    .to_safe_audit_arguments()
}

#[test]
fn both_dtos_are_wired_to_their_declared_policies() {
    assert_policy_matches::<SafeCreateRepo>();
    assert_policy_matches::<SafeUninstallAccount>();
}

#[test]
fn a_description_contributes_only_its_presence_and_size() {
    let safe = create_repo(Some(DESCRIPTION_CANARY));
    assert_within_allowlist(&safe);
    assert_no_canary(&safe, &[DESCRIPTION_CANARY]);

    let values = properties(&safe);
    assert_eq!(string(&values, "owner").as_deref(), Some("acme"));
    assert_eq!(string(&values, "name").as_deref(), Some("site"));
    assert_eq!(values.get("private").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        values.get("description_present").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        values
            .get("description_bytes")
            .and_then(serde_json::Value::as_u64),
        Some(DESCRIPTION_CANARY.len() as u64)
    );
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Parsed);
}

#[test]
fn an_absent_description_records_a_zero_size() {
    let values = properties(&create_repo(None));
    assert_eq!(
        values.get("description_present").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        values
            .get("description_bytes")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
}

/// A name the route would reject is dropped rather than echoed, and the record
/// says so — the counts still describe the attempt.
#[test]
fn an_invalid_name_is_dropped_and_marks_the_record_invalid() {
    let safe = CreateRepoInput {
        owner: "acme",
        name: "canary/../escape",
        private: false,
        description: None,
    }
    .to_safe_audit_arguments();
    let values = properties(&safe);
    assert!(!values.contains_key("name"));
    assert_eq!(string(&values, "owner").as_deref(), Some("acme"));
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Invalid);
    assert_no_canary(&safe, &["canary/../escape"]);
}

#[test]
fn uninstall_records_the_validated_owner_only() {
    let safe = SafeUninstallAccount::new("acme");
    assert_within_allowlist(&safe);
    assert_eq!(string(&properties(&safe), "owner").as_deref(), Some("acme"));
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Parsed);

    let invalid = SafeUninstallAccount::new("canary owner with spaces");
    assert!(properties(&invalid).is_empty());
    assert_eq!(invalid.parse_status(), ArgumentsParseStatus::Invalid);
    assert_no_canary(&invalid, &["canary owner with spaces"]);
}
