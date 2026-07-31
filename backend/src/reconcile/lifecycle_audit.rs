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
//! Create-side effects have no runtime handle yet, so they key on the session's
//! runtime config hash; delete-side effects key on the backend's deterministic
//! runtime handle when it has one. Kubernetes names its Pod from the session id,
//! so its handle is always available; OpenSandbox assigns a sandbox id
//! server-side that `ensure_session` does not return, so its delete rows dedupe
//! per session rather than per incarnation. That limit is honest and bounded —
//! see [`crate::audit::lifecycle`] for the full reasoning.

use crate::audit::event::ServiceIdentity;
use crate::audit::identity::AuditIdentity;
use crate::audit::lifecycle::{
    LifecycleAction, LifecycleAttribution, LifecycleCorrelation, LifecycleReason, LifecycleRuntime,
    SandboxLifecycleV1,
};
use crate::reconcile::desired::{KillReason, SessionRegistration};
use crate::reconcile::execute::ReconcileCtx;

/// The identifying facts a lifecycle record needs about one session, gathered
/// once so the effect sites stay one line each.
pub(crate) struct SessionLifecycleFacts {
    pub session_id: String,
    pub installation_id: Option<i64>,
    pub repo_full_name: Option<String>,
    pub trigger_issue: Option<i64>,
    pub attribution: LifecycleAttribution,
    /// Discriminator for effects with no runtime handle (the session's runtime
    /// config hash).
    pub incarnation_hint: Option<String>,
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
        }
    }

    /// What is known about a session addressed only by id — an orphan kill or a
    /// terminal cleanup, where the registration is already gone. Attribution is
    /// deliberately absent rather than guessed from the repository.
    pub(crate) fn from_session_id(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            installation_id: None,
            repo_full_name: None,
            trigger_issue: None,
            attribution: LifecycleAttribution::default(),
            incarnation_hint: None,
        }
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
        runtime_id: ctx.backend.deterministic_runtime_id(&facts.session_id),
        created_at: None,
        incarnation_hint: facts.incarnation_hint.clone(),
    };
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
        // the field stays absent rather than carrying a fabricated id.
        request_id: None,
    });
    if let Some(reason) = reason {
        event = event.with_reason(reason);
    }
    let _ = ctx.audit.submit_lifecycle(event);
}

/// Emit a `create_requested`/`created`/`create_failed` record whose runtime
/// handle should NOT be used as the incarnation key.
///
/// A `create_requested` names a runtime that does not exist yet, so keying it on
/// the deterministic Pod name would make two spawns of one session share a row
/// even when they are genuinely different incarnations. The config-hash hint is
/// the honest discriminator; [`emit`] already prefers a handle when there is
/// one, so this variant suppresses it explicitly.
pub(crate) fn emit_pending_create(
    ctx: &ReconcileCtx,
    action: LifecycleAction,
    facts: &SessionLifecycleFacts,
    reason: Option<LifecycleReason>,
) {
    let mut event = SandboxLifecycleV1::new(
        action,
        ctx.backend.backend_kind(),
        facts.session_id.clone(),
        AuditIdentity::reconciler(facts.installation_id),
        service_identity(ctx),
    )
    .with_runtime(LifecycleRuntime {
        runtime_id: None,
        created_at: None,
        incarnation_hint: facts.incarnation_hint.clone(),
    })
    .with_attribution(facts.attribution.clone())
    .with_correlation(LifecycleCorrelation {
        repo_full_name: facts.repo_full_name.clone(),
        installation_id: facts.installation_id,
        trigger_issue: facts.trigger_issue,
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
