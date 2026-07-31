//! Tests for the runtime effect verbs and the lifecycle records they write.
//!
//! The interesting assertions are not "the backend was called" — that is the
//! sibling routing suite — but what the deployment's permanent history says
//! happened: that a reconcile retry does not turn one transition into a stream
//! of rows, that a NEW incarnation of the same session is nevertheless its own
//! transition, and that a deletion is as correlatable as the creation it undoes.

use std::sync::Arc;

use k8s_openapi::chrono::{DateTime, Utc};

use crate::models::RepoRef;
use crate::reconcile::desired::{KillReason, RuntimeAudit};
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

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

fn at(epoch_secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(epoch_secs, 0).expect("valid fixed timestamp")
}

/// The audit facts a matched registration produces: full correlation plus the
/// observed runtime's creation instant.
fn attributed(created_at: i64) -> RuntimeAudit {
    RuntimeAudit {
        created_at: Some(at(created_at)),
        installation_id: Some(42),
        trigger_issue: Some(7),
        creator_id: Some(4242),
        creator_login: Some("alice".to_string()),
        trigger_author_id: Some(583231),
        trigger_author_login: Some("fkst-cloud".to_string()),
    }
}

/// The audit facts an orphan runtime with NO durable stamp produces.
fn unattributed(created_at: i64) -> RuntimeAudit {
    RuntimeAudit {
        created_at: Some(at(created_at)),
        ..RuntimeAudit::default()
    }
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

/// Every `deleted` row's event id, in order.
fn deleted_ids(sink: &crate::audit::RecordingSink) -> Vec<String> {
    sink.lifecycle_events()
        .into_iter()
        .filter(|event| event.action.as_str() == "deleted")
        .map(|event| event.event_id.to_string())
        .collect()
}

#[tokio::test]
async fn a_kill_records_the_request_and_the_confirmed_absence() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    kill(
        "sess-abc",
        KillReason::Idle,
        &repo(),
        &attributed(100),
        &ctx,
    )
    .await;
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
    kill(
        "sess-abc",
        KillReason::TriggerClosed,
        &repo(),
        &attributed(100),
        &ctx,
    )
    .await;
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
    kill(
        "sess-abc",
        KillReason::ConfigChanged,
        &repo(),
        &attributed(100),
        &ctx,
    )
    .await;

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
    cleanup_terminal("sess-abc", &repo(), &attributed(100), &ctx).await;
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
    kill(
        "sess-abc",
        KillReason::Idle,
        &repo(),
        &attributed(100),
        &ctx,
    )
    .await;
    kill(
        "sess-abc",
        KillReason::Idle,
        &repo(),
        &attributed(100),
        &ctx,
    )
    .await;

    let ids = deleted_ids(&sink);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
}

#[tokio::test]
async fn a_new_incarnation_of_the_same_session_is_a_distinct_transition() {
    // The whole point of the incarnation key. `sess-abc` is derived from its
    // trigger issue and the Kubernetes Pod name is derived from that, so both
    // repeat verbatim after a respawn; only the runtime's creation instant
    // differs. Without it the second deletion would derive the SAME uuid as the
    // first and PostHog would discard it — the timeline would show one runtime
    // where there were two.
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    kill(
        "sess-abc",
        KillReason::Idle,
        &repo(),
        &attributed(100),
        &ctx,
    )
    .await;
    kill(
        "sess-abc",
        KillReason::Idle,
        &repo(),
        &attributed(900),
        &ctx,
    )
    .await;

    let ids = deleted_ids(&sink);
    assert_eq!(ids.len(), 2);
    assert_ne!(
        ids[0], ids[1],
        "two runtimes of one session must never collapse into a single row"
    );
}

#[tokio::test]
async fn two_different_sessions_never_share_a_lifecycle_event_id() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    kill(
        "sess-abc",
        KillReason::Idle,
        &repo(),
        &attributed(100),
        &ctx,
    )
    .await;
    kill(
        "sess-xyz",
        KillReason::Idle,
        &repo(),
        &attributed(100),
        &ctx,
    )
    .await;

    let ids = deleted_ids(&sink);
    assert_ne!(ids[0], ids[1]);
}

#[tokio::test]
async fn a_deletion_carries_the_correlation_the_pass_knew() {
    // A global admin filtering lifecycle history by repository must see the
    // deletions too, not only the creations.
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    kill(
        "sess-abc",
        KillReason::Idle,
        &repo(),
        &attributed(100),
        &ctx,
    )
    .await;

    let event = &sink.lifecycle_events()[0];
    assert_eq!(
        event.correlation.repo_full_name.as_deref(),
        Some("acme/site")
    );
    assert_eq!(event.correlation.installation_id, Some(42));
    assert_eq!(event.correlation.trigger_issue, Some(7));
    assert_eq!(event.attribution.creator_id, Some(4242));
    assert_eq!(event.attribution.creator_login.as_deref(), Some("alice"));
    assert_eq!(
        event.attribution.trigger_author_login.as_deref(),
        Some("fkst-cloud")
    );
    assert_eq!(
        event.actor.kind.as_str(),
        "system",
        "an autonomous effect is never attributed to a person as its actor"
    );
}

#[tokio::test]
async fn an_unstamped_orphan_kill_asserts_no_attribution_it_cannot_prove() {
    // The repository is always known at the effect boundary, so it is recorded.
    // The creator is NOT: an orphan has no registration and an unstamped runtime
    // says nothing about itself, and guessing from the repository owner or the
    // App identity is exactly what `unknown_legacy` exists to prevent.
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = audited_ctx(backend);
    kill(
        "sess-abc",
        KillReason::TriggerClosed,
        &repo(),
        &unattributed(100),
        &ctx,
    )
    .await;

    let event = &sink.lifecycle_events()[0];
    assert_eq!(event.attribution.creator_id, None);
    assert_eq!(event.attribution.creator_login, None);
    assert_eq!(event.attribution.trigger_author_id, None);
    assert_eq!(event.correlation.installation_id, None);
    assert_eq!(event.correlation.trigger_issue, None);
    assert_eq!(
        event.correlation.repo_full_name.as_deref(),
        Some("acme/site"),
        "the repository is known at the effect boundary and is never dropped"
    );
}
