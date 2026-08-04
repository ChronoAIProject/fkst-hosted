//! Installation-lifecycle webhook events.
//!
//! `installation` and `installation_repositories` share one shape and one job:
//! derive the affected `owner/name` set, nudge the level-based reconciler for each
//! one, bust the App-token caches, and — on a `created`/`added` action — optionally
//! launch best-effort trigger seeding attributed to the verified sender.
//!
//! The side effects sit behind the [`CacheBust`] seam so the dispatch table is
//! unit-testable with a recording fake: no `AppState`, no cluster, no network.
//!
//! Nothing here parses a signature or trusts an unverified body — the handler in
//! [`super`] has already proven the bytes are GitHub's before any of this runs.

use serde::Deserialize;

use crate::models::RepoRef;
use crate::state::AppState;

use super::Handled;

/// Fixed reason stamped on a session whose repo lost the App (#108).
const UNINSTALL_REASON_PREFIX: &str = "GitHub App was uninstalled from or lost access to";

// ---- Webhook payload shapes (only the fields we consume) -------------------

/// `installation` event body. Parsed to derive the affected set (#141) and the
/// human sender used for install-time trigger attribution. No durable record is
/// written, so `repository_selection` / account type are not consumed — when
/// concrete `repositories` are enumerated we evict those, else we evict
/// account-wide by `account.login`.
#[derive(Debug, Deserialize)]
struct InstallationEvent {
    action: String,
    installation: InstallationObject,
    /// Human who initiated the installation action. Optional for compatibility
    /// with old fixtures and defensive handling of incomplete deliveries.
    #[serde(default)]
    sender: Option<SenderObject>,
    /// Present on the `created` event (and `installation_repositories`); the
    /// concrete repos the installation covers when the selection is `selected`.
    #[serde(default)]
    repositories: Vec<RepoObject>,
}

/// `installation_repositories` event body. `added` launches best-effort seeding;
/// cache eviction consumes `repositories_removed` directly.
#[derive(Debug, Deserialize)]
struct InstallationReposEvent {
    action: String,
    installation: InstallationObject,
    #[serde(default)]
    sender: Option<SenderObject>,
    #[serde(default)]
    repositories_added: Vec<RepoObject>,
    #[serde(default)]
    repositories_removed: Vec<RepoObject>,
}

/// The `installation` object shared by both event shapes. We consume `id` for
/// logging and `account.login` for the owner-wide eviction path.
#[derive(Debug, Deserialize)]
struct InstallationObject {
    id: i64,
    account: AccountObject,
}

/// The account (user or org) the App is installed on.
#[derive(Debug, Deserialize)]
struct AccountObject {
    login: String,
}

/// The human who initiated an installation event. Only the login is needed: a
/// bot-authored trigger's effective creator is derived from its sole assignee.
#[derive(Debug, Deserialize)]
struct SenderObject {
    login: String,
}

/// A repository object (we only need its `owner/name` full name).
#[derive(Debug, Deserialize)]
struct RepoObject {
    full_name: String,
}

/// Nudge the Model B reconciler for each repo an installation event names, so it
/// reconverges (spawns for a newly-covered repo, tears down for a removed /
/// suspended one). A no-op unless the reconciler is live (`state.reconciler` is
/// `Some`). Additive to the stateless cache-bust: the reconciler is level-based,
/// so a spurious nudge is harmless. Casing is left as GitHub sends it (its APIs
/// return a consistent canonical `owner/name`, matching the reconciler's own
/// sweep / full-resync producers).
fn enqueue_installation_repos(state: &AppState, installation_id: i64, repos: &[RepoObject]) {
    let Some(reconciler) = &state.reconciler else {
        return;
    };
    for repo in repos {
        if let Some(repo_ref) = repo_ref_from_full_name(&repo.full_name) {
            reconciler.enqueue((installation_id, repo_ref));
        }
    }
}

/// Split a GitHub `owner/name` full name into a [`RepoRef`]. `None` for a malformed
/// name (no `/`), which is skipped rather than enqueued.
fn repo_ref_from_full_name(full_name: &str) -> Option<RepoRef> {
    let (owner, name) = full_name.split_once('/')?;
    Some(RepoRef {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

/// The cache-bust side effects the webhook performs (#141), abstracted behind a
/// seam so the dispatch logic is unit-testable with a recording fake (no live
/// `AppState`, no Mongo — there is none). [`AppState`] is the production impl.
#[async_trait::async_trait]
pub(crate) trait CacheBust: Send + Sync {
    /// Evict the in-memory installation/token caches for one repo AND broadcast
    /// the eviction to other workers (the broadcast is the controller→worker
    /// fan-out on `GithubAppTokens::evict_repo`).
    async fn evict_repo(&self, owner: &str, name: &str);

    /// Evict every in-memory cache entry for `owner`'s repos (account-wide, when
    /// the event enumerates no concrete repos).
    async fn evict_owner(&self, owner: &str);

    /// Fail every active session targeting `owner/name` with `reason`.
    async fn fail_repo(&self, owner: &str, name: &str, reason: &str);

    /// Fail every active session whose repo owner is `owner` with `reason`.
    async fn fail_owner(&self, owner: &str, reason: &str);
}

#[async_trait::async_trait]
impl CacheBust for AppState {
    async fn evict_repo(&self, owner: &str, name: &str) {
        if let Some(github_app) = &self.github_app {
            github_app.evict_repo(owner, name).await;
        }
    }

    async fn evict_owner(&self, owner: &str) {
        if let Some(github_app) = &self.github_app {
            github_app.evict_owner(owner).await;
        }
    }

    async fn fail_repo(&self, owner: &str, name: &str, reason: &str) {
        // v1 has no in-memory session store: a session is a Kubernetes Job, so an
        // App uninstall needs no in-process teardown here — a running session's
        // next token refresh fails and the Job ends on its own. Token caches are
        // evicted by `evict_repo` above.
        tracing::info!(repo = %format!("{owner}/{name}"), reason, "github app uninstalled for repo");
    }

    async fn fail_owner(&self, owner: &str, reason: &str) {
        // No in-memory sessions to fail (see `fail_repo`); token caches are
        // evicted by `evict_owner`.
        tracing::info!(owner, reason, "github app uninstalled for owner");
    }
}

/// Handle an `installation` event. `deleted` / `suspend` evict caches and fail
/// sessions for the enumerated repositories, or account-wide when none are named.
/// `created` optionally spawns best-effort sender-attributed trigger seeding behind
/// `FKST_SEED_TRIGGER_ISSUE_ON_INSTALL`; `unsuspend` has nothing to cache-bust.
/// Seeding runs asynchronously so the webhook returns immediately, and its
/// idempotency probe and failures never affect the webhook's 2xx response.
fn maybe_seed_trigger_issues(
    state: &AppState,
    owner_login: &str,
    installer: Option<&str>,
    repos: &[RepoObject],
) {
    if !state.config.reconcile.seed_trigger_issue_on_install {
        return;
    }
    let owner_repos: Vec<String> = repos.iter().map(|r| canonical(&r.full_name)).collect();
    if owner_repos.is_empty() {
        return;
    }
    let Some(installer) = installer.map(str::trim).filter(|login| !login.is_empty()) else {
        tracing::warn!(
            owner = %owner_login,
            repos = owner_repos.len(),
            "seed: no sender on installation event; skipping seeding (unattributable trigger)"
        );
        return;
    };
    let Some(github) = state.github_app.clone() else {
        tracing::warn!("seed-on-install enabled but the github app is not configured; skipping");
        return;
    };
    let label = state.config.reconcile.substrate_trigger_label.clone();
    let packages = state.config.reconcile.seed_packages.clone();
    let default_manifest = state.config.reconcile.default_manifest.clone();
    let work_label_namespace = state.config.reconcile.work_label_namespace.clone();
    // The seeded intro's dashboard pointer (issue #3379); `None` omits the URL.
    let frontend_url = state.config.log.frontend_url.clone();
    let owner = owner_login.to_string();
    let installer = installer.to_string();
    tokio::spawn(async move {
        crate::reconcile::seed_issue::seed_trigger_issues(
            &github,
            &label,
            &packages,
            default_manifest.as_deref(),
            work_label_namespace.as_deref(),
            &owner,
            &installer,
            &owner_repos,
            frontend_url.as_deref(),
        )
        .await;
    });
}

pub(super) async fn handle_installation(state: &AppState, body: &[u8]) -> Result<Handled, String> {
    let event: InstallationEvent =
        serde_json::from_slice(body).map_err(|e| format!("installation parse: {e}"))?;
    // Model B nudge (PR6): reconcile every enumerated repo (a `deleted`/`suspend`
    // that names concrete repos tears them down; a `created` that names repos
    // spawns for any pending trigger). An account-wide event enumerates no repos,
    // so the periodic full-resync catches it. Additive to the cache-bust below.
    enqueue_installation_repos(state, event.installation.id, &event.repositories);
    if event.action == "created" {
        maybe_seed_trigger_issues(
            state,
            &event.installation.account.login,
            event.sender.as_ref().map(|sender| sender.login.as_str()),
            &event.repositories,
        );
    }
    dispatch_installation(state, &event).await
}

/// Pure dispatch for an `installation` event over the [`CacheBust`] seam (so it
/// is testable with a recording fake). Returns the outcome; the side effects are
/// the eviction + session-fail calls on `effects`.
async fn dispatch_installation<E: CacheBust + ?Sized>(
    effects: &E,
    event: &InstallationEvent,
) -> Result<Handled, String> {
    let login = event.installation.account.login.to_lowercase();
    let repos: Vec<String> = event
        .repositories
        .iter()
        .map(|r| canonical(&r.full_name))
        .collect();

    match event.action.as_str() {
        // A suspended install can no longer mint; treat it like a removal for
        // live sessions so they fail loudly instead of hitting a silent 401.
        "deleted" | "suspend" => {
            if repos.is_empty() {
                // No concrete repos enumerated (an `all` install, or a bare
                // `deleted`): evict + fail account-wide by login.
                evict_and_fail_for_owner(effects, &login).await;
            } else {
                evict_and_fail(effects, &repos).await;
            }
            Ok(Handled::CacheBusted)
        }
        // Nothing to bust on install/unsuspend: the next on-demand resolve picks
        // the (re-)installed coverage up. We deliberately do NOT mint here.
        "created" | "unsuspend" => {
            tracing::debug!(action = %event.action, login = %login, "installation (re)installed; nothing to cache-bust");
            Ok(Handled::Ignored)
        }
        other => {
            tracing::debug!(action = %other, "installation action ignored");
            Ok(Handled::Ignored)
        }
    }
}

/// Handle an `installation_repositories` event: removed repositories are evicted
/// and their sessions failed, while `added` optionally launches asynchronous,
/// sender-attributed trigger seeding.
pub(super) async fn handle_installation_repositories(
    state: &AppState,
    body: &[u8],
) -> Result<Handled, String> {
    let event: InstallationReposEvent = serde_json::from_slice(body)
        .map_err(|e| format!("installation_repositories parse: {e}"))?;
    // Model B nudge (PR6): reconcile both the added AND the removed repos — an
    // added repo may have a pending trigger to spawn, a removed one needs its
    // live session torn down. Additive to the cache-bust below.
    enqueue_installation_repos(state, event.installation.id, &event.repositories_added);
    enqueue_installation_repos(state, event.installation.id, &event.repositories_removed);
    if event.action == "added" {
        maybe_seed_trigger_issues(
            state,
            &event.installation.account.login,
            event.sender.as_ref().map(|sender| sender.login.as_str()),
            &event.repositories_added,
        );
    }
    dispatch_installation_repositories(state, &event).await
}

/// Pure dispatch for an `installation_repositories` event over the [`CacheBust`]
/// seam. Evicts only `repositories_removed` (canonical `owner/name`).
async fn dispatch_installation_repositories<E: CacheBust + ?Sized>(
    effects: &E,
    event: &InstallationReposEvent,
) -> Result<Handled, String> {
    // `repositories_added` requires no action: the next on-demand resolve picks
    // the new coverage up. It is parsed + counted only for traceability; only
    // the removed repos drive a cache bust + session fail.
    tracing::debug!(
        action = %event.action,
        installation_id = event.installation.id,
        added = event.repositories_added.len(),
        removed = event.repositories_removed.len(),
        "installation_repositories event (only removed repos are cache-busted)"
    );
    let removed: Vec<String> = event
        .repositories_removed
        .iter()
        .map(|r| canonical(&r.full_name))
        .collect();
    evict_and_fail(effects, &removed).await;
    Ok(Handled::CacheBusted)
}

/// For each affected `owner/name` full name: evict the token service's caches
/// (which also broadcasts the eviction cluster-wide) and fail any active session
/// targeting that repo. Every step is best-effort and idempotent.
async fn evict_and_fail<E: CacheBust + ?Sized>(effects: &E, repos: &[String]) {
    for full_name in repos {
        let Some((owner, name)) = full_name.split_once('/') else {
            continue;
        };
        effects.evict_repo(owner, name).await;
        let reason = format!("{UNINSTALL_REASON_PREFIX} {full_name}");
        effects.fail_repo(owner, name, &reason).await;
    }
}

/// Account-wide cache bust (#141): evict every cache entry for `login`'s repos
/// and fail every active session whose repo owner matches. Used when an
/// `installation deleted` / `suspend` enumerates no concrete repos.
async fn evict_and_fail_for_owner<E: CacheBust + ?Sized>(effects: &E, login: &str) {
    effects.evict_owner(login).await;
    let reason = format!("{UNINSTALL_REASON_PREFIX} all repos of {login}");
    effects.fail_owner(login, &reason).await;
}

/// Canonicalize a GitHub `owner/name` full name to the stored lowercase form.
fn canonical(full_name: &str) -> String {
    full_name.to_lowercase()
}

#[cfg(test)]
#[path = "installation_tests.rs"]
mod tests;
