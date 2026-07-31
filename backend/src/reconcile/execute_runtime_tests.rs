//! Tests for the runtime effect verbs and the lifecycle records they write.
//!
//! The interesting assertions are not "the backend was called" — that is the
//! sibling routing suite — but what the deployment's permanent history says
//! happened, and that a reconcile retry does not turn one transition into a
//! stream of rows.

use std::sync::Arc;

use crate::reconcile::desired::KillReason;
use crate::reconcile::execute::ReconcileCtx;
use crate::reconcile::execute_test_support::test_ctx;
use crate::session_backend::test_support::FakeSessionBackend;

use super::*;

/// A context whose lifecycle records can be read back.
fn audited_ctx(backend: Arc<FakeSessionBackend>) -> (ReconcileCtx, crate::audit::RecordingSink) {
    let (audit, sink) = crate::audit::AuditHandle::recording();
    let mut ctx = test_ctx(backend);
    ctx.audit = audit;
    (ctx, sink)
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
async fn a_kill_records_the_request_and_the_confirmed_absence() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    kill("sess-abc", KillReason::Idle, &ctx).await;
    assert_eq!(
        recorded(&sink),
        vec![
            ("delete_requested".to_string(), Some("idle".to_string())),
            ("deleted".to_string(), Some("idle".to_string())),
        ]
    );
}

#[tokio::test]
async fn an_already_gone_runtime_still_confirms_the_deletion() {
    // `deleted` means confirmed absence; an idempotent no-op confirms it as well
    // as a delete does, and a silent gap here would look like a leaked runtime.
    let backend = Arc::new(FakeSessionBackend::with_stop_not_found());
    let (ctx, sink) = audited_ctx(backend);
    kill("sess-abc", KillReason::TriggerClosed, &ctx).await;
    assert_eq!(
        recorded(&sink),
        vec![
            (
                "delete_requested".to_string(),
                Some("trigger_closed".to_string())
            ),
            ("deleted".to_string(), Some("runtime_not_found".to_string())),
        ]
    );
}

#[tokio::test]
async fn a_failed_delete_records_a_closed_reason_and_no_error_text() {
    let backend = Arc::new(FakeSessionBackend::with_stop_error());
    let (ctx, sink) = audited_ctx(backend);
    kill("sess-abc", KillReason::ConfigChanged, &ctx).await;

    let events = sink.lifecycle_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].action.as_str(), "delete_failed");
    assert_eq!(
        events[1].reason_code.map(|r| r.as_str()),
        Some("backend_unavailable"),
        "a bounded reason code, never the upstream message"
    );
    // The canary: the scripted backend error text must not have reached the record.
    let rendered = format!("{:?}", events[1]);
    assert!(!rendered.contains("scripted"), "{rendered}");
}

#[tokio::test]
async fn a_terminal_cleanup_records_its_own_bounded_reason() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    cleanup_terminal("sess-abc", &ctx).await;
    assert_eq!(
        recorded(&sink),
        vec![
            (
                "delete_requested".to_string(),
                Some("terminal_cleanup".to_string())
            ),
            ("deleted".to_string(), Some("terminal_cleanup".to_string())),
        ]
    );
}

#[tokio::test]
async fn repeated_kills_of_one_incarnation_dedupe_on_the_event_id() {
    // PostHog deduplicates on the UUID, so a reconcile retry of the same effect
    // must derive the same id rather than writing a second transition row.
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    kill("sess-abc", KillReason::Idle, &ctx).await;
    kill("sess-abc", KillReason::Idle, &ctx).await;

    let ids: Vec<String> = sink
        .lifecycle_events()
        .into_iter()
        .filter(|event| event.action.as_str() == "deleted")
        .map(|event| event.event_id.to_string())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
}

#[tokio::test]
async fn two_different_sessions_never_share_a_lifecycle_event_id() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    kill("sess-abc", KillReason::Idle, &ctx).await;
    kill("sess-xyz", KillReason::Idle, &ctx).await;

    let ids: Vec<String> = sink
        .lifecycle_events()
        .into_iter()
        .filter(|event| event.action.as_str() == "deleted")
        .map(|event| event.event_id.to_string())
        .collect();
    assert_ne!(ids[0], ids[1]);
}

#[tokio::test]
async fn an_orphan_kill_asserts_no_attribution_it_cannot_prove() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    kill("sess-abc", KillReason::TriggerClosed, &ctx).await;

    let event = &sink.lifecycle_events()[0];
    assert_eq!(event.attribution.creator_id, None);
    assert_eq!(event.attribution.creator_login, None);
    assert_eq!(event.correlation.repo_full_name, None);
    assert_eq!(
        event.actor.kind.as_str(),
        "system",
        "an autonomous effect is never attributed to a person as its actor"
    );
}
