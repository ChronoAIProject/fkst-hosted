//! The action executor: turn one pure [`ReconcileAction`] into its Kubernetes /
//! GitHub effect (issue #359 §4.2/§5.3, PR5b).
//!
//! The planner ([`crate::reconcile::desired::plan_repo`]) is a pure function; this
//! module is its effectful other half. Every effect is IDEMPOTENT and BEST-EFFORT:
//! a spawn is a 409-tolerant create, a kill/cleanup is a 404-tolerant delete, a
//! flag/clear is an additive label + one comment. Nothing here ever panics or
//! aborts the caller — every failure is logged with context and swallowed at THIS
//! boundary so one bad action never stalls the reconcile of the rest of the repo.
//!
//! This file is the ACTION ROUTER plus the GitHub issue effects. The effects that
//! change whether a runtime exists live next door, one file per direction, because
//! each is paired with the lifecycle records that make the change part of the
//! deployment's permanent history:
//!
//! - [`super::execute_spawn`] — create side (spawn, credential recovery), with the
//!   launch arguments it needs assembled in [`super::execute_launch_spec`];
//! - [`super::execute_runtime`] — touch-pending, stop, terminal cleanup.
//!
//! Secret hygiene: the minted installation token is serialized into the
//! `github-token` Secret value and never logged; comments/labels carry only public
//! metadata (the offending refs / the parser's 422 message).

use std::sync::Arc;

use crate::config::Config;
use crate::disposable_environment::DisposableEnvironmentRegistry;
use crate::environment_profile::EnvironmentProfileStore;
use crate::github_app::listing::GithubListing;
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::models::RepoRef;
use crate::reconcile::announce::announce_session_comment;
use crate::reconcile::desired::ReconcileAction;
use crate::reconcile::execute_comments::{
    config_rejected_comment, flag_invalid_comment, trigger_unauthorized_comment,
};
use crate::reconcile::execute_runtime;
use crate::reconcile::execute_spawn::{recover_credentials, spawn_session};
use crate::reconcile::retire::retire_work_issues;
use crate::reconcile::work_labels::apply_work_label_namespace;
use crate::session_backend::SessionBackend;

use super::{
    SUBSTRATE_ANNOUNCED_LABEL, SUBSTRATE_CONFIG_REJECTED_LABEL, SUBSTRATE_INVALID_LABEL,
    TRIGGER_UNAUTHORIZED_LABEL,
};

/// Everything the executor needs, bundled so the per-repo driver + the loops can
/// share ONE cheap-to-clone context: the Kubernetes client, the GitHub App token
/// minter (+ comment/label), the listing transport, an HTTP client for the
/// reachability probe, and the loaded config. Every field is itself cheap to clone
/// (all are `Arc`-backed or handles), so a `ReconcileCtx` is cheap to clone into a
/// per-repo task.
#[derive(Clone)]
pub struct ReconcileCtx {
    /// The session runtime the executor drives every pod effect through (spawn /
    /// mark-pending / stop / cleanup / observe). Backend-neutral: the executor never
    /// touches a concrete Kubernetes type, only this `Arc<dyn SessionBackend>`.
    pub backend: Arc<dyn SessionBackend>,
    /// The environment-profile store the spawn pre-flight reads
    /// (`resolve_environment`). Held behind the backend-agnostic
    /// [`EnvironmentProfileStore`] trait so the storage backend is swappable.
    pub env_store: Arc<dyn EnvironmentProfileStore>,
    /// GitHub App token service: mints the session token + posts comments/labels.
    pub github: GithubAppTokens,
    /// Read-side GitHub transport the driver enumerates issues + counts work with.
    pub listing: Arc<dyn GithubListing>,
    /// Author-and-timestamp-aware comment reads. Separate from `listing` because
    /// only the schedule pass needs comment PROVENANCE, and it needs it for a
    /// security reason: run records live on an issue anyone may comment on, so an
    /// untrusted author's marker must never be read as durable state.
    pub comments: Arc<dyn crate::github_app::comments::IssueCommentReader>,
    /// Unauthenticated HTTP client for the package-ref reachability pre-flight.
    pub http: reqwest::Client,
    /// The loaded control-plane config (pod knobs, reconciler knobs, LLM key).
    pub config: Config,
    /// Repos with ≥1 open trigger registration. [`reconcile_repo`] maintains it and
    /// the sweep re-enqueues each member, so a first-spawn repo (registration but no
    /// pod) is reconciled every sweep instead of only by the slow full-resync.
    pub active_repos: crate::reconcile::ActiveRepos,
    /// Per-repo issue-template ensure gate (mirrors `active_repos`): bounds the
    /// version-aware template reconcile to one GitHub round-trip per repo per
    /// (version, TTL) so it is a cheap no-op on the vast majority of reconciles.
    pub ensured_templates: crate::reconcile::EnsuredTemplates,
    /// The shared `session_id -> log-access context` registry the reconciler upserts
    /// each sweep so the identity-gated log-download endpoint can reverse a
    /// `session_id` to its authorization context. A cheap `Arc`-backed handle.
    pub session_access: crate::session_access::SessionAccessRegistry,
    /// Private create-request handoff for disposable environments. Resolution is
    /// creator-bound and entries are removed only after the backend accepts the
    /// complete sandbox credential bundle.
    pub disposable_environments: DisposableEnvironmentRegistry,
    /// The audit sink every runtime lifecycle transition is recorded through, and
    /// the home of the bounded attribution/lifecycle counters. A no-op handle when
    /// capture is disabled, so an unaudited deployment behaves exactly as before.
    pub audit: crate::audit::AuditHandle,
    /// Bounded suppression of repeated attribution backfills, so a runtime whose
    /// stamp can never be completed costs one decision per cooldown rather than
    /// one per sweep.
    pub identity_gate: crate::runtime_identity::IdentityGate,
}

/// Execute ONE action for the repo it belongs to. Best-effort: logs and swallows
/// every error at this boundary (see the module docs). `repo` scopes the GitHub
/// issue effects (flag/clear); the pod effects address the deterministic
/// `fkst-sess-<session_id>` pod directly.
pub async fn execute(action: ReconcileAction, repo: &RepoRef, ctx: &ReconcileCtx) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    match action {
        // The session's FULL effective work-label set (explicit ∪ package-discovered)
        // becomes the pod's comma-joined work label (epic #594 I4), so a session whose
        // `### Work Label` was omitted still wakes on its packages' auto-declared labels.
        ReconcileAction::Spawn {
            reg,
            detected_work_labels,
        } => spawn_session(reg, detected_work_labels, ctx).await,
        ReconcileAction::TouchPending { session_id } => {
            execute_runtime::touch_pending(&session_id, ctx).await
        }
        ReconcileAction::RecoverCredentials {
            reg,
            detected_work_labels,
        } => recover_credentials(reg, detected_work_labels, ctx).await,
        ReconcileAction::Kill {
            session_id,
            reason,
            audit,
        } => execute_runtime::kill(&session_id, reason, repo, &audit, ctx).await,
        ReconcileAction::CleanupTerminal { session_id, audit } => {
            execute_runtime::cleanup_terminal(&session_id, repo, &audit, ctx).await
        }
        ReconcileAction::RetireWorkIssues { work_labels } => {
            // Retire the still-open work issues across EVERY label the retired session
            // claimed (its full effective set, recovered from the pod annotation). An
            // empty set (a pre-multi-label pod that recorded no label) has none to notify.
            if !work_labels.is_empty() {
                retire_work_issues(&ctx.github, ctx.listing.as_ref(), repo, &work_labels).await
            }
        }
        ReconcileAction::FlagInvalid {
            trigger_issue,
            detail,
        } => {
            flag_invalid(
                &ctx.github,
                &owner_repo,
                trigger_issue,
                &flag_invalid_comment(&detail),
            )
            .await
        }
        ReconcileAction::ClearInvalid { trigger_issue } => {
            clear_invalid(&ctx.github, &owner_repo, trigger_issue).await
        }
        ReconcileAction::FlagTriggerUnauthorized {
            trigger_issue,
            detail,
        } => {
            flag_trigger_unauthorized(
                &ctx.github,
                &owner_repo,
                trigger_issue,
                &trigger_unauthorized_comment(&detail),
            )
            .await
        }
        ReconcileAction::ClearTriggerUnauthorized { trigger_issue } => {
            clear_trigger_unauthorized(&ctx.github, &owner_repo, trigger_issue).await
        }
        ReconcileAction::AnnounceSession {
            trigger_issue,
            session_id,
            session_name,
            work_label,
            // The session's FULL effective work-label set (explicit ∪ package-discovered,
            // I2, epic #594) — rendered into the announcement (I5) so a label-less
            // auto-detect session's discovered labels appear, not just the explicit one.
            detected_work_labels,
            packages,
            package_env,
            environment,
            source_branch,
            target_branch,
            auto_merge,
            creator_login,
            full_config_hash,
        } => {
            let effective_labels = match apply_work_label_namespace(
                &detected_work_labels,
                ctx.config.reconcile.work_label_namespace.as_deref(),
            ) {
                Ok(labels) => labels,
                Err(error) => {
                    tracing::error!(
                        trigger_issue,
                        error = %error,
                        "reconcile announce: effective work-label validation failed"
                    );
                    return;
                }
            };
            let effective_explicit = work_label
                .as_ref()
                .and_then(|logical| effective_labels.logical_to_effective.get(logical).cloned());
            // Build the identity-gated log-download link from the configured public
            // base URL; `None` (unset) omits the log line. The endpoint authorizes
            // every request, so the static URL is safe to post.
            let log_url =
                ctx.config.log.public_base_url.as_ref().map(|base| {
                    format!("{}/api/v1/logs/{}", base.trim_end_matches('/'), session_id)
                });
            let comment = announce_session_comment(
                &session_name,
                effective_explicit.as_deref(),
                &effective_labels.effective,
                &packages,
                environment.as_deref(),
                source_branch.as_deref(),
                &target_branch,
                auto_merge,
                &creator_login,
                // The dashboard block renders only when a frontend URL is set.
                ctx.config.log.frontend_url.as_deref(),
                log_url.as_deref(),
                &full_config_hash,
                &package_env,
            );
            announce_session(&ctx.github, &owner_repo, trigger_issue, &comment).await
        }
        ReconcileAction::RejectConfigChange { trigger_issue } => {
            reject_config_change(&ctx.github, &owner_repo, trigger_issue).await
        }
    }
}

// --- GitHub issue effects (testable against a fake transport) -----------------

/// Flag an invalid trigger issue: post `comment`, then latch the invalid label.
/// Both are best-effort + idempotent (label add is additive; the planner emits
/// this only on the FIRST observation of an invalid issue).
pub(crate) async fn flag_invalid(
    github: &GithubAppTokens,
    owner_repo: &str,
    issue: i64,
    comment: &str,
) {
    post_comment_best_effort(github, owner_repo, issue, comment).await;
    if let Err(error) = github
        .add_issue_labels(
            owner_repo,
            issue as u64,
            &[SUBSTRATE_INVALID_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "reconcile: latch invalid label failed");
    }
}

/// Announce a freshly-registered session: post `comment`, then latch the durable
/// announced label. Both are best-effort + idempotent (the label add is additive;
/// the planner emits this only on the FIRST observation of a not-yet-announced valid
/// registration). Mirrors [`flag_invalid`], minus any clear path (announcements are
/// never un-latched — see [`SUBSTRATE_ANNOUNCED_LABEL`]).
async fn announce_session(github: &GithubAppTokens, owner_repo: &str, issue: i64, comment: &str) {
    post_comment_best_effort(github, owner_repo, issue, comment).await;
    if let Err(error) = github
        .add_issue_labels(
            owner_repo,
            issue as u64,
            &[SUBSTRATE_ANNOUNCED_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "reconcile: latch announced label failed");
    }
}

/// Reject a config edit on an already-triggered issue: post the "config is immutable"
/// feedback, then latch the durable rejected label. Both are best-effort + idempotent
/// (the label add is additive; the planner emits this only on the change TRANSITION).
/// Mirrors [`flag_invalid`]/[`announce_session`], minus any clear path — the only way
/// to change config is to close the session and open a new one.
async fn reject_config_change(github: &GithubAppTokens, owner_repo: &str, issue: i64) {
    post_comment_best_effort(github, owner_repo, issue, &config_rejected_comment()).await;
    if let Err(error) = github
        .add_issue_labels(
            owner_repo,
            issue as u64,
            &[SUBSTRATE_CONFIG_REJECTED_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "reconcile: latch config-rejected label failed");
    }
}

/// Clear the invalid label from an issue that now parses (404-tolerant: the label
/// may already be gone).
async fn clear_invalid(github: &GithubAppTokens, owner_repo: &str, issue: i64) {
    match github
        .remove_issue_label(owner_repo, issue as u64, SUBSTRATE_INVALID_LABEL)
        .await
    {
        Ok(()) => {
            tracing::info!(owner_repo = %owner_repo, issue, "reconcile: cleared invalid flag")
        }
        Err(GithubAppError::NotFound { .. }) => {}
        Err(error) => {
            tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "reconcile: clear invalid flag failed")
        }
    }
}

/// Reject an unauthorized trigger with a durable once-only latch. The label is
/// written before the comment: it is the dedupe gate, so a failed label write must
/// not risk an unlatched duplicate comment on the next reconcile.
async fn flag_trigger_unauthorized(
    github: &GithubAppTokens,
    owner_repo: &str,
    issue: i64,
    comment: &str,
) {
    if let Err(error) = github
        .add_issue_labels(
            owner_repo,
            issue as u64,
            &[TRIGGER_UNAUTHORIZED_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "reconcile: latch trigger-unauthorized label failed");
        return;
    }
    post_comment_best_effort(github, owner_repo, issue, comment).await;
}

/// Clear the creator-authorization latch after authority is definitively granted.
async fn clear_trigger_unauthorized(github: &GithubAppTokens, owner_repo: &str, issue: i64) {
    match github
        .remove_issue_label(owner_repo, issue as u64, TRIGGER_UNAUTHORIZED_LABEL)
        .await
    {
        Ok(()) => {
            tracing::info!(owner_repo = %owner_repo, issue, "reconcile: cleared trigger-unauthorized flag")
        }
        Err(GithubAppError::NotFound { .. }) => {}
        Err(error) => {
            tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "reconcile: clear trigger-unauthorized flag failed")
        }
    }
}

/// Post a comment, logging (never propagating) any failure.
pub(crate) async fn post_comment_best_effort(
    github: &GithubAppTokens,
    owner_repo: &str,
    issue: i64,
    body: &str,
) {
    if let Err(error) = github
        .post_issue_comment(owner_repo, issue as u64, body)
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "reconcile: issue comment failed");
    }
}

#[cfg(test)]
#[path = "execute_lifecycle_tests.rs"]
mod lifecycle_tests;
#[cfg(test)]
#[path = "execute_routing_tests.rs"]
mod routing_tests;
#[cfg(test)]
#[path = "execute_tests.rs"]
mod tests;
