//! Transport-layer tests for `HttpGithubApi` (extracted from api.rs to keep it
//! under the 500-line budget; sibling `#[path]` module, mirrors repo.rs).

use wiremock::matchers::{body_partial_json, header, method, path, query_param};
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

#[tokio::test]
async fn get_issue_labels_maps_the_label_names() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/7"))
        .and(header("authorization", "Bearer ghs_tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 7,
            "labels": [{"name": "fkst-degraded"}, {"name": "fkst-substrate-trigger"}]
        })))
        .mount(&server)
        .await;
    let labels = api(&server.uri())
        .get_issue_labels(&SecretString::from("ghs_tok"), "acme", "site", 7)
        .await
        .expect("labels read");
    assert_eq!(labels, vec!["fkst-degraded", "fkst-substrate-trigger"]);
}

#[tokio::test]
async fn get_issue_labels_404_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/7"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let labels = api(&server.uri())
        .get_issue_labels(&SecretString::from("ghs_tok"), "acme", "site", 7)
        .await
        .expect("404 → empty");
    assert!(labels.is_empty());
}

#[tokio::test]
async fn list_issue_comments_maps_the_bodies() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/7/comments"))
        .and(header("authorization", "Bearer ghs_tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"body": "first comment"},
            {"body": "second\n\n<!-- fkst-config-hash: abc123 -->"}
        ])))
        .mount(&server)
        .await;
    let bodies = api(&server.uri())
        .list_issue_comments(&SecretString::from("ghs_tok"), "acme", "site", 7)
        .await
        .expect("comments read");
    assert_eq!(
        bodies,
        vec![
            "first comment".to_string(),
            "second\n\n<!-- fkst-config-hash: abc123 -->".to_string(),
        ]
    );
}

#[tokio::test]
async fn list_issue_comments_404_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/7/comments"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let bodies = api(&server.uri())
        .list_issue_comments(&SecretString::from("ghs_tok"), "acme", "site", 7)
        .await
        .expect("404 → empty");
    assert!(bodies.is_empty());
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

#[tokio::test]
async fn list_open_pulls_projects_number_author_head_ref_and_title() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 1,
                "user": { "login": "fkst-bot" },
                "head": { "sha": "abc", "ref": "devloop/issue/acme/site/42/ready-x" },
                "title": "github-devloop implementation for #42",
            },
            { "number": 2, "user": { "login": "carol" }, "head": { "sha": "def" } },
        ])))
        .mount(&server)
        .await;
    let pulls = api(&server.uri())
        .list_open_pulls(&tok(), "acme", "site")
        .await
        .expect("ok");
    assert_eq!(pulls.len(), 2);
    assert_eq!(pulls[0].number, 1);
    assert_eq!(pulls[0].author_login, "fkst-bot");
    assert_eq!(pulls[0].head_sha, "abc");
    assert_eq!(pulls[0].head_ref, "devloop/issue/acme/site/42/ready-x");
    assert_eq!(pulls[0].title, "github-devloop implementation for #42");
    assert_eq!(pulls[1].author_login, "carol");
    // A PR missing `head.ref` / `title` projects to empty strings, not a panic.
    assert_eq!(pulls[1].head_ref, "");
    assert_eq!(pulls[1].title, "");
}

#[tokio::test]
async fn close_issue_patches_state_closed() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/repos/acme/site/issues/42"))
        .and(header("authorization", "Bearer ghs_tok"))
        .and(body_partial_json(serde_json::json!({ "state": "closed" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "number": 42 })))
        .mount(&server)
        .await;
    api(&server.uri())
        .close_issue(&SecretString::from("ghs_tok"), "acme", "site", 42)
        .await
        .expect("issue closes");
}

#[tokio::test]
async fn close_issue_surfaces_non_success() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/repos/acme/site/issues/42"))
        .respond_with(ResponseTemplate::new(410).set_body_string("gone"))
        .mount(&server)
        .await;
    let err = api(&server.uri())
        .close_issue(&tok(), "acme", "site", 42)
        .await
        .expect_err("410 is an error");
    assert!(matches!(err, GithubAppError::Http(_)));
}

// ---- list_pull_files -----------------------------------------------------

#[tokio::test]
async fn list_pull_files_paginates_until_a_short_page() {
    let server = MockServer::start().await;
    // A FULL page (100) makes the transport request page 2; page 2 is short (1),
    // which ends the loop.
    let full_page: Vec<serde_json::Value> = (0..100)
        .map(|i| {
            serde_json::json!({
                "filename": format!("f{i}.txt"),
                "status": "added",
                "additions": 1,
                "deletions": 0,
                "changes": 1,
                "sha": format!("sha{i}"),
            })
        })
        .collect();
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls/7/files"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&full_page))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls/7/files"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"filename": "last.md", "status": "modified", "additions": 3, "deletions": 1, "changes": 4, "sha": "shalast"}
        ])))
        .mount(&server)
        .await;
    let files = api(&server.uri())
        .list_pull_files("ghs_tok", "acme", "site", 7)
        .await
        .expect("ok");
    assert_eq!(files.len(), 101);
    assert_eq!(files[0].filename, "f0.txt");
    assert_eq!(files[100].filename, "last.md");
    assert_eq!(files[100].additions, 3);
    assert_eq!(files[100].deletions, 1);
}

#[tokio::test]
async fn list_pull_files_carries_previous_filename_for_a_rename() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls/9/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "filename": "src/new.rs", "status": "renamed",
                "additions": 0, "deletions": 0, "changes": 0,
                "sha": "abc", "previous_filename": "src/old.rs"
            }
        ])))
        .mount(&server)
        .await;
    let files = api(&server.uri())
        .list_pull_files("ghs_tok", "acme", "site", 9)
        .await
        .expect("ok");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].filename, "src/new.rs");
    assert_eq!(files[0].status, "renamed");
    assert_eq!(files[0].previous_filename.as_deref(), Some("src/old.rs"));
    assert_eq!(files[0].sha, "abc");
}

// ---- get_blob_raw --------------------------------------------------------

#[tokio::test]
async fn get_blob_raw_returns_bytes_with_the_raw_media_type() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/git/blobs/deadbeef"))
        .and(header("accept", "application/vnd.github.raw"))
        .and(header("authorization", "Bearer ghs_tok"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello bytes".to_vec()))
        .mount(&server)
        .await;
    let bytes = api(&server.uri())
        .get_blob_raw("ghs_tok", "acme", "site", "deadbeef", 1024)
        .await
        .expect("ok");
    assert_eq!(bytes, b"hello bytes");
}

#[tokio::test]
async fn get_blob_raw_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/git/blobs/nope"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let err = api(&server.uri())
        .get_blob_raw("ghs_tok", "acme", "site", "nope", 1024)
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, GithubAppError::NotFound { .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn get_blob_raw_over_cap_is_too_large() {
    let server = MockServer::start().await;
    // 50 bytes with a max of 10 — the Content-Length gate rejects it up front.
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/git/blobs/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 50]))
        .mount(&server)
        .await;
    let err = api(&server.uri())
        .get_blob_raw("ghs_tok", "acme", "site", "big", 10)
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::BlobTooLarge), "got {err:?}");
}

#[tokio::test]
async fn pull_request_mergeable_reads_tri_state() {
    // A computed-true mergeable.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls/7"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "mergeable": true })),
        )
        .mount(&server)
        .await;
    assert_eq!(
        api(&server.uri())
            .pull_request_mergeable(&tok(), "acme", "site", 7)
            .await
            .expect("ok"),
        Some(true)
    );

    // A `null` mergeable => not yet computed => None.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls/8"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "mergeable": serde_json::Value::Null })),
        )
        .mount(&server)
        .await;
    assert_eq!(
        api(&server.uri())
            .pull_request_mergeable(&tok(), "acme", "site", 8)
            .await
            .expect("ok"),
        None
    );
}
