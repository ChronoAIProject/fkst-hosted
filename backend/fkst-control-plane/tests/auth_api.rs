//! Proxy-trusted authentication + `fkst:*` RBAC end-to-end tests (issue #113).
//!
//! fkst-hosted no longer authenticates users: the NyxID proxy verifies the
//! caller and injects the identity into the forwarded request. These tests
//! exercise the router with the injected `X-NyxID-*` headers (no JWKS, no
//! signature verification, no network). The controller is datastore-free
//! (#143), so these tests need no database and run unconditionally.
//!
//! Test cases:
//!  1. Valid identity token carrying `fkst:goal:read` -> 200 on `/api/v1/goals`
//!  2. Identity token with a BAD signature but a valid payload -> 200 (the
//!     proxy is trusted; the signature is never checked and no network occurs)
//!  3. Identity token WITHOUT the required permission -> 403 (action layer)
//!  4. Headers-mode (`X-NyxID-User-Id`, no permissions) -> 403 on a gated route
//!  5. Neither identity token nor `X-NyxID-User-Id` -> 401
//!  6. `fkst:admin` permission bypasses the per-route permission -> 200
//!  7. Both health routes public with auth enabled
//!  8. Auth disabled -> routes open, extractor yields the dev (admin) context
//!  9. Extractor on an unprotected route with auth enabled -> 500

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use fkst_control_plane::auth::{AuthMode, NyxIdAuthSettings};
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

mod support;

/// Header carrying the proxy-injected identity JWT.
const HEADER_IDENTITY_TOKEN: &str = "X-NyxID-Identity-Token";
/// Header carrying just the user id (headers mode).
const HEADER_USER_ID: &str = "X-NyxID-User-Id";

/// Build a JWT-shaped identity token from a claims payload and an arbitrary
/// (unverified) signature segment. The signature is NEVER checked — the proxy
/// is trusted — so callers can pass any value.
fn identity_token(payload: &Value, signature: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    format!("{header}.{body}.{signature}")
}

/// Drain a response into `(status, headers, json-body)`.
async fn drain(response: axum::response::Response) -> (StatusCode, HeaderMap, Value) {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON body")
    };
    (status, headers, body)
}

/// Everything a test with auth enabled needs.
struct AuthTestApp {
    router: axum::Router,
}

/// Build the full app router with auth enabled (proxy-trusted identity). No
/// JWKS endpoint, no service-account NyxID client (`Authorizer::disabled`) —
/// these tests only exercise the authn/RBAC edge, not org lookups.
async fn auth_app() -> AuthTestApp {
    let config = Config::default();
    let goals = GoalIssueStore::new(None);
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());
    let auth_mode = AuthMode::Enabled(NyxIdAuthSettings {
        base_url: "https://nyxid.example.test".to_string(),
    });
    let vault = support::test_vault();
    let router = build_router(AppState {
        binding_store: fkst_control_plane::nyxid_connect::BrokerBindingStore::new(),
        config,
        sessions,
        auth_mode,
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret: None,
        goals,
        vault,
        ornn: None,
    })
    .expect("router");
    AuthTestApp { router }
}

/// Build an app with auth disabled.
async fn no_auth_app() -> axum::Router {
    let config = Config::default();
    let goals = GoalIssueStore::new(None);
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());
    let vault = support::test_vault();
    build_router(AppState {
        binding_store: fkst_control_plane::nyxid_connect::BrokerBindingStore::new(),
        config,
        sessions,
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret: None,
        goals,
        vault,
        ornn: None,
    })
    .expect("router")
}

/// GET `path` with an `X-NyxID-Identity-Token` carrying `payload`.
async fn get_with_identity(
    router: &axum::Router,
    path: &str,
    payload: &Value,
    signature: &str,
) -> (StatusCode, HeaderMap, Value) {
    let token = identity_token(payload, signature);
    let response = router
        .clone()
        .oneshot(
            Request::get(path)
                .header(HEADER_IDENTITY_TOKEN, token)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// GET `path` with a single header pair.
async fn get_with_header(
    router: &axum::Router,
    path: &str,
    name: &str,
    value: &str,
) -> (StatusCode, HeaderMap, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::get(path)
                .header(name, value)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// GET `path` with no injected identity at all.
async fn get_no_identity(router: &axum::Router, path: &str) -> (StatusCode, HeaderMap, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::get(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

// ---- Test cases ----

#[tokio::test]
async fn valid_identity_with_permission_returns_200() {
    let app = auth_app().await;
    let payload = json!({ "sub": "u-1", "permissions": ["fkst:goal:read"] });
    let (status, _h, body) =
        get_with_identity(&app.router, "/api/v1/goals", &payload, "good-sig").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, json!([]), "empty store returns empty array");
}

#[tokio::test]
async fn bad_signature_but_valid_payload_is_accepted() {
    // The trust contract: a junk signature with a valid payload is accepted.
    // fkst-hosted makes NO network call (there is no JWKS endpoint configured
    // and `base_url` is unroutable) — proven by this returning 200 quickly.
    let app = auth_app().await;
    let payload = json!({ "sub": "u-trusted", "permissions": ["fkst:goal:read"] });
    let (status, _h, body) = get_with_identity(
        &app.router,
        "/api/v1/goals",
        &payload,
        "this-signature-is-garbage-and-never-checked",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "trusted payload accepted; body: {body}"
    );
}

#[tokio::test]
async fn identity_without_required_permission_returns_403() {
    let app = auth_app().await;
    // Has SOME permission, but not the one `/api/v1/goals` (read) requires.
    let payload = json!({ "sub": "u-2", "permissions": ["fkst:session:read"] });
    let (status, _h, body) = get_with_identity(&app.router, "/api/v1/goals", &payload, "sig").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(body["error"], "forbidden");
}

#[tokio::test]
async fn headers_mode_has_no_permissions_so_gated_route_is_403() {
    let app = auth_app().await;
    // Headers mode carries an empty permission set -> the action layer denies.
    let (status, _h, body) =
        get_with_header(&app.router, "/api/v1/goals", HEADER_USER_ID, "u-3").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(body["error"], "forbidden");
}

#[tokio::test]
async fn no_injected_identity_returns_401() {
    let app = auth_app().await;
    let (status, _h, body) = get_no_identity(&app.router, "/api/v1/goals").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body: {body}");
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn admin_permission_bypasses_per_route_permission() {
    let app = auth_app().await;
    // `fkst:admin` alone (no `fkst:goal:read`) still passes the action layer.
    let payload = json!({ "sub": "ops", "permissions": ["fkst:admin"] });
    let (status, _h, body) = get_with_identity(&app.router, "/api/v1/goals", &payload, "sig").await;
    assert_eq!(status, StatusCode::OK, "admin bypass; body: {body}");
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn health_routes_are_public_with_auth_enabled() {
    let app = auth_app().await;

    let (status, _h, body) = get_no_identity(&app.router, "/health").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "ok");

    let (status, _h, body) = get_no_identity(&app.router, "/api/v1/health").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn auth_disabled_routes_are_open_and_extractor_yields_dev_admin() {
    let router = no_auth_app().await;
    // No identity needed; the dev context carries `fkst:admin`, so the gated
    // `/api/v1/goals` read passes and returns an empty list.
    let (status, _h, body) = get_no_identity(&router, "/api/v1/goals").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn extractor_on_unprotected_route_with_auth_enabled_returns_500() {
    // Programming-error path: a handler extracting AuthContext on a route NOT
    // behind protect(), with auth enabled, must 500 (the extractor detects the
    // missing extension under AuthMode::Enabled).
    use fkst_control_plane::auth::AuthContext;

    async fn needs_auth(ctx: AuthContext) -> String {
        format!("user={}", ctx.user_id)
    }

    let config = Config::default();
    let goals = GoalIssueStore::new(None);
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());
    let vault = support::test_vault();
    let state = AppState {
        binding_store: fkst_control_plane::nyxid_connect::BrokerBindingStore::new(),
        config,
        sessions,
        auth_mode: AuthMode::Enabled(NyxIdAuthSettings {
            base_url: "https://nyxid.example.test".to_string(),
        }),
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret: None,
        goals,
        vault,
        ornn: None,
    };

    let test_router = axum::Router::new()
        .route("/test-extract", axum::routing::get(needs_auth))
        .with_state(state);

    let response = test_router
        .clone()
        .oneshot(
            Request::get("/test-extract")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    let (status, _h, body) = drain(response).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "expected 500 for extractor on unprotected route, body: {body}"
    );
    assert_eq!(body["error"], "internal");
}
