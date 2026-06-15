//! GitHub App webhook endpoint (issue #108): `POST /api/v1/github/app/webhook`.
//!
//! UNAUTHENTICATED but signature-verified. GitHub does not present a NyxID
//! identity, so this route is mounted OUTSIDE the `/api/v1` auth nest (like
//! `/health`) and instead authenticates the *sender* by an HMAC over the body:
//!
//! 1. Read the body as raw [`Bytes`] — verification MUST run on the exact bytes
//!    GitHub signed. Deserializing then reserializing changes the bytes and
//!    breaks the MAC, so the order is strictly: read raw -> verify -> parse.
//! 2. Compute `HMAC-SHA256(secret, raw_body)` and compare it in CONSTANT TIME
//!    against the `sha256=<hex>` value in `X-Hub-Signature-256`. A missing or
//!    mismatched signature is `401` (never reveals which check failed).
//! 3. Only then parse `X-GitHub-Event` and dispatch.
//!
//! Handled events upsert/remove the persisted installation record, evict the
//! token service's in-memory caches, and (on uninstall / repo-removal) fail any
//! active session that depended on the affected repo. The handler is idempotent
//! (GitHub redelivers) and returns `2xx` quickly.
//!
//! Secret discipline: the webhook secret is never logged; the payload is parsed
//! only for the non-secret installation/repository fields used below.

mod verify;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use secrecy::ExposeSecret;
use serde::Deserialize;

use crate::github_app::MongoInstallationStore;
use crate::models::{AccountType, GithubInstallationDoc, RepositorySelection, COVERS_ALL};
use crate::state::AppState;

use verify::verify_signature;

/// Header carrying the event type (`installation`, `installation_repositories`).
const EVENT_HEADER: &str = "x-github-event";
/// Fixed reason stamped on a session whose repo lost the App (#108).
const UNINSTALL_REASON_PREFIX: &str = "GitHub App was uninstalled from or lost access to";

// ---- Webhook payload shapes (only the fields we consume) -------------------

/// `installation` event body.
#[derive(Debug, Deserialize)]
struct InstallationEvent {
    action: String,
    installation: InstallationObject,
    /// Present on the `created` event (and `installation_repositories`); the
    /// concrete repos the installation covers when `repository_selection` is
    /// `selected`.
    #[serde(default)]
    repositories: Vec<RepoObject>,
}

/// `installation_repositories` event body. The `action` (`added`/`removed`) is
/// informational only — we reconcile against the added/removed arrays directly,
/// so coverage stays correct whichever action GitHub sends.
#[derive(Debug, Deserialize)]
struct InstallationReposEvent {
    action: String,
    installation: InstallationObject,
    #[serde(default)]
    repositories_added: Vec<RepoObject>,
    #[serde(default)]
    repositories_removed: Vec<RepoObject>,
}

/// The `installation` object shared by both event shapes.
#[derive(Debug, Deserialize)]
struct InstallationObject {
    id: i64,
    account: AccountObject,
    #[serde(default)]
    repository_selection: Option<String>,
}

/// The account (user or org) the App is installed on.
#[derive(Debug, Deserialize)]
struct AccountObject {
    login: String,
    #[serde(rename = "type", default)]
    account_type: String,
}

/// A repository object (we only need its `owner/name` full name).
#[derive(Debug, Deserialize)]
struct RepoObject {
    full_name: String,
}

/// Outcome of handling one event, for logging and the response code. Every arm
/// is a `2xx` to GitHub (even "ignored"): a non-2xx triggers a redelivery
/// storm, and an unknown/irrelevant event is not an error.
enum Handled {
    Upserted,
    Removed,
    Suspended,
    ReposChanged,
    Ignored,
}

// ---- Handler ---------------------------------------------------------------

/// `POST /api/v1/github/app/webhook`. See the module docs for the strict
/// verify-then-parse ordering. The route is only mounted when a webhook secret
/// is configured (see `router.rs`), so a `None` secret here is defensive.
async fn webhook(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    // The secret must be configured for this route to do anything; the router
    // only mounts the route when it is set, so this is a defensive 503.
    let Some(secret) = &state.github_app_webhook_secret else {
        tracing::warn!("github webhook received but no webhook secret configured");
        return StatusCode::SERVICE_UNAVAILABLE;
    };

    // STEP 1+2: verify the HMAC over the RAW bytes BEFORE any JSON parse.
    if !verify_signature(secret.expose_secret().as_bytes(), &headers, &body) {
        // Do not distinguish missing vs mismatched: both are 401, no detail.
        tracing::warn!("github webhook signature verification failed");
        return StatusCode::UNAUTHORIZED;
    }

    // STEP 3: parse the event type, then dispatch on the verified body.
    let event = headers
        .get(EVENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let result = match event.as_str() {
        "installation" => handle_installation(&state, &body).await,
        "installation_repositories" => handle_installation_repositories(&state, &body).await,
        other => {
            // ping / membership / etc. — acknowledged but not acted on.
            tracing::debug!(event = %other, "github webhook event ignored");
            Ok(Handled::Ignored)
        }
    };

    match result {
        Ok(handled) => {
            tracing::info!(event = %event, outcome = handled.as_str(), "github webhook handled");
            StatusCode::OK
        }
        Err(detail) => {
            // A processing failure (e.g. a malformed body or a store error) is
            // logged; we still return 202 so GitHub does not hammer redeliveries
            // for a payload we cannot act on. The detail never contains a secret.
            tracing::error!(event = %event, detail = %detail, "github webhook processing failed");
            StatusCode::ACCEPTED
        }
    }
}

impl Handled {
    fn as_str(&self) -> &'static str {
        match self {
            Handled::Upserted => "upserted",
            Handled::Removed => "removed",
            Handled::Suspended => "suspended",
            Handled::ReposChanged => "repos_changed",
            Handled::Ignored => "ignored",
        }
    }
}

/// Handle an `installation` event: `created` / `unsuspend` upsert the record;
/// `deleted` removes it and fails affected sessions; `suspend` marks it
/// suspended (and fails affected sessions — it can no longer mint).
async fn handle_installation(state: &AppState, body: &[u8]) -> Result<Handled, String> {
    let event: InstallationEvent =
        serde_json::from_slice(body).map_err(|e| format!("installation parse: {e}"))?;
    let store = MongoInstallationStore::new(&state.db);
    let inst = &event.installation;
    let selection = RepositorySelection::from_github(inst.repository_selection.as_deref());

    match event.action.as_str() {
        "created" | "unsuspend" => {
            let repos = covered_repos(selection, &event.repositories);
            let doc = GithubInstallationDoc {
                installation_id: inst.id,
                account_login: inst.account.login.to_lowercase(),
                account_type: AccountType::from_github(&inst.account.account_type),
                repository_selection: selection,
                repos,
                suspended: false,
                updated_at: bson::DateTime::now(),
            };
            store
                .upsert(&doc)
                .await
                .map_err(|e| format!("upsert: {e}"))?;
            Ok(Handled::Upserted)
        }
        "deleted" => {
            // Evict the persisted record AND the in-memory caches for every
            // covered repo, then fail any active session on those repos.
            let repos = persisted_repos(&store, inst.id).await;
            store
                .delete(inst.id)
                .await
                .map_err(|e| format!("delete: {e}"))?;
            evict_and_fail(state, &repos).await;
            Ok(Handled::Removed)
        }
        "suspend" => {
            store
                .set_suspended(inst.id, true)
                .await
                .map_err(|e| format!("suspend: {e}"))?;
            // A suspended install cannot mint; treat it like a removal for live
            // sessions so they fail loudly instead of hitting a silent 401.
            let repos = persisted_repos(&store, inst.id).await;
            evict_and_fail(state, &repos).await;
            Ok(Handled::Suspended)
        }
        other => {
            tracing::debug!(action = %other, "installation action ignored");
            Ok(Handled::Ignored)
        }
    }
}

/// Handle an `installation_repositories` event: `added` records the new repos
/// (so resolution is a persistence hit); `removed` drops them, evicts caches,
/// and fails any active session on the removed repos.
async fn handle_installation_repositories(
    state: &AppState,
    body: &[u8],
) -> Result<Handled, String> {
    let event: InstallationReposEvent = serde_json::from_slice(body)
        .map_err(|e| format!("installation_repositories parse: {e}"))?;
    let store = MongoInstallationStore::new(&state.db);
    let inst = &event.installation;
    tracing::debug!(action = %event.action, installation_id = inst.id, "installation_repositories event");

    // Reconcile against the persisted record so a `selected` install's repo set
    // stays accurate. Start from the known set (if any), add/remove, store.
    let mut current: Vec<String> = persisted_repos(&store, inst.id).await;
    // An `all` install never enumerates repos; leave the sentinel untouched.
    let is_all = current.iter().any(|r| r == COVERS_ALL);

    let added: Vec<String> = event
        .repositories_added
        .iter()
        .map(|r| canonical(&r.full_name))
        .collect();
    let removed: Vec<String> = event
        .repositories_removed
        .iter()
        .map(|r| canonical(&r.full_name))
        .collect();

    if !is_all {
        for repo in &added {
            if !current.contains(repo) {
                current.push(repo.clone());
            }
        }
        current.retain(|r| !removed.contains(r));
        // If we had no persisted record, upsert a fresh one so the next resolve
        // hits persistence; otherwise update the repo set in place.
        if store
            .set_repos(inst.id, &current)
            .await
            .map_err(|e| format!("set_repos: {e}"))?
        {
            // updated in place
        } else {
            let selection = RepositorySelection::from_github(inst.repository_selection.as_deref());
            let doc = GithubInstallationDoc {
                installation_id: inst.id,
                account_login: inst.account.login.to_lowercase(),
                account_type: AccountType::from_github(&inst.account.account_type),
                repository_selection: selection,
                repos: current.clone(),
                suspended: false,
                updated_at: bson::DateTime::now(),
            };
            store
                .upsert(&doc)
                .await
                .map_err(|e| format!("upsert: {e}"))?;
        }
    }

    // Evict + fail sessions for the REMOVED repos (the App can no longer act).
    evict_and_fail(state, &removed).await;
    Ok(Handled::ReposChanged)
}

/// The covered-repo list to persist for a `created`/`unsuspend` event: the
/// [`COVERS_ALL`] sentinel for an `all` install, or the canonical full names
/// for a `selected` install.
fn covered_repos(selection: RepositorySelection, repositories: &[RepoObject]) -> Vec<String> {
    match selection {
        RepositorySelection::All => vec![COVERS_ALL.to_string()],
        RepositorySelection::Selected => repositories
            .iter()
            .map(|r| canonical(&r.full_name))
            .collect(),
    }
}

/// The persisted covered-repo full names for an installation id, or empty when
/// nothing is persisted. A store read error is logged and treated as empty (the
/// eviction below is still best-effort over whatever we know).
async fn persisted_repos(store: &MongoInstallationStore, installation_id: i64) -> Vec<String> {
    match store.get(installation_id).await {
        Ok(Some(doc)) => doc.repos,
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(installation_id, error = %error, "failed to read persisted repos");
            Vec::new()
        }
    }
}

/// For each affected `owner/name` full name: evict the token service's caches
/// and persisted record, then fail any active session targeting that repo. The
/// [`COVERS_ALL`] sentinel is skipped (it is not a concrete repo). Every step is
/// best-effort and idempotent.
async fn evict_and_fail(state: &AppState, repos: &[String]) {
    for full_name in repos {
        if full_name == COVERS_ALL {
            continue;
        }
        let Some((owner, name)) = full_name.split_once('/') else {
            continue;
        };
        if let Some(github_app) = &state.github_app {
            github_app.evict_repo(owner, name).await;
        }
        let reason = format!("{UNINSTALL_REASON_PREFIX} {full_name}");
        match state
            .sessions
            .fail_for_uninstalled_repo(owner, name, &reason)
            .await
        {
            Ok(count) if count > 0 => {
                tracing::warn!(
                    repo = %full_name,
                    count,
                    "failed active sessions after github app uninstall"
                );
            }
            Ok(_) => {}
            Err(error) => {
                tracing::error!(repo = %full_name, error = %error, "failed to fail sessions after uninstall");
            }
        }
    }
}

/// Canonicalize a GitHub `owner/name` full name to the stored lowercase form.
fn canonical(full_name: &str) -> String {
    full_name.to_lowercase()
}

/// The webhook route, mounted UNAUTHENTICATED in `router.rs` (outside the
/// `/api/v1` auth nest) but signature-verified inside the handler.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/github/app/webhook", post(webhook))
}

#[cfg(test)]
mod tests {
    // Signature-verification tests live alongside the verifier in `verify.rs`;
    // these cover the payload parsing + coverage-list derivation.
    use super::*;

    #[test]
    fn installation_created_parses_selected_repos() {
        let body = br#"{
            "action": "created",
            "installation": {
                "id": 99,
                "account": { "login": "Acme", "type": "Organization" },
                "repository_selection": "selected"
            },
            "repositories": [{ "full_name": "Acme/Site" }]
        }"#;
        let event: InstallationEvent = serde_json::from_slice(body).expect("parse");
        assert_eq!(event.action, "created");
        assert_eq!(event.installation.id, 99);
        let selection =
            RepositorySelection::from_github(event.installation.repository_selection.as_deref());
        let repos = covered_repos(selection, &event.repositories);
        assert_eq!(repos, vec!["acme/site".to_string()]);
        assert_eq!(
            AccountType::from_github(&event.installation.account.account_type),
            AccountType::Organization
        );
    }

    #[test]
    fn installation_created_all_selection_uses_sentinel() {
        let body = br#"{
            "action": "created",
            "installation": {
                "id": 1,
                "account": { "login": "octocat", "type": "User" },
                "repository_selection": "all"
            }
        }"#;
        let event: InstallationEvent = serde_json::from_slice(body).expect("parse");
        let selection =
            RepositorySelection::from_github(event.installation.repository_selection.as_deref());
        let repos = covered_repos(selection, &event.repositories);
        assert_eq!(repos, vec![COVERS_ALL.to_string()]);
    }

    #[test]
    fn installation_repositories_parses_added_removed() {
        let body = br#"{
            "action": "removed",
            "installation": { "id": 5, "account": { "login": "acme", "type": "Organization" } },
            "repositories_added": [],
            "repositories_removed": [{ "full_name": "acme/old" }]
        }"#;
        let event: InstallationReposEvent = serde_json::from_slice(body).expect("parse");
        assert_eq!(event.action, "removed");
        assert_eq!(event.repositories_removed.len(), 1);
        assert_eq!(
            canonical(&event.repositories_removed[0].full_name),
            "acme/old"
        );
    }
}
