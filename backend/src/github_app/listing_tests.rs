//! wiremock tests for the Model B listing transport (`listing.rs`). Mirror the
//! setup in `api.rs`'s tests: a live `MockServer`, `HttpGithubListing` pointed at
//! it, and success + failure coverage per method.

use secrecy::SecretString;
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const APP_JWT: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.test.payload";
const TOKEN: &str = "ghs_listing_token";

fn listing(server_uri: &str) -> HttpGithubListing {
    HttpGithubListing::new(server_uri).expect("listing client")
}

fn tok() -> SecretString {
    SecretString::from(TOKEN.to_string())
}

fn jwt() -> SecretString {
    SecretString::from(APP_JWT.to_string())
}

// ---- list_issues_by_label -------------------------------------------------

#[tokio::test]
async fn list_issues_maps_fields_and_sends_label_state_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .and(query_param("labels", "fkst-run"))
        .and(query_param("state", "open"))
        .and(query_param("per_page", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 7,
                "title": "Do the thing",
                "labels": [{ "name": "fkst-run" }, { "name": "backend" }],
                "state": "open",
                "assignees": [{ "login": "alice" }, { "login": "bob" }],
                "user": { "login": "carol", "id": 4242 }
            },
            {
                // A pull request: the issues endpoint returns these too and they
                // MUST be filtered out (they carry a `pull_request` object).
                "number": 8,
                "title": "A PR, not an issue",
                "labels": [{ "name": "fkst-run" }],
                "state": "open",
                "assignees": [],
                "user": { "login": "dave", "id": 5 },
                "pull_request": { "url": "https://example/pulls/8" }
            }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let issues = listing(&server.uri())
        .list_issues_by_label(&tok(), "acme", "site", "fkst-run")
        .await
        .expect("ok");

    assert_eq!(issues.len(), 1, "the pull request must be excluded");
    let issue = &issues[0];
    assert_eq!(issue.number, 7);
    assert_eq!(issue.title, "Do the thing");
    assert_eq!(issue.labels, vec!["fkst-run", "backend"]);
    assert_eq!(issue.state, "open");
    assert_eq!(issue.assignees, vec!["alice", "bob"]);
    assert_eq!(issue.user_login, "carol");
    assert_eq!(issue.user_id, 4242);
}

#[tokio::test]
async fn list_issues_follows_link_pagination_across_two_pages() {
    let server = MockServer::start().await;
    let next_link = format!(
        "<{}/repos/acme/site/issues?labels=fkst-run&state=open&per_page=100&page=2>; rel=\"next\"",
        server.uri()
    );

    // Page 1: carries a `Link: rel="next"` header pointing at page 2.
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param_is_missing("page"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", next_link.as_str())
                .set_body_json(serde_json::json!([
                    {
                        "number": 1,
                        "title": "first",
                        "labels": [],
                        "state": "open",
                        "assignees": [],
                        "user": { "login": "u", "id": 1 }
                    }
                ])),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Page 2: no `Link` header -> the loop stops after this page.
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 2,
                "title": "second",
                "labels": [],
                "state": "open",
                "assignees": [],
                "user": { "login": "u", "id": 1 }
            }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let issues = listing(&server.uri())
        .list_issues_by_label(&tok(), "acme", "site", "fkst-run")
        .await
        .expect("ok");

    let numbers: Vec<i64> = issues.iter().map(|i| i.number).collect();
    assert_eq!(numbers, vec![1, 2], "both pages must be collected in order");
}

#[tokio::test]
async fn list_issues_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = listing(&server.uri())
        .list_issues_by_label(&tok(), "acme", "gone", "fkst-run")
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, GithubAppError::NotFound { .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn list_issues_plain_403_is_app_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let err = listing(&server.uri())
        .list_issues_by_label(&tok(), "acme", "site", "fkst-run")
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
}

#[tokio::test]
async fn list_issues_403_with_rate_headers_is_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("retry-after", "30"),
        )
        .mount(&server)
        .await;

    let err = listing(&server.uri())
        .list_issues_by_label(&tok(), "acme", "site", "fkst-run")
        .await
        .expect_err("must fail");
    match err {
        GithubAppError::RateLimited(secs) => assert_eq!(secs, 30),
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

// ---- count_open_issues_with_label -----------------------------------------

#[tokio::test]
async fn count_parses_total_count_and_url_encodes_the_label() {
    let server = MockServer::start().await;
    // A label with a space proves the qualifier (and thus the label) is
    // URL-encoded on the wire: wiremock decodes the query param, so this exact
    // match only succeeds if the client percent/`+`-encoded the space.
    Mock::given(method("GET"))
        .and(path("/search/issues"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .and(query_param(
            "q",
            "repo:acme/site type:issue state:open label:\"needs triage\"",
        ))
        .and(query_param("per_page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 42,
            "incomplete_results": false,
            "items": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    let count = listing(&server.uri())
        .count_open_issues_with_label(&tok(), "acme", "site", "needs triage")
        .await
        .expect("ok");
    assert_eq!(count, 42);
}

#[tokio::test]
async fn count_403_with_rate_headers_is_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search/issues"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("retry-after", "15"),
        )
        .mount(&server)
        .await;

    let err = listing(&server.uri())
        .count_open_issues_with_label(&tok(), "acme", "site", "fkst-run")
        .await
        .expect_err("must fail");
    match err {
        GithubAppError::RateLimited(secs) => assert_eq!(secs, 15),
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

// ---- list_installations ----------------------------------------------------

#[tokio::test]
async fn list_installations_uses_app_jwt_and_paginates() {
    let server = MockServer::start().await;
    let next_link = format!(
        "<{}/app/installations?per_page=100&page=2>; rel=\"next\"",
        server.uri()
    );

    Mock::given(method("GET"))
        .and(path("/app/installations"))
        .and(header(
            "authorization",
            format!("Bearer {APP_JWT}").as_str(),
        ))
        .and(query_param_is_missing("page"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", next_link.as_str())
                .set_body_json(serde_json::json!([
                    { "id": 11, "account": { "login": "acme" } }
                ])),
        )
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/app/installations"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 22, "account": { "login": "globex" } }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let installs = listing(&server.uri())
        .list_installations(&jwt())
        .await
        .expect("ok");

    assert_eq!(
        installs,
        vec![
            InstallationSummary {
                id: 11,
                account_login: "acme".to_string()
            },
            InstallationSummary {
                id: 22,
                account_login: "globex".to_string()
            },
        ]
    );
}

#[tokio::test]
async fn list_installations_401_is_app_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = listing(&server.uri())
        .list_installations(&jwt())
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
}

// ---- list_installation_repos ----------------------------------------------

#[tokio::test]
async fn list_installation_repos_maps_to_repo_refs_and_paginates() {
    let server = MockServer::start().await;
    let next_link = format!(
        "<{}/installation/repositories?per_page=100&page=2>; rel=\"next\"",
        server.uri()
    );

    Mock::given(method("GET"))
        .and(path("/installation/repositories"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .and(query_param_is_missing("page"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", next_link.as_str())
                .set_body_json(serde_json::json!({
                    "total_count": 2,
                    "repositories": [
                        { "name": "site", "owner": { "login": "acme" } }
                    ]
                })),
        )
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/installation/repositories"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 2,
            "repositories": [
                { "name": "tools", "owner": { "login": "globex" } }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let repos = listing(&server.uri())
        .list_installation_repos(&tok())
        .await
        .expect("ok");

    assert_eq!(
        repos,
        vec![
            RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string()
            },
            RepoRef {
                owner: "globex".to_string(),
                name: "tools".to_string()
            },
        ]
    );
}

#[tokio::test]
async fn list_installation_repos_plain_403_is_app_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let err = listing(&server.uri())
        .list_installation_repos(&tok())
        .await
        .expect_err("must fail");
    assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
}
