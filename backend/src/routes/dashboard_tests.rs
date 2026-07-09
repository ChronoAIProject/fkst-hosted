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

const VALID_TRIGGER_BODY: &str = "### Session Name\nsite\n\n### Packages\n\
acme/pkgs@main:packages/devloop\n\n### Work Label\nsite-build\n\n### Auto-merge\ntrue\n";

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
                "user": { "login": "a", "id": 9 }
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
    assert_eq!(issues[0].number, 1);
    assert_eq!(issues[0].state, "closed");
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
fn build_session_groups_trigger_and_work_issues() {
    let trigger = issue(
        5,
        VALID_TRIGGER_BODY,
        &["fkst-substrate-trigger", "fkst-substrate-active"],
        "open",
    );
    let repo = RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    };
    let reg = parse_registration(42, &repo, &trigger).expect("valid trigger parses");
    let work = vec![issue(6, "", &["site-build"], "closed")];
    let s = build_session(&trigger, &reg, work);

    assert_eq!(s.name.as_deref(), Some("site"));
    assert_eq!(s.work_label.as_deref(), Some("site-build"));
    assert_eq!(s.auto_merge, Some(true));
    assert_eq!(
        s.packages,
        vec!["acme/pkgs@main:packages/devloop".to_string()]
    );
    assert_eq!(
        s.status_labels,
        vec![
            "fkst-substrate-trigger".to_string(),
            "fkst-substrate-active".to_string()
        ]
    );
    assert!(s.session_id.is_some());
    assert!(s.invalid_reason.is_none());
    assert_eq!(s.work_issues.len(), 1);
    assert_eq!(s.work_issues[0].number, 6);
    assert_eq!(s.work_issues[0].state, "closed");
}

#[test]
fn build_invalid_session_carries_reason_and_no_work() {
    let trigger = issue(7, "no headings here", &["fkst-substrate-invalid"], "open");
    let s = build_invalid_session(&trigger, "missing ### Session Name".to_string());
    assert!(s.session_id.is_none());
    assert!(s.name.is_none());
    assert_eq!(
        s.invalid_reason.as_deref(),
        Some("missing ### Session Name")
    );
    assert_eq!(s.status_labels, vec!["fkst-substrate-invalid".to_string()]);
    assert!(s.work_issues.is_empty());
    assert!(s.packages.is_empty());
}

#[test]
fn pull_job_round_trips_through_json() {
    let job = PullJob {
        job_id: "9-123".to_string(),
        user_id: 9,
        state: "running".to_string(),
        phase: "scanning sessions".to_string(),
        done: 2,
        total: 5,
        error: None,
    };
    let bytes = serde_json::to_vec(&job).expect("serialize");
    let back: PullJob = serde_json::from_slice(&bytes).expect("deserialize");
    assert_eq!(back.job_id, "9-123");
    assert_eq!(back.user_id, 9);
    assert_eq!(back.state, "running");
    assert_eq!(back.done, 2);
    assert_eq!(back.total, 5);
    assert!(back.error.is_none(), "absent error must round-trip to None");
}

#[test]
fn storage_keys_are_namespaced() {
    assert_eq!(result_key(42), "dashboards/42.json");
    assert_eq!(job_key("42-999"), "dashboards/jobs/42-999.json");
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
