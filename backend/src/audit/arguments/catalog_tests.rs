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
        ("canvas_repo_schedules", CANVAS_REPO_SCHEDULES_FIELDS),
        ("canvas_schedule_detail", CANVAS_SCHEDULE_DETAIL_FIELDS),
        ("canvas_schedule_run", CANVAS_SCHEDULE_RUN_FIELDS),
        ("canvas_pause_schedule", CANVAS_PAUSE_SCHEDULE_FIELDS),
        ("canvas_resume_schedule", CANVAS_RESUME_SCHEDULE_FIELDS),
        ("canvas_run_schedule_now", CANVAS_RUN_SCHEDULE_NOW_FIELDS),
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

/// The exact catalog from the issue, transcribed once more here so a silent edit
/// to the table fails rather than redefining the normative contract.
///
/// EVERY set is pinned, not a sample of them. A DTO's `ALLOWED_FIELDS` IS the
/// catalog constant, so [`super::super::test_support::assert_policy_matches`]
/// compares a constant with itself and can never notice a widening; these
/// literals are the only thing standing between a newly invented property name
/// and a production record, so a set left unpinned is a set with no guard at all.
///
/// The four tests below are split by area purely so a failure names the area
/// that drifted.
#[test]
fn the_normative_auth_and_repository_field_sets_match_the_issue_catalog() {
    assert_eq!(GITHUB_LOGIN_FIELDS, &["flow"]);
    assert_eq!(GITHUB_LOGIN_CALLBACK_FIELDS, &["flow", "result"]);
    assert_eq!(GITHUB_REFRESH_TOKEN_FIELDS, &["flow", "result"]);
    assert_eq!(GITHUB_BROADER_CONNECT_FIELDS, &["flow"]);
    assert_eq!(GITHUB_BROADER_CALLBACK_FIELDS, &["flow", "result"]);
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
    assert_eq!(UNINSTALL_ACCOUNT_FIELDS, &["owner"]);
    assert_eq!(
        INVALID_INPUT_FIELDS,
        &[
            "content_type",
            "content_length_declared",
            "body_bytes_observed"
        ]
    );
}

/// The canvas sets. `canvas_create_session` is the widest and most
/// content-adjacent set in the table — the request it describes carries a
/// session name, install commands, and disposable variable/secret keys and
/// values — which is precisely why its property list is pinned literally.
///
/// `*_refs_truncated` are the issue's required truncation markers rather than
/// additional table columns (see the module docs).
#[test]
fn the_normative_canvas_field_sets_match_the_issue_catalog() {
    assert_eq!(CANVAS_OVERVIEW_FIELDS, &["broader_visibility_requested"]);
    assert_eq!(CANVAS_REPO_SESSIONS_FIELDS, &["owner", "repo"]);
    assert_eq!(
        CANVAS_CREATE_SESSION_FIELDS,
        &[
            "owner",
            "repo",
            "package_refs",
            "package_count",
            "package_refs_truncated",
            "manifest_refs",
            "manifest_count",
            "manifest_refs_truncated",
            "work_label",
            "environment_name",
            "disposable_environment_present",
            "disposable_variable_count",
            "disposable_secret_count",
            "source_branch",
            "target_branch",
            "auto_merge",
            "log_access_count",
            "collaborator_count",
            "output_language"
        ]
    );
    assert_eq!(
        CANVAS_STOP_SESSION_FIELDS,
        &["owner", "repo", "trigger_issue"]
    );
    // The scheduled-workflow surface. Nothing out of a definition's BODY appears
    // here — not the workflow id, not the arguments, not the cadence — because
    // that is author-written content, and a record names what was addressed.
    assert_eq!(CANVAS_REPO_SCHEDULES_FIELDS, &["owner", "repo"]);
    assert_eq!(
        CANVAS_SCHEDULE_DETAIL_FIELDS,
        &["owner", "repo", "schedule_issue"]
    );
    assert_eq!(
        CANVAS_SCHEDULE_RUN_FIELDS,
        &["owner", "repo", "schedule_issue", "slot"]
    );
    assert_eq!(
        CANVAS_PAUSE_SCHEDULE_FIELDS,
        &["owner", "repo", "schedule_issue"]
    );
    assert_eq!(
        CANVAS_RESUME_SCHEDULE_FIELDS,
        &["owner", "repo", "schedule_issue"]
    );
    assert_eq!(
        CANVAS_RUN_SCHEDULE_NOW_FIELDS,
        &["owner", "repo", "schedule_issue"]
    );
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
        CANVAS_SESSION_OUTCOMES_FIELDS,
        &["owner", "repo", "trigger_issue"]
    );
    assert_eq!(
        CANVAS_OUTCOME_BLOB_FIELDS,
        &["owner", "repo", "blob_sha", "download"]
    );
}

/// The environment-profile, log, and observe sets.
#[test]
fn the_normative_environment_and_log_field_sets_match_the_issue_catalog() {
    assert_eq!(
        PUT_USER_ENVIRONMENT_PROFILE_FIELDS,
        &[
            "environment_name",
            "install_command_count",
            "variable_count",
            "secret_count"
        ]
    );
    assert_eq!(GET_USER_ENVIRONMENT_PROFILE_FIELDS, &["environment_name"]);
    assert_eq!(
        DELETE_USER_ENVIRONMENT_PROFILE_FIELDS,
        &["environment_name"]
    );
    assert_eq!(
        DOWNLOAD_SESSION_LOGS_FIELDS,
        &["session_id", "run_id_or_latest", "mode"]
    );
    assert_eq!(LIST_SESSION_RUNS_FIELDS, &["session_id"]);
    assert_eq!(
        SESSION_LOG_MANIFEST_FIELDS,
        &["session_id", "run_id_or_latest"]
    );
    assert_eq!(
        SESSION_LOG_FILE_FIELDS,
        &["session_id", "run_id_or_latest", "file_class", "tail_bytes"]
    );
    assert_eq!(OBSERVE_SESSION_FIELDS, &["session_id", "effective_limit"]);
}

/// The webhook set, the chat turn, and the two reserved operations sets.
///
/// The webhook's is the other set worth pinning literally: everything but
/// `signature_valid` is populated from a request body an unauthenticated caller
/// composed, which only a verified HMAC makes trustworthy.
#[test]
fn the_normative_webhook_chat_and_operations_field_sets_match_the_issue_catalog() {
    assert_eq!(
        GITHUB_APP_WEBHOOK_FIELDS,
        &[
            "signature_valid",
            "event_type",
            "action",
            "installation_id",
            "repo_full_name",
            "trigger_issue",
            "delivery_id",
            "handling"
        ]
    );
    assert_eq!(
        CHAT_TURN_FIELDS,
        &[
            "message_count",
            "user_message_count",
            "assistant_message_count",
            "total_content_bytes",
            "last_message_bytes",
            "broader_visibility_requested"
        ]
    );
    assert_eq!(
        OPERATIONS_LIST_ACTIVITY_FIELDS,
        &[
            "scope",
            "requested_scope",
            "record_kind",
            "from",
            "to",
            "limit",
            "cursor_present",
            "actor_filter_present",
            "session_id",
            "repo_full_name",
            "trigger_issue",
            "request_id",
            "method",
            "operation_id",
            "status",
            // Both accepted status/outcome filters become source predicates, so
            // both are recorded: a record naming only `status` would describe a
            // narrower query than the one that ran.
            "status_class",
            "outcome"
        ]
    );
    assert_eq!(
        OPERATIONS_LIST_SANDBOXES_FIELDS,
        &[
            "scope",
            "requested_scope",
            "session_id",
            "repo_full_name",
            "trigger_issue",
            "status",
            "backend",
            "creator_id",
            "creator_login",
            // Every filter the endpoint accepts is recorded, because every one of
            // them NARROWS an already-authorized row set: a record naming only
            // some of them would describe a wider query than the one that ran.
            // There is no `limit` — the endpoint returns the complete authorized
            // snapshot or an explicit capacity failure.
            "attribution_source"
        ]
    );
}

/// The pinning tests above are only complete while they cover the whole table:
/// a set added to [`all_field_sets`] without a literal above would be guarded by
/// nothing. This count is the reminder to add both.
#[test]
fn every_declared_field_set_is_pinned_by_the_tests_above() {
    const PINNED: usize = 34;
    assert_eq!(
        all_field_sets().len(),
        PINNED,
        "a field set was added or removed: pin it literally in the matching \
         the_normative_*_field_sets_match_the_issue_catalog test too"
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
