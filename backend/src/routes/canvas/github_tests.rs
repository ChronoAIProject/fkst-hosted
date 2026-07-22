//! Wiremock tests for the canvas extension methods on `DashboardGithub`,
//! success AND failure paths each (mirrors `repos_tests.rs`).

use secrecy::SecretString;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::error::AppError;
use crate::routes::dashboard::DashboardGithub;

fn tok() -> SecretString {
    SecretString::from("user-token".to_string())
}

#[tokio::test]
async fn create_issue_omits_assignees_when_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_json(serde_json::json!({
            "title": "session",
            "body": "body",
            "labels": ["fkst-substrate-trigger"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 21,
            "html_url": "https://github.com/acme/site/issues/21"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    gh.create_issue(
        &tok(),
        "acme",
        "site",
        "session",
        "body",
        &["fkst-substrate-trigger".to_string()],
        &[],
    )
    .await
    .expect("created");
}

#[tokio::test]
async fn create_issue_includes_nonempty_assignees_exactly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(body_json(serde_json::json!({
            "title": "work",
            "body": "details",
            "labels": ["site-build"],
            "assignees": ["session-owner"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 22,
            "html_url": "https://github.com/acme/site/issues/22"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    gh.create_issue(
        &tok(),
        "acme",
        "site",
        "work",
        "details",
        &["site-build".to_string()],
        &["session-owner".to_string()],
    )
    .await
    .expect("created");
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

#[tokio::test]
async fn list_pulls_all_maps_fields_and_derives_merged() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .and(query_param("state", "all"))
        .and(query_param("per_page", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 12,
                "title": "devloop implementation for #8",
                "html_url": "https://github.com/acme/site/pull/12",
                "state": "closed",
                "merged_at": "2026-07-04T00:00:00Z",
                "user": { "login": "fkst-test[bot]" },
                "head": { "ref": "devloop/issue/acme/site/8/ready-1" }
            },
            {
                "number": 13,
                "title": "closed unmerged",
                "html_url": "https://github.com/acme/site/pull/13",
                "state": "closed",
                "merged_at": null,
                "user": null,
                "head": null
            }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let pulls = gh.list_pulls_all(&tok(), "acme", "site").await.expect("ok");
    assert_eq!(pulls.len(), 2);
    assert_eq!(pulls[0].number, 12);
    assert_eq!(pulls[0].author, "fkst-test[bot]");
    assert_eq!(pulls[0].head_ref, "devloop/issue/acme/site/8/ready-1");
    assert!(pulls[0].merged, "merged_at set derives merged=true");
    assert_eq!(pulls[0].state, "closed");
    assert!(!pulls[1].merged, "closed without merged_at stays unmerged");
    assert!(pulls[1].author.is_empty(), "missing user defaults closed");
    assert!(pulls[1].head_ref.is_empty(), "missing head defaults closed");
}

#[tokio::test]
async fn list_pulls_all_maps_github_5xx_to_upstream_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let gh = DashboardGithub::new(&server.uri()).unwrap();
    let err = gh
        .list_pulls_all(&tok(), "acme", "site")
        .await
        .expect_err("500 must error");
    let msg = format!("{err}");
    assert!(msg.contains("list_pulls_all"), "names the op: {msg}");
}
