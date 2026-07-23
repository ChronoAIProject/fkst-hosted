use std::sync::Arc;

use super::work_ack_test_support::*;
use super::{
    ack_open_work_issues, ack_open_work_issues_with_bot, work_unauthorized_comment,
    WORK_PICKED_UP_LABEL, WORK_UNAUTHORIZED_LABEL,
};

#[test]
fn unauthorized_renderer_names_creator_collaborator_and_admin_tiers() {
    let body = work_unauthorized_comment("mallory", "demo", "alice", 42);
    assert!(body.contains("@mallory"));
    assert!(body.contains("fkst session `demo`"));
    assert!(body.contains("creator** (@alice)"));
    assert!(body.contains("Session Collaborators"));
    assert!(body.contains("fkst administrators"));
    assert!(body.contains("trigger issue (#42)"));
}

#[tokio::test]
async fn routed_unauthorized_author_is_rejected_label_first() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 99, "mallory", &["alice"])]);
    ack_open_work_issues(
        &tokens(api.clone()),
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &one_label_map(&["fkst-run"]),
        &access(""),
    )
    .await;

    assert_eq!(*api.events.lock().unwrap(), ["label", "comment"]);
    assert_eq!(
        api.labels_added.lock().unwrap()[0].3,
        vec![WORK_UNAUTHORIZED_LABEL.to_string()]
    );
    assert!(!api.labels_added.lock().unwrap()[0]
        .3
        .contains(&WORK_PICKED_UP_LABEL.to_string()));
    assert!(api.comments.lock().unwrap()[0].3.contains("@mallory"));
}

#[tokio::test]
async fn unauthorized_latch_prevents_duplicate_feedback() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue_by(
        5,
        &["fkst-run", WORK_UNAUTHORIZED_LABEL],
        99,
        "mallory",
        &["alice"],
    )]);
    ack_open_work_issues(
        &tokens(api.clone()),
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &one_label_map(&["fkst-run"]),
        &access(""),
    )
    .await;
    assert!(api.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_unauthorized_latch_never_posts_an_undeduped_comment() {
    let api = Arc::new(RecordingApi::with_label_failure());
    let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 99, "mallory", &["alice"])]);
    ack_open_work_issues(
        &tokens(api.clone()),
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &one_label_map(&["fkst-run"]),
        &access(""),
    )
    .await;
    assert!(api.comments.lock().unwrap().is_empty());
}

#[tokio::test]
async fn collaborator_and_global_admin_authors_are_acked() {
    for (author_id, author_login, admins, collaborator) in [
        (99, "bob", "", true),
        (4242, "renamed-admin", "4242", false),
        (99, "deploy-admin", "Deploy-Admin", false),
    ] {
        let api = Arc::new(RecordingApi::default());
        let listing = FakeListing::ok(vec![issue_by(
            5,
            &["fkst-run"],
            author_id,
            author_login,
            &["alice"],
        )]);
        let mut reg = registration("demo", "fkst-run");
        if collaborator {
            reg.collaborators.push("Bob".to_string());
        }
        ack_open_work_issues(
            &tokens(api.clone()),
            &listing,
            &token(),
            &repo(),
            &[reg],
            &one_label_map(&["fkst-run"]),
            &access(admins),
        )
        .await;
        assert_eq!(
            api.labels_added.lock().unwrap()[0].3,
            vec![WORK_PICKED_UP_LABEL.to_string()],
            "{author_login}"
        );
    }
}

#[tokio::test]
async fn repo_admin_and_log_viewer_are_not_authority_tiers() {
    for login in ["repo-owner", "log-viewer"] {
        let api = Arc::new(RecordingApi::default());
        let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 99, login, &["alice"])]);
        let mut reg = registration("demo", "fkst-run");
        reg.log_access.push("log-viewer".to_string());
        ack_open_work_issues(
            &tokens(api.clone()),
            &listing,
            &token(),
            &repo(),
            &[reg],
            &one_label_map(&["fkst-run"]),
            &access(""),
        )
        .await;
        assert_eq!(
            api.labels_added.lock().unwrap()[0].3,
            vec![WORK_UNAUTHORIZED_LABEL.to_string()],
            "{login}"
        );
    }
}

#[tokio::test]
async fn stale_unauthorized_latch_clears_before_authorized_ack() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue_by(
        5,
        &["fkst-run", WORK_UNAUTHORIZED_LABEL],
        7,
        "alice",
        &["alice"],
    )]);
    ack_open_work_issues(
        &tokens(api.clone()),
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &one_label_map(&["fkst-run"]),
        &access(""),
    )
    .await;
    assert_eq!(
        api.labels_removed.lock().unwrap()[0].3,
        WORK_UNAUTHORIZED_LABEL
    );
    assert_eq!(
        api.labels_added.lock().unwrap()[0].3,
        vec![WORK_PICKED_UP_LABEL.to_string()]
    );
}

#[tokio::test]
async fn configured_app_child_clears_unauthorized_latch_and_is_acked() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue_by(
        5,
        &["fkst-run", WORK_UNAUTHORIZED_LABEL],
        9000,
        "app/FKST-App",
        &["alice"],
    )]);
    ack_open_work_issues_with_bot(
        &tokens(api.clone()),
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &one_label_map(&["fkst-run"]),
        &access(""),
        Some("fkst-app[bot]"),
    )
    .await;

    assert_eq!(
        api.labels_removed.lock().unwrap()[0].3,
        WORK_UNAUTHORIZED_LABEL
    );
    assert_eq!(
        api.labels_added.lock().unwrap()[0].3,
        vec![WORK_PICKED_UP_LABEL.to_string()]
    );
}
