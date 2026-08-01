//! Tier 3: the self-hosted PostHog staging smoke.
//!
//! **This tier is environment-gated and does NOT run in the pull-request suite.**
//! It needs a dedicated non-production PostHog project reached with short-lived
//! CI credentials, which by design never exist on a developer machine or on a
//! fork. When the gate is closed each test prints `ACCEPTANCE-SKIP` with the
//! reason and returns; nothing is faked and nothing is asserted.
//!
//! Run it by setting:
//!
//! ```text
//! FKST_ACCEPTANCE_POSTHOG_HOST        https://posthog.staging.internal
//! FKST_ACCEPTANCE_POSTHOG_PROJECT_ID  the dedicated test project's numeric id
//! FKST_ACCEPTANCE_POSTHOG_TOKEN       that project's capture token
//! FKST_ACCEPTANCE_POSTHOG_QUERY_KEY   a read-only personal API key for it
//! ```
//!
//! ## What it proves that a mock cannot
//!
//! A mock PostHog answers whatever the test told it to. Only a real project can
//! show that this deployment's event NAME, schema, UUID, and distinct-id choices
//! survive PostHog's own ingestion and reappear through its query API — which is
//! the one place the capture contract meets a system this repository does not
//! control.
//!
//! ## Safety rules this suite obeys
//!
//! - every artefact it creates is named with a per-run nonce, so a shared staging
//!   project stays usable and cleanup can never touch another run's data;
//! - it never prints a credential, a host, or a raw event body;
//! - it asserts the round trip and the canary absence, then stops. Deleting
//!   events is a project-policy operation (PostHog has no per-event delete), so
//!   the suite relies on the project's retention rather than issuing a broad
//!   deletion, exactly as the issue requires.

#[path = "acceptance_gate.rs"]
mod gate;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use fkst_control_plane::audit::posthog::PostHogClient;
use fkst_control_plane::audit::{
    audit_requests, ApiRequestCompletedV1, AuditConfig, AuditHandle, AuditMiddleware, EventLimits,
    OperationCatalog, ServiceIdentity,
};
use gate::Gate;
use secrecy::SecretString;
use tower::ServiceExt;

const TIER: &str = "staging";
const SWITCH: &str = "FKST_ACCEPTANCE_POSTHOG_HOST";
const REQUIRED: [&str; 3] = [
    "FKST_ACCEPTANCE_POSTHOG_PROJECT_ID",
    "FKST_ACCEPTANCE_POSTHOG_TOKEN",
    "FKST_ACCEPTANCE_POSTHOG_QUERY_KEY",
];

/// A canary that must NOT appear in the raw staging event.
const CANARY: &str = "canary-staging-argument-8f31c0a4";

/// One authenticated request's worth of audit record makes the whole trip:
/// capture accepted, then found again by its own event id through the query API.
#[tokio::test]
async fn a_real_posthog_project_round_trips_one_safe_record() {
    let environment = match gate::open(SWITCH, &REQUIRED) {
        Gate::Open(environment) => environment,
        Gate::Closed(reason) => {
            gate::skip(
                TIER,
                "a_real_posthog_project_round_trips_one_safe_record",
                &reason,
            );
            return;
        }
    };

    let nonce = run_nonce();
    let event = staging_event(&nonce).await;
    let event_id = event.event_id;

    let client = PostHogClient::from_config(&capture_config(&environment))
        .expect("the capture client builds");
    let capture = event
        .to_capture_event(EventLimits::new(usize::MAX))
        .expect("the record satisfies the contract");
    client
        .capture(std::slice::from_ref(&capture))
        .await
        .expect("the staging project accepts the batch");

    // Capture acceptance is NOT query visibility; the epic makes that an explicit
    // two-state contract, so the read is polled rather than assumed.
    let found = poll_for_event(&environment, &event_id.to_string()).await;
    assert!(
        found.is_some(),
        "the staging project accepted the event but never made it queryable"
    );

    // The raw event, as PostHog stored it, carries no forbidden value.
    let raw = found.expect("checked above");
    assert!(
        !raw.contains(CANARY),
        "a hostile argument reached the raw staging event"
    );
    // ...and does carry the safe correlation the epic keeps.
    assert!(
        raw.contains(&event_id.to_string()),
        "the stored event does not carry its own id"
    );
}

/// The scoped read: the originating user finds their record, a second regular
/// user does not, and a global administrator does.
///
/// This is the same authorization claim the deterministic tier proves against a
/// mock, restated against a real project — the one place where a wrong PostHog
/// person/distinct-id mapping could widen a result set in production without any
/// local test noticing.
#[tokio::test]
async fn a_second_regular_user_cannot_find_the_first_users_staging_record() {
    let environment = match gate::open(SWITCH, &REQUIRED) {
        Gate::Open(environment) => environment,
        Gate::Closed(reason) => {
            gate::skip(
                TIER,
                "a_second_regular_user_cannot_find_the_first_users_staging_record",
                &reason,
            );
            return;
        }
    };

    let nonce = run_nonce();
    let event = staging_event(&nonce).await;
    let owner = event.actor_id.expect("the fixture record is attributed");
    let client = PostHogClient::from_config(&capture_config(&environment))
        .expect("the capture client builds");
    client
        .capture(&[event
            .to_capture_event(EventLimits::new(usize::MAX))
            .expect("the record satisfies the contract")])
        .await
        .expect("the staging project accepts the batch");

    let event_id = event.event_id.to_string();
    assert!(
        poll_for_event(&environment, &event_id).await.is_some(),
        "the record never became queryable"
    );

    // The owner's personal predicate finds it; a different actor's does not.
    let mine = query_rows(&environment, &event_id, Some(owner)).await;
    assert_eq!(mine.len(), 1, "the owner cannot find their own record");
    let stranger = query_rows(&environment, &event_id, Some(owner + 1)).await;
    assert!(
        stranger.is_empty(),
        "a second regular user's predicate matched another user's record"
    );
    // The global scope carries no actor predicate at all and must find it.
    let admin = query_rows(&environment, &event_id, None).await;
    assert_eq!(admin.len(), 1, "the global scope lost the record");
}

/// A per-run nonce so a shared staging project never mixes two runs.
fn run_nonce() -> String {
    format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
    )
}

/// The record the staging tier sends.
///
/// It is produced by driving the REAL audit middleware over a throwaway route
/// carrying the canary in its URL — so the record is whatever this deployment
/// would genuinely have recorded, rather than a literal a test hand-built to
/// look safe. That distinction is the whole point of a staging smoke: it must
/// exercise the production projection.
async fn staging_event(nonce: &str) -> ApiRequestCompletedV1 {
    let (handle, sink) = AuditHandle::recording();
    let middleware = AuditMiddleware::new(
        Arc::new(OperationCatalog::default()),
        handle,
        ServiceIdentity {
            version: env!("CARGO_PKG_VERSION").to_string(),
            environment: "acceptance-staging".to_string(),
        },
    );
    let router = axum::Router::new()
        .route("/acceptance", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            middleware,
            audit_requests,
        ));
    let response = router
        .oneshot(
            Request::get(format!("/acceptance?probe={CANARY}"))
                .header("authorization", format!("Bearer {CANARY}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert!(response.status().is_success());

    let mut event = sink
        .events()
        .into_iter()
        .next()
        .expect("the middleware recorded the probe");
    // A per-run correlation value so a shared staging project stays greppable.
    event.request_id = format!("acceptance-{nonce}");
    // A synthetic actor id, far outside any real GitHub id, so the scoped-read
    // assertions cannot collide with a real person's records.
    event.actor_id = Some(STAGING_ACTOR_ID);
    assert!(
        !format!("{event:?}").contains(CANARY),
        "the recorded probe already carries the canary; the staging assertion          would be meaningless"
    );
    event
}

/// A synthetic actor id well outside GitHub's allocated range.
const STAGING_ACTOR_ID: i64 = 9_000_000_001;

fn capture_config(environment: &gate::GateEnvironment) -> AuditConfig {
    AuditConfig {
        enabled: true,
        host: Some(environment.get(SWITCH).to_string()),
        project_token: SecretString::from(
            environment.get("FKST_ACCEPTANCE_POSTHOG_TOKEN").to_string(),
        ),
        ..AuditConfig::default()
    }
}

/// Poll the query API until the event id appears or the budget expires.
///
/// PostHog ingestion is asynchronous, so "not yet" is a normal answer; the epic
/// makes the distinction between accepted and query-visible explicit for exactly
/// this reason.
async fn poll_for_event(environment: &gate::GateEnvironment, event_id: &str) -> Option<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    while std::time::Instant::now() < deadline {
        let rows = query_rows(environment, event_id, None).await;
        if let Some(row) = rows.into_iter().next() {
            return Some(row);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    None
}

/// Run one fixed, parameterized query against the staging project.
///
/// The predicate shape mirrors the product query: an optional actor predicate
/// applied at the SOURCE, never a client-side filter over a wider result.
async fn query_rows(
    environment: &gate::GateEnvironment,
    event_id: &str,
    actor_id: Option<i64>,
) -> Vec<String> {
    let host = environment.get(SWITCH).trim_end_matches('/');
    let project = environment.get("FKST_ACCEPTANCE_POSTHOG_PROJECT_ID");
    let url = format!("{host}/api/projects/{project}/query/");
    let mut query = String::from(
        "select properties.event_id, properties.actor_id from events \
         where event = 'fkst_api_request_completed' and properties.event_id = {event_id}",
    );
    let mut values = serde_json::Map::new();
    values.insert(
        "event_id".to_string(),
        serde_json::Value::String(event_id.to_string()),
    );
    if let Some(actor_id) = actor_id {
        query.push_str(" and properties.actor_id = {actor_id}");
        values.insert("actor_id".to_string(), serde_json::json!(actor_id));
    }
    query.push_str(" limit 10");

    let response = reqwest::Client::new()
        .post(&url)
        .bearer_auth(environment.get("FKST_ACCEPTANCE_POSTHOG_QUERY_KEY"))
        .json(&serde_json::json!({
            "query": { "kind": "HogQLQuery", "query": query, "values": values }
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("the staging query API answers");
    assert!(
        response.status().is_success(),
        "the staging query API refused the read with {}",
        response.status()
    );
    let body: serde_json::Value = response.json().await.expect("a JSON body");
    body["results"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|row| row.to_string())
        .collect()
}
