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
    // Empty EFFECTIVE package set → the reachability pre-flight is a no-op (touches no
    // network); no named environment → the env-store read is skipped. Both preconditions
    // then pass and the spawn reaches `ensure_session` (token mint goes via the fake API).
    // Reachability + package_roots read `effective_packages` (I7), so it is the set that
    // must be emptied here.
    reg.def.packages = Vec::new();
    reg.effective_packages = Vec::new();
    reg.def.environment = None;
    let repo = reg.repo.clone();

    execute(
        ReconcileAction::Spawn {
            reg,
            detected_work_labels: vec![],
        },
        &repo,
        &ctx,
    )
    .await;

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
async fn recover_credentials_action_rebuilds_the_full_bundle_through_ensure_session() {
    let backend = Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend.clone());
    let reg = registration();
    let repo = reg.repo.clone();

    execute(
        ReconcileAction::RecoverCredentials {
            reg,
            detected_work_labels: vec!["fkst-run".to_string()],
        },
        &repo,
        &ctx,
    )
    .await;

    let ensured = backend.ensured.lock().unwrap();
    assert_eq!(ensured.len(), 1);
    assert_eq!(ensured[0].0, "sess-abc");
    assert!(ensured[0].1.contains(&"github-token".to_string()));
    assert!(ensured[0].1.contains(&"llm-api-key".to_string()));
}

fn disposable_registration_and_request() -> (
    crate::reconcile::desired::SessionRegistration,
    crate::disposable_environment::DisposableEnvironmentRequest,
) {
    let mut reg = registration();
    reg.def.packages.clear();
    reg.effective_packages.clear();
    reg.def.environment =
        Some(crate::disposable_environment::DISPOSABLE_ENVIRONMENT_MARKER.to_string());
    let request = crate::disposable_environment::DisposableEnvironmentRequest {
        install: vec!["install tool".to_string()],
        variables: std::collections::BTreeMap::from([("APP_MODE".to_string(), "test".to_string())]),
        secrets: std::collections::BTreeMap::from([(
            "DEPLOY_TOKEN".to_string(),
            "secret".to_string(),
        )]),
    };
    (reg, request)
}

#[tokio::test]
async fn successful_disposable_spawn_consumes_the_private_handoff() {
    use crate::disposable_environment::DisposableEnvironmentLookup;

    let backend = Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend.clone());
    let (reg, request) = disposable_registration_and_request();
    ctx.disposable_environments.insert(
        &reg.repo.owner,
        &reg.repo.name,
        reg.trigger_issue,
        reg.creator_id.unwrap(),
        &request,
    );
    let repo = reg.repo.clone();

    execute(
        ReconcileAction::Spawn {
            reg,
            detected_work_labels: vec![],
        },
        &repo,
        &ctx,
    )
    .await;

    let ensured = backend.ensured.lock().unwrap();
    assert_eq!(ensured.len(), 1);
    for key in [
        "install",
        "secret-keys",
        "userenv.APP_MODE",
        "userenv.DEPLOY_TOKEN",
    ] {
        assert!(
            ensured[0].1.iter().any(|actual| actual == key),
            "missing {key}"
        );
    }
    assert!(matches!(
        ctx.disposable_environments
            .resolve("acme", "site", 7, 583231),
        DisposableEnvironmentLookup::Missing
    ));
}

#[tokio::test]
async fn failed_disposable_spawn_retains_the_private_handoff_for_retry() {
    use crate::disposable_environment::DisposableEnvironmentLookup;

    let backend = Arc::new(FakeSessionBackend::with_ensure_error());
    let ctx = test_ctx(backend.clone());
    let (reg, request) = disposable_registration_and_request();
    ctx.disposable_environments.insert(
        &reg.repo.owner,
        &reg.repo.name,
        reg.trigger_issue,
        reg.creator_id.unwrap(),
        &request,
    );
    let repo = reg.repo.clone();

    execute(
        ReconcileAction::Spawn {
            reg,
            detected_work_labels: vec![],
        },
        &repo,
        &ctx,
    )
    .await;

    assert_eq!(backend.ensured.lock().unwrap().len(), 1);
    assert!(matches!(
        ctx.disposable_environments
            .resolve("acme", "site", 7, 583231),
        DisposableEnvironmentLookup::Found(_)
    ));
}

#[tokio::test]
async fn missing_disposable_handoff_blocks_launch_instead_of_using_an_empty_environment() {
    let backend = Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend.clone());
    let (reg, _request) = disposable_registration_and_request();
    let repo = reg.repo.clone();

    execute(
        ReconcileAction::Spawn {
            reg,
            detected_work_labels: vec![],
        },
        &repo,
        &ctx,
    )
    .await;

    assert!(
        backend.ensured.lock().unwrap().is_empty(),
        "a missing private payload must never launch an empty sandbox"
    );
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
