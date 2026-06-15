//! GitHub App installation-token service: `token_for_repo` with caching,
//! expiry buffering, and typed "not installed" errors carrying an install URL.
//!
//! The module mints short-lived, repo-scoped GitHub installation tokens on
//! demand. Tokens are cached per `(repo, permissions)` pair and re-minted
//! 5 minutes before expiry. Installation IDs are cached for 1 hour.
//!
//! Cache lock discipline: locks are held for map access only; minting happens
//! outside the lock (rare duplicate mints accepted over lock contention).
//!
//! `InstallationGone` invalidates BOTH caches and makes one transparent
//! re-resolve attempt before surfacing.

pub mod api;
pub mod config;
pub mod jwt;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use regex::Regex;
use secrecy::SecretString;

use api::{GithubApi, HttpGithubApi, InstallationTokenRequest};
use jwt::{build_encoding_key, mint_app_jwt};

/// Re-export config for downstream use (main.rs, error.rs).
pub use config::GithubAppConfig;

/// Re-export API types for downstream consumers.
pub use api::{InstallationId, InstallationToken, TokenPermissions};

/// Buffer before token expiry at which we re-mint (5 minutes).
const EXPIRY_BUFFER: Duration = Duration::from_secs(300);

/// How long to cache installation IDs (1 hour).
const INSTALLATION_TTL: Duration = Duration::from_secs(3600);

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
    resolved_at: SystemTime,
}

// ---------------------------------------------------------------------------
// Inner (shared state)
// ---------------------------------------------------------------------------

struct Inner {
    app_id: u64,
    encoding_key: jsonwebtoken::EncodingKey,
    app_slug: Option<String>,
    api: std::sync::Arc<dyn GithubApi>,
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

    /// Construct the service with an injected transport (for tests).
    pub fn with_api(
        config: &GithubAppConfig,
        api: std::sync::Arc<dyn GithubApi>,
    ) -> Result<Self, GithubAppError> {
        let encoding_key =
            build_encoding_key(&config.private_key_pem).map_err(|_| GithubAppError::InvalidKey)?;
        Ok(Self {
            inner: std::sync::Arc::new(Inner {
                app_id: config.app_id,
                encoding_key,
                app_slug: config.app_slug.clone(),
                api,
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
        if !REPO_REF_RE.is_match(owner_repo) {
            return Err(GithubAppError::InvalidRepoRef);
        }

        let perms = perms.unwrap_or_else(default_permissions);
        let key = TokenKey {
            owner_repo: owner_repo.to_string(),
            perms: perms.clone(),
        };

        // 1. Check token cache (lock held for map access only).
        {
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
        let (bare_repo_name, _) = owner_repo.split_once('/').expect("validated by regex");
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

    /// The install URL for this app (if slug is configured).
    pub fn install_url(&self) -> Option<String> {
        self.inner
            .app_slug
            .as_ref()
            .map(|slug| format!("https://github.com/apps/{slug}/installations/new"))
    }

    /// Resolve the installation ID for a repo, using the cache if available.
    async fn resolve_installation(
        &self,
        owner_repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        // Check installation cache.
        {
            let cache = self
                .inner
                .installation_cache
                .lock()
                .expect("installation cache lock");
            if let Some(cached) = cache.get(owner_repo) {
                if cached.resolved_at + INSTALLATION_TTL > SystemTime::now() {
                    return Ok(cached.id);
                }
            }
        }

        // Resolve via API.
        let (owner, repo) = owner_repo.split_once('/').expect("validated by regex");
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

        // Cache the installation ID.
        {
            let mut cache = self
                .inner
                .installation_cache
                .lock()
                .expect("installation cache lock");
            cache.insert(
                owner_repo.to_string(),
                CachedInstallation {
                    id: install_id,
                    resolved_at: SystemTime::now(),
                },
            );
        }

        Ok(install_id)
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
            _req: &InstallationTokenRequest,
        ) -> Result<InstallationToken, GithubAppError> {
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
}
