//! Tests that the executor routes each [`ReconcileAction`] to the right
//! [`SessionBackend`] verb (and swallows a backend `NotFound`), driven against the
//! recording [`FakeSessionBackend`] in [`super::execute_test_support`]. The pod
//! effects never touch a real cluster — the backend is faked and, for the spawn
//! case, the reachability + env pre-flights are made no-ops (empty packages, no
//! named environment) so `execute` reaches `ensure_session`.

use super::*;
use crate::reconcile::execute_test_support::*;

#[tokio::test]
async fn spawn_action_routes_to_ensure_session() {
    let backend = Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend.clone());
    let mut reg = registration();
    // Empty package set → the reachability pre-flight is a no-op (touches no network);
    // no named environment → the env-store read is skipped. Both preconditions then
    // pass and the spawn reaches `ensure_session` (token mint goes via the fake API).
    reg.def.packages = Vec::new();
    reg.def.environment = None;
    let repo = reg.repo.clone();

    execute(ReconcileAction::Spawn(reg), &repo, &ctx).await;

    let ensured = backend.ensured.lock().unwrap();
    assert_eq!(
        ensured.len(),
        1,
        "spawn routes to ensure_session exactly once"
    );
    assert_eq!(
        ensured[0].0, "sess-abc",
        "the right session spec is ensured"
    );
    // The assembled creds carry at least the github-token + llm-api-key files.
    assert!(ensured[0].1.contains(&"github-token".to_string()));
    assert!(ensured[0].1.contains(&"llm-api-key".to_string()));
}

#[tokio::test]
async fn kill_action_routes_to_stop_session_with_reason() {
    let backend = Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend.clone());

    execute(
        ReconcileAction::Kill {
            session_id: "sess-1".to_string(),
            reason: KillReason::Idle,
        },
        &test_repo(),
        &ctx,
    )
    .await;

    let stopped = backend.stopped.lock().unwrap();
    assert_eq!(stopped.len(), 1, "kill routes to stop_session exactly once");
    assert_eq!(stopped[0].0, "sess-1");
    // The kill reason is threaded through to the backend verbatim.
    assert_eq!(stopped[0].1, KillReason::Idle);
}

#[tokio::test]
async fn cleanup_terminal_action_routes_to_remove_terminal() {
    let backend = Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend.clone());

    execute(
        ReconcileAction::CleanupTerminal {
            session_id: "sess-2".to_string(),
        },
        &test_repo(),
        &ctx,
    )
    .await;

    let removed = backend.removed_terminal.lock().unwrap();
    assert_eq!(removed.as_slice(), &["sess-2".to_string()]);
}

#[tokio::test]
async fn touch_pending_action_routes_to_mark_pending() {
    let backend = Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend.clone());

    execute(
        ReconcileAction::TouchPending {
            session_id: "sess-3".to_string(),
        },
        &test_repo(),
        &ctx,
    )
    .await;

    let marked = backend.marked_pending.lock().unwrap();
    assert_eq!(marked.as_slice(), &["sess-3".to_string()]);
}

#[tokio::test]
async fn touch_pending_swallows_not_found() {
    // The backend returns NotFound (a pod deleted between plan and patch); the
    // executor must swallow it and return normally, never panic or propagate.
    let backend = Arc::new(FakeSessionBackend::with_mark_pending_not_found());
    let ctx = test_ctx(backend.clone());

    execute(
        ReconcileAction::TouchPending {
            session_id: "gone".to_string(),
        },
        &test_repo(),
        &ctx,
    )
    .await;

    let marked = backend.marked_pending.lock().unwrap();
    assert_eq!(
        marked.as_slice(),
        &["gone".to_string()],
        "mark_pending was still invoked (its 404 was swallowed)"
    );
}
