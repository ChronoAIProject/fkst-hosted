//! Correctness tests for the in-memory [`SessionRepo`] (#198): the atomic CAS,
//! the `pod_id`/`fencing_token` ownership guard (the at-most-one-engine
//! invariant), the narrow `set_run_key`, the bulk sweeps, and concurrency.

use super::*;
use crate::models::RepoRef;
use bson::doc;

/// A minimal pre-terminal session doc for `owner/name`, `pending`, no fence.
fn session(owner: &str, name: &str) -> SessionDoc {
    SessionDoc {
        id: bson::Uuid::new(),
        package_name: "demo".to_string(),
        status: SessionStatus::Pending,
        pod_id: None,
        fencing_token: None,
        pid: None,
        runtime_dir: None,
        error: None,
        run_key: None,
        owner_user_id: None,
        org_id: None,
        package_names: vec!["demo".to_string()],
        goal_id: None,
        repo: Some(RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        }),
        env_scope: None,
        triggered_by: None,
        nyxid_key_id: None,
        nyxid_key_prefix: None,
        ornn_skills: None,
        terminal_cause: None,
        created_at: bson::DateTime::now(),
        started_at: None,
        stopped_at: None,
    }
}

#[tokio::test]
async fn insert_then_get_round_trips_and_duplicate_is_a_conflict() {
    let repo = SessionRepo::new();
    let s = session("acme", "site");
    repo.insert(&s).await.expect("insert");
    let got = repo.get(s.id).await.expect("get").expect("present");
    assert_eq!(got.id, s.id);
    assert_eq!(got.package_name, "demo");
    // A second insert of the same id is a conflict (Mongo unique-_id parity).
    assert!(matches!(repo.insert(&s).await, Err(AppError::Conflict(_))));
    // An absent id reads None.
    assert!(repo.get(bson::Uuid::new()).await.expect("get").is_none());
}

#[tokio::test]
async fn transition_applies_on_status_match_and_misses_otherwise() {
    let repo = SessionRepo::new();
    let s = session("acme", "site");
    repo.insert(&s).await.unwrap();

    // Pending -> Running applies and returns the post-update doc.
    let updated = repo
        .transition(
            s.id,
            &[SessionStatus::Pending],
            doc! { "status": status_bson(SessionStatus::Running), "pid": 1234 },
        )
        .await
        .unwrap()
        .expect("CAS hit");
    assert_eq!(updated.status, SessionStatus::Running);
    assert_eq!(updated.pid, Some(1234));

    // A second Pending->Running misses (status moved on) and changes nothing.
    let miss = repo
        .transition(
            s.id,
            &[SessionStatus::Pending],
            doc! { "status": status_bson(SessionStatus::Failed) },
        )
        .await
        .unwrap();
    assert!(miss.is_none(), "stale `from` must miss");
    assert_eq!(
        repo.get(s.id).await.unwrap().unwrap().status,
        SessionStatus::Running
    );

    // An absent id misses.
    assert!(repo
        .transition(bson::Uuid::new(), &[SessionStatus::Pending], doc! {})
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn fencing_guard_rejects_a_superseded_writer() {
    let repo = SessionRepo::new();
    let mut s = session("acme", "site");
    // Owned by pod-A on fence 5, Running.
    s.status = SessionStatus::Running;
    s.pod_id = Some("pod-A".to_string());
    s.fencing_token = Some(5);
    repo.insert(&s).await.unwrap();

    // pod-A on fence 5 may write (correct owner).
    let ok = repo
        .transition_guarded(
            s.id,
            &[SessionStatus::Running],
            doc! { "pod_id": "pod-A", "fencing_token": 5i64 },
            doc! { "status": status_bson(SessionStatus::Stopping) },
        )
        .await
        .unwrap();
    assert!(ok.is_some(), "the fence-correct owner writes");
    // Re-arm to Running for the next attempts.
    repo.transition(
        s.id,
        &[SessionStatus::Stopping],
        doc! { "status": status_bson(SessionStatus::Running) },
    )
    .await
    .unwrap()
    .unwrap();

    // A superseded writer (stale fence 4) is rejected — the takeover invariant.
    let stale_fence = repo
        .transition_guarded(
            s.id,
            &[SessionStatus::Running],
            doc! { "pod_id": "pod-A", "fencing_token": 4i64 },
            doc! { "status": status_bson(SessionStatus::Failed) },
        )
        .await
        .unwrap();
    assert!(stale_fence.is_none(), "a stale fence must never land");

    // A different pod (wrong owner) is rejected even on the right fence number.
    let wrong_pod = repo
        .transition_guarded(
            s.id,
            &[SessionStatus::Running],
            doc! { "pod_id": "pod-B", "fencing_token": 5i64 },
            doc! { "status": status_bson(SessionStatus::Failed) },
        )
        .await
        .unwrap();
    assert!(wrong_pod.is_none(), "a non-owner must never land");

    // The doc is untouched (still Running, still pod-A/5).
    let now = repo.get(s.id).await.unwrap().unwrap();
    assert_eq!(now.status, SessionStatus::Running);
    assert_eq!(now.fencing_token, Some(5));
}

#[tokio::test]
async fn fencing_guard_on_an_absent_field_never_matches() {
    let repo = SessionRepo::new();
    let s = session("acme", "site"); // pod_id / fencing_token are None
    repo.insert(&s).await.unwrap();
    // A guard for a concrete pod must not match a doc whose pod_id is absent
    // (Mongo equality-on-absent semantics).
    let miss = repo
        .transition_guarded(
            s.id,
            &[SessionStatus::Pending],
            doc! { "pod_id": "pod-A" },
            doc! { "status": status_bson(SessionStatus::Running) },
        )
        .await
        .unwrap();
    assert!(miss.is_none());
}

#[tokio::test]
async fn set_run_key_is_narrow_and_no_ops_when_absent() {
    let repo = SessionRepo::new();
    let s = session("acme", "site");
    repo.insert(&s).await.unwrap();
    repo.set_run_key(s.id, "run-123").await.unwrap();
    let got = repo.get(s.id).await.unwrap().unwrap();
    assert_eq!(got.run_key.as_deref(), Some("run-123"));
    assert_eq!(
        got.status,
        SessionStatus::Pending,
        "set_run_key never touches status"
    );
    // Absent id is a clean no-op (not an error).
    repo.set_run_key(bson::Uuid::new(), "x").await.unwrap();
}

#[tokio::test]
async fn fail_orphans_fails_pre_terminal_and_leaves_terminal_alone() {
    let repo = SessionRepo::new();
    let pending = session("acme", "a");
    let mut running = session("acme", "b");
    running.status = SessionStatus::Running;
    let mut stopped = session("acme", "c");
    stopped.status = SessionStatus::Stopped;
    repo.insert(&pending).await.unwrap();
    repo.insert(&running).await.unwrap();
    repo.insert(&stopped).await.unwrap();

    let count = repo.fail_orphans().await.unwrap();
    assert_eq!(count, 2, "both pre-terminal sessions failed");
    assert_eq!(
        repo.get(pending.id).await.unwrap().unwrap().status,
        SessionStatus::Failed
    );
    assert_eq!(
        repo.get(running.id).await.unwrap().unwrap().status,
        SessionStatus::Failed
    );
    assert_eq!(
        repo.get(stopped.id).await.unwrap().unwrap().status,
        SessionStatus::Stopped,
        "terminal session untouched"
    );
    assert_eq!(
        repo.get(pending.id)
            .await
            .unwrap()
            .unwrap()
            .error
            .as_deref(),
        Some(ORPHANED_ERROR)
    );
}

#[tokio::test]
async fn bulk_fail_scopes_to_repo_and_owner_and_pre_terminal() {
    let repo = SessionRepo::new();
    let a1 = session("acme", "one");
    let a2 = session("acme", "two");
    let other = session("other", "one");
    let mut done = session("acme", "one");
    done.status = SessionStatus::Stopped;
    for s in [&a1, &a2, &other, &done] {
        repo.insert(s).await.unwrap();
    }

    // Repo-scoped: only acme/one, pre-terminal.
    let ids = repo.active_ids_for_repo("acme", "one").await.unwrap();
    assert_eq!(ids, vec![a1.id]);
    let n = repo
        .fail_active_for_repo("acme", "one", "gone")
        .await
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(
        repo.get(a1.id).await.unwrap().unwrap().status,
        SessionStatus::Failed
    );
    assert_eq!(
        repo.get(a2.id).await.unwrap().unwrap().status,
        SessionStatus::Pending
    );

    // Owner-scoped: every remaining pre-terminal acme/* (a2), not other/*.
    let owner_ids = repo.active_ids_for_owner("acme").await.unwrap();
    assert_eq!(owner_ids, vec![a2.id]);
    let m = repo
        .fail_active_for_owner("acme", "suspended")
        .await
        .unwrap();
    assert_eq!(m, 1);
    assert_eq!(
        repo.get(a2.id).await.unwrap().unwrap().status,
        SessionStatus::Failed
    );
    assert_eq!(
        repo.get(other.id).await.unwrap().unwrap().status,
        SessionStatus::Pending
    );
}

/// The load-bearing concurrency property: many tasks racing the SAME
/// Pending->Running CAS — EXACTLY ONE wins, proving the at-most-one-engine
/// invariant survives the in-memory store (the whole read-check-write is atomic).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_transitions_admit_exactly_one_winner() {
    let repo = SessionRepo::new();
    let s = session("acme", "site");
    repo.insert(&s).await.unwrap();

    let mut handles = Vec::new();
    for _ in 0..64 {
        let repo = repo.clone();
        let id = s.id;
        handles.push(tokio::spawn(async move {
            repo.transition(
                id,
                &[SessionStatus::Pending],
                doc! { "status": status_bson(SessionStatus::Running) },
            )
            .await
            .unwrap()
            .is_some()
        }));
    }
    let mut winners = 0;
    for h in handles {
        if h.await.unwrap() {
            winners += 1;
        }
    }
    assert_eq!(
        winners, 1,
        "exactly one CAS may win the Pending->Running race"
    );
    assert_eq!(
        repo.get(s.id).await.unwrap().unwrap().status,
        SessionStatus::Running
    );
}
