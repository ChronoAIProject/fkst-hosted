//! Unit tests for the work-issue acknowledgment step (enforcement OFF — the pre-R3
//! legacy path). The renderer is pure; the step runs against a recording fake
//! [`GithubApi`] (so no network is touched) plus a fake [`GithubListing`] whose
//! returned issues (or error) are fixed per construction. Covers: acking an un-acked
//! open work issue, skipping an already-acked one, the no-registration no-op, and
//! swallowing a list/post failure. The R3 authority reject path lives in the sibling
//! [`super::authz_tests`]; the shared harness + fixtures live in
//! [`super::work_ack_test_support`].

use super::work_ack_test_support::*;
use super::{ack_open_work_issues, work_ack_comment, WORK_PICKED_UP_LABEL};
use crate::reconcile::work_authz::WorkAuthz;

// ---- renderer ---------------------------------------------------------------

#[test]
fn renders_session_name_work_label_and_outcome() {
    let body = work_ack_comment("mysession", "fkst-run");
    // Headline names the session verbatim in backticks.
    assert!(body.contains("Picked up by fkst session `mysession`."));
    // Names the work label the pod is working, in backticks.
    assert!(body.contains("`fkst-run` issues"));
    // Sets expectations: progress on this issue + a PR (or linked issues) outcome.
    assert!(body.contains("posts its progress on this issue"));
    assert!(body.contains("pull request"));
    assert!(body.contains("linked issues"));
}

// ---- step (enforcement off) -------------------------------------------------

#[tokio::test]
async fn acks_an_unacked_open_work_issue() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // One open work issue that has NOT been acked yet (only the work label).
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::off(),
    )
    .await;

    // Exactly one comment, carrying the rendered ack for the right issue.
    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one comment");
    assert_eq!(comments[0].2, 5);
    assert!(comments[0].3.contains("Picked up by fkst session `demo`."));

    // Exactly one label add: the durable picked-up latch on that issue.
    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 5);
    assert_eq!(added[0].3, vec![WORK_PICKED_UP_LABEL.to_string()]);
}

#[tokio::test]
async fn skips_an_already_acked_issue() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // The issue already carries the picked-up latch → must be skipped.
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run", WORK_PICKED_UP_LABEL])]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::off(),
    )
    .await;

    assert!(
        api.comments.lock().unwrap().is_empty(),
        "an already-acked issue is not re-commented"
    );
    assert!(
        api.labels_added.lock().unwrap().is_empty(),
        "an already-acked issue is not re-latched"
    );
}

#[tokio::test]
async fn no_op_without_registrations() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[],
        &label_map(&["fkst-run"]),
        &WorkAuthz::off(),
    )
    .await;

    assert_eq!(
        listing.list_calls(),
        0,
        "no registrations means the listing is never even queried"
    );
    assert!(api.comments.lock().unwrap().is_empty());
    assert!(api.labels_added.lock().unwrap().is_empty());
}

#[tokio::test]
async fn swallows_a_listing_failure() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::err();

    // Must not panic/propagate — the failure is logged and skipped.
    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::off(),
    )
    .await;

    assert_eq!(listing.list_calls(), 1, "the list was attempted once");
    assert!(
        api.comments.lock().unwrap().is_empty(),
        "a failed list posts nothing"
    );
    assert!(api.labels_added.lock().unwrap().is_empty());
}

#[tokio::test]
async fn swallows_a_comment_failure_but_still_latches() {
    // Mirrors the announce arm: a best-effort comment failure is swallowed, yet the
    // durable latch is still added so the issue is not endlessly re-processed.
    let api = std::sync::Arc::new(RecordingApi::with_comment_failure());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::off(),
    )
    .await;

    assert!(
        api.comments.lock().unwrap().is_empty(),
        "the comment failed (recorded nothing)"
    );
    let added = api.labels_added.lock().unwrap();
    assert_eq!(
        added.len(),
        1,
        "the latch is added despite the comment failure"
    );
    assert_eq!(added[0].3, vec![WORK_PICKED_UP_LABEL.to_string()]);
}
