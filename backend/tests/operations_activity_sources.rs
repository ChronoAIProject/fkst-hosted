//! Source-level and failure-mode tests for `/api/v1/operations/activity`.
//!
//! The theme is the one the epic keeps returning to: authorization happens at
//! the SOURCE, before any limit or cursor, and a source that cannot answer is
//! reported rather than rounded down to an empty page.
//!
//! Keyset pagination and the cursor's own contract live in
//! `operations_activity_paging.rs`.

mod operations_harness;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;

use fkst_control_plane::operations::record::{
    ActivityRecord, ActivitySourceKind, ApiRequestRecord, DeliveryState, RecordActor,
    RecordCorrelation, RecordPrincipal,
};
use fkst_control_plane::operations::source::{
    ActivitySource, SourceError, SourcePage, SourceQuery,
};
use operations_harness::{
    body_json, error_code, harness, item_ids, minutes_ago, Row, Sources, ALICE, ROOT, SESSION,
};

fn dataset() -> Vec<Row> {
    (0..5)
        .map(|i| Row::api(&format!("ev-{i}"), ALICE.0, &minutes_ago(i + 1)))
        .collect()
}

/// The mandatory predicates are in the OUTBOUND query, positioned before the
/// source's own `LIMIT` — not applied to a page fetched without them.
#[tokio::test]
async fn the_outbound_query_carries_the_viewer_predicate_before_the_source_limit() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    harness.page(ALICE, "").await;
    let text = harness.last_query_text().await;
    let limit_at = text.rfind("LIMIT").expect("the query has a LIMIT");
    let actor_at = text
        .find("properties.actor_id = {viewer_actor_id}")
        .expect("the actor predicate reached the source");
    assert!(actor_at < limit_at, "{text}");
    assert!(
        !text.to_ascii_uppercase().contains("OFFSET"),
        "pagination must never use OFFSET: {text}"
    );
}

#[tokio::test]
async fn the_authorized_session_predicate_reaches_the_source_before_the_limit() {
    let mut rows = dataset();
    rows.push(Row::lifecycle("ev-life", SESSION, &minutes_ago(9)));
    let harness = harness(Sources::Posthog(rows), true).await;
    harness
        .page(ALICE, &format!("?record_kind=all&session_id={SESSION}"))
        .await;
    let text = harness.last_query_text().await;
    let limit_at = text.rfind("LIMIT").expect("the query has a LIMIT");
    let session_at = text
        .find("properties.session_id = {authorized_session_id}")
        .expect("the session predicate reached the source");
    assert!(session_at < limit_at, "{text}");
    // The union is parenthesized, so the lifecycle branch cannot escape the
    // actor predicate that guards the API branch. Asserted as the EXACT grouping
    // on the recorded outbound body, not as "an OR appears somewhere": the
    // generator's own unit test can only prove what `build()` returned, and the
    // claim this endpoint rests on is about what PostHog was actually sent.
    let api_branch = "(event IN ({event_request_completed}, {event_request_incomplete}) \
                      AND properties.actor_id = {viewer_actor_id} \
                      AND properties.session_id = {authorized_session_id})";
    let lifecycle_branch =
        "(event = {event_sandbox_lifecycle} AND properties.session_id = {authorized_session_id})";
    assert!(
        text.contains(&format!("({api_branch} OR {lifecycle_branch})")),
        "the outbound union must be fully parenthesized with the actor predicate \
         INSIDE the api branch; got:\n{text}"
    );
    assert_eq!(
        text.matches("properties.actor_id = {viewer_actor_id}")
            .count(),
        1,
        "the actor predicate must appear exactly once, inside the api branch:\n{text}"
    );
}

/// Hostile filter text never becomes query source; it stays a parameter.
#[tokio::test]
async fn a_hostile_filter_value_is_refused_or_parameterized_never_interpolated() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    // The validated identifiers refuse it outright.
    let refused = harness
        .get(ALICE, "?session_id=%27%20OR%201%3D1%20--")
        .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);

    // A value that IS valid still travels as a parameter.
    harness.page(ALICE, "?request_id=req-0001").await;
    let text = harness.last_query_text().await;
    assert!(text.contains("{filter_request_id}"), "{text}");
    assert!(!text.contains("req-0001"), "{text}");
}

/// A malformed row is dropped and the page marked partial; the rest still
/// return, and nothing about the hidden rows leaks into the metadata.
#[tokio::test]
async fn an_undecodable_row_marks_the_page_partial_without_hiding_the_rest() {
    let rows = vec![
        Row::api("ev-good", ALICE.0, &minutes_ago(1)),
        Row::api("ev-bad", ALICE.0, &minutes_ago(2)).malformed(),
    ];
    let harness = harness(Sources::Posthog(rows), true).await;
    let page = harness.page(ALICE, "").await;
    assert_eq!(item_ids(&page), vec!["ev-good"]);
    assert_eq!(page["source_status"]["partial"], true);
    assert_eq!(page["source_status"]["posthog"], "degraded");
    assert_eq!(
        page["source_status"]["message_code"],
        "activity_rows_dropped"
    );
}

#[tokio::test]
async fn an_upstream_auth_or_schema_failure_is_a_bad_gateway() {
    for status in [401u16, 403, 400, 422] {
        let harness = harness(Sources::PosthogFailing(status), true).await;
        let response = harness.get(ALICE, "").await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY, "{status}");
        assert_eq!(error_code(response).await, "upstream_error");
    }
}

#[tokio::test]
async fn a_transient_upstream_failure_is_service_unavailable_never_an_empty_page() {
    for status in [429u16, 500, 503] {
        let harness = harness(Sources::PosthogFailing(status), true).await;
        let response = harness.get(ALICE, "").await;
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{status}"
        );
        let body = body_json(response).await;
        assert_eq!(body["error"], "unavailable");
        assert!(
            body.get("items").is_none(),
            "an outage must never be dressed up as a complete empty page"
        );
    }
}

/// A source that answers from a script, for the relay-merge contract. Issue
/// #5678 replaces it with the real relay; the contract it satisfies is the same.
#[derive(Debug)]
struct ScriptedSource {
    kind: ActivitySourceKind,
    answer: Mutex<Option<Result<SourcePage, SourceError>>>,
    seen: Mutex<Vec<SourceQuery>>,
}

impl ScriptedSource {
    fn ok(kind: ActivitySourceKind, records: Vec<ActivityRecord>) -> Arc<Self> {
        let raw_rows = records.len();
        Arc::new(Self {
            kind,
            answer: Mutex::new(Some(Ok(SourcePage {
                records,
                raw_rows,
                row_errors: 0,
            }))),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn failing(kind: ActivitySourceKind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            answer: Mutex::new(Some(Err(SourceError::Transient { kind: "timeout" }))),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn constraints(&self) -> Vec<SourceQuery> {
        self.seen.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ActivitySource for ScriptedSource {
    fn kind(&self) -> ActivitySourceKind {
        self.kind
    }

    async fn fetch(&self, query: &SourceQuery) -> Result<SourcePage, SourceError> {
        self.seen.lock().expect("lock").push(query.clone());
        self.answer
            .lock()
            .expect("lock")
            .take()
            .unwrap_or(Err(SourceError::Transient {
                kind: "fixture_exhausted",
            }))
    }
}

fn record(
    event_id: &str,
    actor_id: i64,
    minutes: i64,
    source: ActivitySourceKind,
) -> ActivityRecord {
    ActivityRecord::ApiRequest {
        record: Box::new(ApiRequestRecord {
            event_id: event_id.to_string(),
            request_id: None,
            started_at: None,
            completed_at: k8s_openapi::chrono::Utc::now()
                - k8s_openapi::chrono::Duration::minutes(minutes),
            method: "GET".to_string(),
            route_template: "/api/v1/overview".to_string(),
            operation_id: "canvas_overview".to_string(),
            actor: RecordActor {
                kind: Some("github_user".to_string()),
                id: Some(actor_id),
                login: None,
            },
            principal: RecordPrincipal::default(),
            arguments: serde_json::Map::new(),
            arguments_parse_status: Some("parsed".to_string()),
            status_code: Some(200),
            outcome: "success".to_string(),
            error_code: None,
            duration_ms: Some(3),
            correlation: RecordCorrelation::default(),
        }),
        delivery_state: DeliveryState::Queued,
        source,
    }
}

#[tokio::test]
async fn both_sources_receive_the_same_typed_constraint_and_their_rows_merge() {
    let posthog = ScriptedSource::ok(
        ActivitySourceKind::Posthog,
        vec![record("ev-old", ALICE.0, 20, ActivitySourceKind::Posthog)],
    );
    let relay = ScriptedSource::ok(
        ActivitySourceKind::Relay,
        vec![record("ev-new", ALICE.0, 1, ActivitySourceKind::Relay)],
    );
    let harness = harness(
        Sources::Explicit {
            posthog: Some(Arc::clone(&posthog) as Arc<dyn ActivitySource>),
            relay: Some(Arc::clone(&relay) as Arc<dyn ActivitySource>),
        },
        true,
    )
    .await;

    let page = harness.page(ALICE, "").await;
    assert_eq!(item_ids(&page), vec!["ev-new", "ev-old"]);
    assert_eq!(page["source_status"]["relay"], "healthy");
    assert_eq!(page["source_status"]["partial"], false);

    for source in [posthog.constraints(), relay.constraints()] {
        let query = source.first().expect("the source was called").clone();
        assert_eq!(
            query.constraint.required_actor_id(),
            Some(ALICE.0),
            "each source applies the SAME mandatory predicate"
        );
    }
}

#[tokio::test]
async fn posthog_unavailable_still_returns_authorized_relay_rows_marked_partial() {
    let harness = harness(
        Sources::Explicit {
            posthog: Some(ScriptedSource::failing(ActivitySourceKind::Posthog)),
            relay: Some(ScriptedSource::ok(
                ActivitySourceKind::Relay,
                vec![record("ev-relay", ALICE.0, 2, ActivitySourceKind::Relay)],
            )),
        },
        true,
    )
    .await;
    let page = harness.page(ALICE, "").await;
    assert_eq!(item_ids(&page), vec!["ev-relay"]);
    assert_eq!(page["source_status"]["posthog"], "unavailable");
    assert_eq!(page["source_status"]["partial"], true);
    assert_eq!(page["source_status"]["message_code"], "posthog_unavailable");
}

#[tokio::test]
async fn relay_unavailable_still_returns_authorized_posthog_history_marked_partial() {
    let harness = harness(
        Sources::Explicit {
            posthog: Some(ScriptedSource::ok(
                ActivitySourceKind::Posthog,
                vec![record(
                    "ev-history",
                    ALICE.0,
                    2,
                    ActivitySourceKind::Posthog,
                )],
            )),
            relay: Some(ScriptedSource::failing(ActivitySourceKind::Relay)),
        },
        true,
    )
    .await;
    let page = harness.page(ALICE, "").await;
    assert_eq!(item_ids(&page), vec!["ev-history"]);
    assert_eq!(page["source_status"]["relay"], "unavailable");
    assert_eq!(page["source_status"]["partial"], true);
    assert_eq!(page["source_status"]["message_code"], "relay_unavailable");
}

#[tokio::test]
async fn neither_source_available_is_a_503_never_a_complete_empty_page() {
    let harness = harness(
        Sources::Explicit {
            posthog: Some(ScriptedSource::failing(ActivitySourceKind::Posthog)),
            relay: Some(ScriptedSource::failing(ActivitySourceKind::Relay)),
        },
        true,
    )
    .await;
    let response = harness.get(ALICE, "").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(response).await;
    assert!(body.get("items").is_none(), "{body}");
}

/// The tagged union: a lifecycle row must NOT carry fake HTTP fields.
#[tokio::test]
async fn a_lifecycle_item_carries_no_fabricated_method_or_status() {
    let rows = vec![Row::lifecycle("ev-life", SESSION, &minutes_ago(3))];
    let harness = harness(Sources::Posthog(rows), true).await;
    let page = harness
        .page(
            ALICE,
            &format!("?record_kind=sandbox_lifecycle&session_id={SESSION}"),
        )
        .await;
    let item = &page["items"][0];
    assert_eq!(item["record_kind"], "sandbox_lifecycle");
    assert_eq!(item["lifecycle_action"], "created");
    assert_eq!(item["session_id"], SESSION);
    for absent in ["method", "status_code", "route_template", "duration_ms"] {
        assert!(item.get(absent).is_none(), "{absent} in {item}");
    }
    assert_eq!(item["source"], "posthog");
    assert_eq!(item["delivery_state"], "verified_in_posthog");
}

/// The read credentials never reach a response, however the request ends.
#[tokio::test]
async fn no_response_ever_carries_the_query_credential_or_the_host() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    for (who, query) in [
        (ALICE, ""),
        (ALICE, "?scope=all"),
        (ALICE, "?limit=0"),
        (ROOT, "?record_kind=all"),
    ] {
        let response = harness.get(who, query).await;
        let body = body_json(response).await.to_string();
        assert!(!body.contains("phx_read_key"), "{body}");
        assert!(!body.contains("/api/projects/"), "{body}");
        assert!(!body.contains("HogQLQuery"), "{body}");
        assert!(!body.contains("SELECT"), "{body}");
    }
}
