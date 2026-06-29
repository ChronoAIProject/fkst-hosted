//! Session bookkeeping for the API-only control plane.
//!
//! A goal trigger records a `Pending` [`SessionDoc`] that is NEVER run in this
//! process: there is no in-process engine driver, no claim authority, and no
//! worker dispatch. Pod-per-session execution (one Kubernetes Job per session)
//! is rebuilt in milestone #9; the per-session setups (`vault`, `codex`,
//! `nyxid`, `ornn`, `goal_support`) are recorded here now so that work can reuse
//! them without re-plumbing.
//!
//! Concurrency rules (load-bearing):
//! - Every status write goes through the repository CAS
//!   ([`SessionRepo::transition`] / [`SessionRepo::transition_guarded`]); a CAS
//!   miss means a concurrent change won and the caller converges instead of
//!   overwriting.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bson::{doc, Document};
use secrecy::SecretString;

use crate::engine::config::is_reserved_env_key;
use crate::engine::EngineConfig;
use crate::error::AppError;
use crate::github_app::GithubAppTokens;
use crate::goals::labels::session_label;
use crate::goals::{GoalIssueStore, GoalStatus, RepoRef};
use crate::models::{SessionDoc, SessionStatus, TerminalCause};
use crate::nyxid::NyxIdClient;
use crate::ornn::OrnnClient;
use crate::sessions::codex_provider::{self, AssumeConnected, ChronoLlmCheck};
use crate::sessions::repo::{status_bson, terminal_cause_bson, SessionRepo};
use crate::vault::{EnvScopeRef, VaultService};

/// Ownership information stamped onto a new session.
pub struct SessionOwner {
    pub owner_user_id: String,
    pub org_id: Option<String>,
}

/// Information needed to create a session from a goal trigger. The handler
/// resolves and validates this data before passing it here.
pub struct GoalTriggerInfo {
    pub goal_id: bson::Uuid,
    pub repo: RepoRef,
    pub package_names: Vec<String>,
    pub owner_user_id: String,
    pub org_id: Option<String>,
    /// The prior goal status before trigger (captured for compensating CAS).
    pub prior_status: GoalStatus,
    /// Resolved Ornn skill/skillset pins to inject into the session's codex
    /// (issue #114). Already boundary-validated by the trigger handler. `None`
    /// (or empty) means no skills are pinned (the common case).
    pub ornn_skills: Option<Vec<crate::ornn::OrnnSkillPin>>,
}

/// Outcome of a successful `create_for_goal` call.
pub struct GoalTriggerResult {
    pub session_id: bson::Uuid,
    pub goal_status: GoalStatus,
}

/// Cap on the `error` field persisted to a failed session (truncated at a
/// char boundary; the full text is logged). Retained for pod-per-session
/// run-session (milestone #9).
#[allow(dead_code)]
const MAX_ERROR_BYTES: usize = 4096;

/// Shared internals behind the clonable service handle.
pub(super) struct Inner {
    repo: SessionRepo,
    /// Engine configuration (temp root, framework bin). Retained for
    /// pod-per-session run-session (milestone #9); the API-only control plane
    /// never runs an engine in-process.
    #[allow(dead_code)]
    engine: EngineConfig,
    /// Per-session env vault (issue #102), enabled once at startup via
    /// [`SessionService::enable_vault`]. Unset => an EMPTY env profile (legacy
    /// tests / minimal runs). Read by [`resolve_env_profile`] /
    /// [`render_session_codex_config`].
    vault: OnceLock<VaultService>,
    /// Per-session codex LLM-provider config (issue #112), enabled once at
    /// startup via [`SessionService::enable_codex`]. Read by
    /// [`render_session_codex_config`].
    codex: OnceLock<CodexSetup>,
    /// Per-session NyxID token provisioning (issue #111), enabled once at
    /// startup via [`SessionService::enable_nyxid_token`]. Retained for
    /// pod-per-session run-session (milestone #9).
    #[allow(dead_code)]
    nyxid: OnceLock<NyxidSetup>,
    /// Per-session Ornn skill injection (issue #114), enabled once at startup
    /// via [`SessionService::enable_ornn`]. Retained for pod-per-session
    /// run-session (milestone #9).
    #[allow(dead_code)]
    ornn: OnceLock<OrnnClient>,
    /// Goal support layer (issue #63), enabled once at startup via
    /// [`SessionService::enable_goal_support`]. Read by [`goal_status_sync`].
    goal_support: OnceLock<GoalSupport>,
}

/// Per-session codex provider wiring. Carries the operator-pinned chrono-llm
/// DEFAULT values (#112).
struct CodexSetup {
    /// Model the chrono-llm DEFAULT serves (`FKST_HOSTED_CODEX_MODEL`).
    codex_model: String,
    /// NyxID proxy base URL for chrono-llm (`FKST_HOSTED_CHRONO_LLM_BASE_URL`).
    chrono_llm_base_url: String,
}

/// NyxID token provisioning wiring. Retained for pod-per-session run-session
/// (milestone #9).
#[allow(dead_code)]
pub(super) struct NyxidSetup {
    pub(super) client: NyxIdClient,
    /// The NyxID origin the engine talks to, injected as `NYXID_URL`.
    pub(super) origin: String,
    /// TTL for the self-expiring per-session key (#216);
    /// `FKST_SESSION_KEY_TTL_SECS`.
    pub(super) key_ttl: Duration,
}

/// Goal support wiring. The `github_app` half is retained for pod-per-session
/// run-session (milestone #9); `goals` is read by [`goal_status_sync`].
#[allow(dead_code)]
pub(super) struct GoalSupport {
    pub(super) goals: GoalIssueStore,
    pub(super) github_app: GithubAppTokens,
}

/// Clonable bookkeeping service: create / get / stop session documents.
#[derive(Clone)]
pub struct SessionService {
    inner: Arc<Inner>,
}

impl SessionService {
    /// Build the session service. `engine` is retained for pod-per-session
    /// run-session (milestone #9); the API-only control plane never spawns an
    /// engine in-process.
    pub fn new(repo: SessionRepo, engine: EngineConfig) -> Self {
        Self::build(repo, engine)
    }

    fn build(repo: SessionRepo, engine: EngineConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                repo,
                engine,
                vault: OnceLock::new(),
                codex: OnceLock::new(),
                nyxid: OnceLock::new(),
                ornn: OnceLock::new(),
                goal_support: OnceLock::new(),
            }),
        }
    }

    /// Enable goal-support features for this service (best-effort goal-status
    /// sync). Call once at startup; a second call is a logged no-op.
    pub fn enable_goal_support(&self, goals: GoalIssueStore, github_app: GithubAppTokens) {
        if self
            .inner
            .goal_support
            .set(GoalSupport { goals, github_app })
            .is_err()
        {
            tracing::warn!("goal support already enabled; ignoring the second call");
            return;
        }
        tracing::info!("goal support enabled (goal-status sync)");
    }

    /// Enable per-session env injection (issue #102): the session's vault scope
    /// is resolved into an `env_profile`. Call once at startup; a second call is
    /// a logged no-op. When never called the resolver returns an empty profile.
    pub fn enable_vault(&self, vault: VaultService) {
        if self.inner.vault.set(vault).is_err() {
            tracing::warn!("vault already enabled; ignoring the second call");
            return;
        }
        tracing::info!("session env injection enabled (vault wired)");
    }

    /// Enable per-session NyxID token provisioning (issue #111; TTL cleanup
    /// #216). `origin` is the NyxID issuer base URL. Call once at startup; a
    /// second call is a logged no-op. Retained for pod-per-session run-session
    /// (milestone #9).
    pub fn enable_nyxid_token(&self, client: NyxIdClient, origin: String, key_ttl: Duration) {
        if self
            .inner
            .nyxid
            .set(NyxidSetup {
                client,
                origin,
                key_ttl,
            })
            .is_err()
        {
            tracing::warn!("nyxid token provisioning already enabled; ignoring the second call");
            return;
        }
        tracing::info!(
            key_ttl_secs = key_ttl.as_secs(),
            "per-session nyxid token provisioning enabled (self-expiring key)"
        );
    }

    /// Enable per-session codex LLM-provider config (issue #112). `codex_model`
    /// and `chrono_llm_base_url` are the operator-pinned chrono-llm DEFAULT
    /// values. Call once at startup; a second call is a logged no-op.
    pub fn enable_codex(&self, codex_model: String, chrono_llm_base_url: String) {
        if self
            .inner
            .codex
            .set(CodexSetup {
                codex_model,
                chrono_llm_base_url,
            })
            .is_err()
        {
            tracing::warn!("codex provider config already enabled; ignoring the second call");
            return;
        }
        tracing::info!("per-session codex provider config enabled");
    }

    /// Enable per-session Ornn skill injection (issue #114). Call once at
    /// startup; a second call is a logged no-op. Retained for pod-per-session
    /// run-session (milestone #9).
    pub fn enable_ornn(&self, client: OrnnClient) {
        if self.inner.ornn.set(client).is_err() {
            tracing::warn!("ornn skill injection already enabled; ignoring the second call");
            return;
        }
        tracing::info!("per-session ornn skill injection enabled");
    }

    /// The repository handle (startup hooks: orphan sweep; observability).
    pub fn repo(&self) -> &SessionRepo {
        &self.inner.repo
    }

    /// Fetch one session document (pure status projection).
    pub async fn get(&self, id: bson::Uuid) -> Result<Option<SessionDoc>, AppError> {
        self.inner.repo.get(id).await
    }

    /// Create a session from a goal trigger. The control plane is API-only: this
    /// records the session but NEVER runs it (no claim, no spawn, no dispatch,
    /// no token mint). Pod-per-session execution picks up the `Pending` document
    /// later (milestone #9). Steps:
    /// 4. Goal CAS: not_started/stopped/failed -> triggered
    /// 5. Insert SessionDoc (pending)
    /// 6. Set active_session_id + the `fkst-session-<id>` label on the goal
    /// 7. Return result
    ///
    /// On a failed insert after step 4, a compensating CAS returns the goal to
    /// its prior status. `_raw_token` (the triggering user's forwarded access
    /// token) is unused now — it is consumed by pod-per-session execution.
    pub async fn create_for_goal(
        &self,
        goals: &GoalIssueStore,
        trigger: GoalTriggerInfo,
        _raw_token: Option<SecretString>,
    ) -> Result<GoalTriggerResult, AppError> {
        let now = bson::DateTime::now();

        // Step 4: Goal CAS — not_started/stopped/failed -> triggered.
        let triggerable = [
            GoalStatus::NotStarted,
            GoalStatus::Stopped,
            GoalStatus::Failed,
        ];
        goals
            .transition_status(trigger.goal_id, &triggerable, GoalStatus::Triggered, false)
            .await?
            .ok_or_else(|| AppError::Conflict("goal already triggered or running".to_string()))?;
        // Set the (possibly newly-created) repo on the goal + materialize/refresh
        // its issue. Best-effort: the in-memory repo is set regardless; the issue
        // is the durable mirror (#137).
        let _ = goals.set_repo(trigger.goal_id, &trigger.repo).await;

        // Step 5: Insert SessionDoc (pending). It records intent only; no engine
        // is spawned and no worker is dispatched here.
        let first_package = trigger.package_names.first().cloned().unwrap_or_default();
        let session = SessionDoc {
            id: bson::Uuid::new(),
            package_name: first_package,
            status: SessionStatus::Pending,
            pod_id: None,
            fencing_token: None,
            pid: None,
            runtime_dir: None,
            error: None,
            run_key: None,
            owner_user_id: Some(trigger.owner_user_id.clone()),
            org_id: trigger.org_id.clone(),
            package_names: trigger.package_names.clone(),
            goal_id: Some(trigger.goal_id),
            repo: Some(trigger.repo.clone()),
            // Goal sessions resolve their env from the target repo's vault scope
            // (which overlays the owner-wide global scope, repo winning on a key
            // collision — see VaultService::list_for_scope). #102.
            env_scope: Some(EnvScopeRef::repo(&trigger.repo.owner, &trigger.repo.name)),
            triggered_by: Some("goal-trigger".to_string()),
            nyxid_key_id: None,
            nyxid_key_prefix: None,
            // Persisted (resolved, non-secret) so pod-per-session injects the
            // identical pin set (#114). Empty is normalized to `None`.
            ornn_skills: trigger.ornn_skills.clone().filter(|p| !p.is_empty()),
            terminal_cause: None,
            created_at: now,
            started_at: None,
            stopped_at: None,
        };
        if let Err(insert_err) = self.inner.repo.insert(&session).await {
            // Compensating CAS: return goal to prior status.
            let _ = goals
                .transition_status(
                    trigger.goal_id,
                    &[GoalStatus::Triggered],
                    trigger.prior_status,
                    true,
                )
                .await;
            return Err(insert_err);
        }

        // Step 6: Set active_session_id on goal (CAS guarded to triggered).
        let active_set = goals.set_active_session(trigger.goal_id, session.id).await;
        if !active_set.unwrap_or(false) {
            // The goal may have been concurrently modified; the session is still
            // created. Log but do not fail the trigger.
            tracing::warn!(
                goal_id = %trigger.goal_id,
                session_id = %session.id,
                "active_session_id CAS missed; session is still created"
            );
        }
        // Link the session to the goal's issue (#180): ADD `fkst-session-<id>`.
        // Best-effort + logged inside `update_labels` (a label failure never
        // fails the trigger).
        let _ = goals
            .update_labels(
                trigger.goal_id,
                session.id,
                &[&session_label(session.id)],
                &[],
            )
            .await;

        // Step 7: Return result.
        tracing::info!(
            goal_id = %trigger.goal_id,
            session_id = %session.id,
            "goal triggered successfully (session recorded as pending; not run)"
        );
        Ok(GoalTriggerResult {
            session_id: session.id,
            goal_status: GoalStatus::Triggered,
        })
    }

    /// Request a stop. The API-only control plane runs no engine, so there is no
    /// driver to signal: a stop is a pure repository CAS to a terminal stopped
    /// state. Idempotent — a session already terminal answers Ok without a
    /// change; an absent id is NotFound.
    pub async fn request_stop(&self, id: bson::Uuid) -> Result<(), AppError> {
        let transitioned = self
            .inner
            .repo
            .transition(
                id,
                &[
                    SessionStatus::Pending,
                    SessionStatus::Validating,
                    SessionStatus::Running,
                    SessionStatus::Stopping,
                ],
                doc! {
                    "status": status_bson(SessionStatus::Stopped),
                    "terminal_cause": terminal_cause_bson(TerminalCause::Terminated),
                    "stopped_at": bson::DateTime::now(),
                },
            )
            .await?;

        match transitioned {
            Some(session) => {
                tracing::info!(session_id = %id, status = ?session.status, "session stopped");
                Ok(())
            }
            None => match self.inner.repo.get(id).await? {
                Some(session) => {
                    tracing::debug!(session_id = %id, status = ?session.status, "session stop no-op (already terminal)");
                    Ok(())
                }
                None => Err(AppError::NotFound(format!("session not found: {id}"))),
            },
        }
    }

    /// Fail every active session targeting `owner/name` because the GitHub App
    /// was uninstalled from (or had the repo removed from) that repo (issue
    /// #108). A pure repository CAS — there is no in-process driver to signal.
    /// Returns the number of sessions transitioned to Failed.
    ///
    /// `reason` is fixed, operator-authored text (never a secret or a webhook
    /// payload value); it becomes the failed session's user-visible error.
    pub async fn fail_for_uninstalled_repo(
        &self,
        owner: &str,
        name: &str,
        reason: &str,
    ) -> Result<u64, AppError> {
        self.inner.repo.fail_active_for_repo(owner, name, reason).await
    }

    /// Fail every active session whose repo owner is `owner` because the GitHub
    /// App was uninstalled from (or suspended on) the whole account (#141). The
    /// owner-wide counterpart of [`Self::fail_for_uninstalled_repo`]. Returns the
    /// count transitioned.
    ///
    /// `reason` is fixed, operator-authored text (never a secret or a webhook
    /// payload value).
    pub async fn fail_for_uninstalled_owner(
        &self,
        owner: &str,
        reason: &str,
    ) -> Result<u64, AppError> {
        self.inner.repo.fail_active_for_owner(owner, reason).await
    }

    /// Graceful-shutdown hook. The API-only control plane runs no in-process
    /// engine drivers, so there is nothing to drain — kept for the call site in
    /// `main` and the logged outcome.
    pub async fn shutdown(&self) {
        tracing::info!("session shutdown: API-only control plane, no in-process drivers to drain");
    }
}

/// Truncate driver-produced error text at a char boundary so the stored
/// document stays bounded (full text is in the logs). Retained for
/// pod-per-session run-session (milestone #9).
#[allow(dead_code)]
fn truncate_error(text: &str) -> String {
    if text.len() <= MAX_ERROR_BYTES {
        return text.to_string();
    }
    let mut end = MAX_ERROR_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} [truncated]", &text[..end])
}

/// Resolve the engine `env_profile` for a session from the vault's per-scope
/// entries (#102). Returns an EMPTY profile when no vault is wired (legacy tests
/// / minimal runs). Retained for pod-per-session run-session (milestone #9).
#[allow(dead_code)]
pub(super) async fn resolve_env_profile(
    inner: &Arc<Inner>,
    session: &SessionDoc,
) -> Result<BTreeMap<String, SecretString>, AppError> {
    let Some(vault) = inner.vault.get() else {
        return Ok(BTreeMap::new());
    };

    let scope = scope_for_session(session);
    // The owner anchors the lookup; a pre-auth document with no owner resolves
    // to no entries (the empty owner can hold none), which is the safe answer.
    let owner_user_id = session.owner_user_id.as_deref().unwrap_or_default();
    let resolved = vault
        .list_for_scope(owner_user_id, session.org_id.as_deref(), &scope)
        .await?;
    Ok(profile_from_resolved(session.id, resolved))
}

/// Resolve the session's LLM-provider layer from the vault and render the codex
/// `config.toml` STRING. Returns `Ok(None)` when codex config OR the vault is
/// not wired (legacy tests / minimal runs). A render/resolve failure (e.g. the
/// missing chrono-llm 422) propagates as `Err`. No vault value is logged, and
/// the rendered toml never contains a provider key (the key rides `env_key`).
/// Retained for pod-per-session run-session (milestone #9).
#[allow(dead_code)]
pub(super) async fn render_session_codex_config(
    inner: &Arc<Inner>,
    session: &SessionDoc,
) -> Result<Option<String>, AppError> {
    let (Some(codex), Some(vault)) = (inner.codex.get(), inner.vault.get()) else {
        // Codex config or the vault is not wired: nothing to render.
        return Ok(None);
    };

    let scope = scope_for_session(session);
    let owner_user_id = session.owner_user_id.as_deref().unwrap_or_default();
    // v1 connection precondition: assume connected on the online path (the live
    // chrono-llm connection is verified by the documented manual/staging
    // preflight, not on every session start). The seam keeps the 422 mapping
    // exercised by unit tests and lets a future issue swap in a live preflight.
    let check: &dyn ChronoLlmCheck = &AssumeConnected;
    let choice = codex_provider::resolve_provider_choice(
        vault,
        owner_user_id,
        session.org_id.as_deref(),
        &scope,
        check,
    )
    .await?;

    let config_toml = codex_provider::render_codex_config(
        &choice,
        &codex.codex_model,
        &codex.chrono_llm_base_url,
    )?;
    Ok(Some(config_toml))
}

/// Derive the vault scope to resolve for a session: the persisted non-secret
/// `env_scope` pointer wins; otherwise fall back to deriving it from `repo`
/// (repo-scope) or `global`, so a pre-#102 document without `env_scope` still
/// resolves the correct scope on a redrive. Retained for pod-per-session
/// run-session (milestone #9).
#[allow(dead_code)]
fn scope_for_session(session: &SessionDoc) -> EnvScopeRef {
    session
        .env_scope
        .clone()
        .unwrap_or_else(|| match &session.repo {
            Some(repo) => EnvScopeRef::repo(&repo.owner, &repo.name),
            None => EnvScopeRef::global(),
        })
}

/// Build the engine `env_profile` from the vault's resolved entries, dropping
/// any platform-reserved key (`is_reserved_env_key`) as defense in depth — the
/// vault write-validator already rejects them, so a present reserved key is an
/// anomaly worth a warn. Keys (non-secret names) are logged; values never are.
/// Retained for pod-per-session run-session (milestone #9).
#[allow(dead_code)]
fn profile_from_resolved(
    session_id: bson::Uuid,
    resolved: Vec<crate::vault::ResolvedEntry>,
) -> BTreeMap<String, SecretString> {
    let mut profile = BTreeMap::new();
    let mut dropped_reserved = 0usize;
    for entry in resolved {
        if is_reserved_env_key(&entry.key) {
            dropped_reserved += 1;
            continue;
        }
        profile.insert(entry.key, entry.value);
    }
    if dropped_reserved > 0 {
        tracing::warn!(
            session_id = %session_id,
            dropped_reserved,
            "dropped reserved env keys from the resolved session profile"
        );
    }
    profile
}

/// Converge a pre-`running` failure: CAS `validating|stopping -> failed` with
/// the (truncated) error and a stop timestamp, pinned by the fence. Also
/// performs best-effort goal-status sync ({triggered,running} -> failed) when
/// the session is a goal session. Retained for pod-per-session run-session
/// (milestone #9).
#[allow(dead_code)]
async fn fail_session(
    inner: &Inner,
    id: bson::Uuid,
    fence: &Document,
    error: &str,
    goal_id: Option<bson::Uuid>,
) {
    let result = inner
        .repo
        .transition_guarded(
            id,
            &[SessionStatus::Validating, SessionStatus::Stopping],
            fence.clone(),
            doc! {
                "status": status_bson(SessionStatus::Failed),
                "error": truncate_error(error),
                "stopped_at": bson::DateTime::now(),
            },
        )
        .await;
    match result {
        Ok(Some(_)) => tracing::info!(session_id = %id, "session failed"),
        Ok(None) => tracing::warn!(session_id = %id, "fail CAS missed; session already terminal"),
        Err(err) => tracing::error!(session_id = %id, error = %err, "fail CAS errored"),
    }
    // Goal-status sync: {triggered,running} -> failed (best-effort).
    if let Some(goal_id) = goal_id {
        goal_status_sync(
            inner,
            goal_id,
            id,
            &[GoalStatus::Triggered, GoalStatus::Running],
            GoalStatus::Failed,
        )
        .await;
    }
}

/// Best-effort CAS transition of a goal's status. The CAS is guarded with
/// `active_session_id == session_id` so a newer trigger is never clobbered. All
/// errors are logged and swallowed. Retained for pod-per-session run-session
/// (milestone #9).
#[allow(dead_code)]
async fn goal_status_sync(
    inner: &Inner,
    goal_id: bson::Uuid,
    session_id: bson::Uuid,
    from_statuses: &[GoalStatus],
    target: GoalStatus,
) {
    let Some(gs) = inner.goal_support.get() else {
        return;
    };
    let goals = &gs.goals;
    // Guard: only sync if this session is still the goal's active session, so a
    // newer trigger is never clobbered.
    if goals.active_session(goal_id).await != Some(session_id) {
        tracing::debug!(
            goal_id = %goal_id,
            session_id = %session_id,
            "goal-status sync skipped (not the goal's active session)"
        );
        return;
    }
    match goals
        .transition_status(goal_id, from_statuses, target, false)
        .await
    {
        Ok(Some(_)) => tracing::info!(
            goal_id = %goal_id,
            session_id = %session_id,
            target = ?target,
            "goal-status sync applied"
        ),
        Ok(None) => tracing::debug!(
            goal_id = %goal_id,
            session_id = %session_id,
            "goal-status sync CAS missed (concurrent change)"
        ),
        Err(error) => tracing::warn!(
            goal_id = %goal_id,
            session_id = %session_id,
            error = %error,
            "goal-status sync write failed (swallowed)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_error_keeps_short_text_verbatim() {
        assert_eq!(truncate_error("boom"), "boom");
    }

    #[test]
    fn truncate_error_caps_long_text_at_a_char_boundary() {
        let long = "α".repeat(MAX_ERROR_BYTES); // 2 bytes per char
        let truncated = truncate_error(&long);
        assert!(truncated.ends_with(" [truncated]"));
        assert!(truncated.len() <= MAX_ERROR_BYTES + " [truncated]".len());
        // Still valid UTF-8 by construction (String), no panic on slicing.
    }

    // ---- per-session env injection (issue #102) -------------------------------

    use crate::models::RepoRef as ModelRepoRef;
    use crate::vault::ResolvedEntry;
    use secrecy::ExposeSecret;

    /// A minimal `SessionDoc` for the env-resolution helper tests. Only the
    /// fields the helpers read (`id`, `owner_user_id`, `org_id`, `repo`,
    /// `env_scope`) matter; the rest are inert defaults.
    fn env_test_session() -> SessionDoc {
        SessionDoc {
            id: bson::Uuid::new(),
            package_name: "demo".to_string(),
            status: SessionStatus::Pending,
            pod_id: None,
            fencing_token: None,
            pid: None,
            runtime_dir: None,
            error: None,
            run_key: None,
            owner_user_id: Some("user-1".to_string()),
            org_id: None,
            package_names: vec![],
            goal_id: None,
            repo: None,
            env_scope: None,
            triggered_by: None,
            nyxid_key_id: None,
            nyxid_key_prefix: None,
            ornn_skills: None,
            terminal_cause: None,
            created_at: bson::DateTime::now(),
            started_at: None,
            stopped_at: None,
        }
    }

    #[test]
    fn scope_for_session_prefers_the_persisted_env_scope() {
        let mut session = env_test_session();
        // A repo is set but the persisted env_scope must win regardless.
        session.repo = Some(ModelRepoRef {
            owner: "acme".to_string(),
            name: "other".to_string(),
        });
        session.env_scope = Some(EnvScopeRef::repo("acme", "site"));
        assert_eq!(scope_for_session(&session).scope_key(), "repo:acme/site");
    }

    #[test]
    fn scope_for_session_falls_back_to_repo_when_env_scope_absent() {
        // Legacy doc (no env_scope) with a repo resolves to that repo's scope.
        let mut session = env_test_session();
        session.repo = Some(ModelRepoRef {
            owner: "acme".to_string(),
            name: "billing".to_string(),
        });
        assert_eq!(scope_for_session(&session).scope_key(), "repo:acme/billing");
    }

    #[test]
    fn scope_for_session_falls_back_to_global_when_neither_present() {
        // Legacy doc with neither env_scope nor repo resolves owner-wide.
        let session = env_test_session();
        assert_eq!(scope_for_session(&session).scope_key(), "global");
    }

    #[test]
    fn profile_from_resolved_keeps_ordinary_keys_and_drops_reserved() {
        let id = bson::Uuid::new();
        let resolved = vec![
            ResolvedEntry {
                key: "OPENAI_API_KEY".to_string(),
                value: SecretString::from("sk-secret".to_string()),
            },
            ResolvedEntry {
                key: "FOO".to_string(),
                value: SecretString::from("bar".to_string()),
            },
            // Reserved keys must be dropped (defense in depth): a platform
            // prefix, an explicit reserved name, and an allow-listed host var.
            ResolvedEntry {
                key: "FKST_DURABLE_ROOT".to_string(),
                value: SecretString::from("x".to_string()),
            },
            ResolvedEntry {
                key: "GITHUB_TOKEN".to_string(),
                value: SecretString::from("y".to_string()),
            },
            ResolvedEntry {
                key: "PATH".to_string(),
                value: SecretString::from("z".to_string()),
            },
        ];
        let profile = profile_from_resolved(id, resolved);
        assert_eq!(
            profile.keys().cloned().collect::<Vec<_>>(),
            vec!["FOO".to_string(), "OPENAI_API_KEY".to_string()],
            "only the non-reserved keys survive, key-sorted"
        );
        assert_eq!(
            profile.get("OPENAI_API_KEY").map(|v| v.expose_secret()),
            Some("sk-secret")
        );
        assert!(!profile.contains_key("FKST_DURABLE_ROOT"));
        assert!(!profile.contains_key("GITHUB_TOKEN"));
        assert!(!profile.contains_key("PATH"));
    }

    #[test]
    fn profile_from_resolved_is_empty_for_no_entries() {
        let profile = profile_from_resolved(bson::Uuid::new(), Vec::new());
        assert!(profile.is_empty());
    }

    #[tokio::test]
    async fn resolve_env_profile_is_empty_when_no_vault_wired() {
        // A service that never had `enable_vault` called resolves an EMPTY
        // profile — the pre-#102 behaviour for tests / minimal runs. The session
        // store is in-memory, so this needs no datastore.
        let service = SessionService::new(SessionRepo::new(), EngineConfig::default());
        let session = env_test_session();
        let profile = resolve_env_profile(&service.inner, &session)
            .await
            .expect("empty profile, no error");
        assert!(
            profile.is_empty(),
            "no vault wired => empty env profile (legacy behaviour)"
        );
    }
}
