//! Unit tests for the executor's GitHub issue effects (flag / clear / announce /
//! reject). They run against a recording fake [`GithubApi`] so no network is
//! touched; the action routing through the session backend lives in the sibling
//! [`super::routing_tests`], the create-side audit trail in
//! [`super::lifecycle_tests`], and the shared fakes/builders in
//! [`super::execute_test_support`].

use super::*;
use crate::reconcile::announce::announce_session_comment_with_defaults;
use crate::reconcile::execute_test_support::*;

// ---- GitHub issue effects ---------------------------------------------------

#[tokio::test]
async fn flag_invalid_posts_a_comment_and_latches_the_label() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    flag_invalid(&github, "acme/site", 7, "bad body: fix it").await;

    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one comment");
    assert_eq!(
        comments[0],
        ("acme".into(), "site".into(), 7, "bad body: fix it".into())
    );

    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 7);
    assert_eq!(added[0].3, vec![SUBSTRATE_INVALID_LABEL.to_string()]);
}

#[tokio::test]
async fn announce_session_posts_a_comment_and_latches_the_announced_label() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    let body = announce_session_comment_with_defaults(
        "demo",
        Some("fkst-run"),
        &["fkst-run".to_string()],
        &[],
        None,
        false,
        None,
        "cfg99",
    );
    announce_session(&github, "acme/site", 11, &body).await;

    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one comment");
    assert_eq!(comments[0].2, 11);
    assert!(
        comments[0].3.contains("fkst session `demo` registered."),
        "the posted body is the rendered announcement"
    );
    assert!(
        comments[0].3.contains("<!-- fkst-config-hash: cfg99 -->"),
        "the posted body latches the config-hash marker"
    );

    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 11);
    assert_eq!(added[0].3, vec![SUBSTRATE_ANNOUNCED_LABEL.to_string()]);
}

#[tokio::test]
async fn reject_config_change_posts_a_comment_and_latches_the_label() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    reject_config_change(&github, "acme/site", 13).await;

    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one comment");
    assert_eq!(comments[0].2, 13);
    assert!(
        comments[0]
            .3
            .contains("Config changes are not allowed after a session trigger exists."),
        "the posted body is the rejection feedback"
    );

    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 13);
    assert_eq!(
        added[0].3,
        vec![SUBSTRATE_CONFIG_REJECTED_LABEL.to_string()]
    );
}

#[tokio::test]
async fn clear_invalid_removes_the_label() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    clear_invalid(&github, "acme/site", 9).await;

    let removed = api.labels_removed.lock().unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0],
        (
            "acme".into(),
            "site".into(),
            9,
            SUBSTRATE_INVALID_LABEL.into()
        )
    );
}

#[tokio::test]
async fn trigger_unauthorized_latches_before_posting_the_comment() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    flag_trigger_unauthorized(
        &github,
        "acme/site",
        17,
        &trigger_unauthorized_comment("@alice lacks maintain permission"),
    )
    .await;

    assert_eq!(*api.events.lock().unwrap(), ["label-add", "comment"]);
    assert_eq!(
        api.labels_added.lock().unwrap()[0].3,
        vec![TRIGGER_UNAUTHORIZED_LABEL.to_string()]
    );
    assert!(api.comments.lock().unwrap()[0]
        .3
        .contains("The issue body has not been read"));
}

#[tokio::test]
async fn clear_trigger_unauthorized_removes_the_latch() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    clear_trigger_unauthorized(&github, "acme/site", 19).await;

    assert_eq!(
        api.labels_removed.lock().unwrap()[0],
        (
            "acme".into(),
            "site".into(),
            19,
            TRIGGER_UNAUTHORIZED_LABEL.into()
        )
    );
}
