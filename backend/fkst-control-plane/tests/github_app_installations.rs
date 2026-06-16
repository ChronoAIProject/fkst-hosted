//! GitHub App installation lifecycle integration tests (issue #108) against an
//! ephemeral Mongo container (testcontainers) and the real
//! `build_router(AppState)` for the webhook endpoint.
//!
//! Coverage:
//! - `MongoInstallationStore` persistence: upsert / lookup / set_repos /
//!   set_suspended / delete, including an ORGANIZATION-keyed record.
//! - Resolve-from-persistence: `GithubAppTokens` reads the store BEFORE the
//!   GitHub API (the fake API counts its calls and asserts zero on a store hit).
//! - State survives a "restart": a record written by one store handle is read
//!   by a fresh handle over the same DB.
//! - Webhook HTTP: an invalid `X-Hub-Signature-256` is rejected `401`; a signed
//!   `installation created` (org) persists an org-keyed record; a signed
//!   `deleted` evicts the record AND fails an active session on the repo.
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
use fkst_control_plane::github_app::{
    GithubAppConfig, GithubAppTokens, InstallationId, MongoInstallationStore,
};
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::models::{
    AccountType, GithubInstallationDoc, RepoRef, RepositorySelection, SessionStatus,
};
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

/// Fake GitHub transport: counts installation lookups (must stay 0 on a store
/// hit) and always mints a fake token.
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

fn org_doc(id: i64, login: &str, repo_full: &str) -> GithubInstallationDoc {
    GithubInstallationDoc {
        installation_id: id,
        account_login: login.to_lowercase(),
        account_type: AccountType::Organization,
        repository_selection: RepositorySelection::Selected,
        repos: vec![repo_full.to_lowercase()],
        suspended: false,
        updated_at: bson::DateTime::now(),
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

// ---- persistence-only tests ------------------------------------------------

#[tokio::test]
async fn store_upsert_lookup_and_org_keyed_record() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let store = MongoInstallationStore::new(&db);

    // Upsert an ORG-owned selected installation.
    let doc = org_doc(42, "Acme", "Acme/Site");
    store.upsert(&doc).await.expect("upsert");

    // The stored record is org-keyed (account_type Organization, login lower).
    let fetched = store.get(42).await.expect("get").expect("present");
    assert_eq!(fetched.account_type, AccountType::Organization);
    assert_eq!(fetched.account_login, "acme");
    assert_eq!(fetched.repos, vec!["acme/site".to_string()]);

    // Lookup is case-insensitive on owner/name.
    let found = store
        .lookup_repo("ACME", "SITE")
        .await
        .expect("lookup")
        .expect("covered");
    assert_eq!(found.installation_id, 42);

    // An unrelated repo is not covered.
    assert!(store
        .lookup_repo("acme", "other")
        .await
        .expect("lookup")
        .is_none());
}

#[tokio::test]
async fn store_set_repos_set_suspended_and_delete() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let store = MongoInstallationStore::new(&db);
    store
        .upsert(&org_doc(7, "acme", "acme/a"))
        .await
        .expect("upsert");

    // Add a repo via set_repos (the installation_repositories `added` path).
    store
        .set_repos(7, &["acme/a".to_string(), "acme/b".to_string()])
        .await
        .expect("set_repos");
    assert!(store
        .lookup_repo("acme", "b")
        .await
        .expect("lookup")
        .is_some());

    // Suspending hides coverage (a suspended install cannot mint).
    assert!(store.set_suspended(7, true).await.expect("suspend"));
    assert!(
        store
            .lookup_repo("acme", "a")
            .await
            .expect("lookup")
            .is_none(),
        "suspended install resolves to nothing"
    );

    // Delete is idempotent.
    assert_eq!(store.delete(7).await.expect("delete"), 1);
    assert_eq!(store.delete(7).await.expect("re-delete"), 0);
}

#[tokio::test]
async fn resolve_reads_persistence_before_api_and_survives_restart() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    // Seed a persisted installation as if a prior pod / webhook recorded it.
    let store = MongoInstallationStore::new(&db);
    store
        .upsert(&org_doc(555, "acme", "acme/site"))
        .await
        .expect("seed");

    // Build a FRESH token service over the SAME db (simulates a pod restart:
    // empty in-memory caches, durable record intact).
    let api = Arc::new(FakeApi::default());
    let svc = GithubAppTokens::with_api_and_store(
        &app_config(None),
        api.clone(),
        Some(Arc::new(MongoInstallationStore::new(&db))),
    )
    .expect("svc");

    let _ = svc.token_for_repo("acme/site", None).await.expect("token");

    // The DB was consulted; the GitHub installation API was NOT hit.
    assert_eq!(
        api.installation_calls.load(Ordering::SeqCst),
        0,
        "resolution must read persistence before the GitHub API"
    );
}

// ---- webhook HTTP tests ----------------------------------------------------

/// Build a full router with the github_app (fake API + Mongo store) and the
/// webhook secret wired, so the webhook route is mounted and exercises the real
/// persistence + eviction + session-fail path.
fn router_with_webhook(db: Db) -> (axum::Router, SessionRepo) {
    let session_repo = SessionRepo::new(&db);
    let goals = GoalIssueStore::new(None);
    let sessions = SessionService::new(session_repo.clone(), EngineConfig::default());
    let github_app = GithubAppTokens::with_api_and_store(
        &app_config(Some(WEBHOOK_SECRET)),
        Arc::new(FakeApi::default()),
        Some(Arc::new(MongoInstallationStore::new(&db))),
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

    let body = br#"{"action":"created","installation":{"id":1,"account":{"login":"acme","type":"Organization"},"repository_selection":"selected"},"repositories":[]}"#;
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
async fn webhook_installation_created_org_persists_org_keyed_record() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let store = MongoInstallationStore::new(&db);
    let (router, _) = router_with_webhook(db);

    let body = br#"{
        "action":"created",
        "installation":{"id":314,"account":{"login":"Acme","type":"Organization"},"repository_selection":"selected"},
        "repositories":[{"full_name":"Acme/Site"}]
    }"#;
    let sig = webhook_signature(WEBHOOK_SECRET.as_bytes(), body);
    let status = post_webhook(&router, "installation", &sig, body).await;
    assert_eq!(status, StatusCode::OK);

    // The persisted record is org-keyed.
    let doc = store.get(314).await.expect("get").expect("persisted");
    assert_eq!(doc.account_type, AccountType::Organization);
    assert_eq!(doc.account_login, "acme");
    assert_eq!(doc.repos, vec!["acme/site".to_string()]);
    assert!(!doc.suspended);
}

#[tokio::test]
async fn webhook_deleted_evicts_record_and_fails_active_session() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let store = MongoInstallationStore::new(&db);

    // Seed a persisted installation covering acme/site.
    store
        .upsert(&org_doc(900, "acme", "acme/site"))
        .await
        .expect("seed install");

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
        created_at: bson::DateTime::now(),
        started_at: Some(bson::DateTime::now()),
        stopped_at: None,
    };
    session_repo.insert(&session).await.expect("insert session");

    // Deliver a signed `installation deleted` for installation 900.
    let body = br#"{
        "action":"deleted",
        "installation":{"id":900,"account":{"login":"acme","type":"Organization"},"repository_selection":"selected"}
    }"#;
    let sig = webhook_signature(WEBHOOK_SECRET.as_bytes(), body);
    let status = post_webhook(&router, "installation", &sig, body).await;
    assert_eq!(status, StatusCode::OK);

    // The persisted record is gone.
    assert!(
        store.get(900).await.expect("get").is_none(),
        "record evicted"
    );

    // The active session was transitioned to Failed with a clear reason.
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
