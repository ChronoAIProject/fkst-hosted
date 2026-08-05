//! Handler-level wiremock coverage for the schedules surface: the read
//! projection end to end, the trust rule on run records, and both authorization
//! tiers on the writes.

use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audit::arguments::AuditedPath;
use crate::routes::canvas::test_support::{
    auth_headers, mount_app_token, mount_repo_admin, test_app, test_state, viewer_user,
};
use crate::schedule::{render_marker, RunRecord, RunStatus};

use super::*;

const BOT: &str = "fkst-app[bot]";
const DEFINITION: &str = "### Workflow\nsourcing\n\n### Run Mode\ncron: 0 1 * * 1-5\n";

fn issue_json(number: i64, body: &str, labels: &[&str], author_id: i64) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": format!("schedule-{number}"),
        "body": body,
        "state": "open",
        "labels": labels
            .iter()
            .map(|label| serde_json::json!({ "name": label }))
            .collect::<Vec<_>>(),
        "assignees": [{ "login": "alice" }],
        "user": { "login": "shining", "id": author_id },
        "html_url": format!("https://github.com/acme/site/issues/{number}"),
        "created_at": "2026-07-01T00:00:00Z",
        "updated_at": "2026-07-02T00:00:00Z",
        "closed_at": serde_json::Value::Null
    })
}

/// The caller's installation covers acme/site (the read tier).
async fn mount_visibility(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "installations": [{ "id": 77, "account": { "login": "acme" } }]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/installations/77/repositories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "repositories": [{ "name": "site", "owner": { "login": "acme" } }]
        })))
        .mount(server)
        .await;
}

async fn mount_definitions(server: &MockServer, issues: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-scheduled-workflow"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues))
        .mount(server)
        .await;
}

async fn mount_comments(server: &MockServer, number: i64, comments: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/acme/site/issues/{number}/comments")))
        .respond_with(ResponseTemplate::new(200).set_body_json(comments))
        .mount(server)
        .await;
}

fn comment(author: &str, body: &str) -> serde_json::Value {
    serde_json::json!({
        "body": body,
        "user": { "login": author },
        "created_at": "2026-07-27T03:00:00Z",
    })
}

fn record(slot: &str, status: RunStatus) -> String {
    let slot = DateTime::parse_from_rfc3339(slot)
        .expect("valid slot")
        .with_timezone(&Utc);
    render_marker(&RunRecord::new(slot, status, slot))
}

fn state_with_bot(server: &MockServer) -> AppState {
    let mut state = test_state(&server.uri(), Some(test_app(&server.uri())));
    state.config.reconcile.github_bot_login = Some(BOT.to_string());
    state
}

// ---- reads -----------------------------------------------------------------

#[tokio::test]
async fn the_list_projects_every_definition_with_its_cadence_and_state() {
    let server = MockServer::start().await;
    mount_visibility(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    mount_definitions(
        &server,
        serde_json::json!([
            issue_json(50, DEFINITION, &["fkst-scheduled-workflow"], 9),
            issue_json(
                51,
                "### Workflow\nsourcing\n",
                &["fkst-scheduled-workflow"],
                9
            ),
        ]),
    )
    .await;
    mount_comments(&server, 50, serde_json::json!([])).await;
    mount_comments(&server, 51, serde_json::json!([])).await;

    let response = repo_schedules(
        State(state_with_bot(&server)),
        Default::default(),
        AuditedPath(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("the list succeeds");
    assert!(response.installed);
    assert_eq!(response.schedules.len(), 2);
    assert_eq!(response.schedules[0].cadence, "weekdays at 01:00 UTC");
    assert_eq!(
        response.schedules[0].state,
        crate::routes::canvas::schedule_projection::ScheduleLifecycle::Idle
    );
    // The broken one is LISTED with its reason, not silently omitted: a schedule
    // the dashboard hid would be invisible until someone noticed it had stopped.
    assert_eq!(
        response.schedules[1].state,
        crate::routes::canvas::schedule_projection::ScheduleLifecycle::Invalid
    );
    assert!(response.schedules[1]
        .invalid_detail
        .as_deref()
        .expect("a reason")
        .contains("### Run Mode"));
}

#[tokio::test]
async fn a_repository_outside_the_callers_installations_lists_empty_rather_than_erroring() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 0, "installations": []
        })))
        .mount(&server)
        .await;

    let response = repo_schedules(
        State(state_with_bot(&server)),
        Default::default(),
        AuditedPath(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("the list succeeds");
    assert!(!response.installed);
    assert!(response.schedules.is_empty());
}

#[tokio::test]
async fn only_app_authored_comments_count_as_run_history() {
    // The same trust rule the clock applies: a forged record must never make the
    // dashboard show a run that did not happen.
    let server = MockServer::start().await;
    mount_visibility(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    mount_definitions(
        &server,
        serde_json::json!([issue_json(50, DEFINITION, &["fkst-scheduled-workflow"], 9)]),
    )
    .await;
    mount_comments(
        &server,
        50,
        serde_json::json!([
            comment("mallory", &record("2026-07-31T01:00:00Z", RunStatus::Ok)),
            comment(BOT, &record("2026-07-30T01:00:00Z", RunStatus::Failed)),
        ]),
    )
    .await;

    let detail = schedule_detail(
        State(state_with_bot(&server)),
        Default::default(),
        AuditedPath(("acme".to_string(), "site".to_string(), 50)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("the detail succeeds");
    assert_eq!(detail.runs.len(), 1, "{:?}", detail.runs);
    assert_eq!(detail.runs[0].status, "failed");
    assert_eq!(detail.upcoming.len(), 5);
}

#[tokio::test]
async fn an_issue_that_is_not_a_definition_is_404_not_a_half_projection() {
    let server = MockServer::start().await;
    mount_visibility(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    mount_definitions(&server, serde_json::json!([])).await;

    let error = schedule_detail(
        State(state_with_bot(&server)),
        Default::default(),
        AuditedPath(("acme".to_string(), "site".to_string(), 999)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("no such definition");
    assert!(matches!(error, AppError::NotFound(_)), "{error:?}");
}

#[tokio::test]
async fn a_run_detail_needs_a_parseable_slot_and_an_existing_run() {
    let server = MockServer::start().await;
    mount_visibility(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    mount_definitions(
        &server,
        serde_json::json!([issue_json(50, DEFINITION, &["fkst-scheduled-workflow"], 9)]),
    )
    .await;
    mount_comments(
        &server,
        50,
        serde_json::json!([comment(BOT, &record("2026-07-30T01:00:00Z", RunStatus::Ok))]),
    )
    .await;
    let state = state_with_bot(&server);

    let bad_slot = schedule_run(
        State(state.clone()),
        Default::default(),
        AuditedPath((
            "acme".to_string(),
            "site".to_string(),
            50,
            "yesterday".to_string(),
        )),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("not a timestamp");
    assert!(matches!(bad_slot, AppError::Validation(_)), "{bad_slot:?}");

    let missing = schedule_run(
        State(state.clone()),
        Default::default(),
        AuditedPath((
            "acme".to_string(),
            "site".to_string(),
            50,
            "2026-07-29T01:00:00Z".to_string(),
        )),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("no run for that slot");
    assert!(matches!(missing, AppError::NotFound(_)), "{missing:?}");

    let found = schedule_run(
        State(state),
        Default::default(),
        AuditedPath((
            "acme".to_string(),
            "site".to_string(),
            50,
            "2026-07-30T01:00:00Z".to_string(),
        )),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("the run exists");
    assert_eq!(found.run.status, "ok");
}

// ---- writes ----------------------------------------------------------------

/// The write tier's pre-flight read of the definition issue, as the USER.
async fn mount_write_preflight(server: &MockServer, author_id: i64, labels: &[&str]) {
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/50"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(issue_json(50, DEFINITION, labels, author_id)),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn pausing_is_idempotent_and_writes_the_user_applied_label() {
    let server = MockServer::start().await;
    mount_write_preflight(&server, 9, &["fkst-scheduled-workflow"]).await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues/50/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(2)
        .mount(&server)
        .await;

    for _ in 0..2 {
        let status = pause_schedule(
            State(state_with_bot(&server)),
            Default::default(),
            AuditedPath(("acme".to_string(), "site".to_string(), 50)),
            viewer_user(),
            auth_headers(),
        )
        .await
        .expect("pausing twice is safe");
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
}

#[tokio::test]
async fn resuming_an_unpaused_schedule_succeeds() {
    // GitHub answers 404 when the label was not there, which IS the desired state.
    let server = MockServer::start().await;
    mount_write_preflight(&server, 9, &["fkst-scheduled-workflow"]).await;
    Mock::given(method("DELETE"))
        .and(path("/repos/acme/site/issues/50/labels/fkst-cron-paused"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let status = resume_schedule(
        State(state_with_bot(&server)),
        Default::default(),
        AuditedPath(("acme".to_string(), "site".to_string(), 50)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("resuming an unpaused schedule is a no-op, not an error");
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn a_non_definition_issue_cannot_be_paused() {
    // GitHub's label endpoint would happily label any issue the caller can write,
    // so a stale number from the UI must not silently pause something unrelated.
    let server = MockServer::start().await;
    mount_write_preflight(&server, 9, &["bug"]).await;

    let error = pause_schedule(
        State(state_with_bot(&server)),
        Default::default(),
        AuditedPath(("acme".to_string(), "site".to_string(), 50)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("not a scheduled workflow");
    assert!(matches!(error, AppError::NotFound(_)), "{error:?}");
}

#[tokio::test]
async fn a_stranger_may_not_operate_someone_elses_schedule() {
    let server = MockServer::start().await;
    // Authored by 4242, not the viewer (id 9), and the viewer is not a repo admin.
    mount_write_preflight(&server, 4242, &["fkst-scheduled-workflow"]).await;
    mount_repo_admin(&server, "acme", "site", false).await;

    let error = pause_schedule(
        State(state_with_bot(&server)),
        Default::default(),
        AuditedPath(("acme".to_string(), "site".to_string(), 50)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("not the author, not an admin");
    assert!(matches!(error, AppError::Forbidden(_)), "{error:?}");
}

#[tokio::test]
async fn a_repo_admin_may_operate_a_schedule_they_did_not_author() {
    let server = MockServer::start().await;
    mount_write_preflight(&server, 4242, &["fkst-scheduled-workflow"]).await;
    mount_repo_admin(&server, "acme", "site", true).await;
    Mock::given(method("DELETE"))
        .and(path("/repos/acme/site/issues/50/labels/fkst-cron-paused"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    assert_eq!(
        resume_schedule(
            State(state_with_bot(&server)),
            Default::default(),
            AuditedPath(("acme".to_string(), "site".to_string(), 50)),
            viewer_user(),
            auth_headers(),
        )
        .await
        .expect("a repo admin holds session-management authority"),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn run_now_conflicts_while_a_run_is_already_in_flight() {
    let server = MockServer::start().await;
    mount_write_preflight(
        &server,
        9,
        &["fkst-scheduled-workflow", "fkst-cron-running"],
    )
    .await;
    mount_app_token(&server, "acme", "site", 77).await;
    mount_definitions(
        &server,
        serde_json::json!([issue_json(
            50,
            DEFINITION,
            &["fkst-scheduled-workflow", "fkst-cron-running"],
            9
        )]),
    )
    .await;
    mount_comments(&server, 50, serde_json::json!([])).await;

    let error = run_schedule_now(
        State(state_with_bot(&server)),
        Default::default(),
        AuditedPath(("acme".to_string(), "site".to_string(), 50)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("a run is in flight");
    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
}

#[tokio::test]
async fn run_now_conflicts_on_a_definition_that_does_not_parse() {
    let server = MockServer::start().await;
    mount_write_preflight(&server, 9, &["fkst-scheduled-workflow"]).await;
    mount_app_token(&server, "acme", "site", 77).await;
    mount_definitions(
        &server,
        serde_json::json!([issue_json(
            50,
            "### Workflow\nsourcing\n",
            &["fkst-scheduled-workflow"],
            9
        )]),
    )
    .await;
    mount_comments(&server, 50, serde_json::json!([])).await;

    let error = run_schedule_now(
        State(state_with_bot(&server)),
        Default::default(),
        AuditedPath(("acme".to_string(), "site".to_string(), 50)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("an invalid definition is not runnable");
    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
}
