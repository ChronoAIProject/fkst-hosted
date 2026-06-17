//! Proxy-trusted authentication: identity types, the identity decoder, and the
//! Axum middleware/extractor.
//!
//! fkst-hosted is a NyxID *downstream* service. It does NOT authenticate users
//! itself — the NyxID proxy verifies the caller and injects the identity into
//! the forwarded request. This module trusts that injected identity (decode-
//! only, no signature verification, no JWKS) and exposes it as `AuthContext`.
//! See [`identity`] for the trust rationale.

pub mod identity;
pub mod middleware;

use secrecy::SecretString;

/// Whether authentication is enforced and, if so, with which NyxID settings.
#[derive(Debug, Clone)]
pub enum AuthMode {
    /// All routes are open; the extractor yields a dev context. Intentional
    /// local-dev choice (env `FKST_AUTH_ENABLED=false`).
    Disabled,
    /// Proxy-trusted identity is enforced: every `/api/v1/*` route (except
    /// health and the signature-verified webhook) requires a NyxID-injected
    /// identity (`X-NyxID-Identity-Token` or the `X-NyxID-User-*` fallback).
    Enabled(NyxIdAuthSettings),
}

/// NyxID-specific settings, all sourced from `FKST_AUTH_*` environment
/// variables.
///
/// No JWKS settings live here anymore: fkst-hosted trusts the proxy and never
/// fetches keys or verifies user-token signatures (issue #113). `base_url` is
/// retained because it is the NyxID issuer host used by downstream concerns —
/// the per-session NyxID token provisioning (#111) and the `nyxid` org-lookup
/// client both target it.
#[derive(Debug, Clone)]
pub struct NyxIdAuthSettings {
    /// Base URL of the NyxID issuer (e.g. `https://nyxid.example.com`).
    /// Trailing `/` is trimmed at construction. Env: `FKST_AUTH_NYXID_BASE_URL`.
    pub base_url: String,
}

/// Authentication context built from the proxy-injected identity. Inserted into
/// request extensions by the auth middleware and consumed via `FromRequestParts`.
#[derive(Clone)]
pub struct AuthContext {
    /// Subject (`sub`) — the authenticated user identifier.
    pub user_id: String,
    /// Caller email (from the identity token or the `X-NyxID-User-Email`
    /// fallback header). Empty string when unknown.
    pub email: String,
    /// Display name: the token `name`, else the email, else the `sub`.
    pub display_name: String,
    /// Role assignments forwarded by NyxID. Observability only — RBAC is driven
    /// by `permissions`, never by a local role→permission mapping.
    pub roles: Vec<String>,
    /// Fine-grained `fkst:*` permissions assigned by NyxID. The action layer
    /// (`authz::permissions::require_permission`) enforces these.
    pub permissions: Vec<String>,
    /// Group memberships forwarded by NyxID. Observability only.
    pub groups: Vec<String>,
    /// The caller's raw `Authorization: Bearer` access token, when the proxy
    /// forwarded it. Used ONLY to call NyxID back for org lookups and to forward
    /// to downstreams (Ornn, the GitHub proxy) — NEVER for authentication.
    /// `None` in "headers mode" (no bearer forwarded) and in dev. SECRET.
    pub user_access_token: Option<SecretString>,
}

impl AuthContext {
    /// True when the caller carries the platform-admin permission. The admin
    /// permission is the operator escape hatch; NyxID assigns it.
    pub fn has_permission(&self, perm: &str) -> bool {
        self.permissions.iter().any(|p| p == perm)
    }

    /// Borrow the forwarded user access token, or a clear `Unauthorized` error
    /// when absent. Used by call sites that MUST act as the user against NyxID
    /// (credential exchange, Ornn catalog) and cannot degrade gracefully.
    pub fn require_user_token(&self) -> Result<&SecretString, crate::error::AppError> {
        self.user_access_token.as_ref().ok_or_else(|| {
            crate::error::AppError::Unauthorized(
                "this action requires a forwarded user access token".to_string(),
            )
        })
    }

    /// Development fallback used when authentication is disabled. Carries the
    /// platform-admin permission so local dev is never permission-blocked.
    pub fn dev() -> Self {
        Self {
            user_id: "dev-local".into(),
            email: String::new(),
            display_name: "dev-local".into(),
            roles: vec![],
            permissions: vec![crate::authz::permissions::ADMIN.to_string()],
            groups: vec![],
            user_access_token: None,
        }
    }
}

impl std::fmt::Debug for AuthContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthContext")
            .field("user_id", &self.user_id)
            .field("email", &self.email)
            .field("display_name", &self.display_name)
            .field("roles", &self.roles)
            .field("permissions", &self.permissions)
            .field("groups", &self.groups)
            .field("user_access_token", &"<redacted>")
            .finish()
    }
}

/// Authentication-domain errors. Mapped onto `AppError` by the `From` impl.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Neither an identity token nor a `X-NyxID-User-Id` header was present.
    /// Only reachable if the proxy is misconfigured (it always injects
    /// identity), or fkst-hosted is exposed without the proxy in front.
    #[error("missing proxy-injected identity")]
    MissingIdentity,
}
