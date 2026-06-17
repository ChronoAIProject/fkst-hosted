//! Unit tests for [`crate::github_hub::contents`] (#181). Split into its own
//! file (referenced via `#[path]`) so `contents.rs` stays under 500 lines —
//! mirroring the `goals/preflight_tests.rs` / `ornn/client_tests.rs` convention.
//!
//! Every test stands up a `wiremock` `MockServer` and asserts the Git Data API
//! call sequence / bodies, exactly as the `github_app::api` tests do.

use super::*;
use crate::github_app::config::GithubAppConfig;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{body_partial_json, method, path as path_matcher};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// A config whose `api_base` points at the wiremock server, with a freshly
/// generated RSA key so the JWT mint succeeds (mirrors `github_app::contents`).
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

/// Mount the installation-resolve + token-mint mocks the `token_for_repo`
/// path needs so a writer test can reach the Git Data calls.
async fn mount_token_mint(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/installation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 7 })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_matcher("/app/installations/7/access_tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "token": "ghs_write_token",
            "expires_at": "2999-01-01T00:00:00Z"
        })))
        .mount(server)
        .await;
}

/// A responder that records every request path it sees, in order — so a test
/// can assert the exact Git Data API call SEQUENCE.
#[derive(Clone)]
struct RecordingResponder {
    order: Arc<Mutex<Vec<String>>>,
    label: String,
    template: Arc<dyn Fn() -> ResponseTemplate + Send + Sync>,
}

impl Respond for RecordingResponder {
    fn respond(&self, _req: &Request) -> ResponseTemplate {
        self.order.lock().unwrap().push(self.label.clone());
        (self.template)()
    }
}

fn recorder(
    order: &Arc<Mutex<Vec<String>>>,
    label: &str,
    template: impl Fn() -> ResponseTemplate + Send + Sync + 'static,
) -> RecordingResponder {
    RecordingResponder {
        order: Arc::clone(order),
        label: label.to_string(),
        template: Arc::new(template),
    }
}

fn app(server_uri: &str) -> GithubAppTokens {
    GithubAppTokens::new(&test_config(server_uri)).expect("app")
}

fn sample_files() -> Vec<ScaffoldFile> {
    vec![
        ScaffoldFile {
            path: ".fkst/AGENTS.md".to_string(),
            contents: b"hello".to_vec(),
        },
        ScaffoldFile {
            path: ".fkst/packages/example/README.md".to_string(),
            contents: b"readme".to_vec(),
        },
    ]
}

/// Mount the full Git Data happy path against the given default branch, all
/// recording into `order`. Returns nothing; assertions read `order`.
async fn mount_happy_path(server: &MockServer, order: &Arc<Mutex<Vec<String>>>, branch: &str) {
    let branch_owned = branch.to_string();
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site"))
        .respond_with(recorder(order, "repo", move || {
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "default_branch": branch_owned }))
        }))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher(format!(
            "/repos/acme/site/git/ref/heads/{branch}"
        )))
        .respond_with(recorder(order, "ref", || {
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "object": { "sha": "BASECOMMIT" } }))
        }))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/git/commits/BASECOMMIT"))
        .respond_with(recorder(order, "commit_get", || {
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "tree": { "sha": "BASETREE" } }))
        }))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_matcher("/repos/acme/site/git/blobs"))
        .respond_with(recorder(order, "blob", || {
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "BLOBSHA" }))
        }))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_matcher("/repos/acme/site/git/trees"))
        .respond_with(recorder(order, "tree", || {
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "NEWTREE" }))
        }))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_matcher("/repos/acme/site/git/commits"))
        .respond_with(recorder(order, "commit_create", || {
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "NEWCOMMIT" }))
        }))
        .mount(server)
        .await;
    Mock::given(method("PATCH"))
        .and(path_matcher(format!(
            "/repos/acme/site/git/refs/heads/{branch}"
        )))
        .respond_with(recorder(order, "ref_patch", || {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ref": "ok" }))
        }))
        .mount(server)
        .await;
}

#[tokio::test]
async fn happy_path_follows_the_blob_tree_commit_ref_sequence() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    let order = Arc::new(Mutex::new(Vec::new()));
    mount_happy_path(&server, &order, "main").await;

    let result = commit_files(&app(&server.uri()), "acme", "site", "msg", &sample_files())
        .await
        .expect("commit ok");

    assert_eq!(result.commit_sha, "NEWCOMMIT");
    assert_eq!(result.default_branch, "main");
    // Two files → two blob POSTs, in the documented order.
    let seq = order.lock().unwrap().clone();
    assert_eq!(
        seq,
        vec![
            "repo",
            "ref",
            "commit_get",
            "blob",
            "blob",
            "tree",
            "commit_create",
            "ref_patch",
        ],
        "the Git Data API call sequence must be exact"
    );
}

#[tokio::test]
async fn default_branch_is_read_from_the_repo_not_assumed_main() {
    // The repo's default branch is `trunk`; every ref/commit/patch call must
    // target `trunk`, proving the writer never assumes `main`.
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    let order = Arc::new(Mutex::new(Vec::new()));
    mount_happy_path(&server, &order, "trunk").await;

    let result = commit_files(&app(&server.uri()), "acme", "site", "msg", &sample_files())
        .await
        .expect("commit ok");
    assert_eq!(result.default_branch, "trunk");
    // The `main` ref/patch routes were never mounted, so reaching them would
    // 404 and fail the commit; success proves `trunk` was used throughout.
}

#[tokio::test]
async fn blob_bodies_are_base64_and_commit_parents_carry_base_sha() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    // Repo + ref + base-commit GETs.
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "default_branch": "main" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/git/ref/heads/main"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "object": { "sha": "BASECOMMIT" } })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/git/commits/BASECOMMIT"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "tree": { "sha": "BASETREE" } })),
        )
        .mount(&server)
        .await;
    // Blob: assert the EXACT base64 body for "hello" + encoding.
    let hello_b64 = base64::engine::general_purpose::STANDARD.encode(b"hello");
    Mock::given(method("POST"))
        .and(path_matcher("/repos/acme/site/git/blobs"))
        .and(body_partial_json(serde_json::json!({
            "content": hello_b64,
            "encoding": "base64",
        })))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "BLOBSHA" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    // Tree: base_tree must be the base tree sha.
    Mock::given(method("POST"))
        .and(path_matcher("/repos/acme/site/git/trees"))
        .and(body_partial_json(
            serde_json::json!({ "base_tree": "BASETREE" }),
        ))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "NEWTREE" })),
        )
        .mount(&server)
        .await;
    // Commit: parents must be exactly [BASECOMMIT].
    Mock::given(method("POST"))
        .and(path_matcher("/repos/acme/site/git/commits"))
        .and(body_partial_json(serde_json::json!({
            "tree": "NEWTREE",
            "parents": ["BASECOMMIT"],
        })))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "NEWCOMMIT" })),
        )
        .mount(&server)
        .await;
    // Ref patch: must carry the NEW commit sha (no force field).
    Mock::given(method("PATCH"))
        .and(path_matcher("/repos/acme/site/git/refs/heads/main"))
        .and(body_partial_json(serde_json::json!({ "sha": "NEWCOMMIT" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let files = vec![ScaffoldFile {
        path: ".fkst/AGENTS.md".to_string(),
        contents: b"hello".to_vec(),
    }];
    commit_files(&app(&server.uri()), "acme", "site", "msg", &files)
        .await
        .expect("commit ok");
}

#[tokio::test]
async fn repo_404_is_repo_not_found() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = commit_files(&app(&server.uri()), "acme", "site", "msg", &sample_files())
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, ContentsWriteError::RepoNotFound(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn not_installed_maps_to_not_installed() {
    // The installation lookup 404s → the mint surfaces NotInstalled, which
    // commit_files maps to ContentsWriteError::NotInstalled.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/installation"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = commit_files(&app(&server.uri()), "acme", "site", "msg", &sample_files())
        .await
        .expect_err("must fail");
    match err {
        ContentsWriteError::NotInstalled {
            owner_repo,
            install_url,
        } => {
            assert_eq!(owner_repo, "acme/site");
            assert!(
                install_url.is_some_and(|u| u.contains("fkst-test")),
                "the install URL must be preserved"
            );
        }
        other => panic!("expected NotInstalled, got {other:?}"),
    }
}

#[tokio::test]
async fn ref_patch_409_is_conflict() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    let order = Arc::new(Mutex::new(Vec::new()));
    // A higher-priority (lower number) 409 ref-patch mock shadows the
    // happy-path 200 mounted below: in wiremock, ties go to the first mock,
    // but an explicit priority wins regardless of mount order.
    Mock::given(method("PATCH"))
        .and(path_matcher("/repos/acme/site/git/refs/heads/main"))
        .respond_with(ResponseTemplate::new(409))
        .with_priority(1)
        .mount(&server)
        .await;
    mount_happy_path(&server, &order, "main").await;

    let err = commit_files(&app(&server.uri()), "acme", "site", "msg", &sample_files())
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, ContentsWriteError::Conflict(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn base_ref_500_is_upstream() {
    let server = MockServer::start().await;
    mount_token_mint(&server).await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "default_branch": "main" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/repos/acme/site/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let err = commit_files(&app(&server.uri()), "acme", "site", "msg", &sample_files())
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, ContentsWriteError::Upstream(_)),
        "got {err:?}"
    );
}
