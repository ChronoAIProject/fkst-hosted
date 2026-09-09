//! Read-endpoint tests: the scope must arrive complete, the window is bounded,
//! and a page carries only already-authorized rows.

use axum::http::StatusCode;

use crate::audit_relay::http::tests::{call, get_request, json_request};
use crate::audit_relay::test_support::{relay, READ_TOKEN, WRITE_TOKEN};

const ALICE: &str = "a1111111-1111-4111-8111-111111111111";
const BOB: &str = "b1111111-1111-4111-8111-111111111111";
const STARTS: &str = "/internal/v1/audit/request-starts";

/// Seed one durable request for Alice (101) and one for Bob (202).
async fn seed(router: &axum::Router) {
    for (event_id, actor) in [(ALICE, 101i64), (BOB, 202)] {
        call(
            router,
            json_request(
                "POST",
                STARTS,
                Some(WRITE_TOKEN),
                &crate::audit_relay::test_support::start(event_id),
            ),
        )
        .await;
        call(
            router,
            json_request(
                "PUT",
                &format!("/internal/v1/audit/requests/{event_id}/completion"),
                Some(WRITE_TOKEN),
                &crate::audit_relay::test_support::completion(event_id, Some(actor)),
            ),
        )
        .await;
    }
}

fn records_uri(extra: &str) -> String {
    format!(
        "/internal/v1/audit/records?record_kind=api_request\
         &from=2026-07-30T12:00:00.000Z&to=2026-08-01T12:00:00.000Z&limit=50&{extra}"
    )
}

#[tokio::test]
async fn a_personal_read_returns_only_the_named_actors_rows() {
    let (_dir, _state, router) = relay();
    seed(&router).await;
    let (status, body) = call(
        &router,
        get_request(&records_uri("scope=mine&actor_id=101"), Some(READ_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["event_id"], ALICE);
    assert_eq!(rows[0]["record_kind"], "api_request");
    assert_eq!(rows[0]["delivery_state"], "queued");
    // The stored body is echoed verbatim, not rebuilt.
    assert_eq!(rows[0]["terminal"]["status_code"], 200);
}

#[tokio::test]
async fn a_global_read_returns_every_row() {
    let (_dir, _state, router) = relay();
    seed(&router).await;
    let (status, body) = call(
        &router,
        get_request(&records_uri("scope=all"), Some(READ_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["rows"].as_array().expect("rows").len(), 2);
}

#[tokio::test]
async fn a_mine_scope_without_an_actor_id_is_refused_not_widened() {
    let (_dir, _state, router) = relay();
    seed(&router).await;
    let (status, body) = call(
        &router,
        get_request(&records_uri("scope=mine"), Some(READ_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");
}

#[tokio::test]
async fn an_unknown_record_kind_is_refused() {
    let (_dir, _state, router) = relay();
    let uri = "/internal/v1/audit/records?scope=all&record_kind=everything\
               &from=2026-07-30T12:00:00.000Z&to=2026-08-01T12:00:00.000Z&limit=50";
    let (status, _) = call(&router, get_request(uri, Some(READ_TOKEN))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_inverted_or_oversized_window_is_refused() {
    let (_dir, _state, router) = relay();
    let inverted = "/internal/v1/audit/records?scope=all&record_kind=api_request\
                    &from=2026-08-01T12:00:00.000Z&to=2026-07-30T12:00:00.000Z&limit=50";
    let (status, _) = call(&router, get_request(inverted, Some(READ_TOKEN))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let too_wide = "/internal/v1/audit/records?scope=all&record_kind=api_request\
                    &from=2020-01-01T00:00:00.000Z&to=2026-08-01T12:00:00.000Z&limit=50";
    let (status, _) = call(&router, get_request(too_wide, Some(READ_TOKEN))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn half_a_cursor_is_refused() {
    let (_dir, _state, router) = relay();
    let (status, _) = call(
        &router,
        get_request(
            &records_uri("scope=all&cursor_timestamp=2026-07-31T12:00:00.000Z"),
            Some(READ_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_page_is_clamped_to_the_configured_row_ceiling() {
    let (_dir, _state, router) = relay();
    seed(&router).await;
    let (status, body) = call(
        &router,
        get_request(
            "/internal/v1/audit/records?scope=all&record_kind=api_request\
             &from=2026-07-30T12:00:00.000Z&to=2026-08-01T12:00:00.000Z&limit=100000",
            Some(READ_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["rows"].as_array().expect("rows").len(), 2);
}

#[tokio::test]
async fn an_in_flight_request_is_not_returned() {
    let (_dir, _state, router) = relay();
    call(
        &router,
        json_request(
            "POST",
            STARTS,
            Some(WRITE_TOKEN),
            &crate::audit_relay::test_support::start(ALICE),
        ),
    )
    .await;
    let (status, body) = call(
        &router,
        get_request(&records_uri("scope=all"), Some(READ_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["rows"].as_array().expect("rows").is_empty());
}

#[tokio::test]
async fn an_equivalent_timestamp_rendering_still_selects_the_same_rows() {
    // `terminal_at` is TEXT and compared lexicographically, so a window bound
    // written as `+00:00` with no fractional part would sort BELOW the stored
    // `…000Z` form ('.' < 'Z') and silently drop a page of rows. The handler
    // therefore normalizes both bounds before they reach the SQL.
    let (_dir, _state, router) = relay();
    seed(&router).await;

    let (status, canonical_body) = call(
        &router,
        get_request(&records_uri("scope=all"), Some(READ_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let canonical_rows = canonical_body["rows"].as_array().expect("rows").len();
    assert!(canonical_rows > 0, "the fixture must return something");

    // `from` is the SAME instant as the seeded rows' `terminal_at`, written
    // without the millisecond field. As raw text it sorts ABOVE the stored
    // `…12:00:00.120Z` prefix boundary ('Z' > '.'), so an un-normalized bound
    // silently returns an empty, confidently-complete page.
    let uri = "/internal/v1/audit/records?scope=all&record_kind=api_request\
               &from=2026-07-31T12:00:00Z&to=2026-08-01T12:00:00Z&limit=50";
    let (status, body) = call(&router, get_request(uri, Some(READ_TOKEN))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["rows"].as_array().expect("rows").len(),
        canonical_rows,
        "an equivalent rendering of the same window must not change the page"
    );
}
