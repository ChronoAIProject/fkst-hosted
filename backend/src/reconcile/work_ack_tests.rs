use std::sync::Arc;

use super::work_ack_test_support::*;
use super::{
    ack_open_work_issues, work_ack_comment, work_unrouted_comment, WORK_PICKED_UP_LABEL,
    WORK_UNROUTED_LABEL,
};

#[test]
fn ack_renderer_names_session_label_and_outcome() {
    let body = work_ack_comment("mysession", "fkst-run");
    assert!(body.contains("Picked up by fkst session `mysession`."));
    assert!(body.contains("`fkst-run` issues"));
    assert!(body.contains("pull request"));
}

#[test]
fn unrouted_renderer_explains_the_exact_routing_contract() {
    let body = work_unrouted_comment();
    assert!(body.contains("not routed to any session"));
    assert!(body.contains("exactly one assignee"));
    assert!(body.contains("creator of an active fkst session"));
}

#[tokio::test]
async fn routed_authorized_issue_is_acked_once() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);
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

    assert_eq!(api.comments.lock().unwrap().len(), 1);
    assert!(api.comments.lock().unwrap()[0]
        .3
        .contains("Picked up by fkst session `demo`"));
    assert_eq!(
        api.labels_added.lock().unwrap()[0].3,
        vec![WORK_PICKED_UP_LABEL.to_string()]
    );
}

#[tokio::test]
async fn already_acked_issue_is_not_reprocessed() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run", WORK_PICKED_UP_LABEL])]);
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
    assert!(api.labels_added.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_registrations_means_no_listing_or_writes() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);
    ack_open_work_issues(
        &tokens(api.clone()),
        &listing,
        &token(),
        &repo(),
        &[],
        &Default::default(),
        &access(""),
    )
    .await;
    assert_eq!(listing.list_calls(), 0);
    assert!(api.comments.lock().unwrap().is_empty());
}

#[tokio::test]
async fn listing_failure_is_best_effort() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::err();
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
    assert_eq!(listing.list_calls(), 1);
    assert!(api.comments.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ack_comment_failure_still_latches_picked_up() {
    let api = Arc::new(RecordingApi::with_comment_failure());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);
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
    assert_eq!(
        api.labels_added.lock().unwrap()[0].3,
        vec![WORK_PICKED_UP_LABEL.to_string()]
    );
}

#[tokio::test]
async fn label_less_trigger_acks_over_its_full_discovered_label_set() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue(5, &["pkg-work"])]);
    let mut reg = registration("demo", "unused-explicit");
    reg.def.work_label = None;
    ack_open_work_issues(
        &tokens(api.clone()),
        &listing,
        &token(),
        &repo(),
        &[reg],
        &one_label_map(&["pkg-work"]),
        &access(""),
    )
    .await;
    assert!(api.comments.lock().unwrap()[0]
        .3
        .contains("`pkg-work` issues"));
}

#[tokio::test]
async fn each_unrouted_shape_latches_and_comments_once_without_pickup() {
    for (case, assignees) in [
        ("zero", Vec::<&str>::new()),
        ("multiple", vec!["alice", "bob"]),
        ("sessionless", vec!["nobody-active"]),
    ] {
        let api = Arc::new(RecordingApi::default());
        let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 7, "alice", &assignees)]);
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
        assert_eq!(api.comments.lock().unwrap().len(), 1, "{case}");
        assert!(api.comments.lock().unwrap()[0]
            .3
            .contains("not routed to any session"));
        assert_eq!(
            api.labels_added.lock().unwrap()[0].3,
            vec![WORK_UNROUTED_LABEL.to_string()],
            "{case}"
        );
        assert_eq!(*api.events.lock().unwrap(), ["label", "comment"], "{case}");
    }
}

#[tokio::test]
async fn unrouted_latch_dedupes_and_clears_on_correct_assignment() {
    let latched = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue_by(
        5,
        &["fkst-run", WORK_UNROUTED_LABEL],
        7,
        "alice",
        &[],
    )]);
    ack_open_work_issues(
        &tokens(latched.clone()),
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &one_label_map(&["fkst-run"]),
        &access(""),
    )
    .await;
    assert!(latched.comments.lock().unwrap().is_empty());
    assert!(latched.labels_added.lock().unwrap().is_empty());

    let corrected = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue_by(
        5,
        &["fkst-run", WORK_UNROUTED_LABEL, WORK_PICKED_UP_LABEL],
        7,
        "alice",
        &["ALICE"],
    )]);
    ack_open_work_issues(
        &tokens(corrected.clone()),
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &one_label_map(&["fkst-run"]),
        &access(""),
    )
    .await;
    assert_eq!(
        corrected.labels_removed.lock().unwrap()[0].3,
        WORK_UNROUTED_LABEL
    );
}

#[tokio::test]
async fn matching_session_appearing_later_clears_a_parked_issue() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue_by(
        5,
        &["shared", WORK_UNROUTED_LABEL, WORK_PICKED_UP_LABEL],
        8,
        "bob",
        &["bob"],
    )]);
    let bob = registration_for("bob-session", "shared", "bob", Some(8), "sess-bob");
    ack_open_work_issues(
        &tokens(api.clone()),
        &listing,
        &token(),
        &repo(),
        &[bob],
        &label_map(&[("sess-bob", &["shared"])]),
        &access(""),
    )
    .await;
    assert_eq!(api.labels_removed.lock().unwrap()[0].3, WORK_UNROUTED_LABEL);
}

#[tokio::test]
async fn issue_routed_to_another_creator_is_silent_for_this_session() {
    let api = Arc::new(RecordingApi::default());
    let listing = FakeListing::ok(vec![issue_by(5, &["shared"], 8, "bob", &["bob"])]);
    let alice = registration_for("alice-session", "shared", "alice", Some(7), "sess-alice");
    let bob = registration_for("bob-session", "shared", "bob", Some(8), "sess-bob");
    ack_open_work_issues(
        &tokens(api.clone()),
        &listing,
        &token(),
        &repo(),
        &[alice, bob],
        &label_map(&[("sess-alice", &["shared"]), ("sess-bob", &["shared"])]),
        &access(""),
    )
    .await;
    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1);
    assert!(comments[0].3.contains("bob-session"));
    assert!(!comments[0].3.contains("alice-session"));
    assert_eq!(
        listing.list_calls(),
        1,
        "shared label listed once repo-wide"
    );
}
