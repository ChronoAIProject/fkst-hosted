//! Tests for the facts a lifecycle record carries, and for the two emission
//! shapes (with and without a runtime handle).

use crate::audit::AuditHandle;
use crate::models::RepoRef;
use crate::reconcile::execute_test_support::{registration, test_ctx};
use crate::session_backend::test_support::FakeSessionBackend;
use k8s_openapi::chrono::{DateTime, Utc};
use std::sync::Arc;

use super::*;

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

fn at(epoch_secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(epoch_secs, 0).expect("valid fixed timestamp")
}

fn ctx_with_recorder() -> (ReconcileCtx, crate::audit::RecordingSink) {
    let (audit, sink) = AuditHandle::recording();
    let mut ctx = test_ctx(Arc::new(FakeSessionBackend::default()));
    ctx.audit = audit;
    (ctx, sink)
}

#[test]
fn registration_facts_carry_normalized_attribution_and_correlation() {
    let mut reg = registration();
    reg.creator_login = "Alice".to_string();
    reg.trigger_author_login = "fkst-cloud[bot]".to_string();
    let facts = SessionLifecycleFacts::from_registration(&reg, "cfg-1".to_string());

    assert_eq!(facts.session_id, reg.session_id);
    assert_eq!(facts.installation_id, Some(42));
    assert_eq!(facts.repo_full_name.as_deref(), Some("acme/site"));
    assert_eq!(facts.trigger_issue, Some(7));
    assert_eq!(facts.attribution.creator_login.as_deref(), Some("alice"));
    assert_eq!(
        facts.attribution.trigger_author_login.as_deref(),
        Some("fkst-cloud"),
        "the record carries the same normalized login the runtime is stamped with"
    );
    assert_eq!(facts.incarnation_hint.as_deref(), Some("cfg-1"));
}

#[test]
fn an_assignee_derived_creator_keeps_an_absent_id_on_the_record() {
    let mut reg = registration();
    reg.creator_id = None;
    let facts = SessionLifecycleFacts::from_registration(&reg, "cfg-1".to_string());
    assert_eq!(facts.attribution.creator_id, None);
    assert_eq!(
        facts.attribution.trigger_author_id,
        Some(583231),
        "the trigger author's id stays in its own field and is never promoted to the creator's"
    );
}

#[test]
fn delete_side_facts_state_nothing_they_cannot_know() {
    // An orphan kill has no registration left and its runtime carried no stamp.
    // Guessing the repository owner or the App identity as "the creator" is
    // exactly what `unknown_legacy` exists to prevent — but the REPOSITORY is
    // known at the effect boundary and is always recorded.
    let facts = SessionLifecycleFacts::from_runtime_audit(
        "sess-abc",
        &repo(),
        &RuntimeAudit {
            created_at: Some(at(100)),
            ..RuntimeAudit::default()
        },
    );
    assert_eq!(facts.session_id, "sess-abc");
    assert_eq!(facts.installation_id, None);
    assert_eq!(facts.repo_full_name.as_deref(), Some("acme/site"));
    assert_eq!(facts.trigger_issue, None);
    assert_eq!(facts.attribution, LifecycleAttribution::default());
    assert_eq!(facts.incarnation_hint, None);
    assert_eq!(facts.incarnation.created_at, Some(at(100)));
}

#[tokio::test]
async fn an_emitted_record_carries_the_backend_the_runtime_lives_in() {
    let (ctx, sink) = ctx_with_recorder();
    let facts =
        SessionLifecycleFacts::from_runtime_audit("sess-abc", &repo(), &RuntimeAudit::default());
    emit(
        &ctx,
        LifecycleAction::Deleted,
        &facts,
        Some(LifecycleReason::Idle),
    );

    let events = sink.lifecycle_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, LifecycleAction::Deleted);
    assert_eq!(
        events[0].backend,
        crate::runtime_identity::RuntimeBackendKind::Kubernetes
    );
    assert_eq!(events[0].reason_code, Some(LifecycleReason::Idle));
    assert_eq!(
        events[0].runtime.runtime_id.as_deref(),
        Some("fkst-sess-sess-abc"),
        "a backend that names its runtimes deterministically supplies the handle"
    );
}

#[tokio::test]
async fn a_backend_confirmed_incarnation_outranks_the_deterministic_handle() {
    // A Kubernetes Pod name repeats across a respawn; a sandbox id does not. When
    // the backend hands back a concrete handle it must be the one recorded.
    let (ctx, sink) = ctx_with_recorder();
    let facts = SessionLifecycleFacts::from_registration(&registration(), "cfg-1".to_string())
        .for_incarnation(crate::runtime_identity::RuntimeIncarnation::from_handle(
            "sbx-77",
        ));
    emit(&ctx, LifecycleAction::Created, &facts, None);

    let events = sink.lifecycle_events();
    assert_eq!(events[0].runtime.runtime_id.as_deref(), Some("sbx-77"));
}

#[tokio::test]
async fn two_incarnations_of_one_session_never_share_an_event_id() {
    let (ctx, sink) = ctx_with_recorder();
    for created_at in [100, 900] {
        let facts = SessionLifecycleFacts::from_runtime_audit(
            "sess-abc",
            &repo(),
            &RuntimeAudit {
                created_at: Some(at(created_at)),
                ..RuntimeAudit::default()
            },
        );
        emit(&ctx, LifecycleAction::Deleted, &facts, None);
    }

    let events = sink.lifecycle_events();
    assert_ne!(
        events[0].event_id, events[1].event_id,
        "the deterministic handle repeats across a respawn; the creation instant does not"
    );
}

#[test]
fn a_metadata_rejection_is_never_reported_as_an_unavailable_backend() {
    // The two need completely different operator responses, and the raw upstream
    // message can quote the rejected value, so it is never a substitute.
    assert_eq!(
        failure_reason(&crate::session_backend::BackendError::InvalidMetadata),
        LifecycleReason::InvalidMetadata
    );
    assert_eq!(
        failure_reason(&crate::session_backend::BackendError::Other(
            anyhow::anyhow!("connection reset")
        )),
        LifecycleReason::BackendUnavailable
    );
    assert_eq!(
        failure_reason(&crate::session_backend::BackendError::NotFound),
        LifecycleReason::RuntimeNotFound
    );
}

#[tokio::test]
async fn a_pending_create_record_omits_the_runtime_handle() {
    let (ctx, sink) = ctx_with_recorder();
    let facts = SessionLifecycleFacts::from_registration(&registration(), "cfg-1".to_string());
    emit_pending_create(&ctx, LifecycleAction::CreateRequested, &facts, None);

    let events = sink.lifecycle_events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].runtime.runtime_id, None,
        "a runtime that does not exist yet must not be named as though it did"
    );
    assert_eq!(events[0].runtime.incarnation_hint.as_deref(), Some("cfg-1"));
}

#[tokio::test]
async fn a_request_and_its_outcome_are_two_rows_with_two_ids() {
    let (ctx, sink) = ctx_with_recorder();
    let facts = SessionLifecycleFacts::from_registration(&registration(), "cfg-1".to_string());
    emit_pending_create(&ctx, LifecycleAction::CreateRequested, &facts, None);
    emit(&ctx, LifecycleAction::Created, &facts, None);

    let events = sink.lifecycle_events();
    assert_eq!(events.len(), 2);
    assert_ne!(events[0].event_id, events[1].event_id);
}

#[test]
fn every_kill_reason_maps_to_a_closed_lifecycle_reason() {
    assert_eq!(kill_reason(KillReason::Idle), LifecycleReason::Idle);
    assert_eq!(
        kill_reason(KillReason::ConfigChanged),
        LifecycleReason::ConfigChanged
    );
    assert_eq!(
        kill_reason(KillReason::TriggerClosed),
        LifecycleReason::TriggerClosed
    );
}
