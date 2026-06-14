//! GitHub-issues-hub integration tests against the REAL `build_router(AppState)`.
//!
//! A single wiremock `MockServer` plays BOTH the NyxID JWKS endpoint (so the
//! JWT middleware verifies the inbound bearer and populates `raw_token`) AND
//! NyxID itself (the RFC 8693 token-exchange, the connections listing, and the
//! GitHub credential-injection proxy). This exercises the real
//! `NyxIdGithubProxy::from_context` → `exchange_token` → `proxy_github_for`
//! path; no part of the proxy seam is mocked away.
//!
//! Each test gets a fresh Mongo container (testcontainers) and self-skips when
//! Docker is unavailable so `cargo test` stays green without a daemon.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use fkst_hosted_api::auth::{AuthMode, NyxIdAuthSettings};
use fkst_hosted_api::authz::Authorizer;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::goals::GoalRepo;
use fkst_hosted_api::nyxid::{NyxIdClient, GITHUB_PROXY_PATH};
use fkst_hosted_api::packages::{PackageRepository, ShareRepo};
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use fkst_hosted_api::state::AppState;
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rand::rngs::OsRng;
use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use secrecy::SecretString;
use serde_json::{json, Value};
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::mongo::Mongo;
use tower::ServiceExt;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MONGO_TAG: &str = "7";
const ISSUER: &str = "nyxid";
const AUDIENCE: &str = "fkst-test";
const KID: &str = "test-key-1";

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

// ---- RS256 keypair (once per test binary) ----

struct TestKeys {
    encoding_key: EncodingKey,
    n: String,
    e: String,
}

static TEST_KEYS: OnceLock<TestKeys> = OnceLock::new();

fn test_keys() -> &'static TestKeys {
    TEST_KEYS.get_or_init(|| {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA keypair");
        let private_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("private PEM");
        let encoding_key = EncodingKey::from_rsa_pem(private_pem.as_bytes()).expect("encoding key");
        let public_key = private_key.to_public_key();
        let n = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());
        TestKeys { encoding_key, n, e }
    })
}

#[derive(serde::Serialize)]
struct Claims {
    sub: String,
    iss: String,
    aud: String,
    exp: u64,
    iat: u64,
    token_type: String,
    scope: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_secs()
}

/// Mint a valid RS256 access token for the protected routes.
fn valid_token() -> String {
    let now = now_secs();
    let claims = Claims {
        sub: "user-42".to_string(),
        iss: ISSUER.to_string(),
        aud: AUDIENCE.to_string(),
        exp: now + 3600,
        iat: now,
        token_type: "access".to_string(),
        scope: "read write".to_string(),
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    jsonwebtoken::encode(&header, &claims, &test_keys().encoding_key).expect("sign token")
}

fn jwks_body() -> Value {
    let keys = test_keys();
    json!({
        "keys": [{
            "kty": "RSA", "kid": KID, "alg": "RS256", "use": "sig",
            "n": keys.n, "e": keys.e
        }]
    })
}

// ---- App harness ----

struct TestApp {
    _container: testcontainers::ContainerAsync<Mongo>,
    /// Held so the wiremock server (NyxID + JWKS) stays up for the test.
    _server: MockServer,
    router: axum::Router,
}

/// Build the full app with auth ENABLED (JWKS + NyxID both on `server`) and a
/// real `NyxIdClient` wired into the authorizer pointed at the same server.
async fn app(server: MockServer) -> TestApp {
    // The JWKS the middleware verifies the inbound token against.
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_body()))
        .mount(&server)
        .await;
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
    let packages = PackageRepository::new(&db.database);
    let shares = ShareRepo::new(&db.database);
    let goals = GoalRepo::new(&db.database);
    let sessions = SessionService::new(
        SessionRepo::new(&db),
        packages.clone(),
        EngineConfig::default(),
    );

    let nyxid = NyxIdClient::new(
        &server.uri(),
        "sa_test_client".to_string(),
        SecretString::from("sas_test_secret".to_string()),
        Duration::from_secs(30),
    )
    .expect("nyxid client");

    let router = build_router(AppState {
        config,
        db,
        packages,
        shares,
        sessions,
        auth_mode: AuthMode::Enabled(NyxIdAuthSettings {
            base_url: server.uri(),
            issuer: ISSUER.to_string(),
            audience: AUDIENCE.to_string(),
            jwks_cache_ttl: Duration::from_secs(300),
        }),
        authz: Authorizer::new(Some(nyxid)),
        github_app: None,
        goals,
        engine: EngineConfig::default(),
        llm: None,
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
                .header(header::AUTHORIZATION, format!("Bearer {}", valid_token()))
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
        .header(header::AUTHORIZATION, format!("Bearer {}", valid_token()));
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
