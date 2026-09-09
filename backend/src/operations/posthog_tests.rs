//! PostHog query-client and source tests.
//!
//! These drive a real HTTP server, so the assertions are about the ACTUAL
//! outbound request — the query text, the parameter map, and the header — rather
//! than about an intermediate value the production path might not use.

use std::time::Duration;

use secrecy::SecretString;
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::*;
use crate::operations::filters::{ActivityFilters, RecordKind};
use crate::operations::record::DeliveryState;
use crate::operations::source::{ActivitySource, SourceError, SourceQuery};
use crate::operations::test_support::{authorized_session, mine, range};

const VIEWER_ID: i64 = 101;
const VIEWER: &str = "alice";
const SESSION: &str = "sess-alice";
const READ_KEY: &str = "phx_read_only_key";

fn client(server: &MockServer) -> PosthogQueryClient {
    PosthogQueryClient::new(
        format!("{}/api/projects/42/query/", server.uri()),
        SecretString::from(READ_KEY.to_string()),
        Duration::from_millis(2_000),
    )
    .expect("client builds")
}

fn query(record_kind: RecordKind, session: bool) -> SourceQuery {
    let session = session.then(|| authorized_session(SESSION, VIEWER_ID, VIEWER));
    SourceQuery {
        constraint: mine(VIEWER_ID, VIEWER, session),
        record_kind,
        range: range(),
        filters: ActivityFilters::default(),
        cursor: None,
        fetch_limit: 11,
    }
}

/// The `{columns, results}` envelope for one API-request row.
fn envelope(rows: Vec<Value>) -> Value {
    json!({
        "columns": [
            "event", "row_timestamp", "event_id", "method", "route_template",
            "operation_id", "outcome", "actor_id",
        ],
        "results": rows,
    })
}

fn api_row(event_id: &str, actor_id: i64, timestamp: &str) -> Value {
    json!([
        crate::audit::event::EVENT_NAME,
        timestamp,
        event_id,
        "GET",
        "/api/v1/overview",
        "canvas_overview",
        "success",
        actor_id,
    ])
}

/// THE test the whole module exists for: the mandatory predicates are in the
/// OUTBOUND request body, positioned before the source's own `LIMIT`.
#[tokio::test]
async fn the_outbound_request_carries_the_viewer_and_session_predicates_before_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/projects/42/query/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(Vec::new())))
        .mount(&server)
        .await;

    let source = PosthogActivitySource::new(client(&server));
    source
        .fetch(&query(RecordKind::All, true))
        .await
        .expect("the source answered");

    let requests = server
        .received_requests()
        .await
        .expect("wiremock recorded the request");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("the body is JSON");
    let text = body["query"]["query"]
        .as_str()
        .expect("the query text is a string");
    let limit_at = text.rfind("LIMIT").expect("the query has a LIMIT");

    let actor_at = text
        .find("properties.actor_id = {viewer_actor_id}")
        .expect("the actor predicate is in the outbound query");
    let session_at = text
        .find("properties.session_id = {authorized_session_id}")
        .expect("the session predicate is in the outbound query");
    assert!(actor_at < limit_at, "{text}");
    assert!(session_at < limit_at, "{text}");

    // The values ride the parameter map, never the text.
    assert_eq!(body["query"]["values"]["viewer_actor_id"], json!(VIEWER_ID));
    assert_eq!(
        body["query"]["values"]["authorized_session_id"],
        json!(SESSION)
    );
    assert!(!text.contains(SESSION), "{text}");
    assert_eq!(body["query"]["kind"], "HogQLQuery");
}

#[tokio::test]
async fn the_read_key_travels_only_in_the_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header(
            "authorization",
            format!("Bearer {READ_KEY}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(Vec::new())))
        .mount(&server)
        .await;

    let source = PosthogActivitySource::new(client(&server));
    source
        .fetch(&query(RecordKind::ApiRequest, false))
        .await
        .expect("the keyed request is accepted");

    let requests = server.received_requests().await.expect("recorded");
    let body = String::from_utf8_lossy(&requests[0].body);
    assert!(
        !body.contains(READ_KEY),
        "the read key must never appear in the request body"
    );
    // The client's own Debug must not spill it either.
    let rendered = format!("{:?}", client(&server));
    assert!(!rendered.contains(READ_KEY), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
}

#[tokio::test]
async fn a_full_row_set_decodes_into_records() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(vec![
            api_row("ev-1", VIEWER_ID, "2026-07-31T11:59:00.000Z"),
            api_row("ev-2", VIEWER_ID, "2026-07-31T11:58:00.000Z"),
        ])))
        .mount(&server)
        .await;

    let source = PosthogActivitySource::new(client(&server));
    let page = source
        .fetch(&query(RecordKind::ApiRequest, false))
        .await
        .expect("answered");
    assert_eq!(page.records.len(), 2);
    assert_eq!(page.raw_rows, 2);
    assert_eq!(page.row_errors, 0);
    assert_eq!(page.records[0].event_id(), "ev-1");
    assert_eq!(
        page.records[0].delivery_state(),
        DeliveryState::VerifiedInPosthog,
        "a row read back OUT of PostHog is by definition query-visible"
    );
}

/// One malformed row must not hide the well-formed ones, and must be COUNTED so
/// the page can be marked partial.
#[tokio::test]
async fn an_undecodable_row_is_counted_and_the_rest_still_return() {
    let server = MockServer::start().await;
    let mut broken = api_row("ev-bad", VIEWER_ID, "2026-07-31T11:57:00.000Z");
    broken[3] = json!(42); // a numeric `method`
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(vec![
            api_row("ev-1", VIEWER_ID, "2026-07-31T11:59:00.000Z"),
            broken,
        ])))
        .mount(&server)
        .await;

    let source = PosthogActivitySource::new(client(&server));
    let page = source
        .fetch(&query(RecordKind::ApiRequest, false))
        .await
        .expect("answered");
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.raw_rows, 2);
    assert_eq!(page.row_errors, 1);
}

#[tokio::test]
async fn upstream_statuses_split_into_the_documented_fault_classes() {
    for (status, upstream_fault) in [
        (401u16, true),
        (403, true),
        (400, true),
        (422, true),
        (429, false),
        (500, false),
        (503, false),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let source = PosthogActivitySource::new(client(&server));
        let error = source
            .fetch(&query(RecordKind::ApiRequest, false))
            .await
            .expect_err(&format!("status {status}"));
        assert_eq!(
            error.is_upstream_fault(),
            upstream_fault,
            "status {status} classified as {error:?}"
        );
    }
}

#[tokio::test]
async fn a_malformed_or_unrecognizable_response_is_an_upstream_fault() {
    for body in [
        "not json at all".to_string(),
        json!({"results": []}).to_string(),
        json!({"columns": ["event"]}).to_string(),
        json!({"columns": [7], "results": []}).to_string(),
        json!({"columns": ["event"], "results": [{"not": "a row"}]}).to_string(),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body.clone())
                    .insert_header("content-type", "application/json"),
            )
            .mount(&server)
            .await;
        let source = PosthogActivitySource::new(client(&server));
        let error = source
            .fetch(&query(RecordKind::ApiRequest, false))
            .await
            .expect_err(&body);
        assert!(
            error.is_upstream_fault(),
            "a response this build cannot read must never become an empty page: {error:?}"
        );
    }
}

#[tokio::test]
async fn an_oversized_response_body_is_refused_rather_than_buffered() {
    let server = MockServer::start().await;
    // Well past the 8 MiB cap, so the incremental read gives up.
    let huge = "x".repeat(9 * 1024 * 1024);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(huge))
        .mount(&server)
        .await;
    let source = PosthogActivitySource::new(client(&server));
    let error = source
        .fetch(&query(RecordKind::ApiRequest, false))
        .await
        .expect_err("the body limit fires");
    assert_eq!(
        error,
        SourceError::Upstream {
            kind: "oversized_response"
        }
    );
}

#[tokio::test]
async fn a_source_that_never_answers_times_out_as_a_transient_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
        .mount(&server)
        .await;
    let client = PosthogQueryClient::new(
        format!("{}/api/projects/42/query/", server.uri()),
        SecretString::from(READ_KEY.to_string()),
        Duration::from_millis(50),
    )
    .expect("client builds");
    let source = PosthogActivitySource::new(client);
    let error = source
        .fetch(&query(RecordKind::ApiRequest, false))
        .await
        .expect_err("the per-request budget fires");
    assert!(!error.is_upstream_fault(), "{error:?}");
}

/// A responder that applies the request's OWN actor predicate to a fixed
/// dataset — i.e. it behaves the way the real source does.
///
/// It is what makes "hidden rows never affect the page" a meaningful test: the
/// rows exist in the dataset, the predicate hides them at the SOURCE, and the
/// page is byte-identical to one produced from a dataset that never had them.
struct PredicateAwareStore {
    rows: Vec<(i64, Value)>,
}

impl Respond for PredicateAwareStore {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).expect("a JSON body");
        let viewer = body["query"]["values"]["viewer_actor_id"].as_i64();
        let visible: Vec<Value> = self
            .rows
            .iter()
            .filter(|(actor_id, _)| viewer.is_none_or(|viewer| *actor_id == viewer))
            .map(|(_, row)| row.clone())
            .collect();
        ResponseTemplate::new(200).set_body_json(envelope(visible))
    }
}

#[tokio::test]
async fn rows_hidden_by_the_source_predicate_change_nothing_about_the_page() {
    let visible = vec![
        (
            VIEWER_ID,
            api_row("ev-1", VIEWER_ID, "2026-07-31T11:59:00.000Z"),
        ),
        (
            VIEWER_ID,
            api_row("ev-2", VIEWER_ID, "2026-07-31T11:57:00.000Z"),
        ),
    ];
    let with_hidden = vec![
        (999, api_row("ev-hidden-a", 999, "2026-07-31T11:59:30.000Z")),
        (
            VIEWER_ID,
            api_row("ev-1", VIEWER_ID, "2026-07-31T11:59:00.000Z"),
        ),
        (999, api_row("ev-hidden-b", 999, "2026-07-31T11:58:00.000Z")),
        (
            VIEWER_ID,
            api_row("ev-2", VIEWER_ID, "2026-07-31T11:57:00.000Z"),
        ),
        (999, api_row("ev-hidden-c", 999, "2026-07-31T11:56:00.000Z")),
    ];

    let mut pages = Vec::new();
    for rows in [visible, with_hidden] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(PredicateAwareStore { rows })
            .mount(&server)
            .await;
        let source = PosthogActivitySource::new(client(&server));
        let page = source
            .fetch(&query(RecordKind::ApiRequest, false))
            .await
            .expect("answered");
        pages.push((
            page.records
                .iter()
                .map(|record| record.event_id().to_string())
                .collect::<Vec<_>>(),
            page.raw_rows,
            page.row_errors,
        ));
    }
    assert_eq!(
        pages[0], pages[1],
        "hidden rows must not change the items, the page fullness, or the row \
         warnings — they are filtered at the SOURCE and never reach this process"
    );
    assert_eq!(pages[0].0, vec!["ev-1", "ev-2"]);
}
