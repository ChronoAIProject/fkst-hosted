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
//! Secret hygiene: the minted installation token is serialized into the
//! `github-token` Secret value and never logged; comments/labels carry only public
//! metadata (the offending refs / the parser's 422 message).

use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::config::Config;
use crate::github_app::listing::GithubListing;
use crate::github_app::{session_permissions, GithubAppError, GithubAppTokens};
use crate::k8s::env_store::{get_environment, load_environment_for_session};
use crate::k8s::{session_github_token_json, KubeClient, SessionPodSpec};
use crate::models::RepoRef;
use crate::reconcile::announce::announce_session_comment;
use crate::reconcile::desired::{KillReason, ReconcileAction, SessionRegistration};
use crate::reconcile::execute_comments::{
    config_rejected_comment, env_not_ready_comment, env_verify_failed_comment,
    flag_invalid_comment, invalid_refs_comment,
};
use crate::reconcile::reachability;
use crate::reconcile::retire::retire_work_issues;
use crate::session_backend::{BackendError, EnsureOutcome, SessionBackend};
use crate::session_spec::creds::{credential_secret_data, StorageWriterCreds};

use super::{SUBSTRATE_ANNOUNCED_LABEL, SUBSTRATE_CONFIG_REJECTED_LABEL, SUBSTRATE_INVALID_LABEL};

/// The `validation-status` annotation value a fully-written environment carries;
/// only a `ready` environment is injected into a session (mirrors Model A).
const ENV_STATUS_READY: &str = "ready";

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
    /// Kubernetes API client (namespace-bound), still used directly for the
    /// environment-store reads (`resolve_environment`) and the loops' sweep.
    pub kube: KubeClient,
    /// GitHub App token service: mints the session token + posts comments/labels.
    pub github: GithubAppTokens,
    /// Read-side GitHub transport the driver enumerates issues + counts work with.
    pub listing: Arc<dyn GithubListing>,
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
    pub log_registry: crate::log_access::LogAccessRegistry,
}

/// Execute ONE action for the repo it belongs to. Best-effort: logs and swallows
/// every error at this boundary (see the module docs). `repo` scopes the GitHub
/// issue effects (flag/clear); the pod effects address the deterministic
/// `fkst-sess-<session_id>` pod directly.
pub async fn execute(action: ReconcileAction, repo: &RepoRef, ctx: &ReconcileCtx) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    match action {
        ReconcileAction::Spawn(reg) => spawn_session(reg, ctx).await,
        ReconcileAction::TouchPending { session_id } => touch_pending(&session_id, ctx).await,
        ReconcileAction::Kill { session_id, reason } => kill(&session_id, reason, ctx).await,
        ReconcileAction::CleanupTerminal { session_id } => cleanup_terminal(&session_id, ctx).await,
        ReconcileAction::RetireWorkIssues { work_label } => {
            retire_work_issues(&ctx.github, ctx.listing.as_ref(), repo, &work_label).await
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
        ReconcileAction::AnnounceSession {
            trigger_issue,
            session_id,
            session_name,
            work_label,
            packages,
            environment,
            auto_merge,
            full_config_hash,
        } => {
            // Build the identity-gated log-download link from the configured public
            // base URL; `None` (unset) omits the log line. The endpoint authorizes
            // every request, so the static URL is safe to post.
            let log_url =
                ctx.config.log.public_base_url.as_ref().map(|base| {
                    format!("{}/api/v1/logs/{}", base.trim_end_matches('/'), session_id)
                });
            let comment = announce_session_comment(
                &session_name,
                &work_label,
                &packages,
                environment.as_deref(),
                auto_merge,
                log_url.as_deref(),
                &full_config_hash,
            );
            announce_session(&ctx.github, &owner_repo, trigger_issue, &comment).await
        }
        ReconcileAction::RejectConfigChange { trigger_issue } => {
            reject_config_change(&ctx.github, &owner_repo, trigger_issue).await
        }
    }
}

// --- Spawn -------------------------------------------------------------------

/// Spawn a session pod for a desired-but-absent registration: reachability →
/// environment → token → pod. Any gate that fails posts issue feedback and skips
/// the spawn (never a partial pod).
async fn spawn_session(reg: SessionRegistration, ctx: &ReconcileCtx) {
    let owner_repo = format!("{}/{}", reg.repo.owner, reg.repo.name);

    // 1. Reachability: every package ref must resolve on public GitHub. A failure
    //    flags the trigger issue (comment + latch label) and skips the spawn. The
    //    probe is authenticated with the repo's installation token (best-effort mint;
    //    falls back to unauthenticated) so a large package closure across repeated
    //    reconciles rides the 5000/hour token budget, not the 60/hour per-IP one.
    let reach_token = ctx.github.token_for_repo(&owner_repo, None).await.ok();
    if let Err(bad) = reachability::check_reachable(
        &reg.def.packages,
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

    // 2. Environment: a named environment must exist + be `ready` for the author;
    //    otherwise post feedback and skip (fail closed, no doomed pod).
    let user_env = match resolve_environment(
        &ctx.kube,
        reg.trigger_author_id,
        reg.def.environment.as_deref(),
    )
    .await
    {
        EnvResolution::Proceed { user_env } => user_env,
        EnvResolution::Blocked { comment } => {
            post_comment_best_effort(&ctx.github, &owner_repo, reg.trigger_issue, &comment).await;
            return;
        }
    };

    // 3. Mint the least-privilege session token and render the rotating
    //    `{token, expires_at}` JSON the pod's git/gh read.
    let (token, expires_at) = match ctx
        .github
        .token_with_expiry_for_repo(&owner_repo, Some(session_permissions()))
        .await
    {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(session_id = %reg.session_id, error = %error, "reconcile spawn: token mint failed; not spawning");
            return;
        }
    };
    let github_token_json = session_github_token_json(&token, expires_at);

    // 4. Assemble the pod spec from the registration.
    let spec = session_pod_spec_from(&reg, ctx.config.reconcile.github_bot_login.clone());

    // 5. Assemble the per-session credential map (the executor now owns the credential
    //    layout, threading it through the backend as `SecretString`s). Always inject
    //    the write-only SA creds when the control plane configured one (log streaming
    //    is unconditional; the per-session flag was retired). Absent a configured SA
    //    the Secret carries no storage-* keys and the in-pod uploader fails closed —
    //    no bundle, never a crash.
    let storage = storage_writer_creds(&ctx.config);
    let creds: BTreeMap<String, SecretString> = credential_secret_data(
        &github_token_json,
        ctx.config.llm_api_key.expose_secret(),
        user_env.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        storage,
    )
    .into_iter()
    .map(|(k, v)| (k, SecretString::from(v)))
    .collect();

    // 6. Ensure the session runtime exists (409 = already-live no-op). The backend
    //    builds + creates the pod and its owner-referenced creds Secret; on failure it
    //    has already logged the specific error, so an `Err` here is a swallowed no-op.
    match ctx.backend.ensure_session(&spec, creds).await {
        Ok(EnsureOutcome::Created) => {
            tracing::info!(session_id = %spec.session_id, owner = %reg.repo.owner, "reconcile spawn: session pod created")
        }
        Ok(EnsureOutcome::AlreadyLive) => {
            tracing::info!(session_id = %spec.session_id, "reconcile spawn: session pod already live (no-op)")
        }
        Err(_) => {}
    }
}

/// Build the launch spec from a registration (pure; unit-tested). `package_roots`
/// are the refs rendered back to `owner/repo@ref:path`; `bot_login` falls back to
/// empty when unset.
fn session_pod_spec_from(reg: &SessionRegistration, bot_login: Option<String>) -> SessionPodSpec {
    SessionPodSpec {
        session_id: reg.session_id.clone(),
        installation_id: reg.installation_id,
        repo: reg.repo.clone(),
        trigger_issue_number: reg.trigger_issue,
        package_roots: reg
            .def
            .packages
            .iter()
            .map(reachability::render_ref)
            .collect(),
        work_label: reg.def.work_label.clone(),
        bot_login: bot_login.unwrap_or_default(),
        config_hash: reg.config_hash.clone(),
    }
}

/// Resolve the WRITE-ONLY chrono-storage SA creds to inject into a session Secret,
/// or `None` when the control plane has no storage config OR no write-only SA
/// configured (the in-pod uploader then fails closed — no bundle). Borrows the
/// config, exposing the client secret only to copy it into the Secret builder.
fn storage_writer_creds(config: &Config) -> Option<StorageWriterCreds<'_>> {
    let storage = config.storage.as_ref()?;
    let client_id = storage.writer_client_id.as_deref()?;
    let client_secret = storage.writer_client_secret.as_ref()?;
    Some(StorageWriterCreds {
        client_id,
        client_secret: client_secret.expose_secret(),
        token_url: &storage.nyxid_token_url,
        base_url: &storage.base_url,
        bucket: &storage.bucket,
    })
}

// --- Pod lifecycle effects ---------------------------------------------------

/// Refresh a live pod's `last-pending-at` annotation to now (via the backend).
/// 404-tolerant: a pod deleted between the plan and the patch is a benign no-op.
async fn touch_pending(session_id: &str, ctx: &ReconcileCtx) {
    match ctx.backend.mark_pending(session_id).await {
        Ok(()) => tracing::debug!(session_id = %session_id, "reconcile: touched last-pending-at"),
        Err(BackendError::NotFound) => {}
        Err(error) => {
            tracing::warn!(session_id = %session_id, error = %error, "reconcile: touch last-pending-at failed")
        }
    }
}

/// Stop a pod for `reason`, honouring the configured termination grace (via the
/// backend). 404-tolerant (already gone).
async fn kill(session_id: &str, reason: KillReason, ctx: &ReconcileCtx) {
    tracing::info!(session_id = %session_id, ?reason, "reconcile: killing session pod");
    match ctx.backend.stop_session(session_id, reason).await {
        Ok(()) => {}
        Err(BackendError::NotFound) => {}
        Err(error) => {
            tracing::warn!(session_id = %session_id, error = %error, "reconcile: kill delete failed")
        }
    }
}

/// GC a terminal pod (its owner-referenced Secret cascades away in the background,
/// via the backend). 404-tolerant.
async fn cleanup_terminal(session_id: &str, ctx: &ReconcileCtx) {
    match ctx.backend.remove_terminal(session_id).await {
        Ok(()) => {
            tracing::info!(session_id = %session_id, "reconcile: cleaned up terminal session pod")
        }
        Err(BackendError::NotFound) => {}
        Err(error) => {
            tracing::warn!(session_id = %session_id, error = %error, "reconcile: terminal cleanup failed")
        }
    }
}

// --- GitHub issue effects (testable against a fake transport) -----------------

/// Flag an invalid trigger issue: post `comment`, then latch the invalid label.
/// Both are best-effort + idempotent (label add is additive; the planner emits
/// this only on the FIRST observation of an invalid issue).
async fn flag_invalid(github: &GithubAppTokens, owner_repo: &str, issue: i64, comment: &str) {
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

/// Post a comment, logging (never propagating) any failure.
async fn post_comment_best_effort(
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

// --- Environment resolution (mirrors the Model-A webhook pre-flight) ----------

/// The outcome of pre-flighting the issue's named environment.
enum EnvResolution {
    /// Launch with the merged variables/secret VALUES to inject (empty when the
    /// issue declared no environment).
    Proceed { user_env: BTreeMap<String, String> },
    /// Do NOT launch; post `comment` on the trigger issue explaining why.
    Blocked { comment: String },
}

/// Pre-flight the issue's named environment against the AUTHOR's store (keyed by
/// the signed numeric GitHub id). `None` → an empty (no-environment) session. A
/// named selection must EXIST and be `ready`; otherwise (missing, not ready, or a
/// store-read error) the launch is blocked with a feedback comment — fail closed.
async fn resolve_environment(
    kube: &KubeClient,
    author_id: i64,
    environment: Option<&str>,
) -> EnvResolution {
    let name = match environment {
        None => {
            return EnvResolution::Proceed {
                user_env: BTreeMap::new(),
            }
        }
        Some(name) => name,
    };

    match get_environment(kube, author_id, name).await {
        Ok(Some(record)) if record.status == ENV_STATUS_READY => {
            match load_environment_for_session(kube, author_id, name).await {
                Ok(Some((install, user_env))) => {
                    tracing::info!(
                        github_user_id = author_id,
                        environment = %name,
                        install_commands = install.len(),
                        env_vars = user_env.len(),
                        "reconcile spawn: named environment resolved"
                    );
                    EnvResolution::Proceed { user_env }
                }
                Ok(None) => EnvResolution::Blocked {
                    comment: env_not_ready_comment(name),
                },
                Err(error) => {
                    tracing::error!(environment = %name, error = %error, "reconcile spawn: environment load failed");
                    EnvResolution::Blocked {
                        comment: env_verify_failed_comment(name),
                    }
                }
            }
        }
        Ok(_) => EnvResolution::Blocked {
            comment: env_not_ready_comment(name),
        },
        Err(error) => {
            tracing::error!(environment = %name, error = %error, "reconcile spawn: environment pre-flight read failed");
            EnvResolution::Blocked {
                comment: env_verify_failed_comment(name),
            }
        }
    }
}

#[cfg(test)]
#[path = "execute_test_support.rs"]
mod execute_test_support;
#[cfg(test)]
#[path = "execute_routing_tests.rs"]
mod routing_tests;
#[cfg(test)]
#[path = "execute_tests.rs"]
mod tests;
