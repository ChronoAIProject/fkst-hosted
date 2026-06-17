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
//! Stateless cache-bust hint (#141). The handler keeps signature verification,
//! parses the event ONLY to derive the affected `owner/name` set, then evicts
//! the token service's in-memory caches and fails any active session that
//! depended on an affected repo. There is no durable installation record to
//! read or write: the App layer resolves installations on demand and a stale
//! mapping self-corrects at the next mint (the `InstallationGone` backstop). The
//! in-memory eviction is also broadcast cluster-wide via the controller→worker
//! seam on [`crate::github_app::GithubAppTokens::evict_repo`] (a no-op until the
//! channel is wired, #134/#151). The handler is idempotent (GitHub redelivers)
//! and returns `2xx` quickly.
//!
//! Secret discipline: the webhook secret is never logged; the payload is parsed
//! only for the non-secret installation/repository fields used below.

mod verify;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use secrecy::ExposeSecret;
use serde::Deserialize;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

use verify::verify_signature;

/// Header carrying the event type (`installation`, `installation_repositories`).
const EVENT_HEADER: &str = "x-github-event";
/// Fixed reason stamped on a session whose repo lost the App (#108).
const UNINSTALL_REASON_PREFIX: &str = "GitHub App was uninstalled from or lost access to";

// ---- Webhook payload shapes (only the fields we consume) -------------------

/// `installation` event body. Parsed ONLY to derive the affected set (#141): no
/// durable record is written, so `repository_selection` / account type are not
/// consumed — when concrete `repositories` are enumerated we evict those, else
/// we evict account-wide by `account.login`.
#[derive(Debug, Deserialize)]
struct InstallationEvent {
    action: String,
    installation: InstallationObject,
    /// Present on the `created` event (and `installation_repositories`); the
    /// concrete repos the installation covers when the selection is `selected`.
    #[serde(default)]
    repositories: Vec<RepoObject>,
}

/// `installation_repositories` event body. The `action` (`added`/`removed`) is
/// informational only — we evict the `repositories_removed` set directly, so the
/// outcome is correct whichever action GitHub sends.
#[derive(Debug, Deserialize)]
struct InstallationReposEvent {
    action: String,
    installation: InstallationObject,
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

/// A repository object (we only need its `owner/name` full name).
#[derive(Debug, Deserialize)]
struct RepoObject {
    full_name: String,
}

/// Outcome of handling one event, for logging and the response code. Every arm
/// is a `2xx` to GitHub (even "ignored"): a non-2xx triggers a redelivery
/// storm, and an unknown/irrelevant event is not an error.
enum Handled {
    /// Caches were busted (eviction + session-fail) for one or more repos / an
    /// owner. The stateless model has nothing durable to record.
    CacheBusted,
    /// Acknowledged but not acted on (unknown action, or a `created`/`unsuspend`
    /// that needs no cache bust — the next on-demand resolve picks it up).
    Ignored,
}

// ---- Handler ---------------------------------------------------------------

/// `POST /api/v1/github/app/webhook`. See the module docs for the strict
/// verify-then-parse ordering. The route is only mounted when a webhook secret
/// is configured (see `router.rs`), so a `None` secret here is defensive.
#[utoipa::path(
    post,
    path = "/api/v1/github/app/webhook",
    tag = "webhooks",
    operation_id = "github_app_webhook",
    request_body(
        content = serde_json::Value,
        content_type = "application/json",
        description = "Raw GitHub App webhook event (installation / installation_repositories). \
            Authenticated by the `X-Hub-Signature-256` HMAC over the exact body — NOT by a NyxID identity."
    ),
    responses(
        (status = 200, description = "Event handled (e.g. installation caches busted)"),
        (status = 202, description = "Event accepted (no action required)"),
        (status = 401, description = "Missing or mismatched webhook signature"),
        (status = 503, description = "Webhook secret not configured")
    )
)]
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
            Handled::CacheBusted => "cache_busted",
            Handled::Ignored => "ignored",
        }
    }
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
        match self
            .sessions
            .fail_for_uninstalled_repo(owner, name, reason)
            .await
        {
            Ok(count) if count > 0 => {
                tracing::warn!(repo = %format!("{owner}/{name}"), count, "failed active sessions after github app uninstall");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::error!(repo = %format!("{owner}/{name}"), error = %error, "failed to fail sessions after uninstall");
            }
        }
    }

    async fn fail_owner(&self, owner: &str, reason: &str) {
        match self
            .sessions
            .fail_for_uninstalled_owner(owner, reason)
            .await
        {
            Ok(count) if count > 0 => {
                tracing::warn!(owner = %owner, count, "failed active sessions after github app account uninstall/suspend");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::error!(owner = %owner, error = %error, "failed to fail sessions after account uninstall/suspend");
            }
        }
    }
}

/// Handle an `installation` event (#141, cache-bust only): `created` /
/// `unsuspend` need no action (the next on-demand resolve picks the install up);
/// `deleted` / `suspend` evict caches + fail sessions for the affected repos —
/// the enumerated `repositories` when present, else account-wide by login (an
/// `all` install / a bare `deleted` never enumerates concrete repos). Never
/// mints a token.
async fn handle_installation(state: &AppState, body: &[u8]) -> Result<Handled, String> {
    let event: InstallationEvent =
        serde_json::from_slice(body).map_err(|e| format!("installation parse: {e}"))?;
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

/// Handle an `installation_repositories` event (#141, cache-bust only): the
/// `repositories_removed` set is evicted + its sessions failed; `added` needs no
/// action (the next on-demand resolve picks it up). Never mints a token.
async fn handle_installation_repositories(
    state: &AppState,
    body: &[u8],
) -> Result<Handled, String> {
    let event: InstallationReposEvent = serde_json::from_slice(body)
        .map_err(|e| format!("installation_repositories parse: {e}"))?;
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

/// The webhook route, mounted UNAUTHENTICATED in `router.rs` (outside the
/// `/api/v1` auth nest) but signature-verified inside the handler.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(webhook))
}

#[cfg(test)]
mod tests {
    // Signature-verification tests live alongside the verifier in `verify.rs`;
    // these cover the payload parsing + the cache-bust dispatch (#141) over the
    // [`CacheBust`] seam with a recording fake — no `AppState`, no Mongo.
    use super::*;
    use std::sync::Mutex;

    // ---- recording fake -------------------------------------------------------

    /// Records every cache-bust side effect so a test can assert exactly which
    /// repos/owners were evicted + failed. `evict_repo` represents the in-memory
    /// eviction AND the cross-worker broadcast (the production impl's
    /// `GithubAppTokens::evict_repo` fans the eviction out), so its recorded
    /// calls are the "broadcast hook invoked once per affected repo" assertion.
    #[derive(Default)]
    struct FakeCacheBust {
        evicted_repos: Mutex<Vec<String>>,
        evicted_owners: Mutex<Vec<String>>,
        failed_repos: Mutex<Vec<(String, String)>>,
        failed_owners: Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl CacheBust for FakeCacheBust {
        async fn evict_repo(&self, owner: &str, name: &str) {
            self.evicted_repos
                .lock()
                .unwrap()
                .push(format!("{owner}/{name}"));
        }
        async fn evict_owner(&self, owner: &str) {
            self.evicted_owners.lock().unwrap().push(owner.to_string());
        }
        async fn fail_repo(&self, owner: &str, name: &str, reason: &str) {
            self.failed_repos
                .lock()
                .unwrap()
                .push((format!("{owner}/{name}"), reason.to_string()));
        }
        async fn fail_owner(&self, owner: &str, reason: &str) {
            self.failed_owners
                .lock()
                .unwrap()
                .push((owner.to_string(), reason.to_string()));
        }
    }

    // ---- payload parse --------------------------------------------------------

    #[test]
    fn installation_created_parses_selected_repos() {
        let body = br#"{
            "action": "created",
            "installation": {
                "id": 99,
                "account": { "login": "Acme", "type": "Organization" }
            },
            "repositories": [{ "full_name": "Acme/Site" }]
        }"#;
        let event: InstallationEvent = serde_json::from_slice(body).expect("parse");
        assert_eq!(event.action, "created");
        assert_eq!(event.installation.id, 99);
        assert_eq!(event.installation.account.login, "Acme");
        let repos: Vec<String> = event
            .repositories
            .iter()
            .map(|r| canonical(&r.full_name))
            .collect();
        assert_eq!(repos, vec!["acme/site".to_string()]);
    }

    #[tokio::test]
    async fn installation_created_all_selection_uses_owner_wide_eviction() {
        // An `all` install (no enumerated `repositories`) on a `deleted` event
        // selects the owner-wide eviction path, NOT a per-repo one.
        let body = br#"{
            "action": "deleted",
            "installation": {
                "id": 1,
                "account": { "login": "Octocat" }
            }
        }"#;
        let event: InstallationEvent = serde_json::from_slice(body).expect("parse");
        let fake = FakeCacheBust::default();
        let handled = dispatch_installation(&fake, &event)
            .await
            .expect("dispatch");
        assert_eq!(handled.as_str(), "cache_busted");
        // No concrete repos => account-wide eviction by lowercased login.
        assert_eq!(*fake.evicted_owners.lock().unwrap(), vec!["octocat"]);
        assert!(
            fake.evicted_repos.lock().unwrap().is_empty(),
            "no per-repo eviction when nothing is enumerated"
        );
        assert_eq!(fake.failed_owners.lock().unwrap().len(), 1);
    }

    #[test]
    fn installation_repositories_parses_added_removed() {
        let body = br#"{
            "action": "removed",
            "installation": { "id": 5, "account": { "login": "acme" } },
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

    // ---- cache-bust dispatch (#141) ------------------------------------------

    #[tokio::test]
    async fn installation_deleted_evicts_and_fails_without_persistence() {
        // A `deleted` event that enumerates concrete repos evicts + fails each
        // of them; the broadcast hook (the fake's `evict_repo`) is invoked once
        // per affected repo. No Mongo is touched (there is none).
        let body = br#"{
            "action": "deleted",
            "installation": {
                "id": 7,
                "account": { "login": "Acme" }
            },
            "repositories": [
                { "full_name": "Acme/Site" },
                { "full_name": "Acme/Docs" }
            ]
        }"#;
        let event: InstallationEvent = serde_json::from_slice(body).expect("parse");
        let fake = FakeCacheBust::default();
        let handled = dispatch_installation(&fake, &event)
            .await
            .expect("dispatch");
        assert_eq!(handled.as_str(), "cache_busted");

        // evict_repo (= local eviction + cross-worker broadcast) once per repo.
        assert_eq!(
            *fake.evicted_repos.lock().unwrap(),
            vec!["acme/site".to_string(), "acme/docs".to_string()]
        );
        // fail_for_uninstalled_repo called per repo with the uninstall reason.
        let failed = fake.failed_repos.lock().unwrap();
        assert_eq!(failed.len(), 2);
        assert!(failed[0].1.starts_with(UNINSTALL_REASON_PREFIX));
        assert!(failed[0].1.contains("acme/site"));
        // Owner-wide path was NOT taken (concrete repos were enumerated).
        assert!(fake.evicted_owners.lock().unwrap().is_empty());
        assert!(fake.failed_owners.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn installation_repositories_removed_evicts_removed_only() {
        // Only `repositories_removed` is evicted + failed; `repositories_added`
        // is left alone (the next on-demand resolve picks it up).
        let body = br#"{
            "action": "removed",
            "installation": { "id": 5, "account": { "login": "acme" } },
            "repositories_added": [{ "full_name": "acme/fresh" }],
            "repositories_removed": [{ "full_name": "acme/old" }]
        }"#;
        let event: InstallationReposEvent = serde_json::from_slice(body).expect("parse");
        let fake = FakeCacheBust::default();
        let handled = dispatch_installation_repositories(&fake, &event)
            .await
            .expect("dispatch");
        assert_eq!(handled.as_str(), "cache_busted");

        assert_eq!(
            *fake.evicted_repos.lock().unwrap(),
            vec!["acme/old".to_string()],
            "only removed repos are evicted"
        );
        let failed = fake.failed_repos.lock().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "acme/old");
        // The added repo must NOT have been touched.
        assert!(!fake
            .evicted_repos
            .lock()
            .unwrap()
            .contains(&"acme/fresh".to_string()));
    }

    #[tokio::test]
    async fn created_and_unsuspend_are_no_op_cache_busts() {
        // (re)install / unsuspend have nothing to bust: the next on-demand
        // resolve picks the coverage up. The handler never mints.
        for action in ["created", "unsuspend"] {
            let body = format!(
                r#"{{
                    "action": "{action}",
                    "installation": {{ "id": 3, "account": {{ "login": "acme" }} }},
                    "repositories": [{{ "full_name": "acme/site" }}]
                }}"#
            );
            let event: InstallationEvent = serde_json::from_slice(body.as_bytes()).expect("parse");
            let fake = FakeCacheBust::default();
            let handled = dispatch_installation(&fake, &event)
                .await
                .expect("dispatch");
            assert_eq!(handled.as_str(), "ignored", "{action} must be a no-op");
            assert!(fake.evicted_repos.lock().unwrap().is_empty());
            assert!(fake.evicted_owners.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn malformed_installation_body_is_an_error_not_a_panic() {
        // The webhook maps a parse error to 202 (logged); the handler helper
        // surfaces it as `Err` and must not panic. The fake AppState path is
        // not exercised here — `handle_installation` builds the event itself —
        // so we drive the JSON parse boundary directly.
        let bad = br#"{ "action": "deleted", "installation": "not-an-object" }"#;
        let parsed: Result<InstallationEvent, _> = serde_json::from_slice(bad);
        assert!(parsed.is_err(), "malformed body must fail to parse");
    }
}
