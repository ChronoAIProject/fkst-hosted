//! GitHub-issues-hub integration tests against the REAL `build_router(AppState)`.
//!
//! Auth is proxy-trusted (#113): each request carries a NyxID-injected
//! `X-NyxID-Identity-Token` (decoded, not verified) granting `fkst:admin` so it
//! clears the per-route `fkst:github:*` action gate, PLUS the user's
//! `Authorization: Bearer` (captured as `user_access_token`) which the proxy
//! seam exchanges. A single wiremock `MockServer` plays NyxID itself — the
//! RFC 8693 token-exchange, the connections listing, and the GitHub
//! credential-injection proxy — so the real
//! `NyxIdGithubProxy::from_context` → `exchange_token` → `proxy_github_for`
//! path is exercised end-to-end; no part of the proxy seam is mocked away.
//!
//! Each test gets a fresh Mongo container (testcontainers) and self-skips when
//! Docker is unavailable so `cargo test` stays green without a daemon.

use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use fkst_control_plane::auth::{AuthMode, NyxIdAuthSettings};
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::db::Db;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::nyxid::{NyxIdClient, DEFAULT_GITHUB_PROXY_SLUG};
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use http_body_util::BodyExt;
use secrecy::SecretString;
use serde_json::{json, Value};
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::mongo::Mongo;
use tower::ServiceExt;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod support;

const MONGO_TAG: &str = "7";

/// Header carrying the proxy-injected identity JWT.
const HEADER_IDENTITY_TOKEN: &str = "X-NyxID-Identity-Token";

/// GitHub-proxy base path the wiremock matchers expect, built from the default
/// slug so the integration suite tracks the production path shape after the
/// slug was made configurable. The `app(...)` helper builds its `NyxIdClient`
/// with `DEFAULT_GITHUB_PROXY_SLUG`, so this must agree with it.
const GITHUB_PROXY_PATH: &str = "/api/v1/proxy/api-github";
// Compile-time guard: keep the literal above in lockstep with the public
// default slug, so a future slug change cannot silently desync these matchers.
const _: () = assert!(matches!(
    DEFAULT_GITHUB_PROXY_SLUG.as_bytes(),
    b"api-github"
));

/// True when a Docker daemon answers `docker info`.
fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

// ---- Identity / bearer helpers ----

/// The user's `Authorization: Bearer` token captured as `user_access_token` and
/// fed to the RFC 8693 exchange. Any string works (the proxy is trusted; the
/// exchange mock returns the delegated token regardless of the input value).
const USER_ACCESS_TOKEN: &str = "user-inbound-token";

/// A proxy-injected identity token (decode-only; signature never checked). It
/// grants `fkst:admin` so every `fkst:github:*` action gate passes — these
/// tests target the proxy seam, not the RBAC matrix (covered elsewhere).
fn admin_identity_token() -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let body = URL_SAFE_NO_PAD.encode(
        json!({ "sub": "user-42", "permissions": ["fkst:admin"] })
            .to_string()
            .as_bytes(),
    );
    // The signature segment is arbitrary and never verified.
    format!("{header}.{body}.unverified-signature")
}

// ---- App harness ----

struct TestApp {
    _container: testcontainers::ContainerAsync<Mongo>,
    /// Held so the wiremock server (NyxID) stays up for the test.
    _server: MockServer,
    router: axum::Router,
}

/// Build the full app with auth ENABLED (proxy-trusted identity) and a real
/// `NyxIdClient` wired into the authorizer, pointed at `server` (which plays
/// NyxID's exchange / connections / proxy endpoints).
async fn app(server: MockServer) -> TestApp {
    // RFC 8693 token exchange — returns a delegated bearer the proxy will use.
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "delegated_tok", "token_type": "Bearer", "expires_in": 300
        })))
        .mount(&server)
        .await;

    let container = Mongo::default()
        .with_tag(MONGO_TAG)
        .start()
        .await
        .expect("start mongo");
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(27017).await.expect("port");
    let config = Config {
        mongodb_uri: format!("mongodb://{host}:{port}"),
        mongodb_server_selection_timeout_ms: 5000,
        ..Config::default()
    };
    let db = Db::connect(&config).await.expect("connect + ping");
    let goals = GoalIssueStore::new(None);
    let sessions = SessionService::new(SessionRepo::new(&db), EngineConfig::default());

    let nyxid = NyxIdClient::new(
        &server.uri(),
        DEFAULT_GITHUB_PROXY_SLUG,
        "sa_test_client".to_string(),
        SecretString::from("sas_test_secret".to_string()),
        Duration::from_secs(30),
    )
    .expect("nyxid client");

    let vault = support::test_vault(&db);
    let router = build_router(AppState {
        config,
        db,
        sessions,
        auth_mode: AuthMode::Enabled(NyxIdAuthSettings {
            base_url: server.uri(),
        }),
        authz: Authorizer::new(Some(nyxid)),
        github_app: None,
        github_app_webhook_secret: None,
        goals,
        vault,
        ornn: None,
    })
    .expect("router");

    TestApp {
        _container: container,
        _server: server,
        router,
    }
}

/// Mount the connections listing returning the given JSON array.
async fn mount_connections(server: &MockServer, body: Value) {
    Mock::given(method("GET"))
        .and(path("/api/v1/connections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Drain a response into (status, headers, parsed JSON or Null).
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

async fn get(router: &axum::Router, p: &str) -> (StatusCode, HeaderMap, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::get(p)
                .header(HEADER_IDENTITY_TOKEN, admin_identity_token())
                .header(header::AUTHORIZATION, format!("Bearer {USER_ACCESS_TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("respond");
    drain(response).await
}

async fn send_json(
    router: &axum::Router,
    verb: &str,
    p: &str,
    body: &Value,
) -> (StatusCode, HeaderMap, Value) {
    let builder = Request::builder()
        .method(verb)
        .uri(p)
        .header(header::CONTENT_TYPE, "application/json")
        .header(HEADER_IDENTITY_TOKEN, admin_identity_token())
        .header(header::AUTHORIZATION, format!("Bearer {USER_ACCESS_TOKEN}"));
    let response = router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).expect("request"))
        .await
        .expect("respond");
    drain(response).await
}

/// Two linked accounts for multi-account tests.
fn two_connections() -> Value {
    json!([
        { "connection_id": "c-octo", "login": "octocat", "primary": true },
        { "connection_id": "c-hub", "login": "hubber", "primary": false }
    ])
}

/// One linked account for single-target tests.
fn one_connection() -> Value {
    json!([{ "connection_id": "c-octo", "login": "octocat", "primary": true }])
}

// ---- Tests ----

#[tokio::test]
async fn accounts_lists_connections_via_nyxid() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, two_connections()).await;
    let app = app(server).await;

    let (status, _h, body) = get(&app.router, "/api/v1/github/accounts").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let arr = body.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["login"], "octocat");
    assert_eq!(arr[0]["primary"], true);
    assert_eq!(arr[1]["login"], "hubber");
}

#[tokio::test]
async fn issues_aggregate_merges_two_accounts_with_partial_failure() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, two_connections()).await;

    // octocat succeeds with one issue.
    Mock::given(method("GET"))
        .and(path(format!("{GITHUB_PROXY_PATH}/issues")))
        .and(query_param("_nyxid_via", "c-octo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 1, "number": 7, "title": "octo issue", "state": "open",
            "comments": 0, "html_url": "https://github.com/acme/site/issues/7",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "repository_url": "https://api.github.com/repos/acme/site",
            "labels": [], "assignees": []
        }])))
        .mount(&server)
        .await;
    // hubber is rate-limited (403 + x-ratelimit-remaining:0).
    Mock::given(method("GET"))
        .and(path(format!("{GITHUB_PROXY_PATH}/issues")))
        .and(query_param("_nyxid_via", "c-hub"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", "9999999999")
                .set_body_string("rate limited"),
        )
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, _h, body) = get(&app.router, "/api/v1/github/issues").await;
    assert_eq!(status, StatusCode::OK, "fan-out is always 200: {body}");
    let results = body["results"].as_array().expect("results");
    assert_eq!(results.len(), 2);

    let octo = results
        .iter()
        .find(|r| r["account"] == "octocat")
        .expect("octocat");
    assert!(octo["error"].is_null(), "octocat should not error");
    assert_eq!(octo["issues"].as_array().unwrap().len(), 1);

    let hub = results
        .iter()
        .find(|r| r["account"] == "hubber")
        .expect("hubber");
    assert_eq!(hub["error"]["kind"], "rate_limited");
    assert!(hub["issues"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn issues_aggregate_respects_accounts_filter_state_labels_pagination() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, two_connections()).await;

    // Only octocat is queried (accounts filter). Assert the upstream query.
    Mock::given(method("GET"))
        .and(path(format!("{GITHUB_PROXY_PATH}/issues")))
        .and(query_param("_nyxid_via", "c-octo"))
        .and(query_param("filter", "created"))
        .and(query_param("state", "closed"))
        .and(query_param("per_page", "5"))
        .and(query_param("page", "2"))
        .and(query_param("labels", "help wanted,p1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(
                    "link",
                    "<https://api.github.com/issues?page=3>; rel=\"next\"",
                )
                .set_body_json(json!([])),
        )
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, _h, body) = get(
        &app.router,
        "/api/v1/github/issues?accounts=octocat&filter=created&state=closed&labels=help%20wanted,p1&page=2&per_page=5",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let results = body["results"].as_array().expect("results");
    assert_eq!(results.len(), 1, "only octocat should be queried");
    assert_eq!(results[0]["account"], "octocat");
    assert_eq!(results[0]["has_more"], true, "Link rel=next => has_more");
}

#[tokio::test]
async fn create_issue_translates_201_and_copies_ratelimit_headers() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, one_connection()).await;
    Mock::given(method("POST"))
        .and(path(format!("{GITHUB_PROXY_PATH}/repos/acme/site/issues")))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("x-ratelimit-remaining", "4999")
                .insert_header("x-ratelimit-reset", "1700000000")
                .set_body_json(json!({
                    "id": 10, "number": 12, "title": "new issue", "body": "the body",
                    "state": "open", "comments": 0,
                    "html_url": "https://github.com/acme/site/issues/12",
                    "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
                    "repository_url": "https://api.github.com/repos/acme/site",
                    "labels": [], "assignees": []
                })),
        )
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, headers, body) = send_json(
        &app.router,
        "POST",
        "/api/v1/github/repos/acme/site/issues",
        &json!({ "title": "new issue", "body": "the body" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    assert_eq!(body["number"], 12);
    assert_eq!(body["repository"], "acme/site");
    assert_eq!(
        headers.get("x-ratelimit-remaining").unwrap(),
        "4999",
        "rate-limit header copied through"
    );
}

#[tokio::test]
async fn create_issue_with_two_accounts_and_no_account_param_is_422() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, two_connections()).await;
    let app = app(server).await;

    let (status, _h, body) = send_json(
        &app.router,
        "POST",
        "/api/v1/github/repos/acme/site/issues",
        &json!({ "title": "ambiguous" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["error"], "unprocessable");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("multiple GitHub accounts linked"),
        "message: {body}"
    );
}

#[tokio::test]
async fn patch_issue_state_closed_roundtrips() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, one_connection()).await;
    Mock::given(method("PATCH"))
        .and(path(format!(
            "{GITHUB_PROXY_PATH}/repos/acme/site/issues/7"
        )))
        .and(wiremock::matchers::body_partial_json(json!({
            "state": "closed"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "number": 7, "title": "t", "body": "b", "state": "closed",
            "comments": 0, "html_url": "https://github.com/acme/site/issues/7",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-02T00:00:00Z",
            "repository_url": "https://api.github.com/repos/acme/site",
            "labels": [], "assignees": []
        })))
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, _h, body) = send_json(
        &app.router,
        "PATCH",
        "/api/v1/github/repos/acme/site/issues/7",
        &json!({ "state": "closed" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "closed");
}

#[tokio::test]
async fn get_single_issue_includes_body() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, one_connection()).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "{GITHUB_PROXY_PATH}/repos/acme/site/issues/7"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "number": 7, "title": "t", "body": "the issue body", "state": "open",
            "comments": 2, "html_url": "https://github.com/acme/site/issues/7",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "repository_url": "https://api.github.com/repos/acme/site",
            "labels": [{"name":"bug"}], "assignees": [{"login":"octocat"}]
        })))
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, _h, body) = get(&app.router, "/api/v1/github/repos/acme/site/issues/7").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["body"], "the issue body", "single GET includes body");
    assert_eq!(body["labels"][0], "bug");
    assert_eq!(body["assignees"][0], "octocat");
}

#[tokio::test]
async fn comments_list_and_create_roundtrip() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, one_connection()).await;
    // List comments.
    Mock::given(method("GET"))
        .and(path(format!(
            "{GITHUB_PROXY_PATH}/repos/acme/site/issues/7/comments"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 55, "user": {"login": "octocat"}, "body": "looks good",
            "html_url": "https://github.com/acme/site/issues/7#c55",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }])))
        .mount(&server)
        .await;
    // Create comment.
    Mock::given(method("POST"))
        .and(path(format!(
            "{GITHUB_PROXY_PATH}/repos/acme/site/issues/7/comments"
        )))
        .and(wiremock::matchers::body_partial_json(json!({
            "body": "thanks"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 56, "user": {"login": "octocat"}, "body": "thanks",
            "html_url": "https://github.com/acme/site/issues/7#c56",
            "created_at": "2026-01-02T00:00:00Z", "updated_at": "2026-01-02T00:00:00Z"
        })))
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, _h, body) = get(
        &app.router,
        "/api/v1/github/repos/acme/site/issues/7/comments",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["user"], "octocat");

    let (status, _h, body) = send_json(
        &app.router,
        "POST",
        "/api/v1/github/repos/acme/site/issues/7/comments",
        &json!({ "body": "thanks" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    assert_eq!(body["id"], 56);
    assert_eq!(body["body"], "thanks");
}

#[tokio::test]
async fn rate_limited_single_target_is_429_with_retry_after() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, one_connection()).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "{GITHUB_PROXY_PATH}/repos/acme/site/issues/7"
        )))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_string("slow down"),
        )
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, headers, body) = get(&app.router, "/api/v1/github/repos/acme/site/issues/7").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "body: {body}");
    assert_eq!(body["error"], "rate_limited");
    assert_eq!(headers.get("retry-after").unwrap(), "30");
}

#[tokio::test]
async fn github_500_is_502_upstream() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, one_connection()).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "{GITHUB_PROXY_PATH}/repos/acme/site/issues/7"
        )))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, _h, body) = get(&app.router, "/api/v1/github/repos/acme/site/issues/7").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "body: {body}");
    assert_eq!(body["error"], "upstream_error");
}

#[tokio::test]
async fn github_404_is_404() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, one_connection()).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "{GITHUB_PROXY_PATH}/repos/acme/site/issues/7"
        )))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, _h, body) = get(&app.router, "/api/v1/github/repos/acme/site/issues/7").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn per_page_is_clamped_to_50() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_connections(&server, one_connection()).await;
    // The mock ONLY matches when per_page=50; if the handler forwarded 999 the
    // request would 404 at wiremock and the account would carry an error.
    Mock::given(method("GET"))
        .and(path(format!("{GITHUB_PROXY_PATH}/issues")))
        .and(query_param("per_page", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let app = app(server).await;
    let (status, _h, body) = get(&app.router, "/api/v1/github/issues?per_page=999").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let results = body["results"].as_array().expect("results");
    assert_eq!(results.len(), 1);
    assert!(
        results[0]["error"].is_null(),
        "clamped per_page must match the mock: {body}"
    );
}
