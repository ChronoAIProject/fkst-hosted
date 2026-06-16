//! Vault integration tests (database-free, #138).
//!
//! The persistent vault CRUD (`/api/v1/vault/entries`) was removed: secrets are
//! supplied inline at goal trigger and held by the controller in memory only.
//! These tests assert (1) the in-memory `VaultService` resolves inline secrets
//! through the `list_for_scope` contract the session driver consumes (global +
//! repo overlay, repo wins, key-sorted), and (2) the removed HTTP CRUD route is
//! gone (404) through the real `build_router(AppState)`.
//!
//! No Mongo is needed: the router builds over a lazily-constructed `Db` (parsed
//! but never connected) and the vault has no datastore — so these run on any
//! host, with or without Docker.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::auth::AuthMode;
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::db::Db;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use fkst_control_plane::vault::{EnvKind, EnvScopeRef, VaultLimits, VaultService};
use secrecy::{ExposeSecret, SecretString};
use tower::ServiceExt;

/// A lazy `Db`: parses the URI and builds the client but never connects, so a
/// router can be assembled for a routing assertion with no Mongo running.
async fn lazy_db() -> Db {
    Db::from_config(&Config::default()).await.expect("lazy db")
}

/// One inline secret tuple for `set_inline`.
fn secret(key: &str, value: &str) -> (String, EnvKind, SecretString) {
    (
        key.to_string(),
        EnvKind::Secret,
        SecretString::from(value.to_string()),
    )
}

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

#[tokio::test]
async fn inline_secrets_resolve_global_then_repo_overlay() {
    let vault = VaultService::new(VaultLimits::default());
    let repo = EnvScopeRef::repo("acme", "site");
    vault
        .set_inline(
            "u1",
            &EnvScopeRef::global(),
            vec![secret("A_KEY", "ga"), secret("B_KEY", "gb")],
        )
        .expect("global");
    vault
        .set_inline(
            "u1",
            &repo,
            vec![secret("B_KEY", "rb"), secret("C_KEY", "rc")],
        )
        .expect("repo");

    let resolved = vault
        .list_for_scope("u1", None, &repo)
        .await
        .expect("resolve");
    let kv: Vec<(String, String)> = resolved
        .iter()
        .map(|e| (e.key.clone(), e.value.expose_secret().to_string()))
        .collect();
    // Key-sorted; the repo overlay wins B_KEY.
    assert_eq!(
        kv,
        vec![
            ("A_KEY".to_string(), "ga".to_string()),
            ("B_KEY".to_string(), "rb".to_string()),
            ("C_KEY".to_string(), "rc".to_string()),
        ]
    );
}

#[tokio::test]
async fn clear_inline_drops_scope_after_teardown() {
    let vault = VaultService::new(VaultLimits::default());
    let repo = EnvScopeRef::repo("acme", "site");
    vault
        .set_inline("u1", &repo, vec![secret("K", "v")])
        .expect("set");
    assert_eq!(
        vault.list_for_scope("u1", None, &repo).await.unwrap().len(),
        1
    );
    vault.clear_inline("u1", &repo);
    assert!(vault
        .list_for_scope("u1", None, &repo)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn removed_vault_crud_route_returns_404() {
    let db = lazy_db().await;
    let app = router(db, VaultService::new(VaultLimits::default()));

    let requests = [
        Request::get("/api/v1/vault/entries")
            .body(Body::empty())
            .unwrap(),
        Request::put("/api/v1/vault/entries")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
        Request::delete("/api/v1/vault/entries/00000000-0000-0000-0000-000000000000")
            .body(Body::empty())
            .unwrap(),
    ];
    for req in requests {
        let resp = app.clone().oneshot(req).await.expect("response");
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "the persistent vault CRUD must be gone (#138)"
        );
    }
}
