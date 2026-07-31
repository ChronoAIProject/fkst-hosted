//! Emission of sandbox lifecycle audit records at the reconciler's effect
//! boundary (issue #5673, epic `AUD-05`).
//!
//! It lives beside the executor rather than inside it for the same reason
//! [`super::session_contexts`] does: a change here changes what the deployment's
//! permanent history says happened, which deserves to be reviewed on its own,
//! and the executor's job is effects, not analytics.
//!
//! ## Actor and principal
//!
//! Every record here is an AUTONOMOUS effect, so the actor is `system` and the
//! principal is the App installation that executed it (or the reconciler itself
//! when no installation applies). The human creator and trigger author still
//! ride the record — as attribution, never as the actor — because a lifecycle
//! row that claimed a person "did" a reconcile-driven delete would be a lie that
//! an incident review would act on.
//!
//! ## Where the incarnation discriminator comes from
//!
//! Neither the session id nor (on Kubernetes) the Pod name distinguishes a
//! session's second runtime from its first: both are derived from the trigger
//! issue and repeat verbatim across a kill/respawn. So every effect site supplies
//! the discriminator it actually has:
//!
//! - a CREATE that succeeded carries the backend-confirmed
//!   [`RuntimeIncarnation`] — the OpenSandbox sandbox id, or the Kubernetes
//!   Pod's `creationTimestamp`;
//! - a DELETE carries the observed runtime's `created_at`, captured by the
//!   planner in [`RuntimeAudit`] before the runtime went away;
//! - a create REQUEST (and a create that failed) has no runtime at all, so it
//!   keys on the session's runtime config hash: retries of one spawn dedupe,
//!   and a spawn of a changed configuration does not. Two spawns of the SAME
//!   configuration therefore share a `create_requested` row; the `created` rows
//!   that follow stay distinct, and those are what a timeline reads.
//!
//! See [`crate::audit::lifecycle`] for how the discriminator becomes the event
//! id.

use crate::audit::event::ServiceIdentity;
use crate::audit::identity::AuditIdentity;
use crate::audit::lifecycle::{
    LifecycleAction, LifecycleAttribution, LifecycleCorrelation, LifecycleReason, LifecycleRuntime,
    SandboxLifecycleV1,
};
use crate::models::RepoRef;
use crate::reconcile::desired::{KillReason, RuntimeAudit, SessionRegistration};
use crate::reconcile::execute::ReconcileCtx;
use crate::runtime_identity::RuntimeIncarnation;
use crate::session_backend::BackendError;

/// The identifying facts a lifecycle record needs about one session, gathered
/// once so the effect sites stay one line each.
pub(crate) struct SessionLifecycleFacts {
    pub session_id: String,
    pub installation_id: Option<i64>,
    pub repo_full_name: Option<String>,
    pub trigger_issue: Option<i64>,
    pub attribution: LifecycleAttribution,
    /// Discriminator for effects with no runtime at all (the session's runtime
    /// config hash).
    pub incarnation_hint: Option<String>,
    /// The concrete runtime this effect concerns, when the caller knows which
    /// incarnation it is.
    pub incarnation: RuntimeIncarnation,
}

impl SessionLifecycleFacts {
    /// Everything a currently parsed registration knows.
    pub(crate) fn from_registration(reg: &SessionRegistration, incarnation_hint: String) -> Self {
        let identity = crate::runtime_identity::RuntimeIdentityMetadata::new(
            reg.creator_id,
            &reg.creator_login,
            reg.trigger_author_id,
            &reg.trigger_author_login,
        );
        Self {
            session_id: reg.session_id.clone(),
            installation_id: Some(reg.installation_id),
            repo_full_name: Some(format!("{}/{}", reg.repo.owner, reg.repo.name)),
            trigger_issue: Some(reg.trigger_issue),
            attribution: LifecycleAttribution {
                creator_id: identity.creator_id,
                creator_login: non_empty(identity.creator_login),
                trigger_author_id: Some(identity.trigger_author_id),
                trigger_author_login: non_empty(identity.trigger_author_login),
            },
            incarnation_hint: Some(incarnation_hint),
            incarnation: RuntimeIncarnation::default(),
        }
    }

    /// The DELETE-side view: whatever the planner captured about the runtime
    /// being removed, plus the repository the effect is executing for.
    ///
    /// `repo` is always known at the effect boundary — the executor is driving
    /// one repository's actions — so a deletion is never less filterable by
    /// repository than the creation it undoes. Attribution is whatever the
    /// registration or the runtime's own stamp supplied and is never invented.
    pub(crate) fn from_runtime_audit(
        session_id: &str,
        repo: &RepoRef,
        audit: &RuntimeAudit,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            installation_id: audit.installation_id,
            repo_full_name: Some(format!("{}/{}", repo.owner, repo.name)),
            trigger_issue: audit.trigger_issue,
            attribution: LifecycleAttribution {
                creator_id: audit.creator_id,
                creator_login: audit.creator_login.clone(),
                trigger_author_id: audit.trigger_author_id,
                trigger_author_login: audit.trigger_author_login.clone(),
            },
            incarnation_hint: None,
            incarnation: RuntimeIncarnation {
                runtime_id: None,
                created_at: audit.created_at,
            },
        }
    }

    /// Pin the concrete runtime an effect concerns (a backend-confirmed create,
    /// or a live runtime an identity decision was taken against).
    pub(crate) fn for_incarnation(mut self, incarnation: RuntimeIncarnation) -> Self {
        self.incarnation = incarnation;
        self
    }
}

/// Build and submit one lifecycle record.
///
/// Submission is best-effort by design: an audit record must never rewrite the
/// outcome of the effect it describes. A drop is counted and logged by
/// [`crate::audit::AuditHandle::submit_lifecycle`], so it is visible without
/// being fatal.
pub(crate) fn emit(
    ctx: &ReconcileCtx,
    action: LifecycleAction,
    facts: &SessionLifecycleFacts,
    reason: Option<LifecycleReason>,
) {
    let runtime = LifecycleRuntime {
        // The backend's deterministic handle NAMES the runtime, which is worth
        // recording, but it repeats across incarnations — `created_at` is what
        // separates them, so both ride the record.
        runtime_id: facts
            .incarnation
            .runtime_id
            .clone()
            .or_else(|| ctx.backend.deterministic_runtime_id(&facts.session_id)),
        created_at: facts.incarnation.created_at,
        incarnation_hint: facts.incarnation_hint.clone(),
    };
    submit(ctx, action, facts, reason, runtime);
}

/// Emit a `create_requested`/`create_failed` record, whose runtime handle must
/// NOT be used as the incarnation key.
///
/// These name a runtime that does not exist yet, so keying them on the
/// deterministic Pod name would make two spawns of one session share a row even
/// when they are genuinely different incarnations. The config-hash hint is the
/// honest discriminator; [`emit`] already prefers a handle when there is one, so
/// this variant suppresses it explicitly.
pub(crate) fn emit_pending_create(
    ctx: &ReconcileCtx,
    action: LifecycleAction,
    facts: &SessionLifecycleFacts,
    reason: Option<LifecycleReason>,
) {
    let runtime = LifecycleRuntime {
        runtime_id: None,
        created_at: None,
        incarnation_hint: facts.incarnation_hint.clone(),
    };
    submit(ctx, action, facts, reason, runtime);
}

fn submit(
    ctx: &ReconcileCtx,
    action: LifecycleAction,
    facts: &SessionLifecycleFacts,
    reason: Option<LifecycleReason>,
    runtime: LifecycleRuntime,
) {
    let mut event = SandboxLifecycleV1::new(
        action,
        ctx.backend.backend_kind(),
        facts.session_id.clone(),
        AuditIdentity::reconciler(facts.installation_id),
        service_identity(ctx),
    )
    .with_runtime(runtime)
    .with_attribution(facts.attribution.clone())
    .with_correlation(LifecycleCorrelation {
        repo_full_name: facts.repo_full_name.clone(),
        installation_id: facts.installation_id,
        trigger_issue: facts.trigger_issue,
        // Autonomous reconcile effects are not caused by one audited request, so
        // the field stays absent rather than carrying a fabricated id. It is
        // part of the contract for the day an API/webhook call drives a runtime
        // effect directly.
        request_id: None,
    });
    if let Some(reason) = reason {
        event = event.with_reason(reason);
    }
    let _ = ctx.audit.submit_lifecycle(event);
}

/// The closed reason code a kill carries.
pub(crate) fn kill_reason(reason: KillReason) -> LifecycleReason {
    match reason {
        KillReason::Idle => LifecycleReason::Idle,
        KillReason::ConfigChanged => LifecycleReason::ConfigChanged,
        KillReason::TriggerClosed => LifecycleReason::TriggerClosed,
    }
}

/// The closed reason code a failed backend effect carries.
///
/// A metadata rejection is permanent and caused by a value WE tried to write, so
/// it must not be reported as the backend being unavailable: the two need
/// completely different operator responses, and the raw upstream message is
/// never a substitute (it may quote the rejected value).
pub(crate) fn failure_reason(error: &BackendError) -> LifecycleReason {
    match error {
        BackendError::InvalidMetadata => LifecycleReason::InvalidMetadata,
        BackendError::NotFound => LifecycleReason::RuntimeNotFound,
        BackendError::Other(_) => LifecycleReason::BackendUnavailable,
    }
}

/// The emitting deployment, from the same configuration the HTTP middleware
/// stamps, so both contracts name one deployment identically.
fn service_identity(ctx: &ReconcileCtx) -> ServiceIdentity {
    ServiceIdentity {
        version: ctx.config.audit.service_version.clone(),
        environment: ctx.config.audit.environment.clone(),
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
#[path = "lifecycle_audit_tests.rs"]
mod tests;
