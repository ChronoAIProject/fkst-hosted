//! Handler tests for the internal credential-refresh / status-report routes
//! (#151). Split out to keep `mod.rs` under the 500-line file budget; included
//! via `#[path] mod internal_tests;` from `mod.rs`, so `super::*` resolves to
//! the `controller` module's (private) `InternalState`, `internal_router`, and
//! the wire/claim status types.
//!
//! The fence guard is the load-bearing invariant under test: a stale `fencing_id`
//! must NEVER yield a token and must NEVER mutate a claim. Every refusal path is
//! asserted to leave the minter uncalled / the claim status unchanged.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use secrecy::{ExposeSecret, SecretString};
use tower::ServiceExt;

use fkst_shared::protocol::{
    CredentialRefreshRequest, RefreshReason, SessionStatus as WireSessionStatus, StatusReport,
    INTERNAL_AUTH_HEADER, PROTOCOL_VERSION,
};

use super::*;

const TEST_SECRET: &str = "test-internal-secret";

/// A scripted [`SessionTokenMinter`] fake: returns a fixed outcome and records
/// every `(session_id, repo_ref)` it was asked to mint, so a test can assert the
/// fence rejected the call BEFORE it ever reached the minter.
struct RecordingMinter {
    outcome: Mutex<Option<MintResult>>,
    calls: Mutex<Vec<(bson::Uuid, String)>>,
    call_count: AtomicUsize,
}

impl RecordingMinter {
    fn new(outcome: MintResult) -> Self {
        Self {
            outcome: Mutex::new(Some(outcome)),
            calls: Mutex::new(Vec::new()),
            call_count: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl SessionTokenMinter for RecordingMinter {
    async fn mint(&self, session_id: bson::Uuid, repo_ref: &str) -> MintResult {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.calls
            .lock()
            .unwrap()
            .push((session_id, repo_ref.to_string()));
        self.outcome
            .lock()
            .unwrap()
            .take()
            .expect("RecordingMinter mint called more than once")
    }
}

/// A claim map pre-populated with one active claim, returning the (map, session,
/// fence) so a test can drive the handlers against a real fence.
fn populated_claims(owner: &str) -> (Arc<ClaimMap>, bson::Uuid, FencingId) {
    let claims = Arc::new(ClaimMap::new());
    let session = bson::Uuid::new();
    let entry = claims.claim("acme/site", session, None, owner).unwrap();
    (claims, session, entry.fencing_id)
}

/// Build the internal router with the given claims + (optional) minter, behind
/// the real shared-secret auth layer.
fn router(claims: Arc<ClaimMap>, minter: Option<Arc<dyn SessionTokenMinter>>) -> axum::Router {
    let registry = WorkerRegistry::new(Duration::from_secs(30));
    let auth = InternalAuth::new(SecretString::from(TEST_SECRET.to_string()));
    // No reassign driver here: these tests cover the credential-refresh /
    // status-report fence behaviour, which is independent of reassignment.
    internal_router(registry, auth, 10, claims, minter, None)
}

/// POST `body` to `path` with the valid internal-auth header, returning the
/// status + parsed JSON body.
async fn post_json(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(INTERNAL_AUTH_HEADER, TEST_SECRET)
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

fn refresh_req(session: bson::Uuid, fence: FencingId) -> serde_json::Value {
    serde_json::to_value(CredentialRefreshRequest {
        worker_id: "w1".into(),
        protocol_version: PROTOCOL_VERSION,
        session_id: session.to_string(),
        fencing_id: fence,
        repo_ref: "acme/site".into(),
        reason: RefreshReason::Jit,
    })
    .unwrap()
}

fn status_req(
    session: bson::Uuid,
    fence: FencingId,
    status: WireSessionStatus,
) -> serde_json::Value {
    serde_json::to_value(StatusReport {
        worker_id: "w1".into(),
        protocol_version: PROTOCOL_VERSION,
        session_id: session.to_string(),
        fencing_id: fence,
        status,
        terminal: None,
        timestamp_unix_ms: 1,
    })
    .unwrap()
}

// ---- credential-refresh ----------------------------------------------------

#[tokio::test]
async fn refresh_valid_fence_returns_token_and_calls_minter_once() {
    let (claims, session, fence) = populated_claims("w1");
    let minter = Arc::new(RecordingMinter::new(MintResult::Token {
        token: SecretString::from("ghs_fresh_token"),
        expires_at: SystemTime::now() + Duration::from_secs(3600),
    }));
    let app = router(claims, Some(minter.clone()));

    let (status, body) = post_json(
        app,
        "/internal/v1/credential-refresh",
        refresh_req(session, fence),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["gone"], serde_json::Value::Bool(false));
    let token = body["credentials"]["token"]
        .as_str()
        .expect("a token must be present on a valid fence");
    assert_eq!(token, "ghs_fresh_token");
    assert!(
        body["credentials"]["expires_at_unix_ms"].as_i64().unwrap() > 0,
        "expiry must be a positive epoch ms"
    );
    assert_eq!(minter.call_count(), 1, "minter called exactly once");
}

#[tokio::test]
async fn refresh_stale_fence_refuses_without_calling_minter() {
    let (claims, session, fence) = populated_claims("w1");
    // Reassign bumps the fence; the worker's pre-reassign fence is now stale.
    claims.reassign("acme/site", "w2").unwrap();
    let minter = Arc::new(RecordingMinter::new(MintResult::Token {
        token: SecretString::from("ghs_should_not_be_returned"),
        expires_at: SystemTime::now() + Duration::from_secs(3600),
    }));
    let app = router(claims, Some(minter.clone()));

    let (status, body) = post_json(
        app,
        "/internal/v1/credential-refresh",
        refresh_req(session, fence),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["credentials"],
        serde_json::Value::Null,
        "a fenced-off worker is NEVER handed a token"
    );
    assert_eq!(body["gone"], serde_json::Value::Bool(false));
    assert_eq!(
        minter.call_count(),
        0,
        "the fence is rejected BEFORE the minter is ever consulted"
    );
}

#[tokio::test]
async fn refresh_installation_gone_returns_gone_true() {
    let (claims, session, fence) = populated_claims("w1");
    let minter = Arc::new(RecordingMinter::new(MintResult::Gone));
    let app = router(claims, Some(minter));

    let (status, body) = post_json(
        app,
        "/internal/v1/credential-refresh",
        refresh_req(session, fence),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["credentials"], serde_json::Value::Null);
    assert_eq!(body["gone"], serde_json::Value::Bool(true));
}

#[tokio::test]
async fn refresh_transient_failure_returns_no_token_not_gone() {
    let (claims, session, fence) = populated_claims("w1");
    let minter = Arc::new(RecordingMinter::new(MintResult::Failed));
    let app = router(claims, Some(minter));

    let (status, body) = post_json(
        app,
        "/internal/v1/credential-refresh",
        refresh_req(session, fence),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["credentials"],
        serde_json::Value::Null,
        "a transient failure keeps the worker on its current token"
    );
    assert_eq!(
        body["gone"],
        serde_json::Value::Bool(false),
        "transient is not terminal"
    );
}

#[tokio::test]
async fn refresh_without_minter_is_503() {
    let (claims, session, fence) = populated_claims("w1");
    let app = router(claims, None);

    let (status, _body) = post_json(
        app,
        "/internal/v1/credential-refresh",
        refresh_req(session, fence),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn refresh_bad_uuid_is_400() {
    let (claims, _session, fence) = populated_claims("w1");
    let minter = Arc::new(RecordingMinter::new(MintResult::Failed));
    let app = router(claims, Some(minter.clone()));

    let mut body = refresh_req(bson::Uuid::new(), fence);
    body["session_id"] = serde_json::Value::String("not-a-uuid".into());
    let (status, _body) = post_json(app, "/internal/v1/credential-refresh", body).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        minter.call_count(),
        0,
        "a bad uuid never reaches the minter"
    );
}

#[tokio::test]
async fn refresh_protocol_mismatch_is_400() {
    let (claims, session, fence) = populated_claims("w1");
    let app = router(claims, None);

    let mut body = refresh_req(session, fence);
    body["protocol_version"] = serde_json::Value::from(999u32);
    let (status, _body) = post_json(app, "/internal/v1/credential-refresh", body).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---- status-report ---------------------------------------------------------

#[tokio::test]
async fn status_report_valid_fence_applies_and_moves_status() {
    let (claims, session, fence) = populated_claims("w1");
    let app = router(claims.clone(), None);

    let (status, body) = post_json(
        app,
        "/internal/v1/status-report",
        status_req(session, fence, WireSessionStatus::Running),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["applied"], serde_json::Value::Bool(true));
    assert_eq!(
        claims.get("acme/site").unwrap().status,
        ClaimStatus::Running,
        "the claim status moved to Running"
    );
}

#[tokio::test]
async fn status_report_stale_fence_is_not_applied_and_leaves_status() {
    let (claims, session, fence) = populated_claims("w1");
    // Reassign bumps the fence AND resets status to Pending.
    claims.reassign("acme/site", "w2").unwrap();
    let app = router(claims.clone(), None);

    let (status, body) = post_json(
        app,
        "/internal/v1/status-report",
        status_req(session, fence, WireSessionStatus::Running),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["applied"],
        serde_json::Value::Bool(false),
        "a stale fence is a no-op"
    );
    assert_eq!(
        claims.get("acme/site").unwrap().status,
        ClaimStatus::Pending,
        "the claim status is unchanged after a fenced-off report"
    );
}

#[tokio::test]
async fn status_report_terminal_failed_applies_from_active() {
    let (claims, session, fence) = populated_claims("w1");
    let app = router(claims.clone(), None);

    let (status, body) = post_json(
        app,
        "/internal/v1/status-report",
        status_req(session, fence, WireSessionStatus::Failed),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["applied"], serde_json::Value::Bool(true));
    assert_eq!(claims.get("acme/site").unwrap().status, ClaimStatus::Failed);
}

#[tokio::test]
async fn status_report_unknown_session_is_not_applied() {
    let (claims, _session, fence) = populated_claims("w1");
    let app = router(claims.clone(), None);

    let (status, body) = post_json(
        app,
        "/internal/v1/status-report",
        status_req(bson::Uuid::new(), fence, WireSessionStatus::Running),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["applied"], serde_json::Value::Bool(false));
}

#[tokio::test]
async fn status_report_bad_uuid_is_400() {
    let (claims, session, fence) = populated_claims("w1");
    let app = router(claims, None);

    let mut body = status_req(session, fence, WireSessionStatus::Running);
    body["session_id"] = serde_json::Value::String("not-a-uuid".into());
    let (status, _body) = post_json(app, "/internal/v1/status-report", body).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn status_report_protocol_mismatch_is_400() {
    let (claims, session, fence) = populated_claims("w1");
    let app = router(claims, None);

    let mut body = status_req(session, fence, WireSessionStatus::Running);
    body["protocol_version"] = serde_json::Value::from(999u32);
    let (status, _body) = post_json(app, "/internal/v1/status-report", body).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unauthorized_without_secret_header() {
    let (claims, session, fence) = populated_claims("w1");
    let app = router(claims, None);

    // No auth header at all: the middleware rejects with 401.
    let response = app
        .oneshot(
            Request::post("/internal/v1/status-report")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    status_req(session, fence, WireSessionStatus::Running).to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The minted token value must never appear in a tracing event. We assert the
/// handler returns the token in the BODY (the only exposure) but the
/// `RefreshedToken`/`MintResult` Debug never renders it.
#[tokio::test]
async fn mint_result_token_redacts_in_debug() {
    let result = MintResult::Token {
        token: SecretString::from("ghs_super_secret"),
        expires_at: SystemTime::now(),
    };
    let rendered = format!("{result:?}");
    assert!(
        !rendered.contains("ghs_super_secret"),
        "the token must never render in Debug: {rendered}"
    );
    // Sanity: a Token still exposes its secret through ExposeSecret (the body path).
    if let MintResult::Token { token, .. } = result {
        assert_eq!(token.expose_secret(), "ghs_super_secret");
    }
}
