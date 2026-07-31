//! The per-repository attribution backfill sweep (issue #5673).
//!
//! Every runtime created since this feature landed is stamped at launch. This
//! module exists for the ones that were not: a Pod or sandbox created by an
//! older control plane carries no attribution at all, and the ONLY safe evidence
//! for filling it in is a registration that the current sweep just parsed from
//! the trigger issue that runtime belongs to.
//!
//! Four properties make that safe:
//!
//! - **Evidence, never inference.** A runtime with no matching registration —
//!   its trigger closed, made private, or unparseable — stays
//!   [`AttributionSource::UnknownLegacy`](crate::runtime_identity::AttributionSource::UnknownLegacy).
//!   It is never guessed from the repository owner, the App identity, or the
//!   first collaborator.
//! - **Fill, never rewrite.** The decision is
//!   [`crate::runtime_identity::plan`], which returns a conflict rather than an
//!   overwrite whenever a stamped value disagrees.
//! - **No extra API call in the steady state.** The stamp was already read back
//!   as part of the pass's own runtime observation, so a settled runtime costs
//!   one in-memory comparison and no backend request.
//! - **No unbounded retry.** A conflict or a permanent failure parks the session
//!   in [`IdentityGate`](crate::runtime_identity::IdentityGate) so a sweep every
//!   30 seconds cannot turn one broken runtime into a stream of identical
//!   warnings and lifecycle rows.
//!
//! This is deliberately NOT part of [`crate::reconcile::desired::plan_repo`]:
//! attribution is not a lifecycle decision, it can never spawn or kill anything,
//! and keeping it out of the action enum keeps "what may change a runtime's
//! existence" a short, auditable list.

use std::collections::HashMap;

use crate::audit::lifecycle::{LifecycleAction, LifecycleReason};
use crate::reconcile::desired::{LivePod, PodLiveness, SessionRegistration};
use crate::reconcile::execute::ReconcileCtx;
use crate::reconcile::lifecycle_audit::{self, SessionLifecycleFacts};
use crate::runtime_identity::gate::{PERMANENT_COOLDOWN, SETTLE_COOLDOWN};
use crate::runtime_identity::merge::is_settled;
use crate::runtime_identity::{
    AttributionSource, IdentityOperationResult, ObservedRuntimeIdentity, RuntimeIdentityMetadata,
    RuntimeIdentityOutcome, RuntimeIncarnation,
};

/// Backfill missing attribution on every live runtime this pass matched to a
/// currently parsed registration. Best-effort: every failure is logged and
/// counted, never propagated — attribution must not be able to fail a reconcile.
pub(crate) async fn backfill_runtime_identities(
    ctx: &ReconcileCtx,
    regs: &[SessionRegistration],
    live: &[LivePod],
) {
    let live_by_session: HashMap<&str, &LivePod> = live
        .iter()
        .map(|pod| (pod.session_id.as_str(), pod))
        .collect();

    for reg in regs {
        let Some(pod) = live_by_session.get(reg.session_id.as_str()).copied() else {
            // No runtime to stamp. A registration whose runtime has not spawned
            // yet gets its attribution at launch, not here.
            continue;
        };
        // A runtime that is starting up or already going away is not worth a
        // patch: the first will be stamped by its own create, the second is
        // about to stop existing.
        if !matches!(pod.liveness, PodLiveness::Live) {
            continue;
        }
        let desired = RuntimeIdentityMetadata::new(
            reg.creator_id,
            &reg.creator_login,
            reg.trigger_author_id,
            &reg.trigger_author_login,
        );
        // The steady-state fast path: the stamp read during this pass's own
        // observation already says everything the registration can, so there is
        // nothing to do and no backend call to make.
        if is_settled(&pod.identity, &desired) {
            continue;
        }
        apply_identity(ctx, reg, pod, &desired).await;
    }
}

/// Run one gated identity operation and record what it decided.
async fn apply_identity(
    ctx: &ReconcileCtx,
    reg: &SessionRegistration,
    pod: &LivePod,
    desired: &RuntimeIdentityMetadata,
) {
    let backend = ctx.backend.backend_kind();
    if !ctx.identity_gate.allow(&reg.session_id) {
        ctx.audit
            .record_identity_operation(backend, IdentityOperationResult::Suppressed);
        return;
    }

    // An identity decision belongs to the runtime it was taken against, not to
    // the session id — a respawned session must be able to be backfilled again
    // without its record colliding with its predecessor's.
    let facts = SessionLifecycleFacts::from_registration(reg, reg.config_hash.clone())
        .for_incarnation(RuntimeIncarnation::at(pod.created_at));
    match ctx
        .backend
        .ensure_runtime_identity(&reg.session_id, desired)
        .await
    {
        Ok(outcome) => {
            ctx.audit.record_identity_operation(backend, outcome.into());
            match outcome {
                RuntimeIdentityOutcome::Backfilled => {
                    // Park briefly: only long enough that a stale observation
                    // from this same pass cannot re-drive the same patch.
                    ctx.identity_gate.suppress(&reg.session_id, SETTLE_COOLDOWN);
                    lifecycle_audit::emit(ctx, LifecycleAction::IdentityBackfilled, &facts, None);
                    tracing::info!(
                        session_id = %reg.session_id,
                        // The runtime now says `backfilled_current_trigger`, and
                        // this line says the same thing, because a legacy
                        // trigger may have changed since launch: what was
                        // written is current evidence, never original evidence.
                        attribution_source = AttributionSource::BackfilledCurrentTrigger.as_str(),
                        "reconcile identity: backfilled legacy runtime attribution from the current trigger"
                    );
                }
                RuntimeIdentityOutcome::Conflict => {
                    // Nothing but a human editing the trigger (or the runtime
                    // being replaced) can resolve this, so park it for a long
                    // cooldown instead of re-deciding it every sweep.
                    ctx.identity_gate
                        .suppress(&reg.session_id, PERMANENT_COOLDOWN);
                    lifecycle_audit::emit(
                        ctx,
                        LifecycleAction::IdentityConflict,
                        &facts,
                        Some(LifecycleReason::AttributionConflict),
                    );
                    tracing::warn!(
                        session_id = %reg.session_id,
                        attribution_source = observed_source(&pod.identity, true).as_str(),
                        "reconcile identity: runtime attribution disagrees with the current trigger; keeping the stamped values"
                    );
                }
                // Unchanged means the observation was merely stale; NotFound
                // means the runtime vanished between the two. Neither is a
                // transition, so neither emits an event — but a stale-but-settled
                // runtime is still worth naming its source for, since that is
                // the value a later read of the same stamp will report.
                RuntimeIdentityOutcome::Unchanged => tracing::debug!(
                    session_id = %reg.session_id,
                    attribution_source = observed_source(&pod.identity, false).as_str(),
                    "reconcile identity: runtime attribution already complete"
                ),
                RuntimeIdentityOutcome::NotFound => {}
            }
        }
        Err(error) => {
            ctx.audit
                .record_identity_operation(backend, IdentityOperationResult::Failed);
            // A rejected metadata value fails identically on every sweep, and a
            // transport blip is retried by the NEXT sweep after the cooldown —
            // bounded either way, which is what the cooldown buys.
            ctx.identity_gate
                .suppress(&reg.session_id, PERMANENT_COOLDOWN);
            // `reason` is the bounded classification an operator can dashboard
            // on; `error` is the operator-facing detail, carried at the same
            // level and in the same shape as every other backend failure the
            // executor logs. It is deliberately NOT propagated into the
            // lifecycle event, which stays closed reason codes only.
            tracing::warn!(
                session_id = %reg.session_id,
                reason = lifecycle_audit::failure_reason(&error).as_str(),
                error = %error,
                "reconcile identity: attribution patch failed; suppressing retries for a bounded window"
            );
        }
    }
}

/// Classify what a runtime's stamp says about itself, folding in the
/// disagreement the backend just reported.
///
/// The flag is OR-ed, never replaced: the observation may already carry a
/// durable conflict marker written by an earlier pass, and `false` here means
/// only "this pass found no disagreement" — never "the runtime has none".
fn observed_source(observed: &ObservedRuntimeIdentity, conflicting: bool) -> AttributionSource {
    ObservedRuntimeIdentity {
        conflicting: conflicting || observed.conflicting,
        ..observed.clone()
    }
    .attribution_source()
}

#[cfg(test)]
#[path = "runtime_identity_tests.rs"]
mod tests;
