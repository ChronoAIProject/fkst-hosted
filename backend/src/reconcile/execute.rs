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

use secrecy::ExposeSecret;

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::chrono::{DateTime, Utc};
use kube::api::{Api, DeleteParams, Patch, PatchParams};

use crate::config::Config;
use crate::github_app::listing::GithubListing;
use crate::github_app::{session_permissions, GithubAppError, GithubAppTokens};
use crate::k8s::env_store::{get_environment, load_environment_for_session};
use crate::k8s::session_launcher::ANNOTATION_LAST_PENDING_AT;
use crate::k8s::{
    build_session_pod, build_session_secret, create_session_pod, session_github_token_json,
    session_object_name, KubeClient, SessionPodOutcome, SessionPodSpec,
};
use crate::models::RepoRef;
use crate::reconcile::desired::{KillReason, ReconcileAction, SessionRegistration};
use crate::reconcile::reachability;

use super::SUBSTRATE_INVALID_LABEL;

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
    /// Kubernetes API client (namespace-bound) for the session pod/secret effects.
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
    }
}

/// A namespaced Pod API bound to the reconciler's namespace.
fn pods_api(ctx: &ReconcileCtx) -> Api<Pod> {
    Api::namespaced(ctx.kube.client().clone(), ctx.kube.namespace())
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

    // 5. Build + create (409 = already-live no-op).
    let pod = match build_session_pod(&spec, &ctx.config.pod) {
        Ok(pod) => pod,
        Err(error) => {
            tracing::error!(session_id = %reg.session_id, error = %error, "reconcile spawn: pod build failed; not spawning");
            return;
        }
    };
    let secret = build_session_secret(
        &spec,
        &github_token_json,
        &ctx.config.llm_api_key,
        &user_env,
        None,
    );
    match create_session_pod(ctx.kube.client(), pod, secret).await {
        Ok(SessionPodOutcome::Created) => {
            tracing::info!(session_id = %spec.session_id, owner = %reg.repo.owner, "reconcile spawn: session pod created")
        }
        Ok(SessionPodOutcome::AlreadyLive) => {
            tracing::info!(session_id = %spec.session_id, "reconcile spawn: session pod already live (no-op)")
        }
        Err(error) => {
            tracing::error!(session_id = %spec.session_id, error = %error, "reconcile spawn: session pod create failed")
        }
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

// --- Pod lifecycle effects ---------------------------------------------------

/// Refresh a live pod's `last-pending-at` annotation to now (JSON merge patch).
/// 404-tolerant: a pod deleted between the plan and the patch is a benign no-op.
async fn touch_pending(session_id: &str, ctx: &ReconcileCtx) {
    let name = session_object_name(session_id);
    let patch = last_pending_patch(Utc::now());
    match pods_api(ctx)
        .patch(&name, &PatchParams::default(), &Patch::Merge(patch))
        .await
    {
        Ok(_) => tracing::debug!(session_id = %session_id, "reconcile: touched last-pending-at"),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(error) => {
            tracing::warn!(session_id = %session_id, error = %error, "reconcile: touch last-pending-at failed")
        }
    }
}

/// The JSON merge patch that sets `last-pending-at` to `now` (RFC3339). Pure +
/// unit-tested so the annotation key + shape can never drift from the builder.
fn last_pending_patch(now: DateTime<Utc>) -> serde_json::Value {
    let annotations = serde_json::Map::from_iter([(
        ANNOTATION_LAST_PENDING_AT.to_string(),
        serde_json::Value::String(now.to_rfc3339()),
    )]);
    serde_json::json!({ "metadata": { "annotations": serde_json::Value::Object(annotations) } })
}

/// Delete a pod for `reason`, honouring the configured termination grace.
/// 404-tolerant (already gone).
async fn kill(session_id: &str, reason: KillReason, ctx: &ReconcileCtx) {
    let name = session_object_name(session_id);
    let params = kill_delete_params(ctx.config.reconcile.pod_termination_grace_secs);
    tracing::info!(session_id = %session_id, ?reason, "reconcile: killing session pod");
    match pods_api(ctx).delete(&name, &params).await {
        Ok(_) => {}
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(error) => {
            tracing::warn!(session_id = %session_id, error = %error, "reconcile: kill delete failed")
        }
    }
}

/// `DeleteParams` carrying the drain window (`terminationGracePeriodSeconds`). Pure
/// + unit-tested. A grace beyond `u32::MAX` is clamped (never realistically hit).
fn kill_delete_params(grace_secs: u64) -> DeleteParams {
    DeleteParams {
        grace_period_seconds: Some(u32::try_from(grace_secs).unwrap_or(u32::MAX)),
        ..DeleteParams::default()
    }
}

/// GC a terminal pod (its owner-referenced Secret cascades away in the background).
/// 404-tolerant.
async fn cleanup_terminal(session_id: &str, ctx: &ReconcileCtx) {
    let name = session_object_name(session_id);
    match pods_api(ctx)
        .delete(&name, &DeleteParams::background())
        .await
    {
        Ok(_) => {
            tracing::info!(session_id = %session_id, "reconcile: cleaned up terminal session pod")
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {}
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

// --- Feedback comment bodies (pure) ------------------------------------------

fn env_not_ready_comment(name: &str) -> String {
    format!(
        "⚠️ fkst couldn't start this session: environment `{name}` was not found in your account \
         (or isn't ready). Create it first with `PUT /api/v1/users/me/environments/{name}`, then \
         re-trigger. Omit the `### Environment` section to run with no environment."
    )
}

fn env_verify_failed_comment(name: &str) -> String {
    format!(
        "⚠️ fkst couldn't verify environment `{name}` right now (a transient error reading your \
         environments). Please re-trigger in a moment."
    )
}

fn invalid_refs_comment(failures: &[(String, String)]) -> String {
    let mut body = String::from(
        "⚠️ fkst couldn't start this session: one or more `### Packages` refs are not reachable \
         on public GitHub.\n\n",
    );
    for (r, reason) in failures {
        body.push_str(&format!("- `{r}` — {reason}\n"));
    }
    body.push_str(
        "\nEach ref must be `owner/repo@ref:path/to/package` in a PUBLIC repo with an `fkst.toml` \
         at that path. Fix the refs and re-trigger.",
    );
    body
}

fn flag_invalid_comment(detail: &str) -> String {
    format!(
        "⚠️ fkst couldn't parse this trigger issue: {detail}\n\nExpected the \
         `fkst-substrate-trigger` body with `### Session Name`, `### Packages` (one \
         `owner/repo@ref:path` per line), `### Work Label`, and an optional `### Environment`. \
         Fix the issue body and the reconciler will retry."
    )
}

#[cfg(test)]
#[path = "execute_tests.rs"]
mod tests;
