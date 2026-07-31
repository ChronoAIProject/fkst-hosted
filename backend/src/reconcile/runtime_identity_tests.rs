//! The backfill sweep: when it calls the backend, when it deliberately does not,
//! and what a conflict or a permanent failure costs on the sweep after it.

use std::sync::Arc;

use k8s_openapi::chrono::Utc;

use crate::audit::lifecycle::LifecycleAction;
use crate::audit::{AuditHandle, RecordingSink};
use crate::reconcile::desired::LivePod;
use crate::reconcile::execute_test_support::{registration, test_ctx};
use crate::runtime_identity::{
    read as read_identity, stamp_pairs, ObservedRuntimeIdentity, RuntimeIdentityMetadata,
    RuntimeIdentityOutcome, K8S_IDENTITY_KEYS,
};
use crate::session_backend::test_support::FakeSessionBackend;

use super::*;

fn ctx_with(backend: Arc<FakeSessionBackend>) -> (ReconcileCtx, RecordingSink) {
    let (audit, sink) = AuditHandle::recording();
    let mut ctx = test_ctx(backend);
    ctx.audit = audit;
    (ctx, sink)
}

fn pod(session_id: &str, liveness: PodLiveness, identity: ObservedRuntimeIdentity) -> LivePod {
    LivePod {
        session_id: session_id.to_string(),
        trigger_issue: 7,
        liveness,
        created_at: Utc::now(),
        last_pending_at: None,
        config_hash: None,
        work_labels: Vec::new(),
        identity,
    }
}

/// The stamp a runtime launched by the CURRENT registration would carry.
fn settled_identity(reg: &SessionRegistration) -> ObservedRuntimeIdentity {
    let desired = RuntimeIdentityMetadata::new(
        reg.creator_id,
        &reg.creator_login,
        reg.trigger_author_id,
        &reg.trigger_author_login,
    );
    let metadata: std::collections::BTreeMap<String, String> =
        stamp_pairs(&K8S_IDENTITY_KEYS, &desired)
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();
    read_identity(&K8S_IDENTITY_KEYS, &metadata)
}

#[tokio::test]
async fn a_settled_runtime_is_never_patched_on_any_sweep() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = ctx_with(backend.clone());
    let reg = registration();
    let live = [pod(
        &reg.session_id,
        PodLiveness::Live,
        settled_identity(&reg),
    )];

    // Three sweeps, exactly as the reconciler would run them.
    for _ in 0..3 {
        backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;
    }

    assert!(
        backend.identity_calls.lock().unwrap().is_empty(),
        "a complete stamp must cost no backend call, not even a read"
    );
    assert!(sink.lifecycle_events().is_empty());
}

#[tokio::test]
async fn a_legacy_runtime_is_backfilled_from_the_current_registration() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, sink) = ctx_with(backend.clone());
    let reg = registration();
    let live = [pod(
        &reg.session_id,
        PodLiveness::Live,
        ObservedRuntimeIdentity::default(),
    )];

    backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;

    let calls = backend.identity_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, reg.session_id);
    assert_eq!(calls[0].1.creator_login, "author-login");
    assert_eq!(calls[0].1.trigger_author_id, 583231);

    let events = sink.lifecycle_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, LifecycleAction::IdentityBackfilled);
    assert_eq!(events[0].session_id, reg.session_id);
}

#[tokio::test]
async fn a_backfilled_session_is_not_re_patched_on_the_next_sweep() {
    // The runtime's observation is still the PRE-patch one until the next list,
    // so without the settle cooldown the same patch would run again.
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, _sink) = ctx_with(backend.clone());
    let reg = registration();
    let live = [pod(
        &reg.session_id,
        PodLiveness::Live,
        ObservedRuntimeIdentity::default(),
    )];

    backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;
    backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;

    assert_eq!(backend.identity_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_conflict_is_recorded_once_and_then_suppressed() {
    let backend = Arc::new(
        FakeSessionBackend::default().with_identity_outcome(RuntimeIdentityOutcome::Conflict),
    );
    let (ctx, sink) = ctx_with(backend.clone());
    let reg = registration();
    let live = [pod(
        &reg.session_id,
        PodLiveness::Live,
        ObservedRuntimeIdentity::default(),
    )];

    for _ in 0..5 {
        backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;
    }

    assert_eq!(
        backend.identity_calls.lock().unwrap().len(),
        1,
        "a conflict cannot resolve itself, so re-deciding it every sweep is pure spam"
    );
    let events = sink.lifecycle_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, LifecycleAction::IdentityConflict);
    assert_eq!(
        events[0].reason_code,
        Some(crate::audit::lifecycle::LifecycleReason::AttributionConflict)
    );
}

#[tokio::test]
async fn a_backend_failure_is_retried_only_after_its_bounded_cooldown() {
    let backend = Arc::new(FakeSessionBackend::default().with_identity_error());
    let (ctx, sink) = ctx_with(backend.clone());
    let reg = registration();
    let live = [pod(
        &reg.session_id,
        PodLiveness::Live,
        ObservedRuntimeIdentity::default(),
    )];

    for _ in 0..4 {
        backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;
    }

    assert_eq!(backend.identity_calls.lock().unwrap().len(), 1);
    assert!(
        sink.lifecycle_events().is_empty(),
        "a failure to decide is not a decision, so it emits no identity transition"
    );
}

#[tokio::test]
async fn a_vanished_runtime_emits_nothing_and_is_not_suppressed_forever() {
    let backend = Arc::new(
        FakeSessionBackend::default().with_identity_outcome(RuntimeIdentityOutcome::NotFound),
    );
    let (ctx, sink) = ctx_with(backend.clone());
    let reg = registration();
    let live = [pod(
        &reg.session_id,
        PodLiveness::Live,
        ObservedRuntimeIdentity::default(),
    )];

    backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;
    backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;

    assert_eq!(
        backend.identity_calls.lock().unwrap().len(),
        2,
        "a disappeared runtime is a benign race, not a permanent condition to park"
    );
    assert!(sink.lifecycle_events().is_empty());
}

#[tokio::test]
async fn a_stale_observation_that_the_backend_finds_complete_emits_nothing() {
    let backend = Arc::new(
        FakeSessionBackend::default().with_identity_outcome(RuntimeIdentityOutcome::Unchanged),
    );
    let (ctx, sink) = ctx_with(backend.clone());
    let reg = registration();
    let live = [pod(
        &reg.session_id,
        PodLiveness::Live,
        ObservedRuntimeIdentity::default(),
    )];

    backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;

    assert_eq!(backend.identity_calls.lock().unwrap().len(), 1);
    assert!(
        sink.lifecycle_events().is_empty(),
        "no decision was taken, so no identity transition happened"
    );
}

#[tokio::test]
async fn a_runtime_with_no_matching_registration_is_left_unknown_legacy() {
    // The orphan branch already kills it; guessing its creator from the
    // repository would be exactly the invention the epic forbids.
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, _sink) = ctx_with(backend.clone());
    let live = [pod(
        "sess-orphan",
        PodLiveness::Live,
        ObservedRuntimeIdentity::default(),
    )];

    backfill_runtime_identities(&ctx, &[registration()], &live).await;

    assert!(backend.identity_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_starting_or_terminating_runtime_is_left_alone() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, _sink) = ctx_with(backend.clone());
    let reg = registration();
    for liveness in [
        PodLiveness::Starting,
        PodLiveness::Terminating,
        PodLiveness::Terminal,
    ] {
        let live = [pod(
            &reg.session_id,
            liveness,
            ObservedRuntimeIdentity::default(),
        )];
        backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;
    }
    assert!(
        backend.identity_calls.lock().unwrap().is_empty(),
        "a runtime that is starting will stamp itself, and one that is going away is not worth a patch"
    );
}

#[tokio::test]
async fn an_assignee_derived_creator_is_backfilled_without_borrowing_an_id() {
    let backend = Arc::new(FakeSessionBackend::default());
    let (ctx, _sink) = ctx_with(backend.clone());
    let mut reg = registration();
    reg.creator_id = None;
    reg.creator_login = "assignee".to_string();
    let live = [pod(
        &reg.session_id,
        PodLiveness::Live,
        ObservedRuntimeIdentity::default(),
    )];

    backfill_runtime_identities(&ctx, std::slice::from_ref(&reg), &live).await;

    let calls = backend.identity_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1.creator_id, None);
    assert_eq!(calls[0].1.creator_login, "assignee");
    assert_eq!(calls[0].1.trigger_author_id, 583231);
}
