//! The CREATE-side runtime effects: spawning a session's runtime, and rebuilding
//! the credential bundle of one that is already live.
//!
//! Split out of [`super::execute`] so the executor file stays the action router
//! plus its GitHub issue effects. What lives here is everything that can bring a
//! runtime into existence — which is exactly the set of effects that must write a
//! paired lifecycle record (a request before the backend call, a concrete outcome
//! after it), so that a create which never returns still leaves evidence it was
//! attempted.
//!
//! Both verbs keep the executor's discipline: every gate that fails posts issue
//! feedback (or logs) and skips the spawn, never leaving a partial runtime, and
//! nothing here ever propagates an error to the caller.

use crate::audit::lifecycle::LifecycleAction;
use crate::disposable_environment::DISPOSABLE_ENVIRONMENT_MARKER;
use crate::github_app::GithubAppError;
use crate::reconcile::branches::DEFAULT_TARGET_BRANCH;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::execute::{flag_invalid, post_comment_best_effort, ReconcileCtx};
use crate::reconcile::execute_comments::invalid_refs_comment;
use crate::reconcile::execute_launch_spec::{
    resolve_session_credentials, CredentialResolutionError,
};
use crate::reconcile::lifecycle_audit::{self, SessionLifecycleFacts};
use crate::reconcile::reachability;
use crate::session_backend::EnsureOutcome;
use secrecy::ExposeSecret;

/// Spawn a session pod for a desired-but-absent registration: reachability →
/// environment → token → pod. Any gate that fails posts issue feedback and skips
/// the spawn (never a partial pod). `detected_work_labels` is the session's FULL
/// effective work-label set — the reconciler rejects a label-less session upstream, so
/// this is always non-empty here (the pod's comma-joined work label is built from it).
pub(crate) async fn spawn_session(
    reg: SessionRegistration,
    detected_work_labels: Vec<String>,
    ctx: &ReconcileCtx,
) {
    let owner_repo = format!("{}/{}", reg.repo.owner, reg.repo.name);

    // 1. Reachability: every EFFECTIVE package ref (explicit ∪ manifest-expanded, I7)
    //    must resolve on public GitHub. A failure flags the trigger issue (comment +
    //    latch label) and skips the spawn. The probe is authenticated with the repo's
    //    installation token (best-effort mint; falls back to unauthenticated) so a large
    //    package closure across repeated reconciles rides the 5000/hour token budget, not
    //    the 60/hour per-IP one.
    let reach_token = ctx.github.token_for_repo(&owner_repo, None).await.ok();
    if let Err(bad) = reachability::check_reachable(
        &reg.effective_packages,
        &ctx.http,
        &ctx.config.github_api_base_url,
        reach_token.as_ref().map(|t| t.expose_secret()),
    )
    .await
    {
        tracing::info!(
            session_id = %reg.session_id,
            unreachable = bad.len(),
            "reconcile spawn: package refs unreachable; flagging invalid, not spawning"
        );
        flag_invalid(
            &ctx.github,
            &owner_repo,
            reg.trigger_issue,
            &invalid_refs_comment(&bad),
        )
        .await;
        return;
    }

    // Provision the target ref before constructing a runtime. This is retried on
    // every spawn pass, never resets an existing target, and deliberately emits
    // logs only: the spawn path has no durable comment-dedupe latch.
    let Some(branches) = ensure_branch_topology(&reg, ctx).await else {
        return;
    };

    // 2-5. Rebuild the launch spec + complete credential bundle from authoritative
    // sources. The same resolver drives live-runtime recovery, so spawn and restart
    // healing cannot drift to different credential layouts.
    let (spec, creds) = match resolve_session_credentials(
        &reg,
        &detected_work_labels,
        &branches,
        ctx,
    )
    .await
    {
        Ok(ready) => ready,
        Err(CredentialResolutionError::EnvironmentBlocked { comment }) => {
            post_comment_best_effort(&ctx.github, &owner_repo, reg.trigger_issue, &comment).await;
            return;
        }
        Err(CredentialResolutionError::TokenMintFailed(error)) => {
            tracing::error!(session_id = %reg.session_id, error = %error, "reconcile spawn: token mint failed; not spawning");
            return;
        }
        Err(CredentialResolutionError::WorkLabelsInvalid(error)) => {
            tracing::error!(session_id = %reg.session_id, error = %error, "reconcile spawn: effective work labels invalid; not spawning");
            return;
        }
    };

    // 6. Ensure the session runtime exists (409 = already-live no-op). The backend
    //    builds + creates the pod and its owner-referenced creds Secret; on failure it
    //    has already logged the specific error, so an `Err` here is a swallowed no-op.
    //    The lifecycle record is written on BOTH sides of this call: the request
    //    before it, the concrete outcome after it, so a create that never returns
    //    still leaves evidence that it was attempted.
    let facts = SessionLifecycleFacts::from_registration(&reg, spec.config_hash.clone());
    lifecycle_audit::emit_pending_create(ctx, LifecycleAction::CreateRequested, &facts, None);
    match ctx.backend.ensure_session(&spec, creds).await {
        Ok(EnsureOutcome::Created(incarnation)) => {
            complete_disposable_handoff(&reg, ctx);
            // `created` only once a concrete runtime exists — which is exactly
            // what `Created` means. The backend-confirmed incarnation is what
            // keeps a respawn of this same session from deduplicating into its
            // predecessor's row.
            lifecycle_audit::emit(
                ctx,
                LifecycleAction::Created,
                &SessionLifecycleFacts::from_registration(&reg, spec.config_hash.clone())
                    .for_incarnation(incarnation),
                None,
            );
            tracing::info!(session_id = %spec.session_id, owner = %reg.repo.owner, "reconcile spawn: session pod created")
        }
        Ok(EnsureOutcome::AlreadyLive) => {
            complete_disposable_handoff(&reg, ctx);
            // Deliberately NOT a `created` record: nothing was created, and an
            // idempotent no-op observed on every pending sweep is not a
            // transition. The `create_requested` above already deduplicates to
            // one row per configuration.
            tracing::info!(session_id = %spec.session_id, "reconcile spawn: session pod already live (no-op)")
        }
        Err(error) => lifecycle_audit::emit_pending_create(
            ctx,
            LifecycleAction::CreateFailed,
            &facts,
            Some(lifecycle_audit::failure_reason(&error)),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedBranchTopology {
    pub(crate) upstream: String,
    pub(crate) integration: String,
}

/// Resolve the source/upstream branch and ensure the target/integration branch
/// exists. The source remains load-bearing after target creation: github-devloop
/// promotes completed integration work back into it.
pub(crate) async fn ensure_branch_topology(
    reg: &SessionRegistration,
    ctx: &ReconcileCtx,
) -> Option<ResolvedBranchTopology> {
    let owner_repo = format!("{}/{}", reg.repo.owner, reg.repo.name);
    let upstream = match &reg.def.source_branch {
        Some(source) => source.clone(),
        None => match ctx.github.repo_default_branch(&owner_repo).await {
            Ok(source) => source,
            Err(error) => {
                tracing::warn!(
                    session_id = %reg.session_id,
                    error = %error,
                    "reconcile spawn: default source branch lookup failed; retrying next pass"
                );
                return None;
            }
        },
    };
    let integration = reg
        .def
        .target_branch
        .clone()
        .unwrap_or_else(|| DEFAULT_TARGET_BRANCH.to_string());
    let topology = ResolvedBranchTopology {
        upstream,
        integration,
    };

    match ctx
        .github
        .branch_head_sha(&owner_repo, &topology.integration)
        .await
    {
        Ok(Some(_)) => return Some(topology),
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(
                session_id = %reg.session_id,
                target_branch = %topology.integration,
                error = %error,
                "reconcile spawn: target branch lookup failed; retrying next pass"
            );
            return None;
        }
    }

    let source_sha = match ctx
        .github
        .branch_head_sha(&owner_repo, &topology.upstream)
        .await
    {
        Ok(Some(sha)) => sha,
        Ok(None) => {
            tracing::warn!(
                session_id = %reg.session_id,
                source_branch = %topology.upstream,
                "reconcile spawn: source branch disappeared before target provisioning; retrying next pass"
            );
            return None;
        }
        Err(error) => {
            tracing::warn!(
                session_id = %reg.session_id,
                source_branch = %topology.upstream,
                error = %error,
                "reconcile spawn: source branch lookup failed; retrying next pass"
            );
            return None;
        }
    };
    match ctx
        .github
        .create_ref(&owner_repo, &topology.integration, &source_sha)
        .await
    {
        Ok(()) | Err(GithubAppError::RefExists) => Some(topology),
        Err(error) => {
            tracing::warn!(
                session_id = %reg.session_id,
                source_branch = %topology.upstream,
                target_branch = %topology.integration,
                error = %error,
                "reconcile spawn: target branch creation failed; retrying next pass"
            );
            None
        }
    }
}

/// Rebuild and adopt the complete credential bundle for a live pending runtime.
/// `ensure_session` remains the single idempotent backend boundary: Kubernetes keeps
/// its already-live no-op, while OpenSandbox uses the supplied bundle to reconstruct
/// its process-local cache and restore a missing sentinel without replacing the
/// deterministically identified runtime.
pub(crate) async fn recover_credentials(
    reg: SessionRegistration,
    detected_work_labels: Vec<String>,
    ctx: &ReconcileCtx,
) {
    let session_id = reg.session_id.clone();
    match ctx.backend.credential_recovery_needed(&session_id).await {
        Ok(false) => return,
        Ok(true) => {}
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "reconcile credential recovery: need probe failed; retrying next reconcile"
            );
            return;
        }
    }
    // A backend may report recovery needed and then discover the runtime vanished
    // at `ensure_session`. Apply the same branch precondition as the ordinary
    // spawn path before that race can recreate a runtime.
    let Some(branches) = ensure_branch_topology(&reg, ctx).await else {
        return;
    };
    let (spec, creds) = match resolve_session_credentials(
        &reg,
        &detected_work_labels,
        &branches,
        ctx,
    )
    .await
    {
        Ok(ready) => ready,
        Err(CredentialResolutionError::EnvironmentBlocked { .. }) => {
            tracing::warn!(
                session_id = %session_id,
                "reconcile credential recovery: environment unavailable; retrying next reconcile"
            );
            return;
        }
        Err(CredentialResolutionError::TokenMintFailed(error)) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "reconcile credential recovery: token mint failed; retrying next reconcile"
            );
            return;
        }
        Err(CredentialResolutionError::WorkLabelsInvalid(error)) => {
            tracing::error!(
                session_id = %session_id,
                error = %error,
                "reconcile credential recovery: effective work labels invalid"
            );
            return;
        }
    };

    // This call can RECREATE a runtime that vanished, so it is a create effect
    // and gets the same paired records the spawn path does — otherwise a
    // recreate that dies mid-call would leave no evidence it was attempted.
    // Both records key on the config hash, so the sweep-after-sweep repetition
    // this action is capable of collapses into one row per configuration rather
    // than becoming a poll log.
    let facts = SessionLifecycleFacts::from_registration(&reg, spec.config_hash.clone());
    lifecycle_audit::emit_pending_create(ctx, LifecycleAction::CreateRequested, &facts, None);
    match ctx.backend.ensure_session(&spec, creds).await {
        Ok(EnsureOutcome::Created(incarnation)) => {
            complete_disposable_handoff(&reg, ctx);
            // The recovery path normally ADOPTS an existing runtime, which emits
            // no `created`; `Created` here means the runtime had actually
            // vanished and this call recreated it, which is a real transition.
            lifecycle_audit::emit(
                ctx,
                LifecycleAction::Created,
                &SessionLifecycleFacts::from_registration(&reg, spec.config_hash.clone())
                    .for_incarnation(incarnation),
                None,
            );
            tracing::warn!(
                session_id = %session_id,
                "reconcile credential recovery: runtime vanished after observation; recreated with complete credentials"
            )
        }
        Ok(EnsureOutcome::AlreadyLive) => {
            complete_disposable_handoff(&reg, ctx);
            tracing::debug!(
                session_id = %session_id,
                "reconcile credential recovery: complete credential bundle adopted"
            )
        }
        Err(error) => {
            lifecycle_audit::emit_pending_create(
                ctx,
                LifecycleAction::CreateFailed,
                &facts,
                Some(lifecycle_audit::failure_reason(&error)),
            );
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "reconcile credential recovery: backend ensure failed; retrying next reconcile"
            )
        }
    }
}

/// The transient registry is only an API-to-reconciler handoff. Once the
/// backend confirms it accepted the complete bundle, the sandbox/backend owns
/// the material and the control plane forgets it.
fn complete_disposable_handoff(reg: &SessionRegistration, ctx: &ReconcileCtx) {
    if reg.def.environment.as_deref() != Some(DISPOSABLE_ENVIRONMENT_MARKER) {
        return;
    }
    let Some(creator_id) = reg.creator_id else {
        return;
    };
    if ctx.disposable_environments.remove(
        &reg.repo.owner,
        &reg.repo.name,
        reg.trigger_issue,
        creator_id,
    ) {
        tracing::info!(
            session_id = %reg.session_id,
            "reconcile spawn: disposable environment handoff consumed"
        );
    }
}

#[cfg(test)]
#[path = "execute_spawn_tests.rs"]
mod tests;
