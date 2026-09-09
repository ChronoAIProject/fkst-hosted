//! Router tests for the heartbeat verdict as it is actually served: the liveness
//! gate, the fail-open paths, and the producer-declared cadence.

use super::test_support::*;
use crate::routes::logs::test_support::{body_json, get, github_user_ok, AUTHOR_ID, SESSION_ID};
use serde_json::json;

#[tokio::test]
async fn a_live_session_with_no_reports_is_200_empty_and_never_reported() {
    // Not a 404: the session exists and is authorized, there is simply nothing yet —
    // the distinction a user needs in the first ten minutes.
    let (state, _github, _storage) = live_state(&[]).await;
    let response = get(
        state,
        &format!("/api/v1/sessions/{SESSION_ID}/health"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), 200);

    let body = body_json(response).await;
    assert_eq!(body["reports"].as_array().expect("array").len(), 0);
    assert!(body["latest"].is_null());
    assert_eq!(body["staleness"]["state"], "never_reported");
}

#[tokio::test]
async fn storage_unconfigured_is_503_but_dispatch_disabled_is_not() {
    let github = github_user_ok("author", AUTHOR_ID).await;
    let state = health_state(github.uri(), None, None);
    let response = get(
        state,
        &format!("/api/v1/sessions/{SESSION_ID}/health"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), 503, "no object store at all");

    // Session dispatch disabled (`session_backend: None`) must still SERVE.
    let github = github_user_ok("author", AUTHOR_ID).await;
    let index = index_json(json!([entry(
        REPORT_ID,
        "2026-07-30T14:15:00Z",
        "working",
        600
    )]));
    let (storage, _storage_server) = storage_with(&[(index_key(), index.into_bytes())]).await;
    let state = health_state(github.uri(), Some(storage), None);
    let response = get(
        state,
        &format!("/api/v1/sessions/{SESSION_ID}/health"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["staleness"]["state"], "not_running");
    assert_eq!(body["reports"].as_array().expect("array").len(), 1);
}

/// THE false-alarm regression, end to end through the router: a reaped pod is the
/// normal end of a session's work, and must never render as an alarm.

#[tokio::test]
async fn a_reaped_session_reports_not_running_and_still_lists_its_history() {
    let github = github_user_ok("author", AUTHOR_ID).await;
    // A report far older than 2x its interval — "stale" if the gate were missing.
    let index = index_json(json!([entry(
        REPORT_ID,
        "2020-01-01T00:00:00Z",
        "working",
        600
    )]));
    let (storage, _storage_server) = storage_with(&[(index_key(), index.into_bytes())]).await;
    // Default fake => phase None => the runtime is gone.
    let state = health_state(
        github.uri(),
        Some(storage),
        Some(FakeSessionBackend::default()),
    );

    let body = body_json(
        get(
            state,
            &format!("/api/v1/sessions/{SESSION_ID}/health"),
            Some("gho_author"),
        )
        .await,
    )
    .await;

    assert_eq!(body["staleness"]["state"], "not_running");
    assert_ne!(body["staleness"]["state"], "stale");
    assert_eq!(
        body["reports"].as_array().expect("array").len(),
        1,
        "the history stays readable, it simply carries no alarm"
    );
}

#[tokio::test]
async fn a_status_summary_error_yields_not_running_never_stale() {
    let github = github_user_ok("author", AUTHOR_ID).await;
    let index = index_json(json!([entry(
        REPORT_ID,
        "2020-01-01T00:00:00Z",
        "working",
        600
    )]));
    let (storage, _storage_server) = storage_with(&[(index_key(), index.into_bytes())]).await;
    let state = health_state(
        github.uri(),
        Some(storage),
        Some(FakeSessionBackend::default().with_status_error()),
    );

    let body = body_json(
        get(
            state,
            &format!("/api/v1/sessions/{SESSION_ID}/health"),
            Some("gho_author"),
        )
        .await,
    )
    .await;
    assert_eq!(body["staleness"]["state"], "not_running");
}

#[tokio::test]
async fn a_live_runtime_whose_reports_stopped_is_stale() {
    let index = index_json(json!([entry(
        REPORT_ID,
        "2020-01-01T00:00:00Z",
        "working",
        600
    )]));
    let (state, _github, _storage) = live_state(&[(index_key(), index.into_bytes())]).await;

    let body = body_json(
        get(
            state,
            &format!("/api/v1/sessions/{SESSION_ID}/health"),
            Some("gho_author"),
        )
        .await,
    )
    .await;
    assert_eq!(body["staleness"]["state"], "stale");
    assert_eq!(body["staleness"]["expected_interval_secs"], 600);
    assert!(body["staleness"]["age_secs"].as_u64().expect("age") > 1200);
}

#[tokio::test]
async fn the_producers_own_interval_is_read_from_the_report() {
    // A deliberately non-default cadence: with a 10-minute constant this would be
    // stale, so serving `fresh` proves the value came from the report.
    let recent = k8s_openapi::chrono::Utc::now() - k8s_openapi::chrono::Duration::seconds(1500);
    let index = index_json(json!([entry(
        REPORT_ID,
        &recent.to_rfc3339_opts(k8s_openapi::chrono::SecondsFormat::Secs, true),
        "working",
        3600
    )]));
    let (state, _github, _storage) = live_state(&[(index_key(), index.into_bytes())]).await;

    let body = body_json(
        get(
            state,
            &format!("/api/v1/sessions/{SESSION_ID}/health"),
            Some("gho_author"),
        )
        .await,
    )
    .await;
    assert_eq!(body["staleness"]["state"], "fresh");
    assert_eq!(body["staleness"]["expected_interval_secs"], 3600);
}
