//! Handler-level wiremock tests for the per-repo canvas sessions surface,
//! plus unit tests of its pure helpers. The wiremock server plays the
//! user-token reads, the App-token mint, and the issue/PR reads.

use std::sync::Arc;

use axum::extract::{Path, State};
use k8s_openapi::chrono::Utc;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::github_app::listing::IssueSummary;
use crate::reconcile::desired::LivePod;
use crate::routes::canvas::test_support::{
    auth_headers, grant_global_admin, mount_app_token, test_app, test_state, viewer_user,
};
use crate::session_backend::test_support::FakeSessionBackend;
use crate::session_spec::derive_session_id;

const VALID_TRIGGER_BODY: &str = "### Session Name\nsite\n\n### Packages\n\
acme/pkgs@main:packages/devloop\n\n### Manifest\nacme/manifests@main:bundles/site\n\n\
### Work Label\nsite-build\n\n\
### Source Branch\nrelease/v1.2\n\n### Target Branch\nfeature/site\n\n\
### Log Access Allowlist\nalice\n\n### Session Collaborators\nworker\n\n\
### Output Language\nzh-CN\n";

fn issue_json(number: i64, body: &str, labels: &[&str], state: &str) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": format!("issue-{number}"),
        "body": body,
        "state": state,
        "labels": labels
            .iter()
            .map(|label| serde_json::json!({ "name": label }))
            .collect::<Vec<_>>(),
        "user": { "login": "shining", "id": 9 },
        "html_url": format!("https://github.com/acme/site/issues/{number}"),
        "created_at": "2026-07-01T00:00:00Z",
        "updated_at": "2026-07-02T00:00:00Z",
        "closed_at": if state == "closed" { serde_json::json!("2026-07-03T00:00:00Z") } else { serde_json::Value::Null }
    })
}

fn pull_json(
    number: i64,
    author: &str,
    head_ref: &str,
    title: &str,
    state: &str,
    merged: bool,
) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": title,
        "html_url": format!("https://github.com/acme/site/pull/{number}"),
        "state": state,
        "merged_at": if merged { serde_json::json!("2026-07-04T00:00:00Z") } else { serde_json::Value::Null },
        "user": { "login": author },
        "head": { "ref": head_ref }
    })
}

fn work_meta(number: i64, state: &str, labels: &[&str]) -> IssueWithMeta {
    IssueWithMeta {
        summary: IssueSummary {
            number,
            title: format!("work-{number}"),
            body: String::new(),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            state: state.to_string(),
            assignees: Vec::new(),
            user_login: "worker".to_string(),
            user_id: 9,
        },
        html_url: format!("https://github.com/acme/site/issues/{number}"),
        created_at: "2026-07-01T00:00:00Z".to_string(),
        updated_at: "2026-07-02T00:00:00Z".to_string(),
        closed_at: (state == "closed").then(|| "2026-07-03T00:00:00Z".to_string()),
    }
}

async fn mount_installation_covering_site(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "installations": [
                { "id": 77, "account": { "login": "acme" }, "repository_selection": "all" }
            ]
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

#[tokio::test]
async fn repo_sessions_assembles_the_full_detail() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            issue_json(
                5,
                VALID_TRIGGER_BODY,
                &["fkst-substrate-trigger", "fkst-substrate-active"],
                "open"
            ),
            issue_json(6, "no headings", &["fkst-substrate-trigger"], "open"),
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/manifests/contents/bundles/site"))
        .and(query_param("ref", "main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "schemaVersion": 1,
            "name": "site",
            "packages": ["acme/pkgs@main:packages/from-manifest"]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "site-build"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            issue_json(8, "work", &["site-build"], "open"),
            issue_json(9, "done work", &["site-build"], "closed"),
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            // The one devloop PR belonging to work issue #8 (merged).
            pull_json(
                12,
                "fkst-test[bot]",
                "devloop/issue/acme/site/8/ready-1",
                "devloop implementation for #8",
                "closed",
                true
            ),
            // A human PR on a devloop-looking branch: filtered by author.
            pull_json(
                13,
                "human",
                "devloop/issue/acme/site/8/ready-2",
                "manual fix",
                "open",
                false
            ),
            // A bot PR that is NOT devloop (no parseable issue): filtered.
            pull_json(
                14,
                "fkst-test[bot]",
                "fkst/issue-templates-v3",
                "fkst issue templates",
                "open",
                false
            ),
            // A bot devloop PR for an issue outside this session's work label.
            pull_json(
                15,
                "fkst-test[bot]",
                "devloop/issue/acme/site/99/ready-1",
                "devloop implementation for #99",
                "open",
                false
            ),
        ])))
        .mount(&server)
        .await;

    let session_id = derive_session_id(77, "acme", "site", 5);
    let mut state = test_state(&server.uri(), Some(test_app(&server.uri())));
    state.config.reconcile.github_bot_login = Some("fkst-test[bot]".to_string());
    state.config.log.public_base_url = Some("https://fkst.example/".to_string());
    state.session_backend = Some(Arc::new(FakeSessionBackend::default().with_observed(vec![
        LivePod {
            session_id: session_id.clone(),
            trigger_issue: 5,
            liveness: PodLiveness::Live,
            created_at: Utc::now(),
            last_pending_at: None,
            config_hash: None,
            work_labels: vec!["site-build".to_string()],
        },
    ])));

    let Json(view) = repo_sessions(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("200");

    assert_eq!(view.owner, "acme");
    assert_eq!(view.name, "site");
    assert!(view.installed);
    assert_eq!(view.sessions.len(), 2);

    let session = &view.sessions[0];
    assert_eq!(session.session_id.as_deref(), Some(session_id.as_str()));
    assert_eq!(session.name.as_deref(), Some("site"));
    assert_eq!(session.creator, "shining");
    assert_eq!(session.source_branch.as_deref(), Some("release/v1.2"));
    assert_eq!(session.target_branch, "feature/site");
    assert_eq!(session.work_label.as_deref(), Some("site-build"));
    assert_eq!(session.work_labels, vec!["site-build".to_string()]);
    assert_eq!(session.auto_merge, Some(false));
    assert_eq!(
        session.packages,
        vec!["acme/pkgs@main:packages/devloop".to_string()]
    );
    assert_eq!(
        session.manifests,
        vec!["acme/manifests@main:bundles/site".to_string()],
        "the `### Manifest` references round-trip onto the detail"
    );
    assert_eq!(
        session.log_access,
        vec!["alice".to_string()],
        "the `### Log Access Allowlist` grantees round-trip onto the detail"
    );
    assert_eq!(
        session.collaborators,
        vec!["worker".to_string()],
        "the `### Session Collaborators` round-trip onto the detail"
    );
    assert_eq!(
        session.output_lang.as_deref(),
        Some("zh-CN"),
        "the `### Output Language` locale round-trips onto the detail"
    );
    assert!(session.invalid_reason.is_none());
    assert_eq!(
        session.status_labels,
        vec!["fkst-substrate-active".to_string()],
        "the trigger label itself is filtered out of the status projection"
    );
    assert_eq!(session.trigger.number, 5);
    assert_eq!(
        session.trigger.html_url,
        "https://github.com/acme/site/issues/5"
    );
    assert_eq!(session.work_issues.len(), 2);
    assert_eq!(session.work_issues[1].number, 9);
    assert_eq!(
        session.work_issues[1].closed_at.as_deref(),
        Some("2026-07-03T00:00:00Z")
    );
    assert_eq!(
        session.log_url.as_deref(),
        Some(format!("https://fkst.example/api/v1/logs/{session_id}").as_str()),
        "trailing base-URL slash must not double up"
    );
    assert_eq!(session.liveness.as_deref(), Some("live"));
    assert_eq!(
        session.recovery,
        SessionRecoveryProjection {
            state: SessionRecoveryState::Normal,
            reason: SessionRecoveryReason::RuntimeLive,
            open_work_items: 1,
            runtime: SessionRuntimeState::Live,
        }
    );
    assert_eq!(
        session.prs.len(),
        1,
        "only the bot devloop PR for #8 belongs"
    );
    assert_eq!(session.prs[0].number, 12);
    assert!(session.prs[0].merged);
    assert_eq!(session.prs[0].work_issue, Some(8));

    let invalid = &view.sessions[1];
    assert!(invalid.session_id.is_none());
    assert_eq!(invalid.creator, "shining");
    assert_eq!(invalid.source_branch, None);
    assert_eq!(invalid.target_branch, DEFAULT_TARGET_BRANCH);
    assert!(invalid.invalid_reason.is_some());
    assert!(
        invalid.status_labels.is_empty(),
        "the trigger label alone projects no status chips"
    );
    assert!(invalid.work_issues.is_empty());
    assert!(invalid.prs.is_empty());
    assert!(invalid.liveness.is_none());
    assert_eq!(invalid.recovery.state, SessionRecoveryState::Invalid);
    assert_eq!(
        invalid.recovery.reason,
        SessionRecoveryReason::RegistrationInvalid
    );
    assert_eq!(invalid.recovery.runtime, SessionRuntimeState::Unknown);
    assert!(invalid.log_url.is_none());
    assert!(
        invalid.log_access.is_empty(),
        "an unparseable trigger exposes no log-access grantees"
    );
    assert!(
        invalid.collaborators.is_empty(),
        "an unparseable trigger exposes no collaborators"
    );
    assert!(
        invalid.manifests.is_empty(),
        "an unparseable trigger exposes no manifests"
    );
    assert!(
        invalid.output_lang.is_none(),
        "an unparseable trigger exposes no output locale"
    );
}

#[tokio::test]
async fn repo_sessions_projects_manifest_discovered_work_labels() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    let trigger_body = "### Session Name\ndefault-workflows\n\n### Manifest\n\
acme/manifests@main:bundles/default.json\n";
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .and(query_param("state", "all"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([issue_json(
                2,
                trigger_body,
                &["fkst-substrate-trigger", "fkst-substrate-active"],
                "open"
            )])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/manifests/contents/bundles/default.json"))
        .and(query_param("ref", "main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "schemaVersion": 1,
            "name": "default-workflows",
            "packages": ["acme/pkgs@main:packages/workflow-dev"]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/acme/pkgs/contents/packages/workflow-dev/fkst.toml",
        ))
        .and(query_param("ref", "main"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[github]\nwork_labels = [\"fkst-dev\", \"fkst-security\"]\n"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-dev"))
        .and(query_param("state", "all"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([issue_json(
                3,
                "implemented work",
                &["fkst-dev", "fkst-security"],
                "closed"
            )])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-security"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            issue_json(
                3,
                "implemented work",
                &["fkst-dev", "fkst-security"],
                "closed"
            ),
            issue_json(4, "security work", &["fkst-security"], "open")
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .and(query_param("state", "all"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([pull_json(
                6,
                "fkst-test[bot]",
                "devloop/issue/acme/site/3/ready-1",
                "devloop implementation for #3",
                "closed",
                true
            )])),
        )
        .mount(&server)
        .await;

    let mut state = test_state(&server.uri(), Some(test_app(&server.uri())));
    state.config.reconcile.github_bot_login = Some("fkst-test[bot]".to_string());
    let Json(view) = repo_sessions(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("manifest-only session projects work");

    let session = &view.sessions[0];
    assert!(
        session.work_label.is_none(),
        "the trigger remains manifest-only"
    );
    assert_eq!(
        session.work_labels,
        vec!["fkst-dev".to_string(), "fkst-security".to_string()],
        "the detail exposes every manifest/package-discovered queue label"
    );
    let mut work_numbers = session
        .work_issues
        .iter()
        .map(|issue| issue.number)
        .collect::<Vec<_>>();
    work_numbers.sort_unstable();
    assert_eq!(
        work_numbers,
        vec![3, 4],
        "all effective labels are merged and issue #3 is deduplicated"
    );
    assert_eq!(session.prs.len(), 1);
    assert_eq!(session.prs[0].number, 6);
    assert_eq!(session.prs[0].work_issue, Some(3));
}

#[tokio::test]
async fn repo_sessions_canonicalizes_a_case_variant_path() {
    let server = MockServer::start().await;
    // The installation listing carries GitHub's canonical `acme/site`.
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    // No explicit or package-discovered work labels, so the scan reads no work issues.
    let body = "### Session Name\nsite\n\n### Packages\nacme/pkgs@main:packages/devloop\n";
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .and(query_param("state", "all"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([issue_json(
                5,
                body,
                &["fkst-substrate-trigger"],
                "open"
            )])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let Json(view) = repo_sessions(
        State(state),
        Path(("ACME".to_string(), "Site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("200 for the case-variant path");

    assert_eq!(view.owner, "acme", "the response echoes GitHub's casing");
    assert_eq!(view.name, "site");
    assert!(view.installed);
    assert_eq!(view.sessions.len(), 1);
    assert_eq!(
        view.sessions[0].session_id.as_deref(),
        Some(derive_session_id(77, "acme", "site", 5).as_str()),
        "the session id must derive from the canonical casing, never the caller's"
    );
}

#[tokio::test]
async fn repo_sessions_outside_the_callers_installations_is_not_installed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 0,
            "installations": []
        })))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let Json(view) = repo_sessions(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("200 with installed=false");
    assert!(!view.installed);
    assert!(view.sessions.is_empty());
}

#[tokio::test]
async fn global_admin_can_read_a_repo_outside_user_installations() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/app/installations"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "id": 77,
                "account": { "login": "acme", "type": "Organization" },
                "repository_selection": "all"
            }])),
        )
        .mount(&server)
        .await;
    mount_app_token(&server, "acme", "site", 77).await;
    Mock::given(method("GET"))
        .and(path("/installation/repositories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "repositories": [{
                "id": 2,
                "name": "site",
                "owner": { "login": "acme", "type": "Organization" },
                "private": true
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let mut state = test_state(&server.uri(), Some(test_app(&server.uri())));
    grant_global_admin(&mut state, "shining");
    let Json(view) = repo_sessions(
        State(state),
        Path(("ACME".to_string(), "Site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("global admin reads the App-covered repo");

    assert!(view.installed);
    assert_eq!(view.owner, "acme");
    assert_eq!(view.name, "site");
    assert!(view.sessions.is_empty());
    let requests = server.received_requests().await.expect("recorded requests");
    assert!(requests
        .iter()
        .all(|request| !request.url.path().starts_with("/user/")));
}

#[tokio::test]
async fn repo_sessions_without_an_app_is_unavailable() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;

    let state = test_state(&server.uri(), None);
    let err = repo_sessions(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("no App configured is a 503");
    assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
}

#[tokio::test]
async fn repo_sessions_propagates_a_github_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = repo_sessions(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("a 500 from GitHub must error");
    assert!(matches!(err, AppError::Upstream(_)), "got {err:?}");
}

#[tokio::test]
async fn repo_sessions_rejects_a_malformed_owner() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = repo_sessions(
        State(state),
        Path(("bad owner".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("malformed owner is a 400");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

// ---- pure helpers ----------------------------------------------------------

#[test]
fn liveness_label_maps_only_the_three_visible_phases() {
    assert_eq!(liveness_label(PodLiveness::Starting), Some("starting"));
    assert_eq!(liveness_label(PodLiveness::Live), Some("live"));
    assert_eq!(
        liveness_label(PodLiveness::Terminating),
        Some("terminating")
    );
    assert_eq!(liveness_label(PodLiveness::Absent), None);
    assert_eq!(liveness_label(PodLiveness::Terminal), None);
}

#[test]
fn recovery_projection_covers_runtime_convergence_and_observation_gaps() {
    let labels = vec!["fkst-substrate-active".to_string()];
    let cases = [
        (
            Some(PodLiveness::Live),
            SessionRecoveryState::Normal,
            SessionRecoveryReason::RuntimeLive,
            SessionRuntimeState::Live,
        ),
        (
            Some(PodLiveness::Starting),
            SessionRecoveryState::Recovering,
            SessionRecoveryReason::RuntimeStarting,
            SessionRuntimeState::Starting,
        ),
        (
            Some(PodLiveness::Terminating),
            SessionRecoveryState::Recovering,
            SessionRecoveryReason::RuntimeTerminating,
            SessionRuntimeState::Terminating,
        ),
        (
            Some(PodLiveness::Absent),
            SessionRecoveryState::Recovering,
            SessionRecoveryReason::RuntimeAbsent,
            SessionRuntimeState::Absent,
        ),
        (
            Some(PodLiveness::Terminal),
            SessionRecoveryState::Recovering,
            SessionRecoveryReason::RuntimeTerminal,
            SessionRuntimeState::Terminal,
        ),
        (
            None,
            SessionRecoveryState::Unknown,
            SessionRecoveryReason::RuntimeObservationUnavailable,
            SessionRuntimeState::Unknown,
        ),
    ];

    for (liveness, state, reason, runtime) in cases {
        assert_eq!(
            project_session_recovery("open", &labels, 2, liveness),
            SessionRecoveryProjection {
                state,
                reason,
                open_work_items: 2,
                runtime,
            }
        );
    }
}

#[test]
fn observed_session_liveness_distinguishes_absence_from_unavailable_observation() {
    let session_id = "session-1";
    let empty = HashMap::new();
    assert_eq!(
        observed_session_liveness(true, &empty, session_id),
        Some(PodLiveness::Absent),
        "a successful repository observation makes a missing runtime authoritative"
    );
    assert_eq!(
        observed_session_liveness(false, &empty, session_id),
        None,
        "a failed or unavailable observation must not claim the runtime is absent"
    );

    let observed = HashMap::from([(session_id.to_string(), PodLiveness::Terminal)]);
    assert_eq!(
        observed_session_liveness(true, &observed, session_id),
        Some(PodLiveness::Terminal)
    );
}

#[test]
fn recovery_projection_counts_only_open_actionable_work() {
    let work = vec![
        work_meta(1, "open", &["fkst-dev"]),
        work_meta(2, "closed", &["fkst-dev"]),
        work_meta(3, "open", &["fkst-dev", WORK_UNAUTHORIZED_LABEL]),
        work_meta(4, "open", &["fkst-dev", SUBSTRATE_RETIRED_LABEL]),
    ];

    assert_eq!(open_actionable_work_items(&work), 1);
}

#[test]
fn invalid_session_projection_preserves_configuration_rejection_reason() {
    let trigger = work_meta(
        7,
        "open",
        &["fkst-substrate-trigger", SUBSTRATE_CONFIG_REJECTED_LABEL],
    );

    let detail = invalid_session_detail(
        &trigger,
        "invalid registration".to_string(),
        "fkst-substrate-trigger",
        None,
    );

    assert_eq!(detail.recovery.state, SessionRecoveryState::Invalid);
    assert_eq!(
        detail.recovery.reason,
        SessionRecoveryReason::ConfigurationRejected
    );
    assert_eq!(detail.recovery.runtime, SessionRuntimeState::Unknown);
}

#[test]
fn invalid_bot_authored_session_still_projects_its_effective_creator() {
    let mut trigger = work_meta(7, "open", &["fkst-substrate-trigger"]);
    trigger.summary.user_login = "fkst-test[bot]".to_string();
    trigger.summary.user_id = 700;
    trigger.summary.assignees = vec!["seed-owner".to_string()];

    let detail = invalid_session_detail(
        &trigger,
        "invalid registration".to_string(),
        "fkst-substrate-trigger",
        Some("fkst-test"),
    );

    assert_eq!(detail.creator, "seed-owner");
    assert_eq!(detail.source_branch, None);
    assert_eq!(detail.target_branch, DEFAULT_TARGET_BRANCH);
}

#[test]
fn recovery_projection_serializes_as_a_bounded_enum_contract() {
    let projection = project_session_recovery(
        "open",
        &["fkst-substrate-active".to_string()],
        3,
        Some(PodLiveness::Absent),
    );
    let json = serde_json::to_value(projection).expect("serialize recovery projection");

    assert_eq!(
        json,
        serde_json::json!({
            "state": "recovering",
            "reason": "runtime_absent",
            "open_work_items": 3,
            "runtime": "absent"
        })
    );
}

#[test]
fn recovery_projection_applies_terminal_and_degraded_precedence() {
    let case = |state: &str, labels: &[&str], open_work_items, liveness| {
        project_session_recovery(
            state,
            &labels
                .iter()
                .map(|label| label.to_string())
                .collect::<Vec<_>>(),
            open_work_items,
            liveness,
        )
    };

    let idle = case("open", &[], 0, None);
    assert_eq!(idle.state, SessionRecoveryState::Idle);
    assert_eq!(idle.reason, SessionRecoveryReason::NoPendingWork);

    let degraded = case(
        "open",
        &[SUBSTRATE_DEGRADED_LABEL],
        1,
        Some(PodLiveness::Live),
    );
    assert_eq!(degraded.state, SessionRecoveryState::Degraded);
    assert_eq!(
        degraded.reason,
        SessionRecoveryReason::RuntimeHealthDegraded
    );

    let retired = case(
        "closed",
        &[SUBSTRATE_DEGRADED_LABEL],
        1,
        Some(PodLiveness::Live),
    );
    assert_eq!(retired.state, SessionRecoveryState::Retired);
    assert_eq!(retired.reason, SessionRecoveryReason::TriggerClosed);

    let invalid = case(
        "open",
        &[SUBSTRATE_INVALID_LABEL, SUBSTRATE_DEGRADED_LABEL],
        1,
        Some(PodLiveness::Live),
    );
    assert_eq!(invalid.state, SessionRecoveryState::Invalid);
    assert_eq!(invalid.reason, SessionRecoveryReason::RegistrationInvalid);

    let rejected = case(
        "open",
        &[SUBSTRATE_CONFIG_REJECTED_LABEL, SUBSTRATE_INVALID_LABEL],
        1,
        Some(PodLiveness::Live),
    );
    assert_eq!(rejected.state, SessionRecoveryState::Invalid);
    assert_eq!(
        rejected.reason,
        SessionRecoveryReason::ConfigurationRejected
    );
}

#[test]
fn validate_repo_segment_accepts_github_charset_only() {
    for good in ["acme", "a.b_c-d", "Repo1"] {
        validate_repo_segment(good, "owner").expect(good);
    }
    for bad in ["", "a b", "a/b", "a\nb", "é"] {
        assert!(
            validate_repo_segment(bad, "owner").is_err(),
            "{bad:?} must be rejected"
        );
    }
}

#[test]
fn devloop_prs_without_a_bot_login_lists_nothing() {
    let pulls = vec![crate::routes::canvas::github::RepoPull {
        number: 1,
        title: "devloop implementation for #8".to_string(),
        html_url: "u".to_string(),
        state: "open".to_string(),
        merged: false,
        author: "fkst-test[bot]".to_string(),
        head_ref: "devloop/issue/acme/site/8/ready-1".to_string(),
    }];
    assert!(devloop_prs(&pulls, None).is_empty());
    let prs = devloop_prs(&pulls, Some("fkst-test[bot]"));
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].work_issue, Some(8));
}
