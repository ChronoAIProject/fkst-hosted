//! GitHub App installation lifecycle integration tests against an ephemeral
//! Mongo container (testcontainers, for the Mongo-backed session repo) and the
//! real `build_router(AppState)` for the webhook endpoint.
//!
//! Installation resolution is STATELESS (#141): there is no durable installation
//! store. These tests exercise the externally-observable webhook behavior — the
//! cache-bust hint — over the real router:
//! - An invalid `X-Hub-Signature-256` is rejected `401`.
//! - A signed `installation deleted` (enumerating the affected repo) evicts the
//!   in-memory caches and fails an active session on that repo (no persistence).
//! - A malformed signed body answers `202` (never a 5xx / redelivery storm).
//!
//! Self-skips when Docker is unavailable so `cargo test` stays green on runners
//! without a Docker daemon.

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::auth::AuthMode;
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::db::Db;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::github_app::api::{GithubApi, InstallationToken, InstallationTokenRequest};
use fkst_control_plane::github_app::{GithubAppConfig, GithubAppTokens, InstallationId};
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::models::{RepoRef, SessionStatus};
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use hmac::{Hmac, Mac};
use secrecy::SecretString;
use sha2::Sha256;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;
use tower::ServiceExt;

mod support;

const MONGO_TAG: &str = "7";
const WEBHOOK_SECRET: &str = "whsec_integration_test_secret";

fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn mongo() -> (ContainerAsync<Mongo>, Db) {
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
    db.ensure_indexes().await.expect("ensure indexes");
    (container, db)
}

/// A deterministic test RSA PEM so `GithubAppTokens` can be built. Generated at
/// runtime (NOT a real key).
fn test_pem() -> SecretString {
    use rand::rngs::OsRng;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;
    let key = RsaPrivateKey::new(&mut OsRng, 2048).expect("rsa key");
    SecretString::from(key.to_pkcs8_pem(LineEnding::LF).expect("pem").to_string())
}

fn app_config(webhook: Option<&str>) -> GithubAppConfig {
    GithubAppConfig {
        app_id: 12345,
        private_key_pem: test_pem(),
        app_slug: Some("fkst-hosted".to_string()),
        webhook_secret: webhook.map(SecretString::from),
        api_base: "https://api.github.com".to_string(),
    }
}

/// Fake GitHub transport: counts installation lookups (a cold cache probes once)
/// and always mints a fake token.
#[derive(Debug, Default)]
struct FakeApi {
    installation_calls: AtomicUsize,
}

#[async_trait]
impl GithubApi for FakeApi {
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<InstallationId, fkst_control_plane::github_app::GithubAppError> {
        self.installation_calls.fetch_add(1, Ordering::SeqCst);
        Ok(InstallationId(999))
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, fkst_control_plane::github_app::GithubAppError> {
        Ok(InstallationToken {
            token: SecretString::from(format!("ghs_fake_{}", id.0)),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        })
    }
}

fn webhook_signature(secret: &[u8], body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("key");
    mac.update(body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sha256={hex}")
}

// ---- webhook HTTP tests ----------------------------------------------------

/// Build a full router with the github_app (fake API, stateless — no store) and
/// the webhook secret wired, so the webhook route is mounted and exercises the
/// real cache-bust + session-fail path against the Mongo-backed session repo.
fn router_with_webhook(db: Db) -> (axum::Router, SessionRepo) {
    let session_repo = SessionRepo::new(&db);
    let goals = GoalIssueStore::new(None);
    let sessions = SessionService::new(session_repo.clone(), EngineConfig::default());
    let github_app = GithubAppTokens::with_api(
        &app_config(Some(WEBHOOK_SECRET)),
        Arc::new(FakeApi::default()),
    )
    .expect("github app");
    let vault = support::test_vault(&db);
    let router = build_router(AppState {
        config: Config::default(),
        db,
        sessions,
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: Some(github_app),
        github_app_webhook_secret: Some(SecretString::from(WEBHOOK_SECRET)),
        goals,
        vault,
        ornn: None,
    })
    .expect("router");
    (router, session_repo)
}

async fn post_webhook(
    router: &axum::Router,
    event: &str,
    signature: &str,
    body: &[u8],
) -> StatusCode {
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/github/app/webhook")
        .header("content-type", "application/json")
        .header("x-github-event", event)
        .header("x-hub-signature-256", signature)
        .body(Body::from(body.to_vec()))
        .expect("request");
    router
        .clone()
        .oneshot(req)
        .await
        .expect("response")
        .status()
}

#[tokio::test]
async fn webhook_rejects_invalid_signature_401() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let (router, _) = router_with_webhook(db);

    let body = br#"{"action":"created","installation":{"id":1,"account":{"login":"acme"}},"repositories":[]}"#;
    // A signature computed with the WRONG secret.
    let bad = webhook_signature(b"not-the-secret", body);
    let status = post_webhook(&router, "installation", &bad, body).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "bad signature must be 401"
    );
}

#[tokio::test]
async fn webhook_malformed_body_is_202_not_5xx() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let (router, _) = router_with_webhook(db);

    // A correctly-signed but malformed JSON body: the handler must log + answer
    // 202 (never a 5xx, which would trigger a GitHub redelivery storm).
    let body = br#"{"action":"deleted","installation":"not-an-object"}"#;
    let sig = webhook_signature(WEBHOOK_SECRET.as_bytes(), body);
    let status = post_webhook(&router, "installation", &sig, body).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "a malformed but signed body answers 202"
    );
}

#[tokio::test]
async fn webhook_deleted_evicts_and_fails_active_session_without_persistence() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let (router, session_repo) = router_with_webhook(db.clone());

    // Seed an ACTIVE (running) session targeting acme/site.
    let session_id = bson::Uuid::new();
    let session = fkst_control_plane::models::SessionDoc {
        id: session_id,
        package_name: "demo".to_string(),
        status: SessionStatus::Running,
        pod_id: None,
        fencing_token: None,
        pid: None,
        runtime_dir: None,
        error: None,
        run_key: None,
        owner_user_id: Some("u1".to_string()),
        org_id: None,
        package_names: vec![],
        goal_id: Some(bson::Uuid::new()),
        repo: Some(RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        }),
        env_scope: None,
        triggered_by: Some("goal-trigger".to_string()),
        nyxid_key_id: None,
        nyxid_key_prefix: None,
        ornn_skills: None,
        terminal_cause: None,
        created_at: bson::DateTime::now(),
        started_at: Some(bson::DateTime::now()),
        stopped_at: None,
    };
    session_repo.insert(&session).await.expect("insert session");

    // Deliver a signed `installation deleted` that ENUMERATES the affected repo
    // (the per-repo cache-bust path). No durable record exists to read.
    let body = br#"{
        "action":"deleted",
        "installation":{"id":900,"account":{"login":"acme"}},
        "repositories":[{"full_name":"acme/site"}]
    }"#;
    let sig = webhook_signature(WEBHOOK_SECRET.as_bytes(), body);
    let status = post_webhook(&router, "installation", &sig, body).await;
    assert_eq!(status, StatusCode::OK);

    // The active session was transitioned to Failed with a clear reason naming
    // the repo and the uninstall.
    let after = session_repo
        .get(session_id)
        .await
        .expect("get session")
        .expect("session present");
    assert_eq!(after.status, SessionStatus::Failed);
    let err = after.error.expect("error reason");
    assert!(
        err.contains("acme/site") && err.to_lowercase().contains("uninstall"),
        "failure reason must name the repo and the uninstall: {err}"
    );
}
