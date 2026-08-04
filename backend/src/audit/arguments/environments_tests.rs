//! Unit tests for the named-environment safe arguments.

use super::*;
use crate::audit::arguments::test_support::{
    assert_no_canary, assert_policy_matches, assert_within_allowlist, properties, string,
};

const CANARIES: &[&str] = &[
    "canary-install-command",
    "canary-variable-key",
    "canary-variable-value",
    "canary-secret-key",
    "canary-secret-value",
];

#[test]
fn every_environment_dto_is_wired_to_its_declared_policy() {
    assert_policy_matches::<SafePutEnvironmentProfile>();
    assert_policy_matches::<SafeGetEnvironmentProfile>();
    assert_policy_matches::<SafeDeleteEnvironmentProfile>();
}

/// A profile is a bag of secrets: the record keeps the name and three counts,
/// and the DTO has no field that could hold a command, a key, or a value.
#[test]
fn a_put_record_carries_the_name_and_three_counts_only() {
    let safe = PutEnvironmentProfileInput {
        environment_name: "node-20",
        install_command_count: 3,
        variable_count: 2,
        secret_count: 5,
    }
    .to_safe_audit_arguments();
    assert_within_allowlist(&safe);
    assert_no_canary(&safe, CANARIES);

    let values = properties(&safe);
    assert_eq!(values.len(), 4);
    assert_eq!(
        string(&values, "environment_name").as_deref(),
        Some("node-20")
    );
    assert_eq!(
        values
            .get("install_command_count")
            .and_then(serde_json::Value::as_u64),
        Some(3)
    );
    assert_eq!(
        values
            .get("variable_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    assert_eq!(
        values
            .get("secret_count")
            .and_then(serde_json::Value::as_u64),
        Some(5)
    );
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Parsed);
}

#[test]
fn the_read_and_delete_records_carry_only_the_environment_name() {
    for values in [
        properties(&SafeGetEnvironmentProfile::new("node-20")),
        properties(&SafeDeleteEnvironmentProfile::new("node-20")),
    ] {
        assert_eq!(values.len(), 1);
        assert_eq!(
            string(&values, "environment_name").as_deref(),
            Some("node-20")
        );
    }
}

/// A name outside the store's validated form is a probe string, not an
/// environment: it is dropped and the record says the input was invalid.
#[test]
fn an_unvalidated_name_is_dropped_and_marks_the_record_invalid() {
    let read = SafeGetEnvironmentProfile::new("canary-Not_A_Valid_Name");
    assert!(properties(&read).is_empty());
    assert_eq!(read.parse_status(), ArgumentsParseStatus::Invalid);
    assert_no_canary(&read, &["canary-Not_A_Valid_Name"]);

    let removed = SafeDeleteEnvironmentProfile::new("canary-Not_A_Valid_Name");
    assert!(properties(&removed).is_empty());
    assert_eq!(removed.parse_status(), ArgumentsParseStatus::Invalid);
    assert_no_canary(&removed, &["canary-Not_A_Valid_Name"]);

    let put = PutEnvironmentProfileInput {
        environment_name: "canary-Not_A_Valid_Name",
        install_command_count: 1,
        variable_count: 0,
        secret_count: 0,
    }
    .to_safe_audit_arguments();
    assert!(!properties(&put).contains_key("environment_name"));
    assert_eq!(
        properties(&put)
            .get("install_command_count")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "the counts still describe the attempt"
    );
    assert_eq!(put.parse_status(), ArgumentsParseStatus::Invalid);
}

/// The one operation on this surface with no arguments at all reports
/// `not_applicable` rather than pretending something was unavailable.
#[test]
fn listing_profiles_is_declared_as_taking_no_arguments() {
    assert_eq!(
        crate::audit::arguments::test_support::default_status("list_user_environment_profiles"),
        ArgumentsParseStatus::NotApplicable
    );
}
