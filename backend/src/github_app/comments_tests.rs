//! The paging behaviour that matters: a schedule's recent run records must be
//! reachable however long its issue has been accruing comments.

use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn token() -> SecretString {
    SecretString::from("ghs_test".to_string())
}

fn comment(body: &str, login: &str) -> serde_json::Value {
    serde_json::json!({
        "body": body,
        "user": { "login": login },
        "created_at": "2026-07-27T03:00:00Z",
    })
}

#[tokio::test]
async fn a_single_page_issue_costs_one_request_and_keeps_provenance() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/50/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            comment("first", "alice"),
            comment("second", "fkst-app[bot]"),
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let reader = HttpIssueCommentReader::new(&server.uri()).expect("reader");
    let comments = reader
        .list_recent_issue_comments(&token(), "acme", "site", 50, 3)
        .await
        .expect("reads");
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].user_login, "alice");
    assert_eq!(comments[1].user_login, "fkst-app[bot]");
    assert_eq!(comments[1].body, "second");
}

#[tokio::test]
async fn a_long_history_reads_the_newest_pages_not_the_oldest() {
    // The regression this exists to prevent: a schedule firing hourly buries its
    // recent run records past page 1 within days, and reading only page 1 would
    // recover an empty cursor and re-fire the anchor slot forever.
    let server = MockServer::start().await;
    let link = format!(
        "<{}/repos/acme/site/issues/50/comments?page=5>; rel=\"last\"",
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/50/comments"))
        .and(query_param("page", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", link.as_str())
                .set_body_json(serde_json::json!([comment("oldest", "alice")])),
        )
        .mount(&server)
        .await;
    for page in ["4", "5"] {
        Mock::given(method("GET"))
            .and(path("/repos/acme/site/issues/50/comments"))
            .and(query_param("page", page))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([comment(
                    &format!("page-{page}"),
                    "fkst-app[bot]"
                )])),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let reader = HttpIssueCommentReader::new(&server.uri()).expect("reader");
    let comments = reader
        .list_recent_issue_comments(&token(), "acme", "site", 50, 2)
        .await
        .expect("reads");
    let bodies: Vec<&str> = comments.iter().map(|c| c.body.as_str()).collect();
    assert_eq!(
        bodies,
        vec!["page-4", "page-5"],
        "the window is the LAST two pages, in page order"
    );
}

#[tokio::test]
async fn a_window_wider_than_the_history_includes_the_first_page() {
    let server = MockServer::start().await;
    let link = format!(
        "<{}/repos/acme/site/issues/50/comments?page=2>; rel=\"last\"",
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/50/comments"))
        .and(query_param("page", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", link.as_str())
                .set_body_json(serde_json::json!([comment("one", "alice")])),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/50/comments"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([comment("two", "alice")])),
        )
        .expect(1)
        .mount(&server)
        .await;

    let reader = HttpIssueCommentReader::new(&server.uri()).expect("reader");
    let comments = reader
        .list_recent_issue_comments(&token(), "acme", "site", 50, 10)
        .await
        .expect("reads");
    assert_eq!(comments.len(), 2, "page 1 is not re-fetched, just kept");
}

#[tokio::test]
async fn a_vanished_issue_is_an_empty_history_rather_than_a_failure() {
    // Failing the whole repository pass because one schedule issue was deleted
    // would be a far worse trade than reading it as having no history.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/50/comments"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let reader = HttpIssueCommentReader::new(&server.uri()).expect("reader");
    let comments = reader
        .list_recent_issue_comments(&token(), "acme", "site", 50, 3)
        .await
        .expect("404 is an empty list");
    assert!(comments.is_empty());
}

#[tokio::test]
async fn a_transport_failure_propagates_rather_than_reading_as_empty() {
    // The opposite trade from a 404: an empty history from a FAILED read would
    // reset the cursor and re-fire a slot that already ran.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/50/comments"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let reader = HttpIssueCommentReader::new(&server.uri()).expect("reader");
    assert!(reader
        .list_recent_issue_comments(&token(), "acme", "site", 50, 3)
        .await
        .is_err());
}

#[tokio::test]
async fn an_anonymous_comment_reads_as_an_untrusted_author_not_a_decode_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/50/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "body": "ghost", "created_at": "2026-07-27T03:00:00Z" }
        ])))
        .mount(&server)
        .await;

    let reader = HttpIssueCommentReader::new(&server.uri()).expect("reader");
    let comments = reader
        .list_recent_issue_comments(&token(), "acme", "site", 50, 3)
        .await
        .expect("reads");
    assert_eq!(
        comments[0].user_login, "",
        "never matches a configured bot login"
    );
}
