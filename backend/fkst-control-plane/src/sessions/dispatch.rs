//! Controller-side dispatch resolution (issue #151).
//!
//! [`resolve_dispatch`] resolves everything a worker needs to start an engine —
//! the merged env profile, the first GitHub-App installation token (+ expiry),
//! the rendered codex `config.toml`, the resolved Ornn plan, and the per-session
//! mint nonce — into the serializable [`ResolvedDispatch`] wire type.
//!
//! It is DORMANT in this increment: nothing on the live path calls it yet. The
//! in-process [`super::service`] driver (`drive_inner`) stays the live code path
//! and is untouched; the activation increment will swap the in-process spawn for
//! a worker dispatch that consumes this output.
//!
//! Behaviour-equivalence is load-bearing: every value here is produced by the
//! SAME resolution helpers the in-process driver uses, so the two paths can
//! never diverge:
//! - env profile → [`super::service::resolve_env_profile`] (vault scope +
//!   reserved-key filtering), exactly as `drive_inner`'s B3.
//! - first token → `github_app.token_with_expiry_for_repo(repo, None)`, the same
//!   call `build_goal_context` makes (B/B-prep).
//! - NyxID env → [`super::nyxid_token::provision`], merged exactly as B4.
//! - codex toml → [`super::service::render_session_codex_config`] (the render
//!   half shared with B5's `prepare_codex_home`).
//! - Ornn plan → [`crate::ornn::resolve_plan`] (the resolve half of B6's
//!   `inject_pins`).
//! - mint nonce → [`crate::engine::generate_mint_nonce`] (the SAME generator the
//!   engine's runner uses), so the nonce scheme has one source of truth.
//!
//! Secret hygiene (load-bearing): every secret stays a `SecretString`
//! (zeroizing, redacting `Debug`), and nothing secret — the env values, the
//! token, the goal prompt, or the nonce — is ever logged. The goal prompt
//! becomes [`DispatchGoal::description`], a `SecretString`.

use std::sync::Arc;
use std::time::SystemTime;

use fkst_shared::protocol::{CloneSpec, DispatchGoal, JournalPlan, OrnnPlan, ResolvedDispatch};
use secrecy::SecretString;

use super::nyxid_token;
use super::service::{render_session_codex_config, resolve_env_profile, Inner};
use crate::journal::JournalConfig;
use crate::models::SessionDoc;

/// The `git_ref` a freshly-dispatched session clones at. A plain in-process
/// clone (`clone_repo_packages`) checks out the remote's default branch HEAD
/// with no explicit ref, so `HEAD` is the behaviourally-faithful value the
/// worker resolves to the same default branch.
const DEFAULT_GIT_REF: &str = "HEAD";

/// Why a dispatch could not be resolved. Wraps the underlying failure (token
/// mint, NyxID provision, codex render, Ornn resolve, or a missing
/// precondition) so [`resolve_dispatch`] is `Result`-typed and unit-testable.
/// No variant carries a secret — only non-sensitive context — so the error can
/// be logged and surfaced safely.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    /// The session is not a goal session (no `repo` / `goal_id`), so there is
    /// nothing to clone or mint against. Goal-less sessions are unsupported
    /// since #115; this mirrors `drive_inner`'s loud failure.
    #[error("session is not a goal session: {0}")]
    NotAGoalSession(String),
    /// Goal support is not wired, so no GitHub-App token can be minted.
    #[error("goal support not enabled; cannot resolve dispatch")]
    GoalSupportDisabled,
    /// The goal document referenced by the session was not found.
    #[error("goal not found for session")]
    GoalNotFound,
    /// Minting the first installation token for the goal repo failed.
    #[error("failed to mint github token for the goal repo")]
    TokenMint(#[source] crate::github_app::GithubAppError),
    /// Resolving the per-session env profile from the vault failed.
    #[error("failed to resolve session env profile")]
    EnvProfile(#[source] crate::error::AppError),
    /// Provisioning the per-session NyxID token failed.
    #[error("failed to provision nyxid session token")]
    NyxidProvision(#[source] crate::error::AppError),
    /// Rendering the codex `config.toml` failed.
    #[error("failed to render codex config")]
    CodexRender(#[source] crate::error::AppError),
    /// Resolving the pinned Ornn plan failed.
    #[error("failed to resolve ornn plan")]
    OrnnResolve(#[source] crate::error::AppError),
    /// A clock skew made the token expiry un-representable as unix ms.
    #[error("token expiry precedes the unix epoch")]
    TokenExpiryUnrepresentable,
}

/// Resolve a fully-self-contained [`ResolvedDispatch`] for `session` (#151).
///
/// `raw_token` is the triggering user's access token, threaded the same way the
/// in-process driver threads it (present at trigger; absent only when the
/// controller no longer holds it). When NyxID is wired AND a `raw_token` is
/// present, this provisions the per-session NyxID key and merges its entries
/// into the env profile, exactly as `drive_inner`'s B4; when the token is absent
/// it simply skips provisioning (the dormant resolver never escalates — the
/// activation increment owns that policy).
///
/// `worker_id` / `fencing_id` are stamped onto the dispatch so the worker echoes
/// the claim's fence on every controller mutation. Every secret stays a
/// `SecretString`; nothing secret is logged.
pub(super) async fn resolve_dispatch(
    inner: &Arc<Inner>,
    session: &SessionDoc,
    raw_token: Option<&SecretString>,
    worker_id: &str,
    fencing_id: i64,
) -> Result<ResolvedDispatch, DispatchError> {
    let session_id = session.id;

    // 1. The goal a dispatch runs needs a repo (the clone + mint target) and a
    //    goal id (the prompt source). Goal-less sessions are unsupported (#115).
    let repo = session
        .repo
        .clone()
        .ok_or_else(|| DispatchError::NotAGoalSession("missing repo".to_string()))?;
    let goal_id = session
        .goal_id
        .ok_or_else(|| DispatchError::NotAGoalSession("missing goal_id".to_string()))?;

    let gs = inner
        .goal_support
        .get()
        .ok_or(DispatchError::GoalSupportDisabled)?;
    let goal = gs
        .goals
        .get(goal_id)
        .await
        .map_err(DispatchError::EnvProfile)?
        .ok_or(DispatchError::GoalNotFound)?;

    // The DispatchGoal: the prompt (`description`) is sensitive — it becomes a
    // SecretString that never renders in Debug/logs. `goal_id` is the hyphenated
    // UUID string (the goal's stable identity on the wire).
    let dispatch_goal = DispatchGoal {
        goal_id: goal_id.to_string(),
        title: goal.title,
        description: SecretString::from(goal.description),
        repo: repo.clone(),
    };

    // The clone spec: the worker clones `repo` at the default branch HEAD and
    // resolves the session's effective package roots under `<repo>/.fkst/packages/`.
    let clone_spec = CloneSpec {
        repo: repo.clone(),
        git_ref: DEFAULT_GIT_REF.to_string(),
        package_roots: session.effective_package_names(),
    };

    // 2. Mint the FIRST installation token (the same call build_goal_context
    //    makes). The token rides only the SecretString; never logged.
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let (github_token, expires_at) = gs
        .github_app
        .token_with_expiry_for_repo(&owner_repo, None)
        .await
        .map_err(DispatchError::TokenMint)?;
    let github_token_expires_at_unix_ms = system_time_to_unix_ms(expires_at)?;

    // 3. Resolve the env profile from the vault (B3), then provision NyxID and
    //    merge its entries (B4) exactly as the in-process driver does.
    let mut env_profile = resolve_env_profile(inner, session)
        .await
        .map_err(DispatchError::EnvProfile)?;
    if let (Some(setup), Some(token)) = (inner.nyxid.get(), raw_token) {
        let (_handle, entries) = nyxid_token::provision(
            &setup.client,
            session_id,
            &setup.origin,
            token,
            setup.key_ttl,
        )
        .await
        .map_err(DispatchError::NyxidProvision)?;
        // The two entries (the secret key + the non-secret origin) are non-reserved
        // and survive the engine env filter, so merge them into the profile the
        // run starts with — identical to drive_inner's B4 merge.
        for (key, value) in entries {
            env_profile.insert(key, value);
        }
    }

    // 4. Render the codex config.toml (B5's render half). `None` when codex/vault
    //    is unwired — the same skip condition the in-process driver uses.
    let codex_config_toml = render_session_codex_config(inner, session)
        .await
        .map_err(DispatchError::CodexRender)?;

    // 5. Resolve the Ornn plan (B6's resolve half) when Ornn is wired, the
    //    session has the NyxID identity to fetch as, AND it pinned skills. The
    //    presigned URLs the plan carries are sensitive; never logged.
    let ornn = resolve_ornn_plan(inner, session, &env_profile).await?;

    // 6. The per-session mint nonce — the SAME generator the engine runner uses,
    //    so the dispatched nonce is shaped exactly like an in-process one.
    let mint_nonce = SecretString::from(crate::engine::generate_mint_nonce());

    // Key NAMES are non-secret and aid debugging which env a dispatch carries;
    // VALUES are never logged (they are SecretStrings).
    tracing::info!(
        session_id = %session_id,
        worker_id = %worker_id,
        fencing_id,
        env_count = env_profile.len(),
        env_keys = %env_profile.keys().cloned().collect::<Vec<_>>().join(","),
        codex = codex_config_toml.is_some(),
        ornn_skills = ornn.as_ref().map(|p| p.skills.len()).unwrap_or(0),
        "resolved session dispatch"
    );

    Ok(ResolvedDispatch {
        session_id: session_id.to_string(),
        worker_id: worker_id.to_string(),
        fencing_id,
        goal: dispatch_goal,
        clone_spec,
        github_token,
        github_token_expires_at_unix_ms,
        env_profile,
        codex_config_toml,
        ornn,
        // The controller's process journaling config, projected into the wire
        // plan the worker reconstructs a `JournalConfig` from. `None` when
        // journaling is off or its GitHub coordinates are incomplete — exactly
        // the cases where the in-process journaler writes no durable record.
        journal: inner
            .journal
            .get()
            .and_then(|setup| journal_plan(&setup.config)),
        mint_nonce,
    })
}

/// Project the controller's process [`JournalConfig`] into the wire
/// [`JournalPlan`] the dispatch carries. Returns `Some` ONLY when journaling
/// would write a durable record — GitHub enabled with BOTH a repo and a token —
/// which is byte-for-byte the gate the journaler itself applies
/// (`fkst_journal::Journaler::start`): with no repo/token it keeps no durable
/// floor, so shipping `None` makes the worker skip journaling for the SAME
/// configs the in-process driver produces nothing durable for. The
/// `flush_interval` `Duration` becomes whole milliseconds (the unit the app
/// `Config` carries it in). The journal-repo token is cloned as a
/// `SecretString` — it never renders in `Debug`/logs.
fn journal_plan(config: &JournalConfig) -> Option<JournalPlan> {
    if !config.github_enabled {
        return None;
    }
    let (repo, token) = match (&config.github_repo, &config.github_token) {
        (Some(repo), Some(token)) => (repo.clone(), token.clone()),
        _ => return None,
    };
    Some(JournalPlan {
        flush_interval_ms: config.flush_interval.as_millis() as u64,
        flush_max_batch: config.flush_max_batch,
        issue_comments: config.issue_comments,
        activity_comment_enabled: config.activity_comment_enabled,
        cas_max_retries: config.cas_max_retries,
        bootstrap_read_retries: config.bootstrap_read_retries,
        github_branch: config.github_branch.clone(),
        github_repo: repo,
        github_api_base: config.github_api_base.clone(),
        identity_pointers: config.identity_pointers.clone(),
        max_line_bytes: config.max_line_bytes,
        github_token: token,
    })
}

/// Resolve the Ornn injection plan for a dispatch, mirroring `drive_inner`'s B6
/// skip conditions: Ornn must be wired, the session must pin skills, and the
/// NyxID session token (the identity the fetch acts as) must be present in the
/// env profile. Any of those absent yields `None` (nothing to fetch), so a
/// session with no pins is unchanged. The presigned URLs in the plan are
/// sensitive and are never logged here.
async fn resolve_ornn_plan(
    inner: &Arc<Inner>,
    session: &SessionDoc,
    env_profile: &std::collections::BTreeMap<String, SecretString>,
) -> Result<Option<OrnnPlan>, DispatchError> {
    let (Some(client), Some(pins)) = (inner.ornn.get(), session.ornn_skills.as_ref()) else {
        return Ok(None);
    };
    if pins.is_empty() {
        return Ok(None);
    }
    let Some(user_token) = env_profile.get(nyxid_token::NYXID_ACCESS_TOKEN_KEY) else {
        // No NyxID identity to fetch the (visibility-gated) pins as: the same
        // condition the in-process B6 treats as a loud failure. The dormant
        // resolver surfaces it as an error so the activation increment can map
        // it to the identical fail-the-start behaviour.
        return Err(DispatchError::OrnnResolve(
            crate::error::AppError::Unprocessable(
                "cannot resolve pinned Ornn skills without a NyxID session token".to_string(),
            ),
        ));
    };
    let plan = crate::ornn::resolve_plan(client, user_token, pins)
        .await
        .map_err(DispatchError::OrnnResolve)?;
    Ok(Some(plan))
}

/// Convert a [`SystemTime`] expiry to unix milliseconds for the wire. A time
/// before the epoch (clock skew) is an error rather than a silent `0`.
fn system_time_to_unix_ms(when: SystemTime) -> Result<i64, DispatchError> {
    when.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .map_err(|_| DispatchError::TokenExpiryUnrepresentable)
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;

#[cfg(test)]
mod journal_plan_tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::time::Duration;

    #[test]
    fn ships_a_plan_only_for_complete_github_config() {
        let base = JournalConfig {
            github_enabled: true,
            github_repo: Some("acme/journal".to_string()),
            github_token: Some(SecretString::from("ghp_tok")),
            flush_interval: Duration::from_millis(1500),
            ..JournalConfig::default()
        };
        // Complete config → a plan with faithfully mapped fields (ms/repo/token).
        let plan = journal_plan(&base).expect("complete config ships a plan");
        assert_eq!(plan.flush_interval_ms, 1500);
        assert_eq!(plan.github_repo, "acme/journal");
        assert_eq!(plan.github_token.expose_secret(), "ghp_tok");
        // Disabled, or any missing GitHub coordinate → no plan (no durable
        // floor), matching the journaler's own start gate.
        let off = |c: JournalConfig| journal_plan(&c).is_none();
        assert!(off(JournalConfig {
            github_enabled: false,
            ..base.clone()
        }));
        assert!(off(JournalConfig {
            github_repo: None,
            ..base.clone()
        }));
        assert!(off(JournalConfig {
            github_token: None,
            ..base
        }));
    }
}
