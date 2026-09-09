//! What the CREATE-side effects write into the deployment's permanent history.
//!
//! Kept apart from the routing suite (which asserts that the backend verb was
//! reached) and from the issue-effect suite: these tests are about the audit
//! trail, and specifically about the two things it is easy to get silently
//! wrong — a failure reported with the wrong closed reason, and two runtimes of
//! one session collapsing into a single deduplicated row.

use std::sync::Arc;

use super::*;
use crate::audit::AuditHandle;
use crate::reconcile::execute_test_support::*;

/// A context whose lifecycle records can be read back, spawning through the
/// given backend.
fn audited_ctx(backend: Arc<FakeSessionBackend>) -> (ReconcileCtx, crate::audit::RecordingSink) {
    let (audit, sink) = AuditHandle::recording();
    let mut ctx = test_ctx(backend);
    ctx.audit = audit;
    (ctx, sink)
}

/// A registration whose spawn pre-flights are all no-ops, so `execute` reaches
/// `ensure_session` without touching a network or an environment store.
fn spawnable() -> crate::reconcile::desired::SessionRegistration {
    let mut reg = registration();
    reg.def.packages = Vec::new();
    reg.effective_packages = Vec::new();
    reg.def.environment = None;
    reg
}

async fn spawn_once(ctx: &ReconcileCtx) {
    let reg = spawnable();
    let repo = reg.repo.clone();
    execute(
        ReconcileAction::Spawn {
            reg,
            detected_work_labels: vec![],
        },
        &repo,
        ctx,
    )
    .await;
}

/// The `(action, reason)` pairs recorded, in order.
fn recorded(sink: &crate::audit::RecordingSink) -> Vec<(String, Option<String>)> {
    sink.lifecycle_events()
        .into_iter()
        .map(|event| {
            (
                event.action.as_str().to_string(),
                event.reason_code.map(|r| r.as_str().to_string()),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_spawn_records_the_request_and_the_confirmed_runtime() {
    let (ctx, sink) = audited_ctx(Arc::new(FakeSessionBackend::default()));
    spawn_once(&ctx).await;

    assert_eq!(
        recorded(&sink),
        vec![
            ("create_requested".to_string(), None),
            ("created".to_string(), None),
        ]
    );
    let events = sink.lifecycle_events();
    assert_eq!(
        events[0].runtime.runtime_id, None,
        "a runtime that does not exist yet must not be named as though it did"
    );
    assert_eq!(
        events[1].runtime.runtime_id.as_deref(),
        Some("fake-sess-abc-0"),
        "`created` names the runtime the backend actually confirmed"
    );
    assert_eq!(
        events[1].correlation.repo_full_name.as_deref(),
        Some("acme/site")
    );
}

#[tokio::test]
async fn two_spawns_of_one_session_produce_two_distinct_created_rows() {
    // The session id is derived from the trigger issue and the Kubernetes Pod
    // name from that, so both repeat after a kill/respawn. Only the
    // backend-confirmed incarnation differs — and if it did not reach the event
    // id, PostHog would discard the second creation and the timeline would show
    // one runtime where there were two.
    let (ctx, sink) = audited_ctx(Arc::new(FakeSessionBackend::default()));
    spawn_once(&ctx).await;
    spawn_once(&ctx).await;

    let created: Vec<String> = sink
        .lifecycle_events()
        .into_iter()
        .filter(|event| event.action.as_str() == "created")
        .map(|event| event.event_id.to_string())
        .collect();
    assert_eq!(created.len(), 2);
    assert_ne!(created[0], created[1]);
}

#[tokio::test]
async fn retried_create_requests_for_one_configuration_dedupe() {
    // Before a runtime exists there is nothing runtime-shaped to key on, so the
    // config hash is the honest discriminator: retries of one spawn collapse.
    let (ctx, sink) = audited_ctx(Arc::new(FakeSessionBackend::default()));
    spawn_once(&ctx).await;
    spawn_once(&ctx).await;

    let requested: Vec<String> = sink
        .lifecycle_events()
        .into_iter()
        .filter(|event| event.action.as_str() == "create_requested")
        .map(|event| event.event_id.to_string())
        .collect();
    assert_eq!(requested.len(), 2);
    assert_eq!(requested[0], requested[1]);
}

#[tokio::test]
async fn a_failed_create_records_a_bounded_reason_and_no_error_text() {
    let (ctx, sink) = audited_ctx(Arc::new(FakeSessionBackend::with_ensure_error()));
    spawn_once(&ctx).await;

    assert_eq!(
        recorded(&sink),
        vec![
            ("create_requested".to_string(), None),
            (
                "create_failed".to_string(),
                Some("backend_unavailable".to_string())
            ),
        ]
    );
    let rendered = format!("{:?}", sink.lifecycle_events()[1]);
    assert!(!rendered.contains("scripted"), "{rendered}");
}

#[tokio::test]
async fn a_rejected_metadata_value_is_not_reported_as_an_unavailable_backend() {
    // A label-value rejection is permanent and caused by a value WE tried to
    // write; an operator paged for "the backend is down" would look in entirely
    // the wrong place.
    let (ctx, sink) = audited_ctx(Arc::new(FakeSessionBackend::with_ensure_metadata_rejected()));
    spawn_once(&ctx).await;

    assert_eq!(
        recorded(&sink),
        vec![
            ("create_requested".to_string(), None),
            (
                "create_failed".to_string(),
                Some("invalid_metadata".to_string())
            ),
        ]
    );
}

#[tokio::test]
async fn a_credential_recovery_that_recreates_a_runtime_records_both_sides() {
    // The recovery path can RECREATE a vanished runtime, so it is a create
    // effect: a recreate that dies mid-call must not vanish from the history.
    let (ctx, sink) = audited_ctx(Arc::new(FakeSessionBackend::default()));
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

    assert_eq!(
        recorded(&sink),
        vec![
            ("create_requested".to_string(), None),
            ("created".to_string(), None),
        ]
    );
}

#[tokio::test]
async fn a_failed_credential_recovery_still_records_that_it_was_attempted() {
    let (ctx, sink) = audited_ctx(Arc::new(FakeSessionBackend::with_ensure_error()));
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

    assert_eq!(
        recorded(&sink),
        vec![
            ("create_requested".to_string(), None),
            (
                "create_failed".to_string(),
                Some("backend_unavailable".to_string())
            ),
        ]
    );
}
