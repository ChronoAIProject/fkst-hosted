//! NyxID JWT authentication: core types, JWKS cache, token verification,
//! and Axum middleware.

pub mod jwks;
pub mod middleware;
pub mod verify;

use secrecy::SecretString;

/// Whether authentication is enabled and, if so, with which NyxID settings.
#[derive(Debug, Clone)]
pub enum AuthMode {
    /// All routes are open; the extractor yields a dev context. Intentional
    /// local-dev choice (env `FKST_AUTH_ENABLED=false`).
    Disabled,
    /// JWT verification is active; every `/api/v1/*` route (except health)
    /// requires a valid RS256 access token from the configured NyxID issuer.
    Enabled(NyxIdAuthSettings),
}

/// NyxID-specific authentication settings, all sourced from `FKST_AUTH_*`
/// environment variables.
#[derive(Debug, Clone)]
pub struct NyxIdAuthSettings {
    /// Base URL of the NyxID issuer (e.g. `https://nyxid.example.com`).
    /// Trailing `/` is trimmed at construction. Env: `FKST_AUTH_NYXID_BASE_URL`.
    pub base_url: String,
    /// Expected `iss` claim. Env: `FKST_AUTH_ISSUER`, default `"nyxid"`.
    pub issuer: String,
    /// Expected `aud` claim. Env: `FKST_AUTH_AUDIENCE`, default = `base_url`.
    pub audience: String,
    /// How long to cache the JWKS before re-fetching.
    /// Env: `FKST_AUTH_JWKS_CACHE_TTL_SECS`, default 300, 0 rejected.
    pub jwks_cache_ttl: std::time::Duration,
}

/// Authentication context extracted from a verified JWT. Inserted into request
/// extensions by the auth middleware and consumed via `FromRequestParts`.
#[derive(Clone)]
pub struct AuthContext {
    /// Subject (`sub` claim) — the user or service account identifier.
    pub user_id: String,
    /// OAuth2 scopes from the `scope` claim, split on whitespace.
    pub scopes: Vec<String>,
    /// Role assignments from the `roles` claim.
    pub roles: Vec<String>,
    /// Group memberships from the `groups` claim.
    pub groups: Vec<String>,
    /// Fine-grained permissions from the `permissions` claim.
    pub permissions: Vec<String>,
    /// Session ID from the `sid` claim (optional).
    pub session_id: Option<String>,
    /// True when `sa` claim is `Some(true)` (service account).
    pub is_service_account: bool,
    /// Raw `act` claim for delegation (RFC 8693 token exchange). Opaque;
    /// forwarded as-is for downstream policy enforcement.
    pub delegation: Option<serde_json::Value>,
    /// Raw bearer token retained for potential RFC 8693 exchange. SECRET.
    pub raw_token: SecretString,
}

impl AuthContext {
    /// Check whether the token carries a specific scope.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }

    /// Development fallback: a synthetic context with wildcard scope, used
    /// when authentication is disabled.
    pub fn dev() -> Self {
        Self {
            user_id: "dev-local".into(),
            scopes: vec!["*".into()],
            roles: vec![],
            groups: vec![],
            permissions: vec![],
            session_id: None,
            is_service_account: false,
            delegation: None,
            raw_token: SecretString::new("".into()),
        }
    }
}

impl std::fmt::Debug for AuthContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthContext")
            .field("user_id", &self.user_id)
            .field("scopes", &self.scopes)
            .field("roles", &self.roles)
            .field("groups", &self.groups)
            .field("permissions", &self.permissions)
            .field("session_id", &self.session_id)
            .field("is_service_account", &self.is_service_account)
            .field("delegation", &self.delegation)
            .field("raw_token", &"<redacted>")
            .finish()
    }
}

/// Authentication-domain errors. Mapped onto `AppError` by the `From` impl.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing bearer token")]
    MissingToken,
    #[error("malformed authorization header")]
    MalformedHeader,
    #[error("invalid token: {0}")]
    InvalidToken(&'static str),
    #[error("jwks fetch failed: {0}")]
    JwksUnavailable(String),
}
