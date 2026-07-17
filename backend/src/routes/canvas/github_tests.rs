//! Wiremock tests for the canvas extension methods on `DashboardGithub`,
//! success AND failure paths each (mirrors `repos_tests.rs`).

use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::error::AppError;
use crate::routes::dashboard::DashboardGithub;

fn tok() -> SecretString {
    SecretString::from("user-token".to_string())
}

#[tokio::test]
async fn user_org_memberships_maps_role_and_org_paginated() {
    let server = MockServer::start().await;
    let next = format!("{}/user/memberships/orgs?page=2", server.uri());
    Mock::given(method("GET"))
        .and(path("/user/memberships/orgs"))
        .and(query_param("state", "active"))
        .and(query_param("per_page", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                .set_body_json(serde_json::json!([
                    { "role": "admin", "organization": { "login": "acme" } }
                ])),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/memberships/orgs"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "role": "member", "organization": { "login": "other-org" } }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let memberships = gh.user_org_memberships(&tok()).await.expect("ok");
    assert_eq!(memberships.len(), 2);
    assert_eq!(memberships[0].org, "acme");
    assert_eq!(memberships[0].role, "admin");
    assert_eq!(memberships[1].org, "other-org");
    assert_eq!(memberships[1].role, "member");
}

#[tokio::test]
async fn user_org_memberships_maps_401_to_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/memberships/orgs"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let err = gh
        .user_org_memberships(&tok())
        .await
        .expect_err("401 rejects");
    assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
}

#[tokio::test]
async fn user_org_memberships_maps_github_5xx_to_upstream_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/memberships/orgs"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let err = gh.user_org_memberships(&tok()).await.expect_err("must err");
    let msg = format!("{err}");
    assert!(msg.contains("user_org_memberships"), "names the op: {msg}");
}
