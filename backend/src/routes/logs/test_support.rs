//! Shared fixtures for the log-download endpoint tests, driving the REAL
//! `build_router` via `tower::ServiceExt::oneshot` (no TCP bind) with wiremock
//! standing in for GitHub `/user`, the OAuth token endpoint, and chrono-storage.
//!
//! Split out of the test files so both the API-mode ([`super::tests`]) and browser-mode
//! ([`super::tests_browser`]) suites share one set of helpers while each file stays
//! well under the source line budget.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request};
use http_body_util::BodyExt;
use secrecy::SecretString;
use serde_json::Value;
use tower::ServiceExt;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::config::Config;
use crate::log_access::{LogAccessRegistry, LogSessionContext};
use crate::log_config::LogConfig;
use crate::models::RepoRef;
use crate::router::build_router;
use crate::state::AppState;
use crate::storage::{ChronoStorageClient, ChronoStorageConfig};

pub(crate) const SESSION_ID: &str = "sess-abc-123";
pub(crate) const AUTHOR_ID: i64 = 1001;
/// The exact bytes the mocked `/objects/download` read serves — asserted
/// verbatim by the streaming happy-path tests.
pub(crate) const BUNDLE_BYTES: &[u8] = b"\x1f\x8b\x08\x00fkst-log-bundle";

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

/// A registry pre-populated with SESSION_ID's context (author + allow-list).
pub(crate) fn registry(allow: &[&str]) -> LogAccessRegistry {
    let reg = LogAccessRegistry::new();
    reg.upsert(
        SESSION_ID.to_string(),
        LogSessionContext {
            installation_id: 1,
            repo: repo(),
            trigger_issue: 7,
            author_id: AUTHOR_ID,
            log_access: allow.iter().map(|s| s.to_string()).collect(),
        },
    );
    reg
}

/// A [`LogConfig`] with the admin list set and (when `oauth`) the browser-OAuth creds.
pub(crate) fn log_config(admins: &[&str], oauth: bool) -> LogConfig {
    LogConfig {
        admins: admins.iter().map(|s| s.to_string()).collect(),
        public_base_url: oauth.then(|| "https://fkst.example".to_string()),
        oauth_client_id: oauth.then(|| "Iv1.clientid".to_string()),
        oauth_client_secret: oauth.then(|| SecretString::from("oauth-secret".to_string())),
        oauth_base_url: "https://github.test".to_string(),
        frontend_url: None,
    }
}

/// Assemble an [`AppState`] over the given GitHub API base, storage client, log
/// config, and pre-populated registry.
pub(crate) fn state(
    github_base: String,
    storage: Option<Arc<ChronoStorageClient>>,
    log: LogConfig,
    registry: LogAccessRegistry,
) -> AppState {
    let config = Config {
        github_api_base_url: github_base,
        log,
        ..Config::default()
    };
    AppState {
        config,
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage,
        log_registry: registry,
        log_bundle_cache: Default::default(),
    }
}

/// A GitHub `/user` mock returning `{login, id}` for ANY bearer (a valid user token).
pub(crate) async fn github_user_ok(login: &str, id: i64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "login": login,
            "id": id,
        })))
        .mount(&server)
        .await;
    server
}

/// A GitHub `/user` mock that always 401s (a rejected / non-user token).
pub(crate) async fn github_user_401() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    server
}

/// A chrono-storage mock: mints a token, and serves the bundle bytes from the
/// real chrono-bucket read route `/objects/download` (issue #497) — or, when
/// `present` is false, 404 (no object).
pub(crate) async fn storage_server(present: bool) -> (Arc<ChronoStorageClient>, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "sa-token",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    let key = format!("logs/{SESSION_ID}/latest.tar.gz");
    if present {
        // Both API and browser mode stream through `download()`: one
        // authenticated GET returning the raw bundle bytes.
        Mock::given(method("GET"))
            .and(path("/api/buckets/logs/objects/download"))
            .and(query_param("key", key.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(BUNDLE_BYTES.to_vec()))
            .mount(&server)
            .await;
    } else {
        Mock::given(method("GET"))
            .and(path("/api/buckets/logs/objects/download"))
            .and(query_param("key", key))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
    }
    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: "logs".to_string(),
        nyxid_token_url: format!("{}/oauth/token", server.uri()),
        nyxid_client_id: "sa-client".to_string(),
        nyxid_client_secret: SecretString::from("sa-secret".to_string()),
    };
    (
        Arc::new(ChronoStorageClient::new(reqwest::Client::new(), config)),
        server,
    )
}

/// Issue a GET to the built router, optionally with a bearer token.
pub(crate) async fn get(
    state: AppState,
    uri: &str,
    bearer: Option<&str>,
) -> axum::response::Response {
    // The token→identity cache is a process-global; reset it so a token STRING reused
    // across tests (with a different mocked identity) never leaks between them.
    super::identity::clear_cache();
    let router = build_router(state).expect("router builds");
    let mut req = Request::get(uri);
    if let Some(token) = bearer {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    router
        .oneshot(req.body(Body::empty()).expect("request builds"))
        .await
        .expect("router responds")
}

/// Collect a response body as JSON.
pub(crate) async fn body_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

/// Collect a response body as raw bytes (for the streamed-attachment assertion).
pub(crate) async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec()
}

/// The `Location` header of a redirect response.
pub(crate) fn location(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .expect("Location header present")
        .to_str()
        .unwrap()
        .to_string()
}
