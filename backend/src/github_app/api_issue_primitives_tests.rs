//! Focused transport tests for issue creation and assignee writes (#2275).

use secrecy::SecretString;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn api(server_uri: &str) -> HttpGithubApi {
    HttpGithubApi::new(server_uri).expect("api client")
}

fn token() -> SecretString {
    SecretString::from("ghs_tok".to_string())
}

#[tokio::test]
async fn create_issue_without_assignees_keeps_the_legacy_payload() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer ghs_tok"))
        .and(body_json(serde_json::json!({
            "title": "A title",
            "body": "A body",
            "labels": ["fkst-dev"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 42
        })))
        .expect(1)
        .mount(&server)
        .await;

    let number = api(&server.uri())
        .create_issue(
            &token(),
            "acme",
            "site",
            "A title",
            "A body",
            &["fkst-dev".to_string()],
            &[],
        )
        .await
        .expect("issue created");
    assert_eq!(number, 42);
}

#[tokio::test]
async fn create_issue_with_assignees_includes_the_assignees_array() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(body_json(serde_json::json!({
            "title": "A title",
            "body": "A body",
            "labels": ["fkst-dev"],
            "assignees": ["alice"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 43
        })))
        .expect(1)
        .mount(&server)
        .await;

    let number = api(&server.uri())
        .create_issue(
            &token(),
            "acme",
            "site",
            "A title",
            "A body",
            &["fkst-dev".to_string()],
            &["alice".to_string()],
        )
        .await
        .expect("issue created");
    assert_eq!(number, 43);
}

#[tokio::test]
async fn add_issue_assignees_posts_the_assignees_array() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues/7/assignees"))
        .and(header("authorization", "Bearer ghs_tok"))
        .and(body_json(serde_json::json!({
            "assignees": ["alice", "bob"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "assignees": [{ "login": "alice" }, { "login": "bob" }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    api(&server.uri())
        .add_issue_assignees(
            &token(),
            "acme",
            "site",
            7,
            &["alice".to_string(), "bob".to_string()],
        )
        .await
        .expect("assignees added");
}

#[tokio::test]
async fn add_issue_assignees_classifies_auth_and_validation_failures() {
    let auth_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&auth_server)
        .await;
    let auth_error = api(&auth_server.uri())
        .add_issue_assignees(&token(), "acme", "site", 7, &["alice".to_string()])
        .await
        .expect_err("401 must fail");
    assert!(matches!(auth_error, GithubAppError::AppAuth));

    for status in [404_u16, 422_u16] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_string("rejected"))
            .mount(&server)
            .await;
        let error = api(&server.uri())
            .add_issue_assignees(&token(), "acme", "site", 7, &["alice".to_string()])
            .await
            .expect_err("non-success must fail");
        match error {
            GithubAppError::Http(detail) => {
                assert!(detail.contains(&status.to_string()), "got {detail}");
                assert!(detail.contains("rejected"), "got {detail}");
            }
            other => panic!("expected Http for {status}, got {other:?}"),
        }
    }
}
