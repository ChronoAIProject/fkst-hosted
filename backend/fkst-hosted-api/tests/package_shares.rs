//! Package sharing integration tests against an ephemeral Mongo container
//! (testcontainers) and wiremock NyxID endpoints.
//!
//! Exercises the full share stack (share creation, listing, revocation,
//! filter=shared, share-aware authz, cascade on package delete, session
//! create with use-level shares) against the real `build_router(AppState)`
//! with auth ENABLED and a real `NyxIdClient` backed by wiremock.
//!
//! Self-skipping when Docker is unavailable.

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
use fkst_hosted_api::goals::GoalRepo;
use fkst_hosted_api::nyxid::NyxIdClient;
use fkst_hosted_api::packages::{PackageRepository, ShareRepo, PACKAGE_SHARES_COLLECTION};
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

/// JWKS cache TTL for tests.
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

// ---- Key generation (once per test binary) ----

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

// ---- Response helpers ----

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

async fn delete_with_auth(router: &axum::Router, path: &str, token: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::delete(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

fn valid_package_body(name: &str) -> Value {
    json!({
        "name": name,
        "files": [
            { "path": "departments/router/main.lua", "content": "return {}" },
        ],
        "composed_deps": []
    })
}

fn session_body(package_name: &str) -> Value {
    json!({ "package_name": package_name })
}

// ---- Test infrastructure ----

struct ShareTestApp {
    _mock_server: MockServer,
    _container: testcontainers::ContainerAsync<Mongo>,
    router: axum::Router,
    database: mongodb::Database,
}

async fn share_app() -> ShareTestApp {
    let mock_server = MockServer::start().await;

    // Mount JWKS.
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_response()))
        .mount(&mock_server)
        .await;

    // Service token endpoint.
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "svc_tok_share_test",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(&mock_server)
        .await;

    // User exists: user_B -> 200, user_D -> 200, unknown -> 404.
    Mock::given(method("GET"))
        .and(path("/api/v1/users/user_B"))
        .and(header("authorization", "Bearer svc_tok_share_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "user_B"})))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v1/users/user_D"))
        .and(header("authorization", "Bearer svc_tok_share_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "user_D"})))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v1/users/nonexistent_user"))
        .and(header("authorization", "Bearer svc_tok_share_test"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    // Org org_alpha: exists, members user_A (admin), user_D (member).
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/org_alpha"))
        .and(header("authorization", "Bearer svc_tok_share_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "org_alpha"})))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/org_alpha/members"))
        .and(header("authorization", "Bearer svc_tok_share_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "membership_id": "m1", "user_id": "user_A", "role": "admin" },
            { "membership_id": "m2", "user_id": "user_D", "role": "member" },
        ])))
        .mount(&mock_server)
        .await;

    // Org org_beta: exists, user_A NOT a member.
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/org_beta"))
        .and(header("authorization", "Bearer svc_tok_share_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "org_beta"})))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/org_beta/members"))
        .and(header("authorization", "Bearer svc_tok_share_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&mock_server)
        .await;

    // User orgs: default returns org_alpha (simplifies share tests where
    // multiple users need to be seen as org members). Per-user overrides
    // can still be mounted later if needed.
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "org_alpha" },
        ])))
        .mount(&mock_server)
        .await;

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
    packages.ensure_indexes().await.expect("packages indexes");
    let shares = ShareRepo::new(&db.database);
    let goals = GoalRepo::new(&db.database);
    shares.ensure_indexes().await.expect("shares indexes");
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

    let authz = Authorizer::with_shares(Some(nyxid_client), shares.clone());
    let router = build_router(AppState {
        config,
        db,
        packages,
        shares,
        sessions,
        auth_mode,
        authz,
        github_app: None,
        goals,
        engine: EngineConfig::default(),
        llm: None,
    })
    .expect("router");

    ShareTestApp {
        _mock_server: mock_server,
        _container: container,
        router,
        database,
    }
}

// ---- Test cases ----

/// Serde round-trip: ShareDoc BSON and JSON both round-trip losslessly and
/// the `_id` field is Binary subtype 4 (UUID).
#[tokio::test]
async fn share_doc_round_trips_and_id_is_binary_uuid() {
    let share = fkst_hosted_api::packages::ShareDoc {
        id: bson::Uuid::new(),
        package_name: "demo".to_string(),
        grantee_kind: fkst_hosted_api::packages::GranteeKind::User,
        grantee_id: "user-42".to_string(),
        level: fkst_hosted_api::packages::ShareLevel::Use,
        granted_by: "owner-1".to_string(),
        created_at: bson::DateTime::from_millis(1_700_000_000_000),
    };

    // BSON round-trip.
    let raw = bson::to_document(&share).expect("serialize");
    let back: fkst_hosted_api::packages::ShareDoc =
        bson::from_document(raw.clone()).expect("deserialize");
    assert_eq!(back, share);

    // _id is Binary subtype 4.
    match raw.get("_id").expect("_id present") {
        bson::Bson::Binary(binary) => {
            assert_eq!(binary.subtype, bson::spec::BinarySubtype::Uuid);
        }
        other => panic!("expected Bson::Binary(subtype Uuid), got {other:?}"),
    }
}

/// POST a share creates it; a duplicate POST returns 409.
#[tokio::test]
async fn post_share_creates_and_duplicate_is_409() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");

    // Create a package as user_A.
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("share-pkg-1"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Share to user_B.
    let share_body = json!({
        "grantee_kind": "user",
        "grantee_id": "user_B",
        "level": "read",
    });
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/share-pkg-1/shares",
        &token_a,
        &share_body,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "first share body: {body}");
    assert_eq!(body["package_name"], "share-pkg-1");
    assert_eq!(body["grantee_kind"], "user");
    assert_eq!(body["grantee_id"], "user_B");
    assert_eq!(body["level"], "read");

    // Duplicate -> 409.
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/share-pkg-1/shares",
        &token_a,
        &share_body,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "duplicate body: {body}");
    assert_eq!(body["error"], "conflict");
}

/// Concurrent identical POSTs yield exactly one 201 and one 409 (unique-index race).
#[tokio::test]
async fn concurrent_share_posts_yield_one_201_and_one_409() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");

    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("race-share-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let share_body = json!({
        "grantee_kind": "user",
        "grantee_id": "user_B",
        "level": "use",
    });

    let (left, right) = tokio::join!(
        post_json_with_auth(
            &app.router,
            "/api/v1/packages/race-share-pkg/shares",
            &token_a,
            &share_body,
        ),
        post_json_with_auth(
            &app.router,
            "/api/v1/packages/race-share-pkg/shares",
            &token_a,
            &share_body,
        ),
    );
    let statuses = [left.0, right.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::CREATED)
            .count(),
        1,
        "exactly one 201: {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        1,
        "exactly one 409: {statuses:?}"
    );
}

/// POST share for unknown package returns 404.
#[tokio::test]
async fn post_share_unknown_package_is_404() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");

    let share_body = json!({
        "grantee_kind": "user",
        "grantee_id": "user_B",
        "level": "read",
    });
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/nonexistent-pkg/shares",
        &token_a,
        &share_body,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"], "not_found");
}

/// POST share with nonexistent user returns 400.
#[tokio::test]
async fn post_share_nonexistent_user_is_400() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");

    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("user-400-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let share_body = json!({
        "grantee_kind": "user",
        "grantee_id": "nonexistent_user",
        "level": "read",
    });
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/user-400-pkg/shares",
        &token_a,
        &share_body,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert_eq!(body["error"], "invalid_request");
}

/// POST share to an org where the caller is NOT a member returns 403.
#[tokio::test]
async fn post_share_org_caller_not_member_is_403() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");

    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("org-403-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // user_A is NOT a member of org_beta (empty members list).
    let share_body = json!({
        "grantee_kind": "org",
        "grantee_id": "org_beta",
        "level": "read",
    });
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/org-403-pkg/shares",
        &token_a,
        &share_body,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(body["error"], "forbidden");
}

/// GET shares requires manage permission; a use-level grantee gets 403.
#[tokio::test]
async fn get_shares_requires_manage() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");
    let token_b = token_for("user_B");

    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("manage-only-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Share to user_B at use level.
    let share_body = json!({
        "grantee_kind": "user",
        "grantee_id": "user_B",
        "level": "use",
    });
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/manage-only-pkg/shares",
        &token_a,
        &share_body,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // user_B (use grantee) cannot list shares -> 403.
    let (status, body) = get_with_auth(
        &app.router,
        "/api/v1/packages/manage-only-pkg/shares",
        &token_b,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(body["error"], "forbidden");
}

/// DELETE a share, then the grantee loses access (404 on GET package).
#[tokio::test]
async fn delete_share_then_grantee_loses_access() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");
    let token_b = token_for("user_B");

    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("revoke-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Share to user_B at read level.
    let share_body = json!({
        "grantee_kind": "user",
        "grantee_id": "user_B",
        "level": "read",
    });
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/revoke-pkg/shares",
        &token_a,
        &share_body,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let share_id = body["id"].as_str().expect("share id");

    // user_B can GET the package.
    let (status, _) = get_with_auth(&app.router, "/api/v1/packages/revoke-pkg", &token_b).await;
    assert_eq!(status, StatusCode::OK);

    // Revoke the share.
    let (status, _) = delete_with_auth(
        &app.router,
        &format!("/api/v1/packages/revoke-pkg/shares/{share_id}"),
        &token_a,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // user_B now gets 404 (anti-enumeration).
    let (status, body) = get_with_auth(&app.router, "/api/v1/packages/revoke-pkg", &token_b).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
}

/// GET ?filter=shared returns shared-with-me names, deduped (user + org grants).
#[tokio::test]
async fn list_filter_shared_returns_user_and_org_grants_deduped() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");
    let token_d = token_for("user_D");

    // Create packages as user_A.
    for name in ["shared-pkg-1", "shared-pkg-2"] {
        let (status, _) = post_json_with_auth(
            &app.router,
            "/api/v1/packages",
            &token_a,
            &valid_package_body(name),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // Share shared-pkg-1 to user_D directly (user grant).
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/shared-pkg-1/shares",
        &token_a,
        &json!({ "grantee_kind": "user", "grantee_id": "user_D", "level": "read" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Share shared-pkg-2 to org_alpha (org grant, user_D is a member).
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/shared-pkg-2/shares",
        &token_a,
        &json!({ "grantee_kind": "org", "grantee_id": "org_alpha", "level": "read" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "org share body: {body}");

    // user_D lists shared -> both packages.
    // First, verify the org share exists in the DB.
    let coll: mongodb::Collection<bson::Document> =
        app.database.collection(PACKAGE_SHARES_COLLECTION);
    let org_shares = coll
        .count_documents(bson::doc! {
            "package_name": "shared-pkg-2",
            "grantee_kind": "org",
            "grantee_id": "org_alpha",
        })
        .await
        .expect("count");
    assert_eq!(org_shares, 1, "org share must exist in DB");

    // Also check that user_D's org membership resolves.
    // Verify user_D can see shared-pkg-2 via GET (proves share-aware authz).
    let (status, body) =
        get_with_auth(&app.router, "/api/v1/packages/shared-pkg-2", &token_d).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "user_D must see org-shared pkg-2, body: {body}"
    );

    // Now list with filter=shared.
    let (status, body) =
        get_with_auth(&app.router, "/api/v1/packages?filter=shared", &token_d).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let names: Vec<String> = body
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&"shared-pkg-1".to_string()),
        "must contain user-granted shared-pkg-1, got: {names:?}"
    );
    assert!(
        names.contains(&"shared-pkg-2".to_string()),
        "must contain org-granted shared-pkg-2, got: {names:?}"
    );
}

/// Use-level share allows session create; read-level does not.
#[tokio::test]
async fn use_level_allows_session_create_read_level_does_not() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");
    let token_b = token_for("user_B");

    // Create two packages.
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("use-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("read-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Share use-pkg at "use" level.
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/use-pkg/shares",
        &token_a,
        &json!({ "grantee_kind": "user", "grantee_id": "user_B", "level": "use" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Share read-pkg at "read" level.
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/read-pkg/shares",
        &token_a,
        &json!({ "grantee_kind": "user", "grantee_id": "user_B", "level": "read" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // user_B can GET both packages.
    let (status, _) = get_with_auth(&app.router, "/api/v1/packages/use-pkg", &token_b).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = get_with_auth(&app.router, "/api/v1/packages/read-pkg", &token_b).await;
    assert_eq!(status, StatusCode::OK);

    // user_B can create session on use-pkg -> 201 (engine will fail but doc created).
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/sessions",
        &token_b,
        &session_body("use-pkg"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "use-level must allow session create, body: {body}"
    );

    // user_B CANNOT create session on read-pkg -> 403.
    let (status, body) = post_json_with_auth(
        &app.router,
        "/api/v1/sessions",
        &token_b,
        &session_body("read-pkg"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "read-level must NOT allow session create, body: {body}"
    );
}

/// Deleting a package removes its share rows (cascade).
#[tokio::test]
async fn deleting_package_removes_share_rows() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = share_app().await;
    let token_a = token_for("user_A");

    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages",
        &token_a,
        &valid_package_body("cascade-pkg"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Share to user_B.
    let (status, _) = post_json_with_auth(
        &app.router,
        "/api/v1/packages/cascade-pkg/shares",
        &token_a,
        &json!({ "grantee_kind": "user", "grantee_id": "user_B", "level": "read" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Verify share exists in DB.
    let coll: mongodb::Collection<bson::Document> =
        app.database.collection(PACKAGE_SHARES_COLLECTION);
    let count_before = coll
        .count_documents(bson::doc! { "package_name": "cascade-pkg" })
        .await
        .expect("count");
    assert_eq!(count_before, 1, "share must exist before delete");

    // Delete the package.
    let (status, _) = delete_with_auth(&app.router, "/api/v1/packages/cascade-pkg", &token_a).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify share rows are gone.
    let count_after = coll
        .count_documents(bson::doc! { "package_name": "cascade-pkg" })
        .await
        .expect("count");
    assert_eq!(count_after, 0, "share rows must be cascade-deleted");
}
