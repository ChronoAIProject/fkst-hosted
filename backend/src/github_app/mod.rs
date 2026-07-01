//! GitHub App installation-token service: `token_for_repo` with caching,
//! expiry buffering, and typed "not installed" errors carrying an install URL.
//!
//! The module mints short-lived, repo-scoped GitHub installation tokens on
//! demand. Tokens are cached per `(repo, permissions)` pair and re-minted
//! 5 minutes before expiry. Installation IDs are cached in memory only (#141):
//! the durable installation store was removed, so resolution is stateless —
//! cache hit -> on-demand `GET …/installation` probe -> cache. Each cache entry
//! carries a jittered ~15-minute expiry (`INSTALLATION_TTL_BASE` ± up to
//! `INSTALLATION_TTL_JITTER`) so a worker fleet does not re-probe in lockstep.
//!
//! Cache lock discipline: locks are held for map access only; minting happens
//! outside the lock (rare duplicate mints accepted over lock contention).
//!
//! `InstallationGone` invalidates BOTH caches and makes one transparent
//! re-resolve attempt before surfacing — this is the self-correct path that
//! makes the stateless model safe (a stale mapping is repaired at the next mint).

pub mod api;
pub mod config;
pub mod contents;
pub mod jwt;
pub mod listing;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use rand::Rng;
use regex::Regex;
use secrecy::SecretString;

use api::{GithubApi, HttpGithubApi, InstallationTokenRequest};
use jwt::{build_encoding_key, mint_app_jwt};

/// Re-export config for downstream use (main.rs, error.rs).
pub use config::GithubAppConfig;

/// Re-export API types for downstream consumers.
pub use api::{InstallationId, InstallationToken, TokenPermissions};

/// Re-export the Contents READ helper types (#179): the `get_contents` result
/// shapes + the injectable `ContentsReader` abstraction the pre-flight uses.
pub use contents::{ContentsEntry, ContentsListing, ContentsReader};

/// Re-export the Model B listing transport (#359 PR1): the injectable
/// `GithubListing` abstraction + its HTTP impl and result shapes the reconciler
/// enumerates work with.
pub use listing::{GithubListing, HttpGithubListing, InstallationSummary, IssueSummary};

// `InstallationProbe` is defined in this module; it is `pub` already and needs
// no re-export.

/// Buffer before token expiry at which we re-mint (5 minutes).
const EXPIRY_BUFFER: Duration = Duration::from_secs(300);

/// Base TTL for an in-memory installation-cache entry (15 minutes). With the
/// durable installation store removed (#141), each entry is a cheap optimisation
/// over the on-demand `GET …/installation` probe, so the window is deliberately
/// short: a stale mapping self-corrects at the next mint (the `InstallationGone`
/// backstop), and a shorter window bounds how long a removed install lingers.
const INSTALLATION_TTL_BASE: Duration = Duration::from_secs(900);

/// Per-entry jitter added on top of [`INSTALLATION_TTL_BASE`] (0..=5 minutes).
///
/// why: a fleet of N stateless workers that all cold-probe the same repo would
/// otherwise expire and re-probe in lockstep, synchronising an N-wide stampede
/// against the shared 5000/hr GitHub REST budget every TTL window. Spreading the
/// expiry uniformly over a ±5-minute window de-synchronises the refresh so the
/// re-probes smear across the window instead of bunching at one instant.
const INSTALLATION_TTL_JITTER: Duration = Duration::from_secs(300);

/// Sample a uniform random jitter `Duration` in `0..=INSTALLATION_TTL_JITTER`.
/// Added to [`INSTALLATION_TTL_BASE`] when computing a cache entry's `expires_at`
/// so per-entry expiries de-synchronise across the worker fleet (see the const
/// docs above). The inclusive upper bound is intentional: an entry may live for
/// the full base + max-jitter window.
fn rand_jitter() -> Duration {
    let jitter_secs = INSTALLATION_TTL_JITTER.as_secs();
    Duration::from_secs(rand::thread_rng().gen_range(0..=jitter_secs))
}

/// Regex for validating `owner/repo` format.
static REPO_REF_RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
    Regex::new(r"^[A-Za-z0-9_.\-]+/[A-Za-z0-9_.\-]+$").expect("valid regex")
});

/// GitHub App errors, mapped by `error.rs` to HTTP status codes.
#[derive(Clone, thiserror::Error)]
pub enum GithubAppError {
    #[error("github app not installed on {owner_repo}")]
    NotInstalled {
        owner_repo: String,
        install_url: Option<String>,
    },
    #[error("github app installation vanished for {owner_repo}")]
    InstallationGone { owner_repo: String },
    #[error("github contents path not found: {owner_repo}/{path}")]
    NotFound { owner_repo: String, path: String },
    #[error("github app auth failed (key or app id rejected)")]
    AppAuth,
    #[error("github rate limited; reset in {0}s")]
    RateLimited(u64),
    #[error("github token request rejected")]
    TokenRequestRejected(String),
    #[error("invalid github app private key")]
    InvalidKey,
    #[error("invalid repository reference")]
    InvalidRepoRef,
    #[error("github http error")]
    Http(String),
}

impl std::fmt::Debug for GithubAppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled {
                owner_repo,
                install_url,
            } => f
                .debug_struct("NotInstalled")
                .field("owner_repo", owner_repo)
                .field("install_url", install_url)
                .finish(),
            Self::InstallationGone { owner_repo } => f
                .debug_struct("InstallationGone")
                .field("owner_repo", owner_repo)
                .finish(),
            Self::NotFound { owner_repo, path } => f
                .debug_struct("NotFound")
                .field("owner_repo", owner_repo)
                .field("path", path)
                .finish(),
            Self::AppAuth => write!(f, "AppAuth"),
            Self::RateLimited(secs) => f.debug_tuple("RateLimited").field(secs).finish(),
            // Deliberately redact the detail from Debug: it may contain
            // GitHub error messages with token-like strings.
            Self::TokenRequestRejected(_) => write!(f, "TokenRequestRejected(<redacted>)"),
            Self::InvalidKey => write!(f, "InvalidKey"),
            Self::InvalidRepoRef => write!(f, "InvalidRepoRef"),
            // Redact the HTTP context from Debug as well: it may contain
            // response bodies with sensitive data.
            Self::Http(_) => write!(f, "Http(<redacted>)"),
        }
    }
}

/// Outcome of [`GithubAppTokens::probe_installation`] (issue #108): the App's
/// install state for a repo, used to drive the new-repo install hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallationProbe {
    /// A token minted: installed and granted the needed permissions.
    Installed,
    /// No installation covers the repo; the user must install the App.
    NotInstalled { install_url: Option<String> },
    /// An installation exists but the requested permission is still pending
    /// (an org owner must approve the new `administration` permission per #110).
    AwaitingApproval,
}

/// Default session permissions: admin-equivalent access to the target repo for
/// the whole session (issue #110). `administration: write` is GitHub's closest
/// analogue to a repo-admin role (branch protection / rulesets, collaborator &
/// team management, repo settings, visibility, rename/transfer, deploy keys).
///
/// The mint can only request a subset of what the GitHub App was granted; the
/// App must declare these as Read & write Repository permissions or the mint
/// returns 422 (see `docs/github-app.md`). `metadata` is omitted because
/// installation tokens always include `metadata: read` implicitly.
pub fn default_permissions() -> TokenPermissions {
    TokenPermissions {
        contents: Some("write".to_string()),
        pull_requests: Some("write".to_string()),
        issues: Some("write".to_string()),
        administration: Some("write".to_string()),
        metadata: None,
    }
}

/// Least-privilege permissions for the SESSION POD's installation token
/// (Model B, issue #359). The pod needs to push commits (`contents`), manage the
/// driving issue and its labels (`issues`), and open pull requests
/// (`pull_requests`) — but it must NOT administer the repo. This deliberately
/// withholds the `administration: write` that [`default_permissions`] requests,
/// scoping the longer-lived per-session token to the minimum the engine needs
/// (least privilege). `metadata` is omitted because installation tokens always
/// include `metadata: read` implicitly.
pub fn session_permissions() -> TokenPermissions {
    TokenPermissions {
        contents: Some("write".to_string()),
        issues: Some("write".to_string()),
        pull_requests: Some("write".to_string()),
        administration: None,
        metadata: None,
    }
}

// ---------------------------------------------------------------------------
// Cross-worker eviction broadcast seam (#141)
// ---------------------------------------------------------------------------

/// Cross-worker installation-eviction broadcast seam (#141).
///
/// The stateless model (#141) keeps no durable installation record: each pod
/// resolves on demand and caches in memory. The webhook terminates on whichever
/// pod serves public ingress (the controller), so after that pod busts its own
/// caches it must fan the eviction out to every other worker — otherwise a
/// worker's local cache could keep minting for a repo the App was uninstalled
/// from until its own TTL lapses.
///
/// This is the injectable seam for that fan-out. The default
/// [`NoopEvictionBroadcaster`] is wired today: the controller→worker "evict"
/// channel does not yet exist (worker engine execution was deferred to #151;
/// the internal protocol currently only carries register/heartbeat/pull/
/// draining/released), so the controller-local eviction + session-fail still
/// runs and the broadcast is a no-op. Swapping in a real broadcaster that writes
/// to the controller→worker channel is then a one-line change at construction.
///
/// `Send + Sync` so it can live behind the `Arc` shared by every `GithubAppTokens`
/// clone; the method is intentionally synchronous and best-effort (a dead or
/// unreachable worker is logged by the implementation, never fatal — it
/// self-corrects at its next mint via the `InstallationGone` backstop).
pub trait InstallationEvictionBroadcaster: Send + Sync + std::fmt::Debug {
    /// Fan an eviction of `owner/name` out to every other worker (best-effort).
    fn broadcast_evict(&self, owner: &str, name: &str);
}

/// Default no-op broadcaster: the controller→worker evict channel is not wired
/// yet (#134/#151), so the cluster-wide fan-out degrades to controller-local
/// eviction. Each worker self-corrects on its own TTL lapse / next-mint
/// `InstallationGone` until the real channel lands.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopEvictionBroadcaster;

impl InstallationEvictionBroadcaster for NoopEvictionBroadcaster {
    fn broadcast_evict(&self, owner: &str, name: &str) {
        // TODO(#134/#151): fan eviction out to workers over the
        // controller→worker channel. Until that channel exists, the broadcast
        // is a no-op and each worker self-corrects at its next mint.
        tracing::debug!(
            owner = %owner,
            name = %name,
            "installation eviction broadcast is a no-op (controller→worker channel not wired)"
        );
    }
}

// ---------------------------------------------------------------------------
// Cache types
// ---------------------------------------------------------------------------

/// Cache key: `(owner_repo, permissions_hash)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TokenKey {
    owner_repo: String,
    perms: TokenPermissions,
}

#[derive(Debug)]
struct CachedToken {
    token: SecretString,
    expires_at: SystemTime,
}

#[derive(Debug)]
struct CachedInstallation {
    id: InstallationId,
    /// Absolute expiry, computed at insert time as
    /// `now + INSTALLATION_TTL_BASE + rand_jitter()`. Storing the absolute
    /// instant (instead of a `resolved_at` + a fixed TTL) lets each entry carry
    /// its own jittered lifetime so expiries de-synchronise across the fleet.
    expires_at: SystemTime,
}

// ---------------------------------------------------------------------------
// Inner (shared state)
// ---------------------------------------------------------------------------

struct Inner {
    app_id: u64,
    encoding_key: jsonwebtoken::EncodingKey,
    app_slug: Option<String>,
    /// GitHub REST base URL, retained so the Contents READ helper (#179) can
    /// build its direct-`reqwest` transport against the SAME base the `api`
    /// transport uses (trailing `/` trimmed).
    api_base: String,
    api: std::sync::Arc<dyn GithubApi>,
    /// Cross-worker eviction fan-out (#141). `evict_repo` calls this after the
    /// local cache bust so the eviction reaches every other worker. Defaults to
    /// [`NoopEvictionBroadcaster`] until the controller→worker channel is wired
    /// (#134/#151).
    eviction_broadcaster: std::sync::Arc<dyn InstallationEvictionBroadcaster>,
    token_cache: Mutex<HashMap<TokenKey, CachedToken>>,
    installation_cache: Mutex<HashMap<String, CachedInstallation>>,
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Clonable handle to the GitHub App token service.
#[derive(Clone)]
pub struct GithubAppTokens {
    inner: std::sync::Arc<Inner>,
}

impl std::fmt::Debug for GithubAppTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubAppTokens")
            .field("app_id", &self.inner.app_id)
            .field("app_slug", &self.inner.app_slug)
            .finish()
    }
}

impl GithubAppTokens {
    /// Construct the service with the default `HttpGithubApi` transport.
    pub fn new(config: &GithubAppConfig) -> Result<Self, GithubAppError> {
        let api = HttpGithubApi::new(&config.api_base)?;
        Self::with_api(config, std::sync::Arc::new(api))
    }

    /// Construct the service with an injected transport (for tests). Uses the
    /// default [`NoopEvictionBroadcaster`] (#141) — the controller→worker evict
    /// channel is not wired yet, so the cross-worker fan-out degrades to
    /// controller-local eviction.
    pub fn with_api(
        config: &GithubAppConfig,
        api: std::sync::Arc<dyn GithubApi>,
    ) -> Result<Self, GithubAppError> {
        Self::with_api_and_broadcaster(config, api, std::sync::Arc::new(NoopEvictionBroadcaster))
    }

    /// Construct with an injected transport AND eviction broadcaster (#141).
    /// `pub(crate)` so the webhook layer and unit tests can inject a recording
    /// broadcaster to assert the cross-worker fan-out is invoked; the public
    /// constructors remain [`Self::new`] / [`Self::with_api`], both of which
    /// default to [`NoopEvictionBroadcaster`].
    pub(crate) fn with_api_and_broadcaster(
        config: &GithubAppConfig,
        api: std::sync::Arc<dyn GithubApi>,
        eviction_broadcaster: std::sync::Arc<dyn InstallationEvictionBroadcaster>,
    ) -> Result<Self, GithubAppError> {
        let encoding_key =
            build_encoding_key(&config.private_key_pem).map_err(|_| GithubAppError::InvalidKey)?;
        Ok(Self {
            inner: std::sync::Arc::new(Inner {
                app_id: config.app_id,
                encoding_key,
                app_slug: config.app_slug.clone(),
                api_base: config.api_base.trim_end_matches('/').to_string(),
                api,
                eviction_broadcaster,
                token_cache: Mutex::new(HashMap::new()),
                installation_cache: Mutex::new(HashMap::new()),
            }),
        })
    }

    /// Mint (or return cached) installation token for `owner/repo`, returning
    /// only the token. Thin wrapper over [`Self::token_with_expiry_for_repo`]
    /// for callers that do not need the expiry.
    ///
    /// - `perms`: `None` => [`default_permissions()`].
    /// - `owner_repo` must match `^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$`.
    /// - Token cached per `(repo, perms)` until 5 min before expiry.
    /// - `InstallationGone` invalidates both caches + one re-resolve.
    pub async fn token_for_repo(
        &self,
        owner_repo: &str,
        perms: Option<TokenPermissions>,
    ) -> Result<SecretString, GithubAppError> {
        Ok(self.token_with_expiry_for_repo(owner_repo, perms).await?.0)
    }

    /// Post a comment on `owner/repo`'s issue `number` as the App: mint an
    /// installation token, then call the GitHub issues API. Used by the Job
    /// watcher (no user token present) to report a session's final disposition.
    pub async fn post_issue_comment(
        &self,
        owner_repo: &str,
        number: u64,
        body: &str,
    ) -> Result<(), GithubAppError> {
        let (owner, repo) = owner_repo
            .split_once('/')
            .ok_or(GithubAppError::InvalidRepoRef)?;
        let token = self.token_for_repo(owner_repo, None).await?;
        self.inner
            .api
            .create_issue_comment(&token, owner, repo, number, body)
            .await
    }

    /// Add labels to `owner/repo`'s issue `number` as the App (additive).
    pub async fn add_issue_labels(
        &self,
        owner_repo: &str,
        number: u64,
        labels: &[String],
    ) -> Result<(), GithubAppError> {
        let (owner, repo) = owner_repo
            .split_once('/')
            .ok_or(GithubAppError::InvalidRepoRef)?;
        let token = self.token_for_repo(owner_repo, None).await?;
        self.inner
            .api
            .add_issue_labels(&token, owner, repo, number, labels)
            .await
    }

    /// Remove ONE label from `owner/repo`'s issue `number` as the App.
    pub async fn remove_issue_label(
        &self,
        owner_repo: &str,
        number: u64,
        label: &str,
    ) -> Result<(), GithubAppError> {
        let (owner, repo) = owner_repo
            .split_once('/')
            .ok_or(GithubAppError::InvalidRepoRef)?;
        let token = self.token_for_repo(owner_repo, None).await?;
        self.inner
            .api
            .remove_issue_label(&token, owner, repo, number, label)
            .await
    }

    /// Mint (or return cached) installation token for `owner/repo`, returning the
    /// token AND its `expires_at` (issue #107). The expiry is what the goal-token
    /// file writer records as RFC3339 so the credential helper can decide whether
    /// to force a just-in-time re-mint. Same caching / `InstallationGone`
    /// semantics as [`Self::token_for_repo`].
    pub async fn token_with_expiry_for_repo(
        &self,
        owner_repo: &str,
        perms: Option<TokenPermissions>,
    ) -> Result<(SecretString, SystemTime), GithubAppError> {
        self.token_with_expiry_inner(owner_repo, perms, false).await
    }

    /// Like [`Self::token_with_expiry_for_repo`] but ALWAYS re-mints, bypassing the
    /// cache-hit. The token-rotation loop uses this so a long-lived session's mounted
    /// token is extended a FULL TTL every interval: the cached path only re-mints in
    /// the last [`EXPIRY_BUFFER`] before expiry, so a rotation interval longer than
    /// that buffer would otherwise leave the Secret holding an expired token between
    /// the rotation that ran too early and the next one.
    pub async fn token_with_expiry_for_repo_forced(
        &self,
        owner_repo: &str,
        perms: Option<TokenPermissions>,
    ) -> Result<(SecretString, SystemTime), GithubAppError> {
        self.token_with_expiry_inner(owner_repo, perms, true).await
    }

    /// Shared implementation: mint (or, unless `force_refresh`, return a still-valid
    /// cached) installation token for `owner/repo` plus its `expires_at`.
    async fn token_with_expiry_inner(
        &self,
        owner_repo: &str,
        perms: Option<TokenPermissions>,
        force_refresh: bool,
    ) -> Result<(SecretString, SystemTime), GithubAppError> {
        if !REPO_REF_RE.is_match(owner_repo) {
            return Err(GithubAppError::InvalidRepoRef);
        }

        let perms = perms.unwrap_or_else(default_permissions);
        let key = TokenKey {
            owner_repo: owner_repo.to_string(),
            perms: perms.clone(),
        };

        // 1. Check token cache (lock held for map access only) — UNLESS a forced
        //    refresh (the rotation loop) is deliberately extending a live session's
        //    token ahead of the cache's own 5-min-before-expiry re-mint window.
        if !force_refresh {
            let cache = self.inner.token_cache.lock().expect("token cache lock");
            if let Some(cached) = cache.get(&key) {
                if cached.expires_at > SystemTime::now() + EXPIRY_BUFFER {
                    tracing::debug!(
                        owner_repo = %owner_repo,
                        "github app token cache hit"
                    );
                    return Ok((cached.token.clone(), cached.expires_at));
                }
            }
        }

        // 2. Resolve installation ID (from cache or API).
        let install_id = self.resolve_installation(owner_repo).await?;

        // 3. Mint token outside the lock.
        let (_, bare_repo_name) = owner_repo.split_once('/').expect("validated by regex");
        let req = InstallationTokenRequest {
            repositories: vec![bare_repo_name.to_string()],
            permissions: Some(perms.clone()),
        };

        let app_jwt = mint_app_jwt(self.inner.app_id, &self.inner.encoding_key)
            .map_err(|e| GithubAppError::Http(format!("jwt mint: {e}")))?;

        let token_result = self
            .inner
            .api
            .create_installation_token(&app_jwt, install_id, &req)
            .await;

        let token_result = match token_result {
            Ok(token) => token,
            Err(GithubAppError::InstallationGone { .. }) => {
                // Invalidate both caches and retry once.
                tracing::warn!(
                    owner_repo = %owner_repo,
                    "installation gone; invalidating caches and retrying"
                );
                self.invalidate_caches_for_repo(owner_repo);
                let install_id = self.resolve_installation(owner_repo).await?;
                let app_jwt = mint_app_jwt(self.inner.app_id, &self.inner.encoding_key)
                    .map_err(|e| GithubAppError::Http(format!("jwt mint retry: {e}")))?;
                self.inner
                    .api
                    .create_installation_token(&app_jwt, install_id, &req)
                    .await?
            }
            Err(GithubAppError::TokenRequestRejected(detail)) => {
                // A 422 here almost always means the GitHub App was not granted
                // a permission we requested (e.g. `administration`), so the mint
                // can only subset what the App holds. Surface it loudly at the
                // mint site so it is diagnosable on EVERY caller path (including
                // the background token refresh, which otherwise only logs the
                // permission-less Display string). The detail is GitHub's 422
                // message describing the rejected permission, never the token
                // (the token only appears in a 201 success body).
                tracing::error!(
                    owner_repo = %owner_repo,
                    detail = %detail,
                    "github installation-token mint rejected (422); verify the \
                     fkst-hosted GitHub App declares the requested Repository \
                     permissions (administration, pull_requests, contents, \
                     issues) at Read & write and the install was re-approved"
                );
                return Err(GithubAppError::TokenRequestRejected(detail));
            }
            Err(e) => return Err(e),
        };

        tracing::debug!(
            owner_repo = %owner_repo,
            "github app token minted"
        );

        // 4. Store in cache.
        let expires_at = token_result.expires_at;
        {
            let mut cache = self.inner.token_cache.lock().expect("token cache lock");
            cache.insert(
                key,
                CachedToken {
                    token: token_result.token.clone(),
                    expires_at,
                },
            );
        }

        Ok((token_result.token, expires_at))
    }

    /// The configured GitHub REST base URL (trailing `/` trimmed). Used by the
    /// Contents READ helper (#179) to build its transport against the same base.
    pub(crate) fn api_base(&self) -> String {
        self.inner.api_base.clone()
    }

    /// Mint a short-lived App JWT (Bearer credential for the `/app/*` endpoints).
    /// The Model B reconciler's full-resync uses it to enumerate the App's
    /// installations ([`GithubListing::list_installations`]). Never logged.
    pub fn app_jwt(&self) -> Result<SecretString, GithubAppError> {
        mint_app_jwt(self.inner.app_id, &self.inner.encoding_key)
            .map_err(|e| GithubAppError::Http(format!("jwt mint: {e}")))
    }

    /// Mint an installation-WIDE token (all repos the installation covers, default
    /// permissions) for `installation_id`. The Model B reconciler's full-resync
    /// uses it to list the installation's repositories
    /// ([`GithubListing::list_installation_repos`]); unlike
    /// [`Self::token_for_repo`] it is not scoped to a single repo and is not
    /// cached (it is minted once per resync tick). Never logged.
    pub async fn installation_wide_token(
        &self,
        installation_id: i64,
    ) -> Result<SecretString, GithubAppError> {
        let app_jwt = self.app_jwt()?;
        let req = InstallationTokenRequest {
            repositories: Vec::new(),
            permissions: None,
        };
        let token = self
            .inner
            .api
            .create_installation_token(&app_jwt, InstallationId(installation_id as u64), &req)
            .await?;
        Ok(token.token)
    }

    /// The install URL for this app (if slug is configured).
    pub fn install_url(&self) -> Option<String> {
        self.inner
            .app_slug
            .as_ref()
            .map(|slug| format!("https://github.com/apps/{slug}/installations/new"))
    }

    /// Probe whether the App can act on `owner/repo` by attempting a token mint
    /// (issue #108, new-repo install bridge). The mint IS the authoritative
    /// installation check: it resolves the installation AND verifies the App
    /// holds the requested permissions. Distinguishes three states:
    ///
    /// - [`InstallationProbe::Installed`] — a token minted (the App is installed
    ///   and granted the needed permissions).
    /// - [`InstallationProbe::NotInstalled`] — no installation covers the repo;
    ///   carries the install URL for actionable guidance.
    /// - [`InstallationProbe::AwaitingApproval`] — an installation EXISTS but the
    ///   requested permission is still pending (a 422 on mint): for an org this
    ///   is the owner-approval-pending state, not a hard failure.
    ///
    /// Other errors (auth, rate limit, transport) surface as `Err` so the
    /// caller can map them to the right status — the probe is only about the
    /// install lifecycle, not infrastructure failures.
    pub async fn probe_installation(
        &self,
        owner_repo: &str,
    ) -> Result<InstallationProbe, GithubAppError> {
        match self.token_for_repo(owner_repo, None).await {
            Ok(_) => Ok(InstallationProbe::Installed),
            Err(GithubAppError::NotInstalled { .. }) => Ok(InstallationProbe::NotInstalled {
                install_url: self.install_url(),
            }),
            Err(GithubAppError::InstallationGone { .. }) => Ok(InstallationProbe::NotInstalled {
                install_url: self.install_url(),
            }),
            Err(GithubAppError::TokenRequestRejected(_)) => Ok(InstallationProbe::AwaitingApproval),
            Err(other) => Err(other),
        }
    }

    /// Resolve the installation ID for a repo. Stateless (#141), in order:
    ///   1. the in-memory installation cache (jittered ~15-min TTL);
    ///   2. the on-demand `GET /repos/{owner}/{repo}/installation` GitHub probe,
    ///      whose result is cached so the next resolve is a cache hit.
    ///
    /// The durable installation store was removed (#141): a cold pod probes on
    /// demand and a stale mapping self-corrects at the next mint (the
    /// `InstallationGone` backstop in `token_with_expiry_for_repo`).
    async fn resolve_installation(
        &self,
        owner_repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        // 1. In-memory cache (each entry carries its own jittered expiry).
        {
            let cache = self
                .inner
                .installation_cache
                .lock()
                .expect("installation cache lock");
            if let Some(cached) = cache.get(owner_repo) {
                if cached.expires_at > SystemTime::now() {
                    return Ok(cached.id);
                }
            }
        }

        let (owner, repo) = owner_repo.split_once('/').expect("validated by regex");

        // 2. On-demand GitHub probe.
        let app_jwt = mint_app_jwt(self.inner.app_id, &self.inner.encoding_key)
            .map_err(|e| GithubAppError::Http(format!("jwt mint for installation: {e}")))?;

        let install_id = self
            .inner
            .api
            .installation_for_repo(&app_jwt, owner, repo)
            .await
            .map_err(|e| match e {
                GithubAppError::NotInstalled { .. } => GithubAppError::NotInstalled {
                    owner_repo: owner_repo.to_string(),
                    install_url: self.install_url(),
                },
                other => other,
            })?;

        self.cache_installation(owner_repo, install_id);

        Ok(install_id)
    }

    /// Insert/refresh the in-memory installation cache entry for a repo. The
    /// entry's `expires_at` is `now + INSTALLATION_TTL_BASE + rand_jitter()`, so
    /// each entry carries its own jittered lifetime (#141).
    fn cache_installation(&self, owner_repo: &str, id: InstallationId) {
        let expires_at = SystemTime::now() + INSTALLATION_TTL_BASE + rand_jitter();
        let mut cache = self
            .inner
            .installation_cache
            .lock()
            .expect("installation cache lock");
        cache.insert(
            owner_repo.to_string(),
            CachedInstallation { id, expires_at },
        );
    }

    /// Evict the in-memory caches for a repo (#141) and broadcast the eviction
    /// to other workers. Called by the webhook handler on an `installation
    /// deleted` / `repository removed` event so the next mint correctly 404s
    /// instead of reusing a stale id. There is no durable record to forget — the
    /// stateless model relies on the next-mint `InstallationGone` self-correct
    /// for any pod that misses the broadcast.
    #[allow(clippy::unused_async)] // kept async: callers await it and a future broadcast hook may await
    pub async fn evict_repo(&self, owner: &str, repo: &str) {
        let owner_repo = format!("{owner}/{repo}");
        self.invalidate_caches_for_repo(&owner_repo);
        // Fan the eviction out to every other worker (best-effort; a dead worker
        // is logged by the broadcaster, never fatal — it self-corrects at mint).
        self.inner.eviction_broadcaster.broadcast_evict(owner, repo);
        tracing::info!(owner_repo = %owner_repo, "evicted installation caches for repo");
    }

    /// Evict every in-memory cache entry for `owner`'s repos (#141). Used by the
    /// webhook for the account-wide uninstall path (an `installation deleted` /
    /// `suspend` that enumerates no concrete repos): every cache key prefixed
    /// `"{owner}/"` is dropped from both caches so the next mint for any of that
    /// owner's repos correctly re-probes (and 404s if the App is gone).
    ///
    /// No cross-worker broadcast is fanned here: the broadcaster seam is
    /// per-repo and this path has no enumerated repos to name. Each other worker
    /// self-corrects at its next mint via the `InstallationGone` backstop.
    #[allow(clippy::unused_async)] // kept async to mirror evict_repo and stay await-compatible for callers
    pub async fn evict_owner(&self, owner: &str) {
        let prefix = format!("{owner}/");
        {
            let mut cache = self.inner.token_cache.lock().expect("token cache lock");
            cache.retain(|k, _| !k.owner_repo.starts_with(&prefix));
        }
        {
            let mut cache = self
                .inner
                .installation_cache
                .lock()
                .expect("installation cache lock");
            cache.retain(|k, _| !k.starts_with(&prefix));
        }
        tracing::info!(owner = %owner, "evicted installation caches for owner (account-wide)");
    }

    /// Invalidate both token and installation caches for a repo.
    fn invalidate_caches_for_repo(&self, owner_repo: &str) {
        {
            let mut cache = self.inner.token_cache.lock().expect("token cache lock");
            cache.retain(|k, _| k.owner_repo != owner_repo);
        }
        {
            let mut cache = self
                .inner
                .installation_cache
                .lock()
                .expect("installation cache lock");
            cache.remove(owner_repo);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use secrecy::{ExposeSecret, SecretString};

    use super::api::{
        GithubApi, InstallationId, InstallationToken, InstallationTokenRequest, TokenPermissions,
    };
    use super::config::GithubAppConfig;
    use super::*;

    // ---- recording eviction broadcaster (#141) -------------------------------

    /// Records every cross-worker eviction broadcast so a test can assert the
    /// fan-out seam fires exactly once per evicted repo.
    #[derive(Debug, Default)]
    struct RecordingBroadcaster {
        broadcasts: std::sync::Mutex<Vec<String>>,
    }

    impl InstallationEvictionBroadcaster for RecordingBroadcaster {
        fn broadcast_evict(&self, owner: &str, name: &str) {
            self.broadcasts
                .lock()
                .unwrap()
                .push(format!("{owner}/{name}"));
        }
    }

    // ---- fake transport -------------------------------------------------------

    #[derive(Debug, Default)]
    struct FakeApi {
        installation_id: InstallationId,
        mint_count: AtomicUsize,
        installation_calls: AtomicUsize,
        /// If set, the next token mint returns Gone.
        next_mint_gone: std::sync::Mutex<bool>,
        /// If set, the next installation lookup returns NotInstalled.
        next_install_not_found: std::sync::Mutex<bool>,
        /// Records the `repositories` of the most recent token-mint request
        /// (regression coverage for #276 — owner vs repo name).
        last_mint_repos: std::sync::Mutex<Vec<String>>,
    }

    impl FakeApi {
        fn new(id: u64) -> Self {
            Self {
                installation_id: InstallationId(id),
                ..Self::default()
            }
        }

        fn mint_count(&self) -> usize {
            self.mint_count.load(Ordering::SeqCst)
        }

        fn installation_calls(&self) -> usize {
            self.installation_calls.load(Ordering::SeqCst)
        }

        fn last_mint_repos(&self) -> Vec<String> {
            self.last_mint_repos.lock().unwrap().clone()
        }

        fn set_next_mint_gone(&self, gone: bool) {
            *self.next_mint_gone.lock().unwrap() = gone;
        }

        fn set_next_install_not_found(&self, not_found: bool) {
            *self.next_install_not_found.lock().unwrap() = not_found;
        }
    }

    #[async_trait]
    impl GithubApi for FakeApi {
        async fn installation_for_repo(
            &self,
            _app_jwt: &SecretString,
            owner: &str,
            repo: &str,
        ) -> Result<InstallationId, GithubAppError> {
            self.installation_calls.fetch_add(1, Ordering::SeqCst);
            if *self.next_install_not_found.lock().unwrap() {
                return Err(GithubAppError::NotInstalled {
                    owner_repo: format!("{owner}/{repo}"),
                    install_url: None,
                });
            }
            Ok(self.installation_id)
        }

        async fn create_installation_token(
            &self,
            _app_jwt: &SecretString,
            id: InstallationId,
            req: &InstallationTokenRequest,
        ) -> Result<InstallationToken, GithubAppError> {
            *self.last_mint_repos.lock().unwrap() = req.repositories.clone();
            self.mint_count.fetch_add(1, Ordering::SeqCst);
            if *self.next_mint_gone.lock().unwrap() {
                *self.next_mint_gone.lock().unwrap() = false;
                return Err(GithubAppError::InstallationGone {
                    owner_repo: String::new(),
                });
            }
            Ok(InstallationToken {
                token: SecretString::from(format!(
                    "ghs_fake_{}_{}",
                    id.0,
                    self.mint_count.load(Ordering::SeqCst)
                )),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
            })
        }
    }

    fn test_config() -> GithubAppConfig {
        use rand::rngs::OsRng;
        use rsa::pkcs8::{EncodePrivateKey, LineEnding};
        use rsa::RsaPrivateKey;
        let mut rng = OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("key");
        let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");

        GithubAppConfig {
            app_id: 42,
            private_key_pem: SecretString::from(pem.to_string()),
            app_slug: Some("fkst-test".to_string()),
            webhook_secret: None,
            api_base: "https://api.github.com".to_string(),
        }
    }

    fn service(api: Arc<dyn GithubApi>) -> GithubAppTokens {
        let config = test_config();
        GithubAppTokens::with_api(&config, api).expect("service")
    }

    // ---- tests ----------------------------------------------------------------

    #[tokio::test]
    async fn cache_hit_within_expiry() {
        let api = Arc::new(FakeApi::new(1));
        let svc = service(api.clone());

        let t1 = svc.token_for_repo("acme/site", None).await.expect("first");
        let t2 = svc.token_for_repo("acme/site", None).await.expect("second");
        assert_eq!(t1.expose_secret(), t2.expose_secret(), "must be same token");
        assert_eq!(api.mint_count(), 1, "only one mint");
    }

    #[tokio::test]
    async fn mint_scopes_token_to_repo_name_not_owner() {
        // Regression for #276: the installation token must be scoped to the
        // repository NAME (the part after the slash), not the owner (before it).
        // Sending the owner makes GitHub 422 "repository does not exist".
        let api = Arc::new(FakeApi::new(5));
        let svc = service(api.clone());

        svc.token_for_repo("chronoai-shining/hr-fkst", None)
            .await
            .expect("mint");

        assert_eq!(
            api.last_mint_repos(),
            vec!["hr-fkst".to_string()],
            "token must be scoped to the repo name, not the owner"
        );
    }

    #[tokio::test]
    async fn re_mint_inside_buffer() {
        let api = Arc::new(FakeApi::new(1));
        let config = test_config();
        let svc = GithubAppTokens::with_api(&config, api.clone()).expect("svc");

        // Inject an about-to-expire token directly into the cache.
        let key = TokenKey {
            owner_repo: "acme/site".to_string(),
            perms: default_permissions(),
        };
        {
            let mut cache = svc.inner.token_cache.lock().unwrap();
            cache.insert(
                key,
                CachedToken {
                    token: SecretString::from("ghs_expired".to_string()),
                    expires_at: SystemTime::now() + Duration::from_secs(100), // inside 300s buffer
                },
            );
        }

        let t = svc.token_for_repo("acme/site", None).await.expect("ok");
        assert_ne!(t.expose_secret(), "ghs_expired", "must re-mint");
        assert_eq!(api.mint_count(), 1);
    }

    #[tokio::test]
    async fn token_with_expiry_returns_a_future_expiry_and_caches() {
        let api = Arc::new(FakeApi::new(7));
        let svc = service(api.clone());

        let (t1, exp1) = svc
            .token_with_expiry_for_repo("acme/site", None)
            .await
            .expect("first");
        assert!(exp1 > SystemTime::now(), "expiry must be in the future");
        // A second call is a cache hit: same token, same expiry, no new mint.
        let (t2, exp2) = svc
            .token_with_expiry_for_repo("acme/site", None)
            .await
            .expect("second");
        assert_eq!(t1.expose_secret(), t2.expose_secret());
        assert_eq!(exp1, exp2, "cached expiry must be stable");
        assert_eq!(api.mint_count(), 1, "only one mint across both calls");
    }

    #[tokio::test]
    async fn distinct_perms_get_distinct_tokens() {
        let api = Arc::new(FakeApi::new(1));
        let svc = service(api.clone());

        let perms_a = TokenPermissions {
            contents: Some("write".to_string()),
            ..TokenPermissions::default()
        };
        let perms_b = TokenPermissions {
            contents: Some("read".to_string()),
            ..TokenPermissions::default()
        };

        let t1 = svc
            .token_for_repo("acme/site", Some(perms_a))
            .await
            .expect("a");
        let t2 = svc
            .token_for_repo("acme/site", Some(perms_b))
            .await
            .expect("b");
        assert_ne!(
            t1.expose_secret(),
            t2.expose_secret(),
            "different perms => different tokens"
        );
        assert_eq!(api.mint_count(), 2);
    }

    #[tokio::test]
    async fn installation_cached_then_invalidated_on_gone() {
        let api = Arc::new(FakeApi::new(1));
        let svc = service(api.clone());

        // First call caches the installation.
        svc.token_for_repo("acme/site", None).await.expect("first");
        let calls_after_first = api.installation_calls();

        // Second call reuses cached installation AND cached token.
        svc.token_for_repo("acme/site", None).await.expect("second");
        assert_eq!(
            api.installation_calls(),
            calls_after_first,
            "installation should be cached"
        );

        // Inject a nearly-expired token so the next call will try to mint.
        {
            let mut cache = svc.inner.token_cache.lock().unwrap();
            let key = TokenKey {
                owner_repo: "acme/site".to_string(),
                perms: default_permissions(),
            };
            if let Some(cached) = cache.get_mut(&key) {
                cached.expires_at = SystemTime::now() + Duration::from_secs(100);
            }
        }

        // Make the next mint return Gone, then succeed.
        api.set_next_mint_gone(true);
        svc.token_for_repo("acme/site", None)
            .await
            .expect("recovered from Gone");
        // After Gone, caches are invalidated => re-resolve => re-mint.
        assert!(
            api.installation_calls() > calls_after_first,
            "installation must be re-resolved after Gone"
        );
    }

    #[tokio::test]
    async fn not_installed_carries_install_url() {
        let api = Arc::new(FakeApi::new(1));
        api.set_next_install_not_found(true);
        let svc = service(api.clone());

        let err = svc
            .token_for_repo("acme/missing", None)
            .await
            .expect_err("must fail");
        match err {
            GithubAppError::NotInstalled { install_url, .. } => {
                assert!(
                    install_url.is_some(),
                    "must carry install URL when slug configured"
                );
                assert!(
                    install_url.unwrap().contains("fkst-test"),
                    "URL must contain slug"
                );
            }
            other => panic!("expected NotInstalled, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_repo_shape_rejected() {
        let api = Arc::new(FakeApi::new(1));
        let svc = service(api.clone());

        // Cases that don't match ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$
        for bad in &[
            "noslash",
            "a/../../../b",
            "a/b/c",
            "",
            "a/",
            "has space/repo",
        ] {
            let err = svc
                .token_for_repo(bad, None)
                .await
                .expect_err("must reject");
            assert!(
                matches!(err, GithubAppError::InvalidRepoRef),
                "{bad}: got {err:?}"
            );
        }
    }

    #[test]
    fn default_permissions_grants_session_admin_set() {
        // Issue #110: sessions hold admin-equivalent access for the whole
        // session. All four are `write`; `metadata` is implicit on installation
        // tokens, so it is deliberately left unset.
        let perms = default_permissions();
        assert_eq!(perms.contents.as_deref(), Some("write"));
        assert_eq!(perms.pull_requests.as_deref(), Some("write"));
        assert_eq!(perms.issues.as_deref(), Some("write"));
        assert_eq!(perms.administration.as_deref(), Some("write"));
        assert_eq!(perms.metadata, None);
    }

    #[test]
    fn session_permissions_is_least_privilege_without_administration() {
        // Issue #359 (Model B): the session pod's token grants write to
        // contents/issues/pull_requests but MUST NOT carry `administration`
        // (the one permission the admin-equivalent `default_permissions` adds).
        let perms = session_permissions();
        assert_eq!(perms.contents.as_deref(), Some("write"));
        assert_eq!(perms.issues.as_deref(), Some("write"));
        assert_eq!(perms.pull_requests.as_deref(), Some("write"));
        assert_eq!(
            perms.administration, None,
            "session token must not request administration"
        );
        assert_eq!(perms.metadata, None);
        // Concretely contrast the admin-equivalent default.
        assert_eq!(
            default_permissions().administration.as_deref(),
            Some("write")
        );
    }

    // ---- stateless resolution (#141) -----------------------------------------

    #[tokio::test]
    async fn resolve_uses_on_demand_probe_then_caches() {
        // First mint cold-probes the installation once; a second mint within the
        // TTL is a cache hit (no second probe). Replaces the removed durable
        // store: resolution is now probe-on-cold-cache, in-memory only.
        let api = Arc::new(FakeApi::new(1));
        let svc = service(api.clone());

        let _ = svc.token_for_repo("acme/site", None).await.expect("first");
        assert_eq!(
            api.installation_calls(),
            1,
            "the cold cache triggers exactly one on-demand installation probe"
        );

        let _ = svc.token_for_repo("acme/site", None).await.expect("second");
        assert_eq!(
            api.installation_calls(),
            1,
            "a second call within TTL is a cache hit (no extra probe)"
        );
    }

    #[tokio::test]
    async fn evict_repo_busts_installation_cache() {
        // Prime the cache, evict, then assert the next resolve re-probes (the
        // cache was busted) AND the cross-worker broadcast fired once per evict.
        let api = Arc::new(FakeApi::new(1));
        let broadcaster = Arc::new(RecordingBroadcaster::default());
        let config = test_config();
        let svc =
            GithubAppTokens::with_api_and_broadcaster(&config, api.clone(), broadcaster.clone())
                .expect("svc");

        let _ = svc.token_for_repo("acme/site", None).await.expect("prime");
        assert_eq!(api.installation_calls(), 1, "primed with one probe");

        svc.evict_repo("acme", "site").await;
        assert_eq!(
            broadcaster.broadcasts.lock().unwrap().clone(),
            vec!["acme/site".to_string()],
            "evict_repo broadcasts the eviction to other workers exactly once"
        );

        // The token cache was also busted, so the next call re-resolves the
        // installation (the probe count grows).
        let _ = svc
            .token_for_repo("acme/site", None)
            .await
            .expect("after evict");
        assert_eq!(
            api.installation_calls(),
            2,
            "installation cache was busted; resolve re-probes after eviction"
        );
    }

    #[test]
    fn installation_ttl_has_jitter_within_bounds() {
        // The jitter helper must stay within [0, JITTER], and the computed
        // expiry within [now+BASE, now+BASE+JITTER]. Sample many times to catch
        // an off-by-one at either inclusive bound.
        let jitter_max = INSTALLATION_TTL_JITTER;
        for _ in 0..1000 {
            let jitter = rand_jitter();
            assert!(
                jitter <= jitter_max,
                "jitter {jitter:?} exceeds max {jitter_max:?}"
            );
            // (Duration is unsigned, so the lower bound `>= 0` is structural.)

            let before = SystemTime::now();
            let expires_at = before + INSTALLATION_TTL_BASE + rand_jitter();
            let lower = before + INSTALLATION_TTL_BASE;
            let upper = before + INSTALLATION_TTL_BASE + INSTALLATION_TTL_JITTER;
            assert!(
                expires_at >= lower,
                "expiry {expires_at:?} earlier than now+BASE {lower:?}"
            );
            assert!(
                expires_at <= upper,
                "expiry {expires_at:?} later than now+BASE+JITTER {upper:?}"
            );
        }
    }

    #[tokio::test]
    async fn probe_installation_distinguishes_states() {
        // Installed: a token mints.
        let api = Arc::new(FakeApi::new(1));
        let svc = service(api.clone());
        assert_eq!(
            svc.probe_installation("acme/site").await.expect("probe"),
            InstallationProbe::Installed
        );

        // Not installed: the installation lookup 404s.
        let api = Arc::new(FakeApi::new(1));
        api.set_next_install_not_found(true);
        let svc = service(api.clone());
        match svc.probe_installation("acme/missing").await.expect("probe") {
            InstallationProbe::NotInstalled { install_url } => {
                assert!(install_url.is_some(), "carries install url when slug set");
            }
            other => panic!("expected NotInstalled, got {other:?}"),
        }
    }
}
