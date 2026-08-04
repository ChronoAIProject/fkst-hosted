//! Unit tests for the canvas read/identifier safe arguments.

use super::*;
use crate::audit::arguments::test_support::{
    assert_no_canary, assert_policy_matches, assert_within_allowlist, properties, string,
};

#[test]
fn every_canvas_read_dto_is_wired_to_its_declared_policy() {
    assert_policy_matches::<SafeCanvasOverview>();
    assert_policy_matches::<SafeCanvasRepoSessions>();
    assert_policy_matches::<SafeCanvasStopSession>();
    assert_policy_matches::<SafeCanvasSessionOutcomes>();
    assert_policy_matches::<SafeCanvasOutcomeBlob>();
}

/// The broader-visibility header is a GitHub credential: only its PRESENCE is a
/// property, and the DTO has no field that could hold the value.
#[test]
fn the_overview_records_only_the_broader_visibility_flag() {
    for requested in [true, false] {
        let safe = SafeCanvasOverview::new(requested);
        let values = properties(&safe);
        assert_eq!(values.len(), 1);
        assert_eq!(
            values
                .get("broader_visibility_requested")
                .and_then(|v| v.as_bool()),
            Some(requested)
        );
        assert_no_canary(&safe, &["canary-broader-token"]);
    }
}

#[test]
fn the_repository_pair_is_recorded_in_its_validated_form() {
    let safe = SafeCanvasRepoSessions::new("acme", "site");
    assert_within_allowlist(&safe);
    let values = properties(&safe);
    assert_eq!(string(&values, "owner").as_deref(), Some("acme"));
    assert_eq!(string(&values, "repo").as_deref(), Some("site"));
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Parsed);
}

/// A half-identified repository would be a correlation handle that matches
/// nothing, so an unvalidated segment drops BOTH the field and the `parsed`
/// status.
#[test]
fn an_unvalidated_segment_is_dropped_and_marks_the_record_invalid() {
    let safe = SafeCanvasRepoSessions::new("acme", "canary site/../escape");
    let values = properties(&safe);
    assert_eq!(string(&values, "owner").as_deref(), Some("acme"));
    assert!(!values.contains_key("repo"));
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Invalid);
    assert_no_canary(&safe, &["canary site/../escape"]);
}

#[test]
fn the_issue_scoped_reads_carry_their_trigger_issue() {
    for (safe, expected) in [
        (
            properties(&SafeCanvasStopSession::new("acme", "site", 42)),
            42,
        ),
        (
            properties(&SafeCanvasSessionOutcomes::new("acme", "site", 7)),
            7,
        ),
    ] {
        assert_eq!(
            safe.get("trigger_issue")
                .and_then(serde_json::Value::as_i64),
            Some(expected)
        );
        assert_eq!(string(&safe, "owner").as_deref(), Some("acme"));
        assert_eq!(string(&safe, "repo").as_deref(), Some("site"));
    }
}

/// `?name=` is caller-supplied free text that drives the download filename; the
/// blob sha already identifies the object exactly, in a validated form.
#[test]
fn the_blob_endpoint_records_the_sha_and_never_the_requested_name() {
    let safe = OutcomeBlobInput {
        owner: "acme",
        repo: "site",
        blob_sha: "deadbeefcafe",
        download: true,
    }
    .to_safe_audit_arguments();
    assert_within_allowlist(&safe);
    let values = properties(&safe);
    assert_eq!(string(&values, "blob_sha").as_deref(), Some("deadbeefcafe"));
    assert_eq!(values.get("download").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        values.len(),
        4,
        "owner, repo, blob_sha, download and no more"
    );
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Parsed);
}

#[test]
fn a_non_hex_blob_sha_is_dropped_rather_than_echoed() {
    let safe = OutcomeBlobInput {
        owner: "acme",
        repo: "site",
        blob_sha: "canary-not-a-sha",
        download: false,
    }
    .to_safe_audit_arguments();
    assert!(!properties(&safe).contains_key("blob_sha"));
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Invalid);
    assert_no_canary(&safe, &["canary-not-a-sha"]);
}
