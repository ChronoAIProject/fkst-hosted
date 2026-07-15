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
        "id": 1000 + name.len() as i64,
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
            { "id": 5, "name": "bare", "owner": { "login": "x", "type": "User" }, "private": false }
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

#[tokio::test]
async fn user_orgs_lists_logins_paginated() {
    let server = MockServer::start().await;
    let next = format!("{}/user/orgs?page=2", server.uri());
    Mock::given(method("GET"))
        .and(path("/user/orgs"))
        .and(query_param("per_page", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                .set_body_json(serde_json::json!([{ "login": "ChronoAIProject" }])),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/orgs"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([{ "login": "aevatarAI" }])),
        )
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let orgs = gh.user_orgs(&tok()).await.expect("ok");
    assert_eq!(
        orgs,
        vec!["ChronoAIProject".to_string(), "aevatarAI".to_string()]
    );
}

#[tokio::test]
async fn create_repo_personal_posts_user_repos_and_maps_the_created_repo() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/user/repos"))
        .and(wiremock::matchers::body_json(serde_json::json!({
            "name": "fresh", "private": true, "description": "hello"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": 4242,
            "name": "fresh",
            "owner": { "login": "shining", "type": "User" },
            "private": true,
            "permissions": { "admin": true }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let repo = gh
        .create_repo(&tok(), None, "fresh", true, Some("hello"))
        .await
        .expect("created");
    assert_eq!(repo.owner, "shining");
    assert_eq!(repo.name, "fresh");
    assert!(repo.private);
    assert!(!repo.org);
    assert!(repo.admin);
}

#[tokio::test]
async fn create_repo_org_posts_to_the_org_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orgs/acme/repos"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": 4343,
            "name": "site",
            "owner": { "login": "acme", "type": "Organization" },
            "private": false
        })))
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let repo = gh
        .create_repo(&tok(), Some("acme"), "site", false, None)
        .await
        .expect("created");
    assert!(repo.org);
    assert!(
        repo.admin,
        "create response without permissions defaults open"
    );
}

#[tokio::test]
async fn create_repo_403_names_the_administration_permission() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/user/repos"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "message": "Resource not accessible by integration"
        })))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let err = gh
        .create_repo(&tok(), None, "x", true, None)
        .await
        .expect_err("403 must error");
    let msg = format!("{err}");
    assert!(msg.contains("Resource not accessible"), "{msg}");
    assert!(
        msg.contains("Administration"),
        "names the missing grant: {msg}"
    );
}

#[tokio::test]
async fn create_repo_422_passes_githubs_message_through() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/user/repos"))
        .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
            "message": "name already exists on this account"
        })))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let err = gh
        .create_repo(&tok(), None, "dup", true, None)
        .await
        .expect_err("422 must error");
    let msg = format!("{err}");
    assert!(msg.contains("name already exists"), "{msg}");
    assert!(msg.starts_with("invalid request"), "maps to 400: {msg}");
}

#[tokio::test]
async fn remove_installation_repo_deletes_by_ids() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/user/installations/42/repositories/777"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    gh.remove_installation_repo(&tok(), 42, 777)
        .await
        .expect("removed");
}

#[tokio::test]
async fn delete_installation_uses_the_app_jwt_route() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/app/installations/146704012"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    gh.delete_installation(&tok(), 146704012)
        .await
        .expect("uninstalled");
}

#[tokio::test]
async fn delete_helpers_carry_githubs_message_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/app/installations/1"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let err = gh
        .delete_installation(&tok(), 1)
        .await
        .expect_err("404 must error");
    assert!(format!("{err}").contains("delete_installation"), "{err}");
}

#[tokio::test]
async fn repo_id_reads_the_repo_object() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/shining/notes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 777, "name": "notes", "owner": { "login": "shining", "type": "User" }
        })))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    assert_eq!(
        gh.repo_id(&tok(), "shining", "notes").await.expect("ok"),
        777
    );
}

#[tokio::test]
async fn user_installations_carry_the_repository_selection() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "installations": [{ "id": 9, "account": { "login": "acme" }, "repository_selection": "selected" }]
        })))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let insts = gh.user_installations(&tok()).await.expect("ok");
    assert_eq!(insts[0].repository_selection, "selected");
}
