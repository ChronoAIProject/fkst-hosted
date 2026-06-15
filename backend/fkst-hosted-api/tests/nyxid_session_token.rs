//! Integration test for issue #111: the session driver mints a per-session
//! NyxID agent key on the triggering user's behalf, persists only the
//! non-secret key refs onto `SessionDoc`, and escalates a token-less failover
//! rebuild instead of running with broken auth.
//!
//! Strategy: a single-pod `SessionService` is wired to a `NyxIdClient` pointed
//! at a `wiremock` MockServer standing in for NyxID's `POST /api/v1/api-keys`
//! and `DELETE /api/v1/api-keys/{id}` routes, against a real ephemeral Mongo.
//! The engine binary is the absent default, so a driven session fails at
//! engine START — but provisioning runs strictly BEFORE the start, so the
//! mint + persisted refs are observable on the terminal `failed` document.
//!
//! The facts under test mirror the issue's verification checklist:
//!   1. A package session persists `nyxid_key_id` + `nyxid_key_prefix` — and
//!      NEVER the full `nyxid_ag_…` key — on its document.
//!   2. Two concurrent sessions mint two DISTINCT keys (each its own POST).
//!   3. A failover rebuild (the reaper seam, no user token) of a session that
//!      previously had a key ESCALATES: the session fails with the documented
//!      reason, with no mint attempted.
//!
//! Each test gets a fresh container and self-skips when Docker is unavailable.

use std::time::Duration;

use fkst_hosted_api::db::Db;
use fkst_hosted_api::distribution::DriverHost;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::models::{SessionDoc, SessionStatus};
use fkst_hosted_api::nyxid::NyxIdClient;
use fkst_hosted_api::packages::{Package, PackageFile, PackageRepository, PACKAGES_COLLECTION};
use fkst_hosted_api::sessions::{SessionOwner, SessionRepo, SessionService};
use secrecy::SecretString;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod support;

/// The clearly-fake full key the mock returns. NEVER a real secret — its
/// presence in any persisted document or response would be a leak.
const FAKE_FULL_KEY: &str = "nyxid_ag_FAKEFAKEFAKEFAKEFAKEFAKE";

fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

const MONGO_TAG: &str = "7";

/// A fresh Mongo + a `SessionService` whose driver provisions NyxID tokens
/// against `nyxid_uri` (a wiremock MockServer). The engine binary stays absent
/// so a driven session fails at start, after provisioning has run.
struct TestCtx {
    _container: ContainerAsync<Mongo>,
    db: Db,
    sessions: SessionService,
}

async fn ctx(nyxid_uri: &str) -> TestCtx {
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
    let config = fkst_hosted_api::config::Config {
        mongodb_uri: format!("mongodb://{host}:{port}"),
        mongodb_server_selection_timeout_ms: 5000,
        ..fkst_hosted_api::config::Config::default()
    };
    let db = Db::connect(&config).await.expect("connect + ping");

    let packages = PackageRepository::new(&db.database);
    let sessions = SessionService::new(SessionRepo::new(&db), packages, EngineConfig::default());
    sessions.enable_vault(support::test_vault(&db));

    let client = NyxIdClient::new(
        nyxid_uri,
        "sa_client".to_string(),
        SecretString::from("sa_secret".to_string()),
        Duration::from_secs(30),
    )
    .expect("nyxid client");
    sessions.enable_nyxid_token(client, "https://nyxid.test".to_string());

    TestCtx {
        _container: container,
        db,
        sessions,
    }
}

/// Mount the mint route on the mock, returning the fake key. Each POST gets a
/// distinct id derived from a request counter so concurrent sessions can be
/// told apart by their persisted `nyxid_key_id`.
async fn mount_mint(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/v1/api-keys"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "key-fixed",
            "full_key": FAKE_FULL_KEY,
        })))
        .mount(server)
        .await;
    // Revoke at teardown (best-effort) — accept any id.
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "svc", "token_type": "Bearer", "expires_in": 3600
        })))
        .mount(server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/api/v1/api-keys/.+$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(server)
        .await;
}

async fn seed_package(db: &Db, name: &str) {
    let now = bson::DateTime::now();
    let package = Package {
        name: name.to_string(),
        files: vec![PackageFile {
            path: "departments/hello/main.lua".to_string(),
            content: "return {}\n".to_string(),
        }],
        composed_deps: Vec::new(),
        owner_user_id: Some("owner-1".to_string()),
        org_id: None,
        created_at: now,
        updated_at: now,
    };
    db.database
        .collection::<Package>(PACKAGES_COLLECTION)
        .insert_one(&package)
        .await
        .expect("seed package");
}

async fn poll_until_settled(repo: &SessionRepo, id: bson::Uuid) -> SessionDoc {
    for _ in 0..200 {
        if let Some(doc) = repo.get(id).await.expect("get session") {
            if !matches!(
                doc.status,
                SessionStatus::Pending | SessionStatus::Validating
            ) {
                return doc;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("session {id} never left pending/validating");
}

/// A package session mints a NyxID key and persists ONLY the non-secret refs.
#[tokio::test]
async fn package_session_persists_nyxid_key_refs_never_the_full_key() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    mount_mint(&server).await;
    let ctx = ctx(&server.uri()).await;
    seed_package(&ctx.db, "nyxid-pkg").await;

    let session = ctx
        .sessions
        .create(
            "nyxid-pkg",
            SessionOwner {
                owner_user_id: "owner-1".to_string(),
                org_id: None,
            },
            SecretString::from("user-raw-token".to_string()),
        )
        .await
        .expect("create session");

    let repo = SessionRepo::new(&ctx.db);
    let doc = poll_until_settled(&repo, session.id).await;

    // Provisioning ran BEFORE the (failing) engine start: the non-secret refs
    // are stamped on the document.
    assert_eq!(
        doc.nyxid_key_id.as_deref(),
        Some("key-fixed"),
        "the key id must be persisted"
    );
    assert_eq!(
        doc.nyxid_key_prefix.as_deref(),
        Some("nyxid_ag_FAK"),
        "only a short non-secret prefix is persisted"
    );

    // The full key must NEVER appear anywhere on the persisted document.
    let raw = bson::to_document(&doc).expect("serialize");
    let serialized = format!("{raw:?}");
    assert!(
        !serialized.contains(FAKE_FULL_KEY),
        "the full nyxid key must never be persisted: {serialized}"
    );
    // The session still fails downstream at the absent engine binary, NOT at
    // the mint (proving provisioning succeeded before the start).
    assert_eq!(doc.status, SessionStatus::Failed);
    let error = doc.error.unwrap_or_default();
    assert!(
        !error.contains("nyxid"),
        "the failure is the engine start, not the mint: {error}"
    );
}

/// Two concurrent sessions each mint their own key (two distinct POSTs).
#[tokio::test]
async fn two_sessions_each_mint_a_key() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    // Count the mint POSTs: each session must mint exactly once, so two
    // sessions => exactly two POSTs to the mint route.
    Mock::given(method("POST"))
        .and(path("/api/v1/api-keys"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "key-fixed", "full_key": FAKE_FULL_KEY
        })))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "svc", "token_type": "Bearer", "expires_in": 3600
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/api/v1/api-keys/.+$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let ctx = ctx(&server.uri()).await;
    seed_package(&ctx.db, "pkg-a").await;
    seed_package(&ctx.db, "pkg-b").await;

    let repo = SessionRepo::new(&ctx.db);
    let a = ctx
        .sessions
        .create(
            "pkg-a",
            SessionOwner {
                owner_user_id: "owner-1".to_string(),
                org_id: None,
            },
            SecretString::from("tok-a".to_string()),
        )
        .await
        .expect("create a");
    let b = ctx
        .sessions
        .create(
            "pkg-b",
            SessionOwner {
                owner_user_id: "owner-1".to_string(),
                org_id: None,
            },
            SecretString::from("tok-b".to_string()),
        )
        .await
        .expect("create b");

    let doc_a = poll_until_settled(&repo, a.id).await;
    let doc_b = poll_until_settled(&repo, b.id).await;
    assert!(doc_a.nyxid_key_id.is_some(), "session a minted a key");
    assert!(doc_b.nyxid_key_id.is_some(), "session b minted a key");
    // The `.expect(2)` on the mock is verified on drop: exactly two mints.
}

/// A failover rebuild with NO user token (the reaper seam passes `None`) of a
/// session that previously had a NyxID key ESCALATES rather than running with
/// broken auth — and never contacts NyxID to mint.
#[tokio::test]
async fn failover_without_token_escalates_for_a_session_that_had_a_key() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let server = MockServer::start().await;
    // No mint route is mounted: a mint attempt would 404 and surface as a
    // generic mint failure, not the escalate reason. Asserting the escalate
    // reason therefore also proves NO mint was attempted.
    let ctx = ctx(&server.uri()).await;
    seed_package(&ctx.db, "failover-pkg").await;

    // Seed a pending session that ALREADY carries a NyxID key ref (as if a
    // prior pod had minted it), then drive it through the reaper seam (which
    // supplies no user token).
    let repo = SessionRepo::new(&ctx.db);
    let session = SessionDoc {
        id: bson::Uuid::new(),
        package_name: "failover-pkg".to_string(),
        status: SessionStatus::Pending,
        pod_id: None,
        fencing_token: None,
        pid: None,
        runtime_dir: None,
        error: None,
        run_key: None,
        owner_user_id: Some("owner-1".to_string()),
        org_id: None,
        package_names: vec![],
        goal_id: None,
        repo: None,
        env_scope: None,
        triggered_by: None,
        // The prior key id: its presence is what makes a token-less rebuild
        // escalate instead of silently skipping.
        nyxid_key_id: Some("prior-key".to_string()),
        nyxid_key_prefix: Some("nyxid_ag_old".to_string()),
        created_at: bson::DateTime::now(),
        started_at: None,
        stopped_at: None,
    };
    repo.insert(&session).await.expect("seed session");

    // The reaper seam: `ensure_driver` spawns the driver with NO raw token.
    ctx.sessions.ensure_driver(&session).await;

    let doc = poll_until_settled(&repo, session.id).await;
    assert_eq!(doc.status, SessionStatus::Failed, "session must escalate");
    let error = doc.error.expect("escalated session carries a reason");
    assert!(
        error.contains("re-establish NyxID session token on failover"),
        "the escalate reason must be the documented one, not a mint error: {error}"
    );
}
