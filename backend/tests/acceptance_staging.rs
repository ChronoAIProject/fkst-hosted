//! Tier 3: the self-hosted PostHog staging smoke.
//!
//! **This tier is environment-gated and does NOT run in the pull-request suite.**
//! It needs a dedicated non-production PostHog project and a deployed audit relay
//! reached with short-lived CI credentials, which by design never exist on a
//! developer machine or on a fork. When the gate is closed each test prints
//! `ACCEPTANCE-SKIP` with the reason and returns; nothing is faked and nothing is
//! asserted.
//!
//! Run it by setting:
//!
//! ```text
//! FKST_ACCEPTANCE_POSTHOG_HOST        https://posthog.staging.internal
//! FKST_ACCEPTANCE_POSTHOG_PROJECT_ID  the dedicated test project's numeric id
//! FKST_ACCEPTANCE_POSTHOG_TOKEN       that project's capture token
//! FKST_ACCEPTANCE_POSTHOG_QUERY_KEY   a read-only personal API key for it
//! FKST_ACCEPTANCE_RELAY_URL           the staging audit relay's URL
//! FKST_ACCEPTANCE_RELAY_WRITE_TOKEN   its write credential
//! FKST_ACCEPTANCE_RELAY_READ_TOKEN    its read credential
//! ```
//!
//! ## The six steps the issue specifies
//!
//! 1. send an authenticated harmless request with a known test request id;
//! 2. the relay confirms a durable start and a durable completion;
//! 3. PostHog capture accepts it and verification observes the UUID;
//! 4. the originating regular user finds the exact safe record through
//!    `GET /api/v1/operations/activity?scope=mine&request_id=…`, a second regular
//!    user cannot, and a global admin can in the `all` scope;
//! 5. the raw PostHog event carries no canary;
//! 6. cleanup expires only the dedicated test event by project retention.
//!
//! Step 4 goes through the PRODUCT ROUTER, not a hand-written query. That is the
//! whole point of running against a real project: a bespoke HogQL statement
//! would prove PostHog can filter, which nobody doubts, while leaving the
//! product's `AuthenticatedViewer`, scope gate, HogQL builder, cursor binding,
//! and merge layer — the things that actually decide who sees what — untested
//! against a real server.
//!
//! ## What it proves that a mock cannot
//!
//! A mock PostHog answers whatever the test told it to. Only a real project can
//! show that this deployment's event NAME, schema, UUID, distinct-id, and person
//! -profile choices survive PostHog's own ingestion and reappear through the
//! product's fixed query — which is the one place the capture contract meets a
//! system this repository does not control.
//!
//! ## Safety rules this suite obeys
//!
//! - every artefact it creates is named with a per-run nonce, so a shared staging
//!   project stays usable and cleanup can never touch another run's data;
//! - its actors carry synthetic ids far outside GitHub's allocated range, so no
//!   assertion can collide with a real person's rows;
//! - it never prints a credential, a host, or a raw event body;
//! - it asserts the round trip and the canary absence, then stops. Deleting
//!   events is a project-policy operation (PostHog has no per-event delete), so
//!   the suite relies on the project's retention rather than issuing a broad
//!   deletion, exactly as the issue requires.

mod staging_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use fkst_control_plane::audit::posthog::PostHogClient;
use fkst_control_plane::audit::relay::{AuditDelivery, AuditDeliveryMode, RelayClientMetrics};
use fkst_control_plane::audit::{
    audit_requests, Actor, ApiRequestCompletedV1, AuditHandle, AuditMiddleware,
    AuthenticationMethod, EventLimits, OperationCatalog, RequestIdentity, ServiceIdentity,
};
use fkst_control_plane::audit_relay::protocol::format_instant;
use fkst_control_plane::audit_relay::query::RecordsQueryV1;
use k8s_openapi::chrono::{Duration as ChronoDuration, Utc};
use serde_json::Value;
use staging_support::gate::{self, Gate};
use staging_support::probe::PostHogProfile;
use staging_support::{artifact, probe, Product, ALICE, BOB, GRACE, REQUIRED, SWITCH};
use tower::ServiceExt;

const TIER: &str = "staging";

/// A canary that must NOT appear in the raw staging event.
const CANARY: &str = "canary-staging-argument-8f31c0a4";

/// The whole round trip: request, durable relay record, PostHog capture,
/// verification, and the product's own scoped read.
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
    let mut profile = PostHogProfile::default();
    probe::observe_server(&environment, &mut profile).await;

    // ---- step 1: one authenticated harmless request, with a known request id.
    let nonce = staging_support::run_nonce();
    let request_id = format!("acceptance-{nonce}");
    let event = staging_event(&request_id).await;
    let event_id = event.event_id;

    // ---- step 2: the relay confirms a durable start AND a durable completion.
    //
    // Driven through `AuditDelivery` in REQUIRED mode, which is the production
    // policy: a best-effort delivery would swallow a relay outage and let this
    // step pass without anything ever becoming durable.
    let relay = staging_support::relay_client(&environment);
    let delivery = AuditDelivery::with_client(
        AuditDeliveryMode::Required,
        relay.clone(),
        60,
        RelayClientMetrics::new(),
    );
    delivery
        .register_start(
            &RequestIdentity {
                request_id: event.request_id.clone(),
                method: event.method.clone(),
                route_template: event.route_template.clone(),
                operation_id: event.operation_id.clone(),
            },
            event.event_id,
            event.started_at,
            &event.service.version,
            &event.service.environment,
        )
        .await
        .expect("the staging relay makes the start durable before the handler");
    delivery
        .complete(&event)
        .await
        .expect("the staging relay makes the completion durable before the response");

    let durable = relay
        .read_records(
            &RecordsQueryV1 {
                scope: "mine".to_string(),
                actor_id: Some(ALICE.0),
                record_kind: "api_request".to_string(),
                from: format_instant(Utc::now() - ChronoDuration::hours(1)),
                to: format_instant(Utc::now() + ChronoDuration::hours(1)),
                limit: 100,
                ..RecordsQueryV1::default()
            },
            Duration::from_secs(30),
        )
        .await
        .expect("the staging relay answers a scoped read")
        .rows;
    let stored = durable
        .iter()
        .find(|row| row.event_id == event_id.to_string())
        .unwrap_or_else(|| panic!("the relay did not make the record durable"));
    assert_eq!(
        stored.terminal["outcome"], "success",
        "the durable record did not close as a terminal success"
    );

    // ---- step 3: capture accepts, and verification observes the UUID.
    let client = PostHogClient::from_config(&staging_support::capture_config(&environment))
        .expect("the capture client builds");
    let capture = event
        .to_capture_event(EventLimits::new(usize::MAX))
        .expect("the record satisfies the contract");
    let captured_at = Instant::now();
    client
        .capture(std::slice::from_ref(&capture))
        .await
        .expect("the staging project accepts the batch");

    // Capture acceptance is NOT query visibility; the epic makes that an explicit
    // two-state contract, so the read is polled rather than assumed.
    let raw = poll_raw_event(&environment, &event_id.to_string(), &mut profile)
        .await
        .expect("the staging project accepted the event but never made it queryable");
    profile.visibility_lag_secs = Some(captured_at.elapsed().as_secs());

    // ---- step 5: the raw event, as PostHog stored it, carries no canary.
    assert!(
        !raw.contains(CANARY),
        "a hostile argument reached the raw staging event"
    );
    assert!(
        raw.contains(&event_id.to_string()),
        "the stored event does not carry its own id"
    );

    // ---- step 4 (owner half): the PRODUCT finds it under `scope=mine`.
    let product = Product::start(&environment).await;
    let mine = product
        .event_ids(ALICE, &format!("?scope=mine&request_id={request_id}"))
        .await;
    assert!(
        mine.contains(&event_id.to_string()),
        "the originating user could not find their own record through the product"
    );

    // ---- step 6: nothing is deleted. See the module docs.
    artifact::write("posthog-staging.json", &profile.to_json());
}

/// The isolation half of step 4, against a real project.
///
/// This is the same authorization claim the deterministic tier proves against a
/// mock, restated where a wrong PostHog person/distinct-id mapping, a silently
/// dropped predicate, or a server-side query rewrite could widen a result set in
/// production without any local test noticing.
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

    let nonce = staging_support::run_nonce();
    let request_id = format!("acceptance-{nonce}");
    let event = staging_event(&request_id).await;
    let event_id = event.event_id.to_string();

    let client = PostHogClient::from_config(&staging_support::capture_config(&environment))
        .expect("the capture client builds");
    client
        .capture(&[event
            .to_capture_event(EventLimits::new(usize::MAX))
            .expect("the record satisfies the contract")])
        .await
        .expect("the staging project accepts the batch");
    let mut profile = PostHogProfile::default();
    assert!(
        poll_raw_event(&environment, &event_id, &mut profile)
            .await
            .is_some(),
        "the record never became queryable"
    );

    let product = Product::start(&environment).await;
    let filter = format!("?scope=mine&request_id={request_id}");

    // The owner sees exactly their own row.
    let mine = product.event_ids(ALICE, &filter).await;
    assert!(
        mine.contains(&event_id),
        "the owner cannot find their own record"
    );

    // A second regular user, asking the identical question, sees nothing. The
    // filter is the same; only the verified viewer differs, so a pass here is
    // about the source predicate rather than about the filter.
    let stranger = product.event_ids(BOB, &filter).await;
    assert!(
        !stranger.contains(&event_id),
        "a second regular user reached another user's record"
    );

    // A regular user cannot widen their own scope, even by asking.
    let widened = product
        .activity(BOB, &format!("?scope=all&request_id={request_id}"))
        .await;
    assert_eq!(
        widened.status(),
        axum::http::StatusCode::FORBIDDEN,
        "a regular user was allowed into the global scope"
    );

    // The global administrator does see it in the `all` scope.
    let admin = product
        .event_ids(GRACE, &format!("?scope=all&request_id={request_id}"))
        .await;
    assert!(
        admin.contains(&event_id),
        "the global scope lost the record"
    );
}

/// The record the staging tier sends.
///
/// It is produced by driving the REAL audit middleware over a throwaway route
/// carrying the canary in its URL — so the record is whatever this deployment
/// would genuinely have recorded, rather than a literal a test hand-built to
/// look safe. That distinction is the whole point of a staging smoke: it must
/// exercise the production projection.
///
/// Only the identity is substituted afterwards, and it is substituted
/// CONSISTENTLY (canonical id, nested actor id, and login together), because the
/// event contract rejects a record whose ids disagree — the very check that keeps
/// a row from being attributed to the wrong person.
async fn staging_event(request_id: &str) -> ApiRequestCompletedV1 {
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
    event.request_id = request_id.to_string();
    event.actor = Actor::github_user(ALICE.0, ALICE.1, AuthenticationMethod::Bearer);
    event.actor_id = Some(ALICE.0);
    assert!(
        !format!("{event:?}").contains(CANARY),
        "the recorded probe already carries the canary; the staging assertion \
         would be meaningless"
    );
    event
}

/// Poll the staging project until the event id appears or the budget expires.
///
/// PostHog ingestion is asynchronous, so "not yet" is a normal answer; the epic
/// makes the distinction between accepted and query-visible explicit for exactly
/// this reason. The first successful body is also where the API-behaviour
/// observations come from — recording the envelope of a response the suite had
/// to make anyway costs nothing and is what the evidence artifact needs.
async fn poll_raw_event(
    environment: &gate::GateEnvironment,
    event_id: &str,
    profile: &mut PostHogProfile,
) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        let body = raw_query(environment, event_id).await;
        probe::observe_query_envelope(&body, profile);
        // A placeholder that survived as a parameter proves the server honoured
        // the binding rather than treating `{event_id}` as literal text: a
        // literal would match nothing at all.
        if let Some(row) = body["results"].as_array().and_then(|rows| rows.first()) {
            profile.honours_placeholders = Some(true);
            return Some(row.to_string());
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    None
}

/// One fixed, parameterized read of the RAW stored event.
///
/// Deliberately unauthorized-by-design and used only for step 5's redaction
/// assertion: it reads the event as PostHog stored it, which the product query
/// never exposes (the product projects a safe subset). Every authorization claim
/// in this suite goes through [`Product`] instead.
async fn raw_query(environment: &gate::GateEnvironment, event_id: &str) -> Value {
    let url = format!(
        "{}/api/projects/{}/query/",
        staging_support::host(environment),
        staging_support::project(environment)
    );
    let response = reqwest::Client::new()
        .post(&url)
        .bearer_auth(environment.get("FKST_ACCEPTANCE_POSTHOG_QUERY_KEY"))
        .json(&serde_json::json!({
            "query": {
                "kind": "HogQLQuery",
                "query": "select properties from events \
                          where event = 'fkst_api_request_completed' \
                          and properties.event_id = {event_id} limit 1",
                "values": { "event_id": event_id },
            }
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
    response.json().await.expect("a JSON body")
}
