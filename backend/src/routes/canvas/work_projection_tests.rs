//! Ownership tests for the canvas work projection.
//!
//! The projection's job is to answer "which work issues are THIS session's". A
//! label match alone cannot answer that: sharing a work label across creators is
//! intended, and the sole-assignee routing rule is what separates the sessions.
//! These tests drive the real function against a mock GitHub so the ownership
//! rule is exercised through the same path the dashboard uses.

use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::github_app::listing::IssueSummary;
use crate::models::RepoRef;
use crate::routes::canvas::parse_trigger_registration;

const SHARED_LABEL: &str = "shared-label";

/// A trigger issue whose session watches [`SHARED_LABEL`]. `author` becomes the
/// session's effective creator (human-authored trigger), which is exactly the
/// login a work issue's sole assignee must match.
///
/// The package ref is present because a registration resolves its effective
/// package set; its `fkst.toml` is deliberately left unmounted (404), which
/// contributes no discovered labels and leaves the explicit one authoritative.
fn trigger(number: i64, author: &str, author_id: i64) -> IssueSummary {
    IssueSummary {
        number,
        title: format!("trigger-{number}"),
        body: format!(
            "### Session Name\nsess-{number}\n\n### Packages\n\
             acme/pkgs@main:packages/devloop\n\n### Work Label\n{SHARED_LABEL}\n"
        ),
        labels: vec!["fkst-substrate-trigger".to_string()],
        state: "open".to_string(),
        assignees: Vec::new(),
        user_login: author.to_string(),
        user_id: author_id,
        created_at: k8s_openapi::chrono::DateTime::UNIX_EPOCH,
    }
}

/// A work-issue payload as GitHub returns it, carrying `assignees` verbatim.
fn work_issue(number: i64, assignees: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": format!("work-{number}"),
        "body": "",
        "state": "open",
        "labels": [{ "name": SHARED_LABEL }],
        "assignees": assignees
            .iter()
            .map(|login| serde_json::json!({ "login": login }))
            .collect::<Vec<_>>(),
        "user": { "login": "author", "id": 1 },
        "html_url": format!("https://github.com/acme/site/issues/{number}"),
        "created_at": "2026-07-01T00:00:00Z",
        "updated_at": "2026-07-02T00:00:00Z",
        "closed_at": serde_json::Value::Null,
    })
}

/// Project the two fixture sessions against one shared label whose issues carry
/// `assignees`, returning `(session-a issue numbers, session-b issue numbers)`.
///
/// Session A's creator is `shining`; session B's is `otherdev`.
async fn project(issues: Vec<serde_json::Value>) -> (Vec<i64>, Vec<i64>) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", SHARED_LABEL))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues))
        .mount(&server)
        .await;

    let repo = RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    };
    let mut regs = vec![
        parse_trigger_registration(77, &repo, &trigger(5, "shining", 9), None)
            .expect("session a registration"),
        parse_trigger_registration(77, &repo, &trigger(6, "otherdev", 11), None)
            .expect("session b registration"),
    ];
    let session_a = regs[0].session_id.clone();
    let session_b = regs[1].session_id.clone();

    let gh = DashboardGithub {
        api_base: server.uri(),
        client: reqwest::Client::new(),
    };
    let projection = work_issues_by_session(
        &gh,
        &SecretString::from("test-token"),
        "acme",
        "site",
        &mut regs,
        None,
        &[],
    )
    .await
    .expect("projection succeeds");

    // Both sessions must resolve, or a "disjoint" assertion could pass vacuously
    // because one side was dropped rather than filtered.
    assert!(
        projection.labels_by_session.contains_key(&session_a)
            && projection.labels_by_session.contains_key(&session_b),
        "both fixture sessions must resolve their label sets"
    );

    let numbers = |session: &str| -> Vec<i64> {
        projection
            .issues_by_session
            .get(session)
            .map(|issues| issues.iter().map(|issue| issue.summary.number).collect())
            .unwrap_or_default()
    };
    (numbers(&session_a), numbers(&session_b))
}

#[tokio::test]
async fn sessions_sharing_a_label_project_disjoint_work_items() {
    // The regression: both sessions watch SHARED_LABEL, so a label-only
    // projection would hand each of them BOTH issues.
    let (a, b) = project(vec![
        work_issue(10, &["shining"]),
        work_issue(11, &["otherdev"]),
    ])
    .await;
    assert_eq!(a, vec![10], "session a sees only the issue assigned to it");
    assert_eq!(b, vec![11], "session b sees only the issue assigned to it");
}

#[tokio::test]
async fn an_unassigned_issue_is_projected_to_neither_session() {
    let (a, b) = project(vec![work_issue(10, &[])]).await;
    assert!(
        a.is_empty() && b.is_empty(),
        "no assignee is no routing key, so the issue belongs to nobody: {a:?} / {b:?}"
    );
}

#[tokio::test]
async fn a_multiply_assigned_issue_is_projected_to_neither_session() {
    // Naming both creators must not route to both — ambiguous is unrouted.
    let (a, b) = project(vec![work_issue(10, &["shining", "otherdev"])]).await;
    assert!(
        a.is_empty() && b.is_empty(),
        "several assignees leave the issue unrouted: {a:?} / {b:?}"
    );
}

#[tokio::test]
async fn an_issue_assigned_to_a_stranger_is_projected_to_neither_session() {
    let (a, b) = project(vec![work_issue(10, &["nobody"])]).await;
    assert!(
        a.is_empty() && b.is_empty(),
        "a sole assignee who is neither creator routes nowhere: {a:?} / {b:?}"
    );
}

#[tokio::test]
async fn assignee_matching_is_case_insensitive() {
    // GitHub logins are case-insensitive, so a differently-cased assignee is the
    // same person and must still route.
    let (a, b) = project(vec![
        work_issue(10, &["SHINING"]),
        work_issue(11, &["OtherDev"]),
    ])
    .await;
    assert_eq!(a, vec![10]);
    assert_eq!(b, vec![11]);
}

#[tokio::test]
async fn the_full_mix_routes_each_issue_to_at_most_one_session() {
    // Every case at once, which is what a real repository looks like: the counts
    // each session's detail view derives must come only from its own items.
    let (a, b) = project(vec![
        work_issue(10, &["shining"]),
        work_issue(11, &["otherdev"]),
        work_issue(12, &[]),
        work_issue(13, &["shining", "otherdev"]),
        work_issue(14, &["SHINING"]),
        work_issue(15, &["nobody"]),
    ])
    .await;
    assert_eq!(a, vec![10, 14]);
    assert_eq!(b, vec![11]);
    // Disjointness is the property the session detail view depends on.
    assert!(
        a.iter().all(|number| !b.contains(number)),
        "no issue may be projected to more than one session: {a:?} / {b:?}"
    );
}
