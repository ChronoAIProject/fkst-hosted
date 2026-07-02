//! Transport-layer tests for `HttpGithubApi` (extracted from api.rs to keep it
//! under the 500-line budget; sibling `#[path]` module, mirrors repo.rs).

use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const APP_JWT: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.test.payload";

fn api(server_uri: &str) -> HttpGithubApi {
    HttpGithubApi::new(server_uri).expect("api client")
}

fn jwt() -> SecretString {
    SecretString::from(APP_JWT.to_string())
}

// ---- installation_for_repo -----------------------------------------------

#[tokio::test]
async fn installation_lookup_sends_bearer_on_correct_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/installation"))
        .and(header(
            "authorization",
            format!("Bearer {APP_JWT}").as_str(),
        ))
        .and(header("accept", "application/vnd.github+json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 99999 })))
        .expect(1)
        .mount(&server)
        .await;

    let id = api(&server.uri())
        .installation_for_repo(&jwt(), "acme", "site")
        .await
        .expect("ok");
    assert_eq!(id, InstallationId(99999));
}

#[tokio::test]
async fn installation_404_is_not_installed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = api(&server.uri())
        .installation_for_repo(&jwt(), "acme", "site")
        .await
        .expect_err("must fail");
    match err {
        GithubAppError::NotInstalled { owner_repo, .. } => {
            assert_eq!(owner_repo, "acme/site");
        }
        other => panic!("expected NotInstalled, got {other:?}"),
    }
}

// ---- create_installation_token -------------------------------------------

#[tokio::test]
async fn token_mint_posts_bare_repo_names_and_permissions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/app/installations/42/access_tokens"))
        .and(header(
            "authorization",
            format!("Bearer {APP_JWT}").as_str(),
        ))
        .and(body_partial_json(serde_json::json!({
            "repositories": ["site"],
            "permissions": { "contents": "write", "issues": "read" }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "token": "ghs_testtoken123",
            "expires_at": "2026-06-12T12:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let result = api(&server.uri())
        .create_installation_token(
            &jwt(),
            InstallationId(42),
            &InstallationTokenRequest {
                repositories: vec!["site".to_string()],
                permissions: Some(TokenPermissions {
                    contents: Some("write".to_string()),
                    issues: Some("read".to_string()),
                    ..TokenPermissions::default()
                }),
            },
        )
        .await
        .expect("ok");

    assert_eq!(result.token.expose_secret(), "ghs_testtoken123");
}

#[tokio::test]
async fn token_mint_serializes_admin_and_pull_requests() {
    // Issue #110: the elevated session permission set must reach GitHub in
    // the request body. Assert the serialized `permissions` object carries
    // `administration:write` and `pull_requests:write` (alongside the
    // existing contents/issues writes).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/app/installations/7/access_tokens"))
        .and(body_partial_json(serde_json::json!({
            "permissions": {
                "contents": "write",
                "pull_requests": "write",
                "issues": "write",
                "administration": "write"
            }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "token": "ghs_admintoken",
            "expires_at": "2026-06-12T12:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let result = api(&server.uri())
        .create_installation_token(
            &jwt(),
            InstallationId(7),
            &InstallationTokenRequest {
                repositories: vec!["site".to_string()],
                permissions: Some(TokenPermissions {
                    contents: Some("write".to_string()),
                    pull_requests: Some("write".to_string()),
                    issues: Some("write".to_string()),
                    administration: Some("write".to_string()),
                    metadata: None,
                }),
            },
        )
        .await
        .expect("ok");

    assert_eq!(result.token.expose_secret(), "ghs_admintoken");
}

#[tokio::test]
async fn token_mint_parses_expires_at() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "token": "ghs_xyz",
            "expires_at": "2026-06-12T13:00:00Z"
        })))
        .mount(&server)
        .await;

    let result = api(&server.uri())
        .create_installation_token(
            &jwt(),
            InstallationId(1),
            &InstallationTokenRequest {
                repositories: vec![],
                permissions: None,
            },
        )
        .await
        .expect("ok");

    // Verify that expires_at was parsed (non-zero SystemTime).
    assert!(result.expires_at > SystemTime::UNIX_EPOCH);
}

#[tokio::test]
async fn token_mint_404_is_installation_gone() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = api(&server.uri())
        .create_installation_token(
            &jwt(),
            InstallationId(1),
            &InstallationTokenRequest {
                repositories: vec![],
                permissions: None,
            },
        )
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, GithubAppError::InstallationGone { .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn token_mint_422_is_token_request_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(422)
                .set_body_json(serde_json::json!({ "message": "permission not granted" })),
        )
        .mount(&server)
        .await;

    let err = api(&server.uri())
        .create_installation_token(
            &jwt(),
            InstallationId(1),
            &InstallationTokenRequest {
                repositories: vec![],
                permissions: None,
            },
        )
        .await
        .expect_err("must fail");
    match err {
        GithubAppError::TokenRequestRejected(detail) => {
            assert!(detail.contains("permission not granted"), "got {detail}");
        }
        other => panic!("expected TokenRequestRejected, got {other:?}"),
    }
}

#[tokio::test]
async fn token_mint_401_is_app_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = api(&server.uri())
        .create_installation_token(
            &jwt(),
            InstallationId(1),
            &InstallationTokenRequest {
                repositories: vec![],
                permissions: None,
            },
        )
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
}

#[tokio::test]
async fn token_mint_plain_403_is_app_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let err = api(&server.uri())
        .create_installation_token(
            &jwt(),
            InstallationId(1),
            &InstallationTokenRequest {
                repositories: vec![],
                permissions: None,
            },
        )
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
}

#[tokio::test]
async fn token_mint_403_with_rate_headers_is_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("retry-after", "45"),
        )
        .mount(&server)
        .await;

    let err = api(&server.uri())
        .create_installation_token(
            &jwt(),
            InstallationId(1),
            &InstallationTokenRequest {
                repositories: vec![],
                permissions: None,
            },
        )
        .await
        .expect_err("must fail");
    match err {
        GithubAppError::RateLimited(secs) => assert_eq!(secs, 45),
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn installation_401_is_app_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = api(&server.uri())
        .installation_for_repo(&jwt(), "a", "b")
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
}

#[tokio::test]
async fn installation_plain_403_is_app_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let err = api(&server.uri())
        .installation_for_repo(&jwt(), "a", "b")
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
}

#[tokio::test]
async fn installation_403_with_rate_headers_is_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", "9999999999"),
        )
        .mount(&server)
        .await;

    let err = api(&server.uri())
        .installation_for_repo(&jwt(), "a", "b")
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::RateLimited(_)), "got {err:?}");
}

#[tokio::test]
async fn installation_token_debug_never_shows_token() {
    let token = InstallationToken {
        token: SecretString::from("ghs_supersecret".to_string()),
        expires_at: SystemTime::UNIX_EPOCH,
    };
    let debug = format!("{token:?}");
    assert!(!debug.contains("ghs_supersecret"), "token leaked");
    assert!(debug.contains("<redacted>"));
}

#[tokio::test]
async fn create_issue_comment_posts_to_the_issue() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues/7/comments"))
        .and(header("authorization", "Bearer ghs_tok"))
        .and(body_partial_json(serde_json::json!({"body": "done"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": 1})))
        .mount(&server)
        .await;
    api(&server.uri())
        .create_issue_comment(&SecretString::from("ghs_tok"), "acme", "site", 7, "done")
        .await
        .expect("comment posts");
}

#[tokio::test]
async fn add_issue_labels_posts_additively() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues/7/labels"))
        .and(header("authorization", "Bearer ghs_tok"))
        .and(body_partial_json(
            serde_json::json!({"labels": ["fkst-completed"]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;
    api(&server.uri())
        .add_issue_labels(
            &SecretString::from("ghs_tok"),
            "acme",
            "site",
            7,
            &["fkst-completed".to_string()],
        )
        .await
        .expect("labels added");
}

#[tokio::test]
async fn remove_issue_label_tolerates_404() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/repos/acme/site/issues/7/labels/fkst-running"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    api(&server.uri())
        .remove_issue_label(
            &SecretString::from("ghs_tok"),
            "acme",
            "site",
            7,
            "fkst-running",
        )
        .await
        .expect("404 tolerated");
}

// ---- template-reconcile write transport ----------------------------------

fn tok() -> SecretString {
    SecretString::from("ghs_tok".to_string())
}

#[tokio::test]
async fn content_file_404_is_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/acme/site/contents/.github/ISSUE_TEMPLATE/config.yml",
        ))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let got = api(&server.uri())
        .content_file(
            &tok(),
            "acme",
            "site",
            ".github/ISSUE_TEMPLATE/config.yml",
            None,
        )
        .await
        .expect("ok");
    assert!(got.is_none(), "404 must map to None");
}

#[tokio::test]
async fn content_file_returns_sha_and_content() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/contents/x.md"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "sha": "blobsha123",
            "content": "aGVsbG8=\n",
        })))
        .mount(&server)
        .await;
    let got = api(&server.uri())
        .content_file(&tok(), "acme", "site", "x.md", Some("main"))
        .await
        .expect("ok")
        .expect("some");
    assert_eq!(got.sha, "blobsha123");
    assert_eq!(got.content_base64, "aGVsbG8=\n");
}

#[tokio::test]
async fn repo_default_branch_reads_field() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "default_branch": "trunk" })),
        )
        .mount(&server)
        .await;
    let branch = api(&server.uri())
        .repo_default_branch(&tok(), "acme", "site")
        .await
        .expect("ok");
    assert_eq!(branch, "trunk");
}

#[tokio::test]
async fn branch_head_sha_reads_object_sha() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": { "sha": "headsha999" }
        })))
        .mount(&server)
        .await;
    let sha = api(&server.uri())
        .branch_head_sha(&tok(), "acme", "site", "main")
        .await
        .expect("ok");
    assert_eq!(sha, "headsha999");
}

#[tokio::test]
async fn create_ref_422_is_ref_exists() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/git/refs"))
        .and(body_partial_json(serde_json::json!({
            "ref": "refs/heads/fkst/issue-templates-v1",
            "sha": "headsha999",
        })))
        .respond_with(ResponseTemplate::new(422))
        .mount(&server)
        .await;
    let err = api(&server.uri())
        .create_ref(
            &tok(),
            "acme",
            "site",
            "fkst/issue-templates-v1",
            "headsha999",
        )
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::RefExists), "got {err:?}");
}

#[tokio::test]
async fn create_ref_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/git/refs"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    api(&server.uri())
        .create_ref(&tok(), "acme", "site", "fkst/x", "sha")
        .await
        .expect("created");
}

#[tokio::test]
async fn put_file_new_omits_sha() {
    let server = MockServer::start().await;
    // Match ONLY when the body carries content+branch but NO sha (the CREATE
    // path). A `sha`-bearing body would not match this mock, so a success
    // proves `sha` was omitted.
    Mock::given(method("PUT"))
        .and(path("/repos/acme/site/contents/a.md"))
        .and(body_partial_json(serde_json::json!({
            "content": "Zm9v",
            "branch": "fkst/x",
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    api(&server.uri())
        .put_file(
            &tok(),
            "acme",
            "site",
            "a.md",
            "msg",
            "Zm9v",
            "fkst/x",
            None,
        )
        .await
        .expect("created without sha");
}

#[tokio::test]
async fn put_file_update_includes_sha() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/acme/site/contents/a.md"))
        .and(body_partial_json(serde_json::json!({ "sha": "abc123" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    api(&server.uri())
        .put_file(
            &tok(),
            "acme",
            "site",
            "a.md",
            "msg",
            "Zm9v",
            "fkst/x",
            Some("abc123"),
        )
        .await
        .expect("updated with sha");
}

#[tokio::test]
async fn create_pull_request_returns_number() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/pulls"))
        .and(body_partial_json(serde_json::json!({
            "head": "fkst/x",
            "base": "main",
        })))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "number": 321 })),
        )
        .mount(&server)
        .await;
    let number = api(&server.uri())
        .create_pull_request(&tok(), "acme", "site", "t", "fkst/x", "main", "body")
        .await
        .expect("ok");
    assert_eq!(number, 321);
}

#[tokio::test]
async fn merge_pull_request_puts_merge() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/repos/acme/site/pulls/321/merge"))
        .and(body_partial_json(
            serde_json::json!({ "merge_method": "merge" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    api(&server.uri())
        .merge_pull_request(&tok(), "acme", "site", 321, "t")
        .await
        .expect("merged");
}

#[tokio::test]
async fn delete_ref_tolerates_404() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/repos/acme/site/git/refs/heads/fkst/x"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    api(&server.uri())
        .delete_ref(&tok(), "acme", "site", "fkst/x")
        .await
        .expect("404 tolerated");
}
