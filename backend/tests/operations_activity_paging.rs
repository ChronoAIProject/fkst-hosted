//! Keyset-pagination and cursor tests for `/api/v1/operations/activity`.
//!
//! Split from `operations_activity_sources.rs`, which owns the source-predicate
//! and failure-mode halves of the same surface. The theme here is narrower and
//! worth isolating: a page is defined by the last row the caller actually
//! received, never by an offset into a result set authorization filtered — and a
//! cursor is a value a CALLER holds, so everything it carries is re-checked
//! before it is adopted.

mod operations_harness;

use axum::http::StatusCode;
use k8s_openapi::chrono::{DateTime, Duration, SecondsFormat, Utc};

use fkst_control_plane::operations::cursor::{self, CursorBinding, CursorKey};
use fkst_control_plane::operations::filters::{ActivityFilters, RecordKind, TimeRange};
use operations_harness::{
    error_code, harness, item_ids, minutes_ago, Row, Sources, ALICE, ROOT, SESSION,
};

fn dataset() -> Vec<Row> {
    (0..5)
        .map(|i| Row::api(&format!("ev-{i}"), ALICE.0, &minutes_ago(i + 1)))
        .collect()
}

#[tokio::test]
async fn pages_tile_with_a_keyset_cursor_and_never_repeat_a_row() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let first = harness.page(ALICE, "?limit=2").await;
    assert_eq!(item_ids(&first), vec!["ev-0", "ev-1"]);
    let cursor = first["next_cursor"].as_str().expect("another page exists");

    let second = harness
        .page(ALICE, &format!("?limit=2&cursor={cursor}"))
        .await;
    assert_eq!(item_ids(&second), vec!["ev-2", "ev-3"]);

    let third = harness
        .page(
            ALICE,
            &format!(
                "?limit=2&cursor={}",
                second["next_cursor"].as_str().expect("a third page")
            ),
        )
        .await;
    assert_eq!(item_ids(&third), vec!["ev-4"]);
    assert!(
        third["next_cursor"].is_null(),
        "the final page carries no cursor"
    );
}

/// A cursor is bound to its query: reusing it under a different viewer, scope,
/// or filter is a stable `400`, never a silent reset to page one.
#[tokio::test]
async fn a_cursor_from_another_query_is_refused_rather_than_silently_reset() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let first = harness.page(ALICE, "?limit=2").await;
    let cursor = first["next_cursor"]
        .as_str()
        .expect("another page exists")
        .to_string();

    for query in [
        format!("?limit=2&cursor={cursor}&method=GET"),
        format!("?limit=2&cursor={cursor}&record_kind=all&session_id={SESSION}"),
        format!("?limit=2&cursor={cursor}&from=2026-07-30T00:00:00Z&to=2026-07-30T01:00:00Z"),
    ] {
        let response = harness.get(ALICE, &query).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(error_code(response).await, "invalid_activity_cursor");
    }

    // Another VIEWER cannot resume this page either.
    let foreign = harness
        .get(ROOT, &format!("?scope=mine&limit=2&cursor={cursor}"))
        .await;
    assert_eq!(foreign.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code(foreign).await, "invalid_activity_cursor");

    // The page SIZE is deliberately not part of the query's identity: resuming
    // the same query with a different page size is a legitimate client choice.
    let resized = harness
        .page(ALICE, &format!("?limit=3&cursor={cursor}"))
        .await;
    assert_eq!(item_ids(&resized), vec!["ev-2", "ev-3", "ev-4"]);
}

/// A cursor's WINDOW is the one component a caller can choose freely and still
/// produce a matching digest — the digest is a plain SHA-256 over public data,
/// not a MAC, and everything else in it is re-derived server-side. So the
/// configured maximum range must be enforced on the resume path too, or an
/// admitted user could mint a 230-year window and buy a full-table scan per
/// request.
///
/// Both cursors below carry a CORRECT digest, so the control case proves the
/// refusal comes from the range bound and not from the binding check.
#[tokio::test]
async fn a_forged_cursor_cannot_widen_the_window_past_the_configured_maximum() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let now = Utc::now();

    // Control: a legitimately-shaped window inside the deployment maximum, with
    // a self-computed digest, resumes normally.
    let accepted = forged_cursor(now - Duration::days(7), now);
    let page = harness
        .page(ALICE, &format!("?limit=2&cursor={accepted}"))
        .await;
    assert_eq!(
        page["from"],
        (now - Duration::days(7)).to_rfc3339_opts(SecondsFormat::Millis, true),
        "the resumed page runs over the window its cursor names, not a re-derived one"
    );

    for (label, from, to) in [
        (
            "a window wider than FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS",
            now - Duration::days(365 * 60),
            now + Duration::days(365 * 170),
        ),
        (
            "a window entirely in the future",
            now + Duration::days(1),
            now + Duration::days(2),
        ),
    ] {
        let cursor = forged_cursor(from, to);
        let response = harness.get(ALICE, &format!("?cursor={cursor}")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
        assert_eq!(
            error_code(response).await,
            "invalid_activity_cursor",
            "{label}"
        );
    }
}

/// Mint a cursor for Alice's default personal query over an arbitrary window,
/// with the digest the server itself would compute for that window.
fn forged_cursor(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    let binding = CursorBinding {
        scope: "mine",
        viewer_id: Some(ALICE.0),
        session_id: None,
        record_kind: RecordKind::ApiRequest,
        range: TimeRange { from, to },
        filters: ActivityFilters::default(),
    };
    cursor::encode(
        &CursorKey {
            timestamp: to,
            event_id: "ev-forged".to_string(),
        },
        &binding,
    )
    .expect("the fixture cursor encodes")
}

#[tokio::test]
async fn a_malformed_or_oversized_cursor_is_the_same_stable_four_hundred() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    for cursor in ["abc", "!!!!", &"A".repeat(600)] {
        let response = harness.get(ALICE, &format!("?cursor={cursor}")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{cursor}");
        assert_eq!(error_code(response).await, "invalid_activity_cursor");
    }
}

/// Rows sharing a timestamp still page deterministically on the event id.
#[tokio::test]
async fn identical_timestamps_page_deterministically() {
    let shared = minutes_ago(7);
    let rows: Vec<Row> = ["ev-a", "ev-b", "ev-c"]
        .into_iter()
        .map(|id| Row::api(id, ALICE.0, &shared))
        .collect();
    let harness = harness(Sources::Posthog(rows), true).await;
    let page = harness.page(ALICE, "?limit=3").await;
    assert_eq!(item_ids(&page), vec!["ev-c", "ev-b", "ev-a"]);
}
