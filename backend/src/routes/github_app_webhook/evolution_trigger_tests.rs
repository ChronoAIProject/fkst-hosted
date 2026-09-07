//! Relevance and nudging for the Evolution webhook classes.
//!
//! The predicates are tested directly because they encode the spec's event table,
//! and the enqueue paths are tested through a real channel so "relevant" and
//! "actually nudged" cannot drift apart.

use super::*;
use crate::config::Config;
use crate::reconcile::{reconcile_channel, ReconcileDispatcher, ReconcileHandle};

fn state(reconciler: Option<ReconcileHandle>) -> AppState {
    AppState {
        config: Config::default(),
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: reconciler.map(|handle| ReconcileDispatcher::from_handle(&handle)),
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        operations: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: crate::state::empty_self_router(),
        chat: None,
        audit: Default::default(),
    }
}

fn repo_json(default_branch: Option<&str>) -> serde_json::Value {
    let mut repo = serde_json::json!({ "owner": { "login": "acme" }, "name": "site" });
    if let Some(branch) = default_branch {
        repo["default_branch"] = serde_json::json!(branch);
    }
    repo
}

fn push_body(git_ref: &str, default_branch: Option<&str>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "ref": git_ref,
        "repository": repo_json(default_branch),
        "installation": { "id": 42 }
    }))
    .expect("serialize")
}

fn pr_body(action: &str, base: &str, merged: bool, default_branch: Option<&str>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "action": action,
        "pull_request": { "number": 412, "base": { "ref": base }, "merged": merged },
        "repository": repo_json(default_branch),
        "installation": { "id": 42 }
    }))
    .expect("serialize")
}

fn release_body(action: &str, tag: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "action": action,
        "release": { "tag_name": tag },
        "repository": repo_json(Some("develop")),
        "installation": { "id": 42 }
    }))
    .expect("serialize")
}

fn repository_body(action: &str, default_branch_changed: bool) -> Vec<u8> {
    let mut body = serde_json::json!({
        "action": action,
        "repository": repo_json(Some("develop")),
        "installation": { "id": 42 }
    });
    if default_branch_changed {
        body["changes"] = serde_json::json!({ "default_branch": { "from": "main" } });
    } else {
        body["changes"] = serde_json::json!({ "description": { "from": "x" } });
    }
    serde_json::to_vec(&body).expect("serialize")
}

const ACME_SITE: (i64, &str, &str) = (42, "acme", "site");

fn expect_enqueued(rx: &mut tokio::sync::mpsc::Receiver<(i64, RepoRef)>) {
    let got = rx.try_recv().expect("one key enqueued");
    assert_eq!(
        got,
        (
            ACME_SITE.0,
            RepoRef {
                owner: ACME_SITE.1.to_string(),
                name: ACME_SITE.2.to_string()
            }
        )
    );
}

// ---- ref parsing -----------------------------------------------------------

#[test]
fn branch_of_ref_accepts_heads_and_rejects_tags() {
    assert_eq!(branch_of_ref("refs/heads/develop"), Some("develop"));
    assert_eq!(
        branch_of_ref("refs/heads/release/v1.2"),
        Some("release/v1.2")
    );
    // A tag push can never be the default branch; filtering it here stops a
    // tag literally named after the branch from matching.
    assert_eq!(branch_of_ref("refs/tags/v1.0"), None);
    assert_eq!(branch_of_ref("refs/notes/commits"), None);
}

// ---- push ------------------------------------------------------------------

#[test]
fn push_is_relevant_only_on_the_current_default_branch() {
    assert!(push_is_relevant("refs/heads/develop", Some("develop")));
    assert!(!push_is_relevant("refs/heads/feature/x", Some("develop")));
    assert!(!push_is_relevant("refs/tags/v1.0", Some("develop")));
}

#[test]
fn push_without_a_default_branch_nudges_rather_than_drops() {
    // A spurious reconcile is cheap and idempotent; a dropped one leaves the
    // trusted head uncovered until the next full resync.
    assert!(push_is_relevant("refs/heads/anything", None));
}

#[tokio::test]
async fn a_default_branch_push_enqueues() {
    let (handle, mut rx) = reconcile_channel(8);
    let st = state(Some(handle));
    let handled = classify_push(&st, &push_body("refs/heads/develop", Some("develop")))
        .await
        .expect("ok");
    assert_eq!(handled.as_str(), "reconciled");
    expect_enqueued(&mut rx);
}

#[tokio::test]
async fn a_topic_branch_push_is_ignored_and_does_not_enqueue() {
    let (handle, mut rx) = reconcile_channel(8);
    let st = state(Some(handle));
    let handled = classify_push(&st, &push_body("refs/heads/feature/x", Some("develop")))
        .await
        .expect("ok");
    assert_eq!(handled.as_str(), "ignored");
    assert!(rx.try_recv().is_err());
}

// ---- pull_request ----------------------------------------------------------

#[test]
fn pull_request_relevance_matches_the_specified_actions() {
    for action in [
        "opened",
        "reopened",
        "synchronize",
        "ready_for_review",
        "edited",
        "closed",
    ] {
        assert!(
            pull_request_is_relevant(action, "develop", Some("develop")),
            "{action} must be relevant"
        );
    }
    for action in ["assigned", "labeled", "review_requested", "locked", ""] {
        assert!(
            !pull_request_is_relevant(action, "develop", Some("develop")),
            "{action} must be irrelevant"
        );
    }
}

#[test]
fn a_pull_request_against_another_base_is_irrelevant() {
    // Evolution observes the trusted branch; a PR onto a release branch is not
    // its business.
    assert!(!pull_request_is_relevant(
        "opened",
        "release/v1",
        Some("develop")
    ));
}

#[tokio::test]
async fn a_merged_pull_request_enqueues_the_same_hint_as_its_push() {
    // The merge produces BOTH events. Both must converge on one repository hint
    // so the pair cannot become two work items.
    let (handle, mut rx) = reconcile_channel(8);
    let st = state(Some(handle));

    classify_pull_request(&st, &pr_body("closed", "develop", true, Some("develop")))
        .await
        .expect("ok");
    expect_enqueued(&mut rx);

    classify_push(&st, &push_body("refs/heads/develop", Some("develop")))
        .await
        .expect("ok");
    expect_enqueued(&mut rx);
}

// ---- release ---------------------------------------------------------------

#[test]
fn only_a_published_release_is_relevant() {
    assert!(release_is_relevant("published", "v1.2.0"));
    for action in ["created", "edited", "deleted", "prereleased", "released"] {
        assert!(!release_is_relevant(action, "v1.2.0"), "{action}");
    }
}

#[test]
fn an_evolution_owned_release_never_triggers_a_rebuild() {
    // Without this the two-phase publication protocol re-triggers the very cycle
    // that produced it: rebuild -> publish -> rebuild.
    assert!(!release_is_relevant(
        "published",
        "fkst-evolution/0123456789abcdef"
    ));
    // A tag that merely mentions the name is not ours — the prefix is exact.
    assert!(release_is_relevant("published", "v1-fkst-evolution/x"));
}

#[tokio::test]
async fn a_product_release_enqueues_and_an_evolution_release_does_not() {
    let (handle, mut rx) = reconcile_channel(8);
    let st = state(Some(handle));

    let handled = classify_release(&st, &release_body("published", "v1.2.0"))
        .await
        .expect("ok");
    assert_eq!(handled.as_str(), "reconciled");
    expect_enqueued(&mut rx);

    let handled = classify_release(
        &st,
        &release_body("published", "fkst-evolution/0123456789abcdef"),
    )
    .await
    .expect("ok");
    assert_eq!(handled.as_str(), "ignored");
    assert!(rx.try_recv().is_err());
}

// ---- repository ------------------------------------------------------------

#[test]
fn a_repository_event_matters_only_when_the_default_branch_moved() {
    let changed = RepositoryChanges {
        default_branch: Some(serde_json::json!({ "from": "main" })),
    };
    let other = RepositoryChanges {
        default_branch: None,
    };
    assert!(repository_is_relevant("edited", Some(&changed)));
    assert!(!repository_is_relevant("edited", Some(&other)));
    assert!(!repository_is_relevant("edited", None));
    assert!(!repository_is_relevant("privatized", Some(&changed)));
}

#[tokio::test]
async fn a_default_branch_change_enqueues() {
    let (handle, mut rx) = reconcile_channel(8);
    let st = state(Some(handle));
    let handled = classify_repository(&st, &repository_body("edited", true))
        .await
        .expect("ok");
    assert_eq!(handled.as_str(), "reconciled");
    expect_enqueued(&mut rx);

    let handled = classify_repository(&st, &repository_body("edited", false))
        .await
        .expect("ok");
    assert_eq!(handled.as_str(), "ignored");
    assert!(rx.try_recv().is_err());
}

// ---- shared behaviour ------------------------------------------------------

#[tokio::test]
async fn without_a_reconciler_every_class_is_acknowledged_not_enqueued() {
    let st = state(None);
    for handled in [
        classify_push(&st, &push_body("refs/heads/develop", Some("develop")))
            .await
            .expect("ok"),
        classify_pull_request(&st, &pr_body("opened", "develop", false, Some("develop")))
            .await
            .expect("ok"),
        classify_release(&st, &release_body("published", "v1.0.0"))
            .await
            .expect("ok"),
        classify_repository(&st, &repository_body("edited", true))
            .await
            .expect("ok"),
    ] {
        assert_eq!(handled.as_str(), "ignored");
    }
}

#[tokio::test]
async fn a_malformed_body_is_an_error_the_caller_maps_to_202() {
    let st = state(None);
    assert!(classify_push(&st, b"{").await.is_err());
    assert!(classify_pull_request(&st, b"{}").await.is_err());
    assert!(classify_release(&st, b"[]").await.is_err());
    assert!(classify_repository(&st, b"null").await.is_err());
}
