//! Unit tests for the log/observe safe arguments.

use super::*;
use crate::audit::arguments::test_support::{
    assert_no_canary, assert_policy_matches, assert_within_allowlist, properties, string,
};

const SESSION: &str = "8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e";

#[test]
fn every_log_dto_is_wired_to_its_declared_policy() {
    assert_policy_matches::<SafeDownloadSessionLogs>();
    assert_policy_matches::<SafeListSessionRuns>();
    assert_policy_matches::<SafeSessionLogManifest>();
    assert_policy_matches::<SafeSessionLogFile>();
    assert_policy_matches::<SafeObserveSession>();
}

#[test]
fn a_download_record_names_the_session_the_run_and_the_transport() {
    for (mode, expected) in [
        (LogDownloadMode::Bearer, "bearer"),
        (LogDownloadMode::BrowserRedirect, "browser_redirect"),
    ] {
        let safe = SafeDownloadSessionLogs::new(SESSION, Some("run-7"), mode);
        assert_within_allowlist(&safe);
        let values = properties(&safe);
        assert_eq!(string(&values, "session_id").as_deref(), Some(SESSION));
        assert_eq!(
            string(&values, "run_id_or_latest").as_deref(),
            Some("run-7")
        );
        assert_eq!(string(&values, "mode").as_deref(), Some(expected));
        assert_eq!(safe.parse_status(), ArgumentsParseStatus::Parsed);
    }
}

#[test]
fn an_absent_run_selector_records_the_authoritative_bundle() {
    let values = properties(&SafeDownloadSessionLogs::new(
        SESSION,
        None,
        LogDownloadMode::Bearer,
    ));
    assert_eq!(
        string(&values, "run_id_or_latest").as_deref(),
        Some("latest")
    );
}

/// A `?run=` value that is not a run id is never echoed; the record says the
/// input was invalid and keeps only what did validate.
#[test]
fn an_invalid_run_selector_is_dropped_rather_than_echoed() {
    let safe = SafeDownloadSessionLogs::new(
        SESSION,
        Some("canary-run/../escape"),
        LogDownloadMode::Bearer,
    );
    let values = properties(&safe);
    assert!(!values.contains_key("run_id_or_latest"));
    assert_eq!(string(&values, "session_id").as_deref(), Some(SESSION));
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Invalid);
    assert_no_canary(&safe, &["canary-run/../escape"]);
}

#[test]
fn the_runs_and_manifest_records_carry_their_identifiers_only() {
    let runs = properties(&SafeListSessionRuns::new(SESSION));
    assert_eq!(runs.len(), 1);
    assert_eq!(string(&runs, "session_id").as_deref(), Some(SESSION));

    let manifest = properties(&SafeSessionLogManifest::new(SESSION, Some("latest")));
    assert_eq!(manifest.len(), 2);
    assert_eq!(
        string(&manifest, "run_id_or_latest").as_deref(),
        Some("latest")
    );
}

/// The requested PATH is replaced by the bundle's own bounded class: an
/// unmatched path is a probe string, and the file's content is the response.
#[test]
fn a_file_read_records_a_bounded_class_and_never_the_requested_path() {
    for (label, expected) in [
        ("Driver", "driver"),
        ("Supervise", "supervise"),
        ("Codex", "codex"),
        ("Misc", "misc"),
        ("README", "readme"),
        ("Meta", "meta"),
        ("canary-unknown-label", "other"),
    ] {
        let safe = SessionLogFileInput {
            session_id: SESSION,
            run: None,
            file_label: label,
            tail_bytes: Some(4096),
        }
        .to_safe_audit_arguments();
        assert_within_allowlist(&safe);
        assert_no_canary(&safe, &["canary-unknown-label", "canary-log-content"]);
        let values = properties(&safe);
        assert_eq!(string(&values, "file_class").as_deref(), Some(expected));
        assert_eq!(
            values.get("tail_bytes").and_then(serde_json::Value::as_u64),
            Some(4096)
        );
    }
}

#[test]
fn an_absent_tail_selector_is_simply_omitted() {
    let values = properties(
        &SessionLogFileInput {
            session_id: SESSION,
            run: None,
            file_label: "Codex",
            tail_bytes: None,
        }
        .to_safe_audit_arguments(),
    );
    assert!(!values.contains_key("tail_bytes"));
}

/// The record must describe EXECUTION, so the clamped limit is what travels.
#[test]
fn observe_records_the_clamped_limit_the_handler_executed_with() {
    let safe = SafeObserveSession::new(SESSION, 10_000);
    assert_within_allowlist(&safe);
    let values = properties(&safe);
    assert_eq!(values.len(), 2);
    assert_eq!(
        values
            .get("effective_limit")
            .and_then(serde_json::Value::as_u64),
        Some(10_000)
    );
    assert_eq!(safe.parse_status(), ArgumentsParseStatus::Parsed);
}

#[test]
fn an_unvalidated_session_id_is_dropped_from_every_log_record() {
    let hostile = "canary-session/../escape";
    for safe in [
        properties(&SafeListSessionRuns::new(hostile)),
        properties(&SafeObserveSession::new(hostile, 500)),
        properties(&SafeSessionLogManifest::new(hostile, None)),
    ] {
        assert!(!safe.contains_key("session_id"));
        let rendered = serde_json::to_string(&safe).expect("serializes");
        assert!(!rendered.contains(hostile), "{rendered}");
    }
    assert_eq!(
        SafeListSessionRuns::new(hostile).parse_status(),
        ArgumentsParseStatus::Invalid
    );
}
