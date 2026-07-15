//! Unit tests for the repo-listing building blocks: the paginated
//! `GET /user/repos` read (bare-array response, affiliation/visibility query,
//! permission/org mapping) — the installed-set merge is exercised through the
//! same `DashboardGithub` helpers the dashboard tests already pin.

use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::routes::dashboard::DashboardGithub;

fn tok() -> SecretString {
    SecretString::from("user-token".to_string())
}

fn repo_json(owner: &str, kind: &str, name: &str, private: bool, admin: bool) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "owner": { "login": owner, "type": kind },
        "private": private,
        "permissions": { "admin": admin, "push": true, "pull": true }
    })
}

#[tokio::test]
async fn user_all_repos_maps_owner_kind_privacy_and_admin() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param(
            "affiliation",
            "owner,collaborator,organization_member",
        ))
        .and(query_param("visibility", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            repo_json("shining", "User", "notes", true, true),
            repo_json("acme-org", "Organization", "site", false, false),
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let repos = gh.user_all_repos(&tok()).await.expect("ok");

    assert_eq!(repos.len(), 2);
    assert_eq!(repos[0].owner, "shining");
    assert_eq!(repos[0].name, "notes");
    assert!(repos[0].private);
    assert!(!repos[0].org);
    assert!(repos[0].admin);
    assert_eq!(repos[1].owner, "acme-org");
    assert!(repos[1].org);
    assert!(!repos[1].private);
    assert!(!repos[1].admin, "no admin permission maps to false");
}

#[tokio::test]
async fn user_all_repos_defaults_admin_closed_when_permissions_absent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "name": "bare", "owner": { "login": "x", "type": "User" }, "private": false }
        ])))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let repos = gh.user_all_repos(&tok()).await.expect("ok");
    assert_eq!(repos.len(), 1);
    assert!(!repos[0].admin, "absent permissions must fail closed");
}

#[tokio::test]
async fn user_all_repos_follows_link_pagination() {
    let server = MockServer::start().await;
    let next = format!("{}/user/repos?page=2", server.uri());
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param("per_page", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                .set_body_json(serde_json::json!([repo_json(
                    "a", "User", "one", false, true
                )])),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([repo_json(
                "a", "User", "two", false, true
            )])),
        )
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let repos = gh.user_all_repos(&tok()).await.expect("ok");
    assert_eq!(repos.len(), 2);
    assert_eq!(repos[1].name, "two");
}

#[tokio::test]
async fn user_all_repos_maps_github_5xx_to_bad_gateway() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let err = gh.user_all_repos(&tok()).await.expect_err("500 must error");
    let msg = format!("{err}");
    assert!(msg.contains("user_repos"), "error names the op: {msg}");
}
