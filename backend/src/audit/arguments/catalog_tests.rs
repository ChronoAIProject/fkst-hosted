//! Unit tests for the normative allowlist table.
//!
//! These pin the SHAPE of the table (no duplicate property, no empty list) and
//! the exact field sets issue #5671 declares normative. The per-DTO tests prove
//! each DTO uses the matching constant; the integration guard proves every live
//! operation has one.

use super::*;
use std::collections::BTreeSet;

/// Every list this module declares, with the operation it belongs to.
fn all_field_sets() -> Vec<(&'static str, &'static [&'static str])> {
    vec![
        ("invalid_input", INVALID_INPUT_FIELDS),
        ("github_login", GITHUB_LOGIN_FIELDS),
        ("github_login_callback", GITHUB_LOGIN_CALLBACK_FIELDS),
        ("github_refresh_token", GITHUB_REFRESH_TOKEN_FIELDS),
        ("github_broader_connect", GITHUB_BROADER_CONNECT_FIELDS),
        ("github_broader_callback", GITHUB_BROADER_CALLBACK_FIELDS),
        (
            "session_logs_oauth_callback",
            SESSION_LOGS_OAUTH_CALLBACK_FIELDS,
        ),
        ("create_repo", CREATE_REPO_FIELDS),
        ("uninstall_account", UNINSTALL_ACCOUNT_FIELDS),
        ("canvas_overview", CANVAS_OVERVIEW_FIELDS),
        ("canvas_repo_sessions", CANVAS_REPO_SESSIONS_FIELDS),
        ("canvas_create_session", CANVAS_CREATE_SESSION_FIELDS),
        ("canvas_stop_session", CANVAS_STOP_SESSION_FIELDS),
        ("canvas_create_work_item", CANVAS_CREATE_WORK_ITEM_FIELDS),
        ("canvas_session_outcomes", CANVAS_SESSION_OUTCOMES_FIELDS),
        ("canvas_outcome_blob", CANVAS_OUTCOME_BLOB_FIELDS),
        (
            "put_user_environment_profile",
            PUT_USER_ENVIRONMENT_PROFILE_FIELDS,
        ),
        (
            "get_user_environment_profile",
            GET_USER_ENVIRONMENT_PROFILE_FIELDS,
        ),
        (
            "delete_user_environment_profile",
            DELETE_USER_ENVIRONMENT_PROFILE_FIELDS,
        ),
        ("download_session_logs", DOWNLOAD_SESSION_LOGS_FIELDS),
        ("list_session_runs", LIST_SESSION_RUNS_FIELDS),
        ("session_log_manifest", SESSION_LOG_MANIFEST_FIELDS),
        ("session_log_file", SESSION_LOG_FILE_FIELDS),
        ("observe_session", OBSERVE_SESSION_FIELDS),
        ("github_app_webhook", GITHUB_APP_WEBHOOK_FIELDS),
        ("chat_turn", CHAT_TURN_FIELDS),
        ("operations_list_activity", OPERATIONS_LIST_ACTIVITY_FIELDS),
        (
            "operations_list_sandboxes",
            OPERATIONS_LIST_SANDBOXES_FIELDS,
        ),
    ]
}

#[test]
fn no_field_set_is_empty_or_repeats_a_property() {
    for (operation, fields) in all_field_sets() {
        assert!(!fields.is_empty(), "{operation} declares no properties");
        let unique: BTreeSet<&str> = fields.iter().copied().collect();
        assert_eq!(
            unique.len(),
            fields.len(),
            "{operation} repeats a property name"
        );
    }
}

/// Property names are the analytics store's column names and, downstream, the
/// UI's facets: an unbounded or punctuation-bearing name would break both.
#[test]
fn every_property_name_is_a_stable_snake_case_identifier() {
    for (operation, fields) in all_field_sets() {
        for field in fields {
            assert!(
                !field.is_empty()
                    && field
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{operation} declares the non-snake_case property {field}"
            );
        }
    }
}

/// The exact catalog from the issue, transcribed once more here so a silent
/// edit to the table fails rather than redefining the normative contract.
#[test]
fn the_normative_field_sets_match_the_issue_catalog() {
    assert_eq!(GITHUB_LOGIN_FIELDS, &["flow"]);
    assert_eq!(GITHUB_LOGIN_CALLBACK_FIELDS, &["flow", "result"]);
    assert_eq!(
        SESSION_LOGS_OAUTH_CALLBACK_FIELDS,
        &["flow", "session_id", "result"]
    );
    assert_eq!(
        CREATE_REPO_FIELDS,
        &[
            "owner",
            "name",
            "private",
            "description_present",
            "description_bytes"
        ]
    );
    assert_eq!(CANVAS_OVERVIEW_FIELDS, &["broader_visibility_requested"]);
    assert_eq!(
        CANVAS_CREATE_WORK_ITEM_FIELDS,
        &[
            "owner",
            "repo",
            "trigger_issue",
            "selected_label",
            "title_bytes",
            "body_present",
            "body_bytes"
        ]
    );
    assert_eq!(
        CANVAS_OUTCOME_BLOB_FIELDS,
        &["owner", "repo", "blob_sha", "download"]
    );
    assert_eq!(
        PUT_USER_ENVIRONMENT_PROFILE_FIELDS,
        &[
            "environment_name",
            "install_command_count",
            "variable_count",
            "secret_count"
        ]
    );
    assert_eq!(
        SESSION_LOG_FILE_FIELDS,
        &["session_id", "run_id_or_latest", "file_class", "tail_bytes"]
    );
    assert_eq!(OBSERVE_SESSION_FIELDS, &["session_id", "effective_limit"]);
    assert_eq!(
        INVALID_INPUT_FIELDS,
        &[
            "content_type",
            "content_length_declared",
            "body_bytes_observed"
        ]
    );
}

/// The forbidden columns, named as an explicit denial. A property that appears
/// here has to be deleted from the table, not merely renamed.
#[test]
fn no_field_set_names_a_forbidden_property() {
    const FORBIDDEN: &[&str] = &[
        "token",
        "access_token",
        "refresh_token",
        "authorization",
        "cookie",
        "code",
        "state",
        "signature",
        "body",
        "title",
        "description",
        "message",
        "path",
        "query",
        "url",
        "content",
        "install",
        "variables",
        "secrets",
        "name_query",
        "cursor",
        "actor_login",
    ];
    for (operation, fields) in all_field_sets() {
        for field in fields {
            assert!(
                !FORBIDDEN.contains(field),
                "{operation} declares the forbidden property {field}"
            );
        }
    }
}

#[test]
fn a_spec_carries_its_dto_name_and_field_list() {
    let spec = SafeArgumentSpec::new("SafeThing", GITHUB_LOGIN_FIELDS);
    assert_eq!(spec.dto, "SafeThing");
    assert_eq!(spec.fields, GITHUB_LOGIN_FIELDS);
}
