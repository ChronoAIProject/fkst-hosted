use super::*;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tok() -> SecretString {
    SecretString::from("user-token".to_string())
}

fn issue(number: i64, body: &str, labels: &[&str], state: &str) -> IssueSummary {
    IssueSummary {
        number,
        title: format!("issue-{number}"),
        body: body.to_string(),
        labels: labels.iter().map(|s| s.to_string()).collect(),
        state: state.to_string(),
        assignees: Vec::new(),
        user_login: "author".to_string(),
        user_id: 9,
    }
}

#[tokio::test]
async fn user_installations_maps_id_and_account() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "installations": [{ "id": 42, "account": { "login": "acme" } }]
        })))
        .mount(&server)
        .await;
    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let installs = gh.user_installations(&tok()).await.expect("ok");
    assert_eq!(installs.len(), 1);
    assert_eq!(installs[0].id, 42);
    assert_eq!(installs[0].account, "acme");
}

#[tokio::test]
async fn user_installation_repos_maps_owner_and_name() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations/42/repositories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "repositories": [{ "name": "site", "owner": { "login": "acme" } }]
        })))
        .mount(&server)
        .await;
    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let repos = gh.user_installation_repos(&tok(), 42).await.expect("ok");
    assert_eq!(
        repos,
        vec![RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string()
        }]
    );
}

#[tokio::test]
async fn issues_by_label_all_uses_state_all_and_filters_prs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("state", "all"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 1, "title": "trigger", "body": "b", "state": "closed",
                "labels": [{ "name": "fkst-substrate-trigger" }],
                "user": { "login": "a", "id": 9 },
                "html_url": "https://github.com/acme/site/issues/1",
                "created_at": "2026-07-01T00:00:00Z",
                "updated_at": "2026-07-02T00:00:00Z",
                "closed_at": "2026-07-03T00:00:00Z"
            },
            {
                "number": 2, "title": "a pr", "state": "open",
                "user": { "login": "a", "id": 9 }, "pull_request": { "url": "x" }
            }
        ])))
        .mount(&server)
        .await;
    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let issues = gh
        .issues_by_label_all(&tok(), "acme", "site", "fkst-substrate-trigger")
        .await
        .expect("ok");
    assert_eq!(issues.len(), 1, "the pull request must be filtered out");
    assert_eq!(issues[0].summary.number, 1);
    assert_eq!(issues[0].summary.state, "closed");
    assert_eq!(issues[0].html_url, "https://github.com/acme/site/issues/1");
    assert_eq!(issues[0].created_at, "2026-07-01T00:00:00Z");
    assert_eq!(issues[0].updated_at, "2026-07-02T00:00:00Z");
    assert_eq!(issues[0].closed_at.as_deref(), Some("2026-07-03T00:00:00Z"));
}

#[tokio::test]
async fn issues_by_label_defaults_missing_meta_and_open_closed_at_to_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("state", "open"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 3, "title": "open trigger", "body": "b", "state": "open",
                "labels": [], "user": { "login": "a", "id": 9 },
                "closed_at": null
            }
        ])))
        .mount(&server)
        .await;
    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let issues = gh
        .issues_by_label(&tok(), "acme", "site", "fkst-substrate-trigger", "open")
        .await
        .expect("ok");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].summary.number, 3);
    assert!(issues[0].closed_at.is_none(), "open issue has no closed_at");
    assert!(
        issues[0].html_url.is_empty(),
        "missing html_url must default, not fail the listing"
    );
}

#[tokio::test]
async fn a_401_from_github_is_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let err = gh
        .user_installations(&tok())
        .await
        .expect_err("401 rejects");
    assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
}

#[test]
fn status_labels_keeps_only_fkst_labels() {
    let i = issue(
        1,
        "",
        &["bug", "fkst-degraded", "enhancement", "fkst-picked-up"],
        "open",
    );
    assert_eq!(
        status_labels(&i),
        vec!["fkst-degraded".to_string(), "fkst-picked-up".to_string()]
    );
}
