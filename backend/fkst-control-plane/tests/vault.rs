//! Vault integration tests against an ephemeral Mongo container
//! (testcontainers). Covers the repo CRUD + unique index, the service-level
//! validation/caps, the encrypt-store-resolve round trip with scope precedence,
//! and the full HTTP CRUD through the real `build_router(AppState)` (redaction,
//! authz 403, reserved-key 422, validation caps). Self-skips when Docker is
//! unavailable so `cargo test` stays green on runners without a Docker daemon.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL;
use base64::Engine as _;
use fkst_control_plane::auth::AuthMode;
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::db::Db;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use fkst_control_plane::vault::{EnvKind, EnvScopeRef, VaultLimits, VaultService, WriteRequest};
use http_body_util::BodyExt;
use secrecy::ExposeSecret;
use tower::ServiceExt;
use zeroize::Zeroizing;

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

const MONGO_TAG: &str = "7";

/// Deterministic 32-byte base64 KEK for the tests. NOT a real secret.
fn test_key() -> String {
    BASE64.encode([5u8; 32])
}

use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;

/// Start an ephemeral Mongo and return a `Db` over it.
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
    (container, db)
}

fn vault_service(db: &Db) -> VaultService {
    VaultService::with_local_key(&db.database, &test_key(), VaultLimits::default()).expect("vault")
}

fn write(owner: &str, scope: EnvScopeRef, key: &str, kind: EnvKind, value: &str) -> WriteRequest {
    WriteRequest {
        owner_user_id: owner.to_string(),
        org_id: None,
        scope,
        key: key.to_string(),
        kind,
        value: Zeroizing::new(value.to_string()),
    }
}

// ---- repo / service tests --------------------------------------------------

#[tokio::test]
async fn ensure_indexes_is_idempotent_and_creates_the_unique_index() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let svc = vault_service(&db);
    svc.repo().ensure_indexes().await.expect("first ensure");
    svc.repo().ensure_indexes().await.expect("second ensure");

    let mut cursor = db
        .vault_entries()
        .list_indexes()
        .await
        .expect("list_indexes");
    let mut names = Vec::new();
    while cursor.advance().await.expect("advance") {
        let model = cursor.deserialize_current().expect("model");
        if let Some(name) = model.options.and_then(|o| o.name) {
            names.push(name);
        }
    }
    assert!(
        names.contains(&"vault_owner_scope_key_unique".to_string()),
        "unique index must exist, got {names:?}"
    );
}

#[tokio::test]
async fn secret_round_trips_encrypt_store_resolve() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let svc = vault_service(&db);
    svc.repo().ensure_indexes().await.expect("indexes");

    let stored = svc
        .upsert(write(
            "u1",
            EnvScopeRef::global(),
            "OPENAI_API_KEY",
            EnvKind::Secret,
            "sk-very-secret",
        ))
        .await
        .expect("upsert secret");
    // Stored doc holds ciphertext, never the plaintext.
    assert!(stored.value_plain.is_none());
    assert!(stored.value_enc.is_some());
    assert_eq!(stored.masked_hint.as_deref(), Some("…cret"));

    // The raw BSON in Mongo must NOT contain the plaintext anywhere.
    let raw = db
        .vault_entries()
        .find_one(bson::doc! { "key": "OPENAI_API_KEY" })
        .await
        .expect("find")
        .expect("present");
    assert!(
        !format!("{raw:?}").contains("sk-very-secret"),
        "plaintext secret found in stored BSON"
    );

    // Resolve decrypts it back to the original.
    let resolved = svc
        .list_for_scope("u1", None, &EnvScopeRef::global())
        .await
        .expect("resolve");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].key, "OPENAI_API_KEY");
    assert_eq!(resolved[0].value.expose_secret(), "sk-very-secret");
}

#[tokio::test]
async fn list_for_scope_repo_overrides_global_one_entry_per_key() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let svc = vault_service(&db);
    svc.repo().ensure_indexes().await.expect("indexes");

    // Global: SHARED + ONLY_GLOBAL.
    svc.upsert(write(
        "u1",
        EnvScopeRef::global(),
        "SHARED",
        EnvKind::Variable,
        "global-value",
    ))
    .await
    .expect("global shared");
    svc.upsert(write(
        "u1",
        EnvScopeRef::global(),
        "ONLY_GLOBAL",
        EnvKind::Variable,
        "g",
    ))
    .await
    .expect("global only");
    // Repo overrides SHARED + adds ONLY_REPO (a secret).
    let repo = EnvScopeRef::repo("acme", "site");
    svc.upsert(write(
        "u1",
        repo.clone(),
        "SHARED",
        EnvKind::Variable,
        "repo-value",
    ))
    .await
    .expect("repo shared");
    svc.upsert(write(
        "u1",
        repo.clone(),
        "ONLY_REPO",
        EnvKind::Secret,
        "repo-secret",
    ))
    .await
    .expect("repo secret");

    let resolved = svc
        .list_for_scope("u1", None, &repo)
        .await
        .expect("resolve");
    let map: std::collections::HashMap<_, _> = resolved
        .iter()
        .map(|e| (e.key.clone(), e.value.expose_secret().to_string()))
        .collect();
    // Exactly one entry per key (no duplicate SHARED).
    assert_eq!(resolved.len(), 3, "got {resolved:?}");
    assert_eq!(map["SHARED"], "repo-value", "repo must override global");
    assert_eq!(map["ONLY_GLOBAL"], "g");
    assert_eq!(map["ONLY_REPO"], "repo-secret");

    // The global scope on its own still sees only the two global entries.
    let global = svc
        .list_for_scope("u1", None, &EnvScopeRef::global())
        .await
        .expect("global resolve");
    assert_eq!(global.len(), 2);
}

#[tokio::test]
async fn upsert_replaces_value_for_same_key() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let svc = vault_service(&db);
    svc.repo().ensure_indexes().await.expect("indexes");

    let first = svc
        .upsert(write(
            "u1",
            EnvScopeRef::global(),
            "K",
            EnvKind::Secret,
            "v1",
        ))
        .await
        .expect("first");
    let second = svc
        .upsert(write(
            "u1",
            EnvScopeRef::global(),
            "K",
            EnvKind::Secret,
            "v2",
        ))
        .await
        .expect("second");
    // Same identity (id unchanged) — an update, not a new doc.
    assert_eq!(first.id, second.id);
    let resolved = svc
        .list_for_scope("u1", None, &EnvScopeRef::global())
        .await
        .expect("resolve");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].value.expose_secret(), "v2");
}

#[tokio::test]
async fn entries_per_scope_cap_is_enforced() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let svc = VaultService::with_local_key(
        &db.database,
        &test_key(),
        VaultLimits {
            value_byte_cap: 64 * 1024,
            entries_per_scope_cap: 2,
        },
    )
    .expect("vault");
    svc.repo().ensure_indexes().await.expect("indexes");

    svc.upsert(write(
        "u1",
        EnvScopeRef::global(),
        "A",
        EnvKind::Variable,
        "1",
    ))
    .await
    .expect("A");
    svc.upsert(write(
        "u1",
        EnvScopeRef::global(),
        "B",
        EnvKind::Variable,
        "1",
    ))
    .await
    .expect("B");
    // Third NEW key trips the cap.
    let err = svc
        .upsert(write(
            "u1",
            EnvScopeRef::global(),
            "C",
            EnvKind::Variable,
            "1",
        ))
        .await
        .expect_err("cap must trip");
    assert!(
        matches!(err, fkst_control_plane::error::AppError::Unprocessable(_)),
        "got {err:?}"
    );
    // Updating an EXISTING key never trips the cap.
    svc.upsert(write(
        "u1",
        EnvScopeRef::global(),
        "A",
        EnvKind::Variable,
        "2",
    ))
    .await
    .expect("update of existing key allowed at cap");
}

// ---- HTTP CRUD tests through the real router -------------------------------

/// Build a router (auth disabled: the dev context owns everything) over `db`.
fn router(db: Db, vault: VaultService) -> axum::Router {
    let goals = GoalIssueStore::new(None);
    let sessions = SessionService::new(SessionRepo::new(&db), EngineConfig::default());
    build_router(AppState {
        config: Config::default(),
        db,
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

/// Build a router with proxy-trusted auth ENABLED, so a request can carry an
/// identity token with a SPECIFIC (non-admin) permission set — needed to test
/// the two-layer model where the action layer passes but the object layer
/// (ownership) denies.
fn router_auth_enabled(db: Db, vault: VaultService) -> axum::Router {
    let goals = GoalIssueStore::new(None);
    let sessions = SessionService::new(SessionRepo::new(&db), EngineConfig::default());
    build_router(AppState {
        config: Config::default(),
        db,
        sessions,
        auth_mode: AuthMode::Enabled(fkst_control_plane::auth::NyxIdAuthSettings {
            base_url: "https://nyxid.example.test".to_string(),
        }),
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret: None,
        goals,
        vault,
        ornn: None,
    })
    .expect("router")
}

/// A proxy-injected identity token granting exactly `permissions`, for the
/// given `sub`. Decode-only; the signature segment is never verified.
fn identity_header(sub: &str, permissions: &[&str]) -> String {
    let header = BASE64_URL.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let payload = serde_json::json!({ "sub": sub, "permissions": permissions });
    let body = BASE64_URL.encode(payload.to_string().as_bytes());
    format!("{header}.{body}.unverified-signature")
}

async fn send(router: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = router.clone().oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json")
    };
    (status, json)
}

fn put(body: serde_json::Value) -> Request<Body> {
    Request::put("/api/v1/vault/entries")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

#[tokio::test]
async fn http_put_secret_then_get_never_returns_value() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let vault = vault_service(&db);
    vault.repo().ensure_indexes().await.expect("indexes");
    let app = router(db, vault);

    let (status, body) = send(
        &app,
        put(serde_json::json!({
            "scope": { "global": true },
            "key": "OPENAI_API_KEY",
            "kind": "secret",
            "value": "sk-leaky-secret"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.get("value").is_none(), "PUT echoed the secret value");
    assert_eq!(body["masked_hint"], "…cret");

    // GET must not include the secret value anywhere.
    let (status, body) = send(
        &app,
        Request::get("/api/v1/vault/entries?scope=global")
            .body(Body::empty())
            .expect("req"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rendered = body.to_string();
    assert!(
        !rendered.contains("sk-leaky-secret"),
        "GET leaked the secret value: {rendered}"
    );
    assert!(body[0].get("value").is_none(), "secret carried a value");
    assert_eq!(body[0]["masked_hint"], "…cret");
}

#[tokio::test]
async fn http_put_variable_then_get_returns_value() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let vault = vault_service(&db);
    vault.repo().ensure_indexes().await.expect("indexes");
    let app = router(db, vault);

    send(
        &app,
        put(serde_json::json!({
            "scope": { "global": true },
            "key": "LOG_LEVEL",
            "kind": "variable",
            "value": "debug"
        })),
    )
    .await;
    let (status, body) = send(
        &app,
        Request::get("/api/v1/vault/entries?scope=global")
            .body(Body::empty())
            .expect("req"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body[0]["value"], "debug", "variable value must be returned");
    assert_eq!(body[0]["kind"], "variable");
}

#[tokio::test]
async fn http_reserved_key_is_422() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let vault = vault_service(&db);
    vault.repo().ensure_indexes().await.expect("indexes");
    let app = router(db, vault);

    for key in ["FKST_FOO", "GITHUB_TOKEN", "PATH"] {
        let (status, _body) = send(
            &app,
            put(serde_json::json!({
                "scope": { "global": true },
                "key": key,
                "kind": "secret",
                "value": "x"
            })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "reserved key {key} must be 422"
        );
    }
}

#[tokio::test]
async fn http_invalid_key_name_is_422() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let vault = vault_service(&db);
    vault.repo().ensure_indexes().await.expect("indexes");
    let app = router(db, vault);

    let (status, _body) = send(
        &app,
        put(serde_json::json!({
            "scope": { "global": true },
            "key": "1-bad-key",
            "kind": "variable",
            "value": "x"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn http_oversized_value_is_422() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let vault = VaultService::with_local_key(
        &db.database,
        &test_key(),
        VaultLimits {
            value_byte_cap: 8,
            entries_per_scope_cap: 100,
        },
    )
    .expect("vault");
    vault.repo().ensure_indexes().await.expect("indexes");
    let app = router(db, vault);

    let (status, _body) = send(
        &app,
        put(serde_json::json!({
            "scope": { "global": true },
            "key": "BIG",
            "kind": "secret",
            "value": "0123456789ABCDEF"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn http_delete_returns_204_then_404() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let vault = vault_service(&db);
    vault.repo().ensure_indexes().await.expect("indexes");
    let app = router(db, vault);

    let (_status, body) = send(
        &app,
        put(serde_json::json!({
            "scope": { "global": true },
            "key": "TO_DELETE",
            "kind": "variable",
            "value": "v"
        })),
    )
    .await;
    let id = body["id"].as_str().expect("id").to_string();

    let response = app
        .clone()
        .oneshot(
            Request::delete(format!("/api/v1/vault/entries/{id}"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // A second delete is a 404.
    let response = app
        .oneshot(
            Request::delete(format!("/api/v1/vault/entries/{id}"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_delete_of_another_owner_entry_is_403() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_c, db) = mongo().await;
    let vault = vault_service(&db);
    vault.repo().ensure_indexes().await.expect("indexes");

    // Seed an entry owned by a DIFFERENT user directly via the service.
    let stored = vault
        .upsert(write(
            "someone-else",
            EnvScopeRef::global(),
            "OTHERS",
            EnvKind::Variable,
            "v",
        ))
        .await
        .expect("seed");
    // Two-layer model: the caller HAS the `fkst:vault:delete` action permission
    // (action layer passes) but is NOT the owner and not an admin, so the
    // object layer (Manage-tier ownership) must forbid. Auth is enabled so a
    // specific, non-admin permission set can be injected.
    let app = router_auth_enabled(db, vault);
    let response = app
        .oneshot(
            Request::delete(format!("/api/v1/vault/entries/{}", stored.id))
                .header(
                    "X-NyxID-Identity-Token",
                    identity_header("not-the-owner", &["fkst:vault:delete"]),
                )
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
