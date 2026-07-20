//! Unit tests for the R3 work-issue AUTHORITY reject surface (epic #572). When the
//! operator opts into enforcement, the ack step gains a reject arm: a work-label
//! issue whose author is NOT authorized to raise work for the session is never
//! picked up — instead it is rejected once (label-first `fkst-unauthorized` latch,
//! then comment). Covers the full-label-set enforcement, the once-only latch, the
//! best-effort failure arms (#4/#5), and the self-heal clear path (#6). Split out of
//! the sibling ack tests ([`super::tests`]) to keep each file under the 500-line
//! limit; the shared harness + fixtures live in [`super::work_ack_test_support`].

use super::work_ack_test_support::*;
use super::{
    ack_open_work_issues, work_unauthorized_comment, WORK_PICKED_UP_LABEL, WORK_UNAUTHORIZED_LABEL,
};
use crate::reconcile::work_authz::WorkAuthz;

// ---- renderer ---------------------------------------------------------------

#[test]
fn renders_unauthorized_comment() {
    let body = work_unauthorized_comment("mallory", "demo", 42);
    // Names the offending author and the session, and points at the trigger issue.
    assert!(body.contains("@mallory is not authorized"));
    assert!(body.contains("`demo`"));
    assert!(body.contains("#42"));
    // States who MAY raise work and that the issue is not picked up.
    assert!(body.contains("author"));
    assert!(body.contains("Session Collaborators"));
    assert!(body.contains("admins / organization owners"));
    assert!(body.contains("will NOT be picked up"));
}

// ---- reject surface ---------------------------------------------------------

#[tokio::test]
async fn rejects_an_unauthorized_author_once_and_never_picks_it_up() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // Enforcement ON, empty admin set. The issue is raised by a stranger (id 999) —
    // not the trigger author (7), not an admin, not a collaborator.
    let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 999, "mallory")]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::enforcing(vec![]),
    )
    .await;

    // Exactly one comment — the rejection — naming the unauthorized author.
    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one (reject) comment");
    assert_eq!(comments[0].2, 5);
    assert!(comments[0].3.contains("@mallory is not authorized"));

    // Exactly one label add — the unauthorized latch — and NEVER the picked-up one.
    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 5);
    assert_eq!(added[0].3, vec![WORK_UNAUTHORIZED_LABEL.to_string()]);
    assert!(
        !added[0].3.iter().any(|l| l == WORK_PICKED_UP_LABEL),
        "an unauthorized issue is never picked up"
    );
}

#[tokio::test]
async fn enforcement_processes_the_full_label_set_not_just_the_explicit_label() {
    // #2: the reject surface must enforce over the session's FULL set (explicit ∪
    // package-discovered), the same set the pending gate authorizes over — so an
    // unauthorized issue on a DISCOVERED label is rejected, not only one on the
    // explicit `### Work Label`.
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue_by(5, &["pkg-label"], 999, "mallory")]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run", "pkg-label"]),
        &WorkAuthz::enforcing(vec![]),
    )
    .await;

    // Both labels were listed (2 calls), the shared issue deduped to one rejection.
    assert_eq!(
        listing.list_calls(),
        2,
        "the full label set is queried under enforcement"
    );
    let added = api.labels_added.lock().unwrap();
    assert_eq!(
        added.len(),
        1,
        "the discovered-label issue is rejected once"
    );
    assert_eq!(added[0].2, 5);
    assert_eq!(added[0].3, vec![WORK_UNAUTHORIZED_LABEL.to_string()]);
}

#[tokio::test]
async fn off_processes_only_the_explicit_label() {
    // #3 regression: flag OFF stays byte-identical to pre-R3 — only the explicit
    // `### Work Label` is processed, never the discovered set.
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run", "pkg-label"]),
        &WorkAuthz::off(),
    )
    .await;

    assert_eq!(
        listing.list_calls(),
        1,
        "flag off queries only the explicit label"
    );
}

#[tokio::test]
async fn already_rejected_issue_is_not_re_rejected() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // The stranger's issue already carries the unauthorized latch → skip entirely.
    let listing = FakeListing::ok(vec![issue_by(
        5,
        &["fkst-run", WORK_UNAUTHORIZED_LABEL],
        999,
        "mallory",
    )]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::enforcing(vec![]),
    )
    .await;

    assert!(
        api.comments.lock().unwrap().is_empty(),
        "an already-rejected issue is not re-commented"
    );
    assert!(
        api.labels_added.lock().unwrap().is_empty(),
        "an already-rejected issue is not re-latched"
    );
}

#[tokio::test]
async fn reject_latches_before_commenting_and_skips_the_comment_on_latch_failure() {
    // #4/#5: the label is the once-only gate, applied FIRST. If the latch write
    // fails, the comment must be SKIPPED (so a later pass can retry both without
    // double-posting) and the issue is NOT picked up.
    let api = std::sync::Arc::new(RecordingApi::with_label_failure());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 999, "mallory")]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::enforcing(vec![]),
    )
    .await;

    assert!(
        api.comments.lock().unwrap().is_empty(),
        "the comment is skipped when the latch write fails (no double-post next pass)"
    );
    assert!(
        api.labels_added.lock().unwrap().is_empty(),
        "the failed latch recorded nothing; the issue is never picked up either"
    );
    assert!(api.labels_removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reject_still_not_picked_up_when_the_comment_fails() {
    // #5: the label latch succeeds (label-first) but the reject comment fails — the
    // issue must still NOT be acked/picked-up, and the latch is in place so it is
    // not endlessly re-processed.
    let api = std::sync::Arc::new(RecordingApi::with_comment_failure());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 999, "mallory")]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::enforcing(vec![]),
    )
    .await;

    assert!(
        api.comments.lock().unwrap().is_empty(),
        "the comment failed (recorded nothing)"
    );
    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "the unauthorized latch still landed");
    assert_eq!(added[0].3, vec![WORK_UNAUTHORIZED_LABEL.to_string()]);
    assert!(
        !added[0].3.iter().any(|l| l == WORK_PICKED_UP_LABEL),
        "never picked up"
    );
}

#[tokio::test]
async fn authorized_author_is_still_acked_under_enforcement() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // Enforcement ON, but the issue is raised by the session's own trigger author
    // (id 7) — so it is acked normally, never rejected.
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::enforcing(vec![]),
    )
    .await;

    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1);
    assert!(comments[0].3.contains("Picked up by fkst session `demo`."));
    let added = api.labels_added.lock().unwrap();
    assert_eq!(added[0].3, vec![WORK_PICKED_UP_LABEL.to_string()]);
}

#[tokio::test]
async fn admin_author_is_acked_under_enforcement() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // The issue is raised by a repo admin (id 500), not the session author.
    let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 500, "octo-admin")]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::enforcing(vec![admin(500, "octo-admin")]),
    )
    .await;

    let added = api.labels_added.lock().unwrap();
    assert_eq!(
        added[0].3,
        vec![WORK_PICKED_UP_LABEL.to_string()],
        "an admin's work issue is picked up, not rejected"
    );
}

#[tokio::test]
async fn empty_admins_still_rejects_a_stranger() {
    // #3: flag ON + admin lookup FAILED this pass (empty admin set) STILL enforces
    // author ∪ collaborators — a stranger is rejected, not waved through.
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 999, "mallory")]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::enforcing(Vec::new()),
    )
    .await;

    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].3, vec![WORK_UNAUTHORIZED_LABEL.to_string()]);
}

#[tokio::test]
async fn self_heals_a_stale_unauthorized_label_when_author_is_now_authorized() {
    // #6: an issue still carrying `fkst-unauthorized` whose author is NOW authorized
    // (admin tier recovered / author became a repo admin) has the stale label CLEARED
    // and is then acked normally — no lingering misleading label.
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue_by(
        5,
        &["fkst-run", WORK_UNAUTHORIZED_LABEL],
        500,
        "octo-admin",
    )]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
        &label_map(&["fkst-run"]),
        &WorkAuthz::enforcing(vec![admin(500, "octo-admin")]),
    )
    .await;

    // The stale unauthorized label is cleared...
    let removed = api.labels_removed.lock().unwrap();
    assert_eq!(
        removed.len(),
        1,
        "the stale unauthorized label is cleared once"
    );
    assert_eq!(removed[0].2, 5);
    assert_eq!(removed[0].3, WORK_UNAUTHORIZED_LABEL);
    // ...and the issue is then acked normally.
    let added = api.labels_added.lock().unwrap();
    assert_eq!(added[0].3, vec![WORK_PICKED_UP_LABEL.to_string()]);
    assert!(api
        .comments
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.3.contains("Picked up by fkst session `demo`.")));
}

#[tokio::test]
async fn enforcement_off_acks_every_author_and_never_rejects() {
    // #3 regression: flag off acks a stranger exactly as pre-R3, and never rejects.
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue_by(5, &["fkst-run"], 999, "mallory")]);

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

    let comments = api.comments.lock().unwrap();
    assert_eq!(
        comments.len(),
        1,
        "the stranger's issue is acked, not rejected"
    );
    assert!(comments[0].3.contains("Picked up by fkst session `demo`."));
    let added = api.labels_added.lock().unwrap();
    assert_eq!(added[0].3, vec![WORK_PICKED_UP_LABEL.to_string()]);
    assert!(
        !added
            .iter()
            .any(|c| c.3.iter().any(|l| l == WORK_UNAUTHORIZED_LABEL)),
        "enforcement off never latches the unauthorized label"
    );
    assert!(api.labels_removed.lock().unwrap().is_empty());
}
