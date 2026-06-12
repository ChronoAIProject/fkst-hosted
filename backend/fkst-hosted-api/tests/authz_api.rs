//! End-to-end authorization matrix tests.
//!
//! Exercises the full authz stack (JWT auth middleware + Authorizer + NyxID
//! client) against wiremock NyxID endpoints and testcontainers MongoDB. Builds
//! the real `build_router(AppState)` with auth ENABLED and a real
//! `NyxIdClient` backed by wiremock so org-role and user-orgs calls are
//! exercised end-to-end.
//!
//! Self-skipping when Docker is unavailable.
//!
//! Test cases (DoD matrix):
//!  1. Owner sees and manages their package
//!  2. Stranger gets 404 (anti-enumeration)
//!  3. Org viewer reads but cannot create session
//!  4. Org member creates and stops sessions
//!  5. create_into_org_requires_membership
//!  6. legacy_package_remains_usable
//!  7. list_filters_to_visible_set

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use fkst_hosted_api::auth::{AuthMode, NyxIdAuthSettings};
use fkst_hosted_api::authz::Authorizer;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::nyxid::NyxIdClient;
use fkst_hosted_api::packages::PackageRepository;
use fkst_hosted_api::packages::PACKAGES_COLLECTION;
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
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Mongo image tag.
const MONGO_TAG: &str = "7";

/// JWT issuer for tests.
const ISSUER: &str = "nyxid";

/// JWT audience for tests.
const AUDIENCE: &str = "fkst-test";

/// Key ID used for the test keypair.
const KID: &str = "test-key-1";

/// JWKS cache TTL for tests (short so refreshes are fast).
const JWKS_TTL_SECS: u64 = 2;

/// NyxID client credentials for wiremock.
const NYXID_CLIENT_ID: &str = "sa_test_client";
const NYXID_CLIENT_SECRET: &str = "sas_test_secret";

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

// ---- Key generation (once per test binary, reused from auth_api.rs) ----

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
        let encoding_key =
            EncodingKey::from_rsa_pem(private_pem.as_bytes()).expect("encoding key from PEM");
        let public_key = private_key.to_public_key();
        let n_bytes = public_key.n().to_bytes_be();
        let e_bytes = public_key.e().to_bytes_be();
        let n = URL_SAFE_NO_PAD.encode(&n_bytes);
        let e = URL_SAFE_NO_PAD.encode(&e_bytes);
        TestKeys { encoding_key, n, e }
    })
}

// ---- JWT helpers ----

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    exp: u64,
    iat: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_secs()
}

fn claims_for(user: &str) -> TestClaims {
    let now = now_secs();
    TestClaims {
        sub: user.to_string(),
        iss: ISSUER.to_string(),
        aud: AUDIENCE.to_string(),
        exp: now + 3600,
        iat: now,
        token_type: Some("access".to_string()),
        scope: Some("read write".to_string()),
    }
}

fn sign_token(claims: &TestClaims) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    jsonwebtoken::encode(&header, claims, &test_keys().encoding_key).expect("sign token")
}

fn token_for(user: &str) -> String {
    sign_token(&claims_for(user))
}

// ---- JWKS response ----

fn jwks_response() -> Value {
    let keys = test_keys();
    json!({
        "keys": [{
            "kty": "RSA",
            "kid": KID,
            "alg": "RS256",
            "use": "sig",
            "n": keys.n,
            "e": keys.e
        }]
    })
}

// ---- Drain response helper ----

async fn drain(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
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
    (status, body)
}

// ---- Test infrastructure ----

/// Mount all NyxID wiremock mocks needed for the authorization tests.
///
/// Wiremock endpoints:
/// - `POST /oauth/token` — service token exchange (returns a fake service token)
/// - `GET /api/v1/orgs/{org_id}/members` — org role lookups
async fn mount_nyxid_mocks(server: &MockServer) {
    // Service token endpoint — always returns a valid token.
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "svc_tok_authz_test",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(server)
        .await;

    // Org members for org_alpha: user_C is viewer, user_D is member.
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/org_alpha/members"))
        .and(header("authorization", "Bearer svc_tok_authz_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "membership_id": "m1", "user_id": "user_C", "role": "viewer" },
            { "membership_id": "m2", "user_id": "user_D", "role": "member" },
        ])))
        .mount(server)
        .await;

    // User orgs: default to empty (no orgs). Individual tests mount
    // per-user overrides with more specific matching. The Authorizer
    // forwards the caller's own JWT (not the service token), so we
    // cannot match on a fixed Authorization header value.
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(server)
        .await;
}

// ---- HTTP helpers ----

async fn get_with_auth(router: &axum::Router, path: &str, token: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::get(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

async fn post_json_with_auth(
    router: &axum::Router,
    path: &str,
    token: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::post(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

async fn post_empty_with_auth(
    router: &axum::Router,
    path: &str,
    token: &str,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::post(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// A minimal valid package body for API requests.
fn valid_package_body(name: &str) -> Value {
    json!({
        "name": name,
        "files": [
            { "path": "departments/router/main.lua", "content": "return {}" },
        ],
        "composed_deps": []
    })
}

/// A minimal valid package body for API requests with an org_id.
fn valid_package_body_with_org(name: &str, org_id: &str) -> Value {
    json!({
        "name": name,
        "files": [
            { "path": "departments/router/main.lua", "content": "return {}" },
        ],
        "composed_deps": [],
        "org_id": org_id
    })
}

/// Session create request body.
fn session_body(package_name: &str) -> Value {
    json!({ "package_name": package_name })
}

// ---- Test infrastructure with DB access ----

/// Extended test app that also exposes the database and mock server for
/// direct manipulation. Individual tests can mount additional mocks on
/// `mock_server` after construction.
struct AuthzTestAppWithDb {
    mock_server: MockServer,
    _container: testcontainers::ContainerAsync<Mongo>,
    router: axum::Router,
    database: mongodb::Database,
}

async fn authz_app_with_db() -> AuthzTestAppWithDb {
    let mock_server = MockServer::start().await;

    // Mount JWKS.
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_response()))
        .mount(&mock_server)
        .await;

    // Mount NyxID service mocks.
    mount_nyxid_mocks(&mock_server).await;

    let container = Mongo::default()
        .with_tag(MONGO_TAG)
        .start()
        .await
        .expect("start mongo");
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(27017)
        .await
        .expect("container port");
    let config = Config {
        mongodb_uri: format!("mongodb://{host}:{port}"),
        mongodb_server_selection_timeout_ms: 5000,
        ..Config::default()
    };
    let db = Db::connect(&config).await.expect("connect + ping");
    let database = db.database.clone();
    let packages = PackageRepository::new(&db.database);
    let sessions = SessionService::new(
        SessionRepo::new(&db),
        packages.clone(),
        EngineConfig::default(),
    );

    let nyxid_client = NyxIdClient::new(
        &mock_server.uri(),
        NYXID_CLIENT_ID.to_string(),
        SecretString::from(NYXID_CLIENT_SECRET.to_string()),
        Duration::from_secs(30),
    )
    .expect("NyxIdClient build");

    let auth_mode = AuthMode::Enabled(NyxIdAuthSettings {
        base_url: mock_server.uri(),
        issuer: ISSUER.to_string(),
        audience: AUDIENCE.to_string(),
        jwks_cache_ttl: Duration::from_secs(JWKS_TTL_SECS),
    });

    let router = build_router(AppState {
        config,
        db,
        packages,
        sessions,
        auth_mode,
        authz: Authorizer::new(Some(nyxid_client)),
    })
    .expect("router");

    AuthzTestAppWithDb {
        mock_server,
        _container: container,
        router,
        database,
    }
}

/// Insert a legacy package directly into MongoDB (no owner_user_id, no org_id).
async fn insert_legacy_package_db(database: &mongodb::Database, name: &str) {
    let now = bson::DateTime::now();
    let doc = bson::doc! {
        "_id": name,
        "files": [{ "path": "departments/x/main.lua", "content": "return {}" }],
        "composed_deps": [],
        "created_at": now,
        "updated_at": now,
    };
    database
        .collection::<bson::Document>(PACKAGES_COLLECTION)
        .insert_one(doc)
        .await
        .expect("insert legacy package");
}

/// Mount user-orgs mocks for specific users. Each token gets a mock that
/// returns org_alpha. The default empty-orgs mock from `mount_nyxid_mocks`
/// serves as fallback for all other users. Wiremock processes mocks in
/// reverse registration order (last registered wins), so these per-user
/// mocks take precedence over the empty default.
async fn mount_user_in_org(server: &MockServer, tokens: &[&str]) {
    for token in tokens {
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs"))
            .and(header("authorization", format!("Bearer {token}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "id": "org_alpha" },
            ])))
            .mount(server)
            .await;
    }
}

// ---- Test cases ----

#[tokio::test]
async fn owner_sees_and_manages_their_package() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = authz_app_with_db().await;
    let token_a = token_for("user_A");

    // Create package as user A.
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("my-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create body: {body}");
    assert_eq!(body["name"], "my-pkg");

    // GET as user A -> 200.
    let (status, body) = get_with_auth(&app.router, "/api/v1/packages/my-pkg", &token_a).await;
    assert_eq!(status, StatusCode::OK, "get body: {body}");
    assert_eq!(body["name"], "my-pkg");
    assert_eq!(body["owner_user_id"], "user_A");

    // List as user A -> contains the package.
    let (status, body) = get_with_auth(&app.router, "/api/v1/packages", &token_a).await;
    assert_eq!(status, StatusCode::OK, "list body: {body}");
    let names = body.as_array().expect("array");
    assert!(
        names.iter().any(|n| n == "my-pkg"),
        "list must contain my-pkg, got: {body}"
    );
}

#[tokio::test]
async fn stranger_gets_404_anti_enumeration() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = authz_app_with_db().await;
    let token_a = token_for("user_A");
    let token_b = token_for("user_B");

    // Create package as user A.
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("secret-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // GET as user B (stranger) -> 404 (anti-enumeration: same as truly absent).
    let (status, body) = get_with_auth(&app.router, "/api/v1/packages/secret-pkg", &token_b).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "stranger should get 404, body: {body}"
    );
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn org_viewer_reads_but_cannot_create_session() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = authz_app_with_db().await;
    let token_c = token_for("user_C"); // viewer in org_alpha
    let token_d = token_for("user_D"); // member in org_alpha

    // Mount per-user org mocks so user_C and user_D are seen as org members.
    mount_user_in_org(&app.mock_server, &[&token_c, &token_d]).await;

    // user_D is a member of org_alpha, so they can create into org_alpha.
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_d,
        &valid_package_body_with_org("org-pkg", "org_alpha"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create org pkg body: {body}");

    // Viewer (user_C) can GET -> 200.
    let (status, body) = get_with_auth(&app.router, "/api/v1/packages/org-pkg", &token_c).await;
    assert_eq!(status, StatusCode::OK, "viewer get body: {body}");
    assert_eq!(body["name"], "org-pkg");

    // Viewer (user_C) cannot POST session -> 403.
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/sessions",
        &token_c,
        &session_body("org-pkg"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer session create body: {body}"
    );
    assert_eq!(body["error"], "forbidden");
}

#[tokio::test]
async fn org_member_creates_and_stops_sessions() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = authz_app_with_db().await;
    let token_d = token_for("user_D"); // member in org_alpha

    // Mount per-user org mock so user_D is seen as org member.
    mount_user_in_org(&app.mock_server, &[&token_d]).await;

    // Create package in org_alpha as user_D.
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_d,
        &valid_package_body_with_org("session-pkg", "org_alpha"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create body: {body}");

    // Member POST session -> 201 (engine will fail to start but document is created).
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/sessions",
        &token_d,
        &session_body("session-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "session create body: {body}");
    let session_id = body["id"].as_str().expect("session id");

    // Give the driver a moment to process (it will fail since no engine binary,
    // but the session doc is created).
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Member POST stop -> 202.
    let (status, body) = post_empty_with_auth(
        &app.router,
        &format!("/api/v1/sessions/{session_id}/stop"),
        &token_d,
    )
    .await;
    // The stop should succeed with 202 regardless of engine state.
    assert_eq!(status, StatusCode::ACCEPTED, "session stop body: {body}");
}

#[tokio::test]
async fn create_into_org_requires_membership() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = authz_app_with_db().await;
    let token_b = token_for("user_B"); // NOT in org_alpha

    // user_B tries to create a package in org_alpha -> 403.
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_b,
        &valid_package_body_with_org("stolen-pkg", "org_alpha"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-member org create body: {body}"
    );
    assert_eq!(body["error"], "forbidden");
}

#[tokio::test]
async fn legacy_package_remains_usable() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = authz_app_with_db().await;
    let token_x = token_for("user_X"); // any authenticated user

    // Insert a legacy package directly into Mongo (no owner_user_id/org_id).
    insert_legacy_package_db(&app.database, "old-pkg").await;

    // Any authenticated user can GET -> 200.
    let (status, body) = get_with_auth(&app.router, "/api/v1/packages/old-pkg", &token_x).await;
    assert_eq!(status, StatusCode::OK, "legacy get body: {body}");
    assert_eq!(body["name"], "old-pkg");
    assert!(body["owner_user_id"].is_null(), "legacy has no owner");
}

#[tokio::test]
async fn list_filters_to_visible_set() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = authz_app_with_db().await;
    let token_a = token_for("user_A");
    let token_b = token_for("user_B");
    let token_d = token_for("user_D"); // member in org_alpha

    // Mount per-user org mock: only user_D is in org_alpha.
    mount_user_in_org(&app.mock_server, &[&token_d]).await;

    // Create package owned by user_A.
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("pkg-a"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Create package in org_alpha as user_D.
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_d,
        &valid_package_body_with_org("pkg-org", "org_alpha"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Insert legacy package.
    insert_legacy_package_db(&app.database, "pkg-legacy").await;

    // user_A list: sees own (pkg-a) + legacy (pkg-legacy) only.
    let (status, body) = get_with_auth(&app.router, "/api/v1/packages", &token_a).await;
    assert_eq!(status, StatusCode::OK, "user_A list body: {body}");
    let names_a: Vec<String> = body
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        names_a.contains(&"pkg-a".to_string()),
        "user_A must see own pkg-a, got: {names_a:?}"
    );
    assert!(
        names_a.contains(&"pkg-legacy".to_string()),
        "user_A must see legacy pkg-legacy, got: {names_a:?}"
    );
    assert!(
        !names_a.contains(&"pkg-org".to_string()),
        "user_A must NOT see org pkg-org, got: {names_a:?}"
    );

    // user_B list: sees only legacy.
    let (status, body) = get_with_auth(&app.router, "/api/v1/packages", &token_b).await;
    assert_eq!(status, StatusCode::OK, "user_B list body: {body}");
    let names_b: Vec<String> = body
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names_b,
        vec!["pkg-legacy"],
        "user_B must see only legacy, got: {names_b:?}"
    );

    // user_D (org member) list: sees org package + legacy.
    let (status, body) = get_with_auth(&app.router, "/api/v1/packages", &token_d).await;
    assert_eq!(status, StatusCode::OK, "user_D list body: {body}");
    let names_d: Vec<String> = body
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        names_d.contains(&"pkg-org".to_string()),
        "user_D must see org pkg-org, got: {names_d:?}"
    );
    assert!(
        names_d.contains(&"pkg-legacy".to_string()),
        "user_D must see legacy pkg-legacy, got: {names_d:?}"
    );
}
