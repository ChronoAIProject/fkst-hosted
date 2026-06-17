//! Handler + scaffold-conformance tests for [`crate::routes::repos`] (#181).
//! Split into its own file (referenced via `#[path]`) so `repos.rs` stays
//! under 500 lines, mirroring the `goals/preflight_tests.rs` convention.
//!
//! The handler core (`run_fkst_setup`) is exercised against a wiremock-backed
//! [`GithubAppTokens`] so the installation probe, idempotency probe, commit,
//! and error mapping are tested without a full `AppState`.

use super::*;
use crate::github_app::config::GithubAppConfig;
use fkst_engine::materialize::{PackageFile, PreparedPackage};
use secrecy::SecretString;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_config(api_base: &str) -> GithubAppConfig {
    use rand::rngs::OsRng;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;
    let mut rng = OsRng;
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("key");
    let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");
    GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(pem.to_string()),
        app_slug: Some("fkst-test".to_string()),
        webhook_secret: None,
        api_base: api_base.to_string(),
    }
}

fn app(server_uri: &str) -> GithubAppTokens {
    GithubAppTokens::new(&test_config(server_uri)).expect("app")
}

/// Mount the installation-resolve + token-mint mocks (`token_for_repo` /
/// `probe_installation` both go through these).
async fn mount_token_mint(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/installation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 7 })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_matcher("/app/installations/7/access_tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "token": "ghs_setup_token",
            "expires_at": "2999-01-01T00:00:00Z"
        })))
        .mount(server)
        .await;
}

/// Mount the full Git Data write happy path for the given default branch.
async fn mount_write(server: &MockServer, branch: &str) {
    let branch_owned = branch.to_string();
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "default_branch": branch_owned }))
        })
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher(format!(
            "/repos/acme/site/git/ref/heads/{branch}"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "object": { "sha": "BASECOMMIT" } })),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/git/commits/BASECOMMIT"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "tree": { "sha": "BASETREE" } })),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_matcher("/repos/acme/site/git/blobs"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "BLOBSHA" })),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_matcher("/repos/acme/site/git/trees"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "NEWTREE" })),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_matcher("/repos/acme/site/git/commits"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "NEWCOMMIT" })),
        )
        .mount(server)
        .await;
    Mock::given(method("PATCH"))
        .and(path_matcher(format!(
            "/repos/acme/site/git/refs/heads/{branch}"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn fresh_repo_returns_201_with_the_three_paths_and_a_commit() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    // `.fkst` absent → 404 on the idempotency probe.
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/contents/.fkst"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    // Default branch is the NON-`main` `trunk` (proves it is read, not assumed).
    mount_write(&server, "trunk").await;

    let (status, body) = run_fkst_setup(&app(&server.uri()), "acme", "site", false)
        .await
        .expect("setup ok");
    assert_eq!(status, StatusCode::CREATED);
    assert!(!body.already_initialized);
    assert_eq!(body.default_branch.as_deref(), Some("trunk"));
    assert_eq!(body.commit_sha.as_deref(), Some("NEWCOMMIT"));
    assert_eq!(
        body.created_paths,
        vec![
            ".fkst/packages/example/departments/example/main.lua".to_string(),
            ".fkst/packages/example/README.md".to_string(),
            ".fkst/AGENTS.md".to_string(),
        ]
    );
}

#[tokio::test]
async fn existing_fkst_without_force_is_a_200_no_op() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    // `.fkst` present → 200 dir listing on the idempotency probe. No write
    // mocks are mounted, so any blob/tree/commit/ref call would 404 the
    // commit and fail the test — proving the no-op path makes none of them.
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/contents/.fkst"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "name": "AGENTS.md", "path": ".fkst/AGENTS.md", "type": "file" }
        ])))
        .mount(&server)
        .await;

    let (status, body) = run_fkst_setup(&app(&server.uri()), "acme", "site", false)
        .await
        .expect("setup ok");
    assert_eq!(status, StatusCode::OK);
    assert!(body.already_initialized);
    assert!(body.created_paths.is_empty());
    assert!(body.commit_sha.is_none());
}

#[tokio::test]
async fn force_over_existing_fkst_recommits_as_200() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/contents/.fkst"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "name": "AGENTS.md", "path": ".fkst/AGENTS.md", "type": "file" }
        ])))
        .mount(&server)
        .await;
    mount_write(&server, "main").await;

    let (status, body) = run_fkst_setup(&app(&server.uri()), "acme", "site", true)
        .await
        .expect("setup ok");
    assert_eq!(status, StatusCode::OK, "a forced re-commit is a 200");
    assert!(!body.already_initialized);
    assert_eq!(body.commit_sha.as_deref(), Some("NEWCOMMIT"));
    assert_eq!(body.created_paths.len(), 3);
}

#[tokio::test]
async fn app_not_installed_is_unprocessable_with_install_hint() {
    let server = MockServer::start().await;
    // The installation lookup 404s → probe → NotInstalled.
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/installation"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = run_fkst_setup(&app(&server.uri()), "acme", "site", false)
        .await
        .expect_err("must fail");
    match err {
        AppError::Unprocessable(msg) => {
            assert!(msg.contains("not installed"), "got {msg}");
            assert!(
                msg.contains("fkst-test"),
                "install URL hint must be present: {msg}"
            );
        }
        other => panic!("expected Unprocessable, got {other:?}"),
    }
}

#[tokio::test]
async fn repo_not_found_on_write_is_404() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    // `.fkst` probe 404 → fresh → proceed to write; the repo GET then 404s.
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/contents/.fkst"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = run_fkst_setup(&app(&server.uri()), "acme", "site", false)
        .await
        .expect_err("must fail");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn ref_conflict_on_write_is_409() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/contents/.fkst"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    mount_write(&server, "main").await;
    // Shadow the happy-path ref patch with a 409 (higher priority).
    Mock::given(method("PATCH"))
        .and(path_matcher("/repos/acme/site/git/refs/heads/main"))
        .respond_with(ResponseTemplate::new(409))
        .with_priority(1)
        .mount(&server)
        .await;

    let err = run_fkst_setup(&app(&server.uri()), "acme", "site", false)
        .await
        .expect_err("must fail");
    assert!(matches!(err, AppError::Conflict(_)), "got {err:?}");
}

fn ctx_with(perms: &[&str]) -> AuthContext {
    AuthContext {
        user_id: "u".to_string(),
        email: String::new(),
        display_name: "u".to_string(),
        roles: vec![],
        permissions: perms.iter().map(|p| p.to_string()).collect(),
        groups: vec![],
        user_access_token: Some(SecretString::new("t".into())),
    }
}

#[test]
fn missing_repo_setup_permission_is_403() {
    // The action-layer gate the handler runs first: a caller without
    // `fkst:repo:setup` (only `fkst:goal:create`) is forbidden.
    let ctx = ctx_with(&[permissions::GOAL_CREATE]);
    let err = require_permission(&ctx, permissions::REPO_SETUP).expect_err("must deny");
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
    // With the permission, the action layer passes.
    let ok_ctx = ctx_with(&[permissions::REPO_SETUP]);
    assert!(require_permission(&ok_ctx, permissions::REPO_SETUP).is_ok());
}

#[tokio::test]
async fn org_id_with_non_writer_is_403() {
    // An Authorizer with no NyxID client returns no org role, so a non-admin
    // caller is denied org-writer — mirroring the handler's org gate.
    use crate::authz::Authorizer;
    let authz = Authorizer::new(None);
    let ctx = ctx_with(&[permissions::REPO_SETUP]);
    let err = authz
        .require_org_writer(&ctx, "org-7")
        .await
        .expect_err("non-writer must be denied");
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
}

#[test]
fn malformed_owner_or_name_is_400() {
    assert!(matches!(
        validate_repo_ref("bad owner!", "site"),
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        validate_repo_ref("acme", "bad/name"),
        Err(AppError::Validation(_))
    ));
    assert!(validate_repo_ref("acme", "site").is_ok());
}

/// #115 conformance guard: the example package the scaffold writes must pass
/// the engine's `PreparedPackage::validate()`. Rebuild the package from the
/// scaffold consts (entry at the package-root-relative `departments/example/
/// main.lua`) and assert it validates.
#[test]
fn example_scaffold_package_passes_engine_validation() {
    let package = PreparedPackage {
        package_name: repos_scaffold::EXAMPLE_PACKAGE_NAME.to_string(),
        files: vec![
            PackageFile {
                path: repos_scaffold::EXAMPLE_ENTRY_RELATIVE.to_string(),
                content: repos_scaffold::EXAMPLE_MAIN_LUA.to_string(),
            },
            PackageFile {
                path: "README.md".to_string(),
                content: repos_scaffold::EXAMPLE_README_MD.to_string(),
            },
        ],
        composed_deps: vec![],
    };
    package
        .validate()
        .expect("the example scaffold package must pass #115 engine validation");
}
