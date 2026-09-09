//! Router tests for the health read surface: authorization equivalence with the log
//! path, the listing shape, and one full report (including the traversal guard).

use super::test_support::*;
use crate::routes::logs::test_support::{
    body_json, get, github_user_401, github_user_ok, AUTHOR_ID, SESSION_ID,
};
use serde_json::json;

#[tokio::test]
async fn an_authorized_caller_gets_the_listing_newest_first() {
    let index = index_json(json!([
        entry(REPORT_ID, "2026-07-30T14:15:00Z", "stalled", 600),
        entry("older-report", "2026-07-30T14:05:00Z", "working", 600),
    ]));
    let (state, _github, _storage) = live_state(&[(index_key(), index.into_bytes())]).await;

    let response = get(
        state,
        &format!("/api/v1/sessions/{SESSION_ID}/health"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), 200);

    let body = body_json(response).await;
    assert_eq!(body["session_id"], SESSION_ID);
    assert_eq!(body["reports"].as_array().expect("array").len(), 2);
    assert_eq!(body["reports"][0]["id"], REPORT_ID);
    assert_eq!(body["reports"][0]["status"], "stalled");
    assert_eq!(body["reports"][0]["status_raw"], "stalled");
    assert_eq!(body["reports"][0]["headline"], "a headline");
    assert_eq!(body["reports"][0]["producer"], "fkst-health@0.1.0");
    assert_eq!(body["latest"]["id"], REPORT_ID);
}

#[tokio::test]
async fn an_unauthorized_caller_gets_403() {
    let github = github_user_ok("stranger", 9999).await;
    let (storage, _storage_server) = storage_with(&[]).await;
    let state = health_state(github.uri(), Some(storage), None);

    let response = get(
        state,
        &format!("/api/v1/sessions/{SESSION_ID}/health"),
        Some("gho_stranger"),
    )
    .await;
    assert_eq!(response.status(), 403);
}

#[tokio::test]
async fn an_unknown_session_is_404_and_reveals_nothing_more() {
    let github = github_user_ok("author", AUTHOR_ID).await;
    let (storage, _storage_server) = storage_with(&[]).await;
    let state = health_state(github.uri(), Some(storage), None);

    let response = get(
        state,
        "/api/v1/sessions/unknown-session/health",
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn an_invalid_token_is_401() {
    let github = github_user_401().await;
    let (storage, _storage_server) = storage_with(&[]).await;
    let state = health_state(github.uri(), Some(storage), None);
    let response = get(
        state,
        &format!("/api/v1/sessions/{SESSION_ID}/health"),
        Some("gho_bogus"),
    )
    .await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn a_known_report_id_returns_the_parsed_fields_and_a_verbatim_body() {
    let body_markdown = "## What this session is doing\n\n---\n\n+++ not front matter +++\n";
    let index = index_json(json!([entry(
        REPORT_ID,
        "2026-07-30T14:15:00Z",
        "stalled",
        600
    )]));
    let (state, _github, _storage) = live_state(&[
        (index_key(), index.into_bytes()),
        (
            format!("health/{SESSION_ID}/{REPORT_ID}.md"),
            report_body(body_markdown).into_bytes(),
        ),
    ])
    .await;

    let response = get(
        state,
        &format!("/api/v1/sessions/{SESSION_ID}/health/{REPORT_ID}"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), 200);

    let body = body_json(response).await;
    assert_eq!(body["id"], REPORT_ID);
    assert_eq!(body["status"], "stalled");
    assert_eq!(body["status_raw"], "stalled");
    assert_eq!(body["confidence"], "high");
    assert_eq!(body["window_start"], "2026-07-30T14:05:00Z");
    assert_eq!(body["expected_interval_secs"], 600);
    assert_eq!(body["evidence"][0]["key"], "codex_runs_started");
    assert_eq!(body["work_items"][0]["number"], 812);
    assert_eq!(
        body["body_markdown"], body_markdown,
        "the body round-trips byte for byte, including embedded --- and +++"
    );
}

#[tokio::test]
async fn an_unknown_or_traversal_shaped_report_id_is_404_without_a_storage_read() {
    let index = index_json(json!([entry(
        REPORT_ID,
        "2026-07-30T14:15:00Z",
        "working",
        600
    )]));
    let (state, _github, storage_server) = live_state(&[(index_key(), index.into_bytes())]).await;

    for id in ["no-such-report", "..%2F..%2Fetc%2Fpasswd", "latest"] {
        let response = get(
            state.clone(),
            &format!("/api/v1/sessions/{SESSION_ID}/health/{id}"),
            Some("gho_author"),
        )
        .await;
        assert_eq!(response.status(), 404, "id {id}");
    }

    // Only the index was ever fetched — never a caller-shaped key.
    let downloads: Vec<String> = storage_server
        .received_requests()
        .await
        .expect("requests")
        .into_iter()
        .filter_map(|request| {
            request
                .url
                .query_pairs()
                .find(|(name, _)| name == "key")
                .map(|(_, value)| value.to_string())
        })
        .collect();
    assert!(
        downloads.iter().all(|key| key == &index_key()),
        "a caller-supplied id must never form a storage key: {downloads:?}"
    );
}

#[tokio::test]
async fn a_repeated_listing_within_the_ttl_issues_one_storage_read() {
    // A dashboard rendering N session cards must not issue N uncached storage GETs.
    let index = index_json(json!([entry(
        REPORT_ID,
        "2026-07-30T14:15:00Z",
        "working",
        600
    )]));
    let (state, _github, storage_server) = live_state(&[(index_key(), index.into_bytes())]).await;

    for _ in 0..3 {
        let response = get(
            state.clone(),
            &format!("/api/v1/sessions/{SESSION_ID}/health"),
            Some("gho_author"),
        )
        .await;
        assert_eq!(response.status(), 200);
    }

    let index_reads = storage_server
        .received_requests()
        .await
        .expect("requests")
        .into_iter()
        .filter(|request| {
            request
                .url
                .query_pairs()
                .any(|(name, value)| name == "key" && value == index_key())
        })
        .count();
    assert_eq!(index_reads, 1, "the TTL cache served the repeats");
}

#[tokio::test]
async fn a_corrupt_index_is_served_as_empty_rather_than_erroring() {
    let (state, _github, _storage) = live_state(&[(index_key(), b"{ truncated".to_vec())]).await;

    let response = get(
        state,
        &format!("/api/v1/sessions/{SESSION_ID}/health"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["reports"].as_array().expect("array").len(), 0);
    assert_eq!(body["staleness"]["state"], "never_reported");
}

#[tokio::test]
async fn an_unrecognized_status_maps_to_unknown_and_keeps_the_raw_string() {
    let index = index_json(json!([entry(
        REPORT_ID,
        "2026-07-30T14:15:00Z",
        "thriving",
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
    assert_eq!(body["reports"][0]["status"], "unknown");
    assert_eq!(body["reports"][0]["status_raw"], "thriving");
}
