//! GitHub issues hub: aggregate a user's GitHub issues across ALL their linked
//! GitHub accounts and run single-target issue operations, reaching GitHub ONLY
//! through NyxID's credential-injection proxy presenting the caller's OWN
//! forwarded user token (no RFC 8693 token exchange).
//!
//! The [`GithubProxy`] trait is the single seam: every byte of GitHub access
//! goes through it, so there is zero direct GitHub HTTP in the hub. The
//! production [`NyxIdGithubProxy`] routes through [`crate::nyxid`]; tests swap
//! in a wiremock-backed NyxID and exercise the real trait impl.
//!
//! Security: issue bodies and tokens are NEVER logged — only counts and sizes.

pub mod contents;
pub mod fanout;
pub mod service;
pub mod types;

// The Git Data API WRITER (#181): the first contents-write surface in the repo.
// It uses the GitHub App installation token (NOT the read-only `GithubProxy`).
pub use contents::{commit_files, CommitResult, ContentsWriteError, ScaffoldFile};

use axum::body::Bytes;
use reqwest::header::HeaderMap;
use reqwest::Method;

use crate::auth::AuthContext;
use crate::authz::Authorizer;
use crate::error::AppError;
use secrecy::SecretString;

use crate::nyxid::{GithubConnection, NyxIdClient, NyxIdError};

/// The single seam for all GitHub access. Implementations reach GitHub only via
/// a credential-injection proxy; the hub never holds a GitHub token.
#[async_trait::async_trait]
pub trait GithubProxy: Send + Sync {
    /// List the caller's linked GitHub accounts (one per linked credential).
    async fn accounts(&self) -> Result<Vec<GithubConnection>, ProxyError>;

    /// Proxy a single GitHub REST request pinned to one linked account.
    /// `account_selector` is the connection's routing handle (its
    /// `connection_id`, resolved by the service from [`accounts`]); the service
    /// keeps the human login separately for response attribution.
    /// `path_and_query` is a GitHub API path (it may carry a query string).
    ///
    /// [`accounts`]: GithubProxy::accounts
    async fn request(
        &self,
        account_selector: &str,
        method: Method,
        path_and_query: &str,
        body: Option<serde_json::Value>,
    ) -> Result<ProxyResponse, ProxyError>;
}

/// A proxied GitHub response, decoupled from `reqwest` so the service layer
/// stays transport-agnostic and easily mockable.
pub struct ProxyResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Bytes,
}

/// Failures of the proxy seam itself (NOT a GitHub HTTP status — those are
/// carried as the `status` on a successful [`ProxyResponse`]). Credential-free.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// The delegated token could not be obtained / was rejected by NyxID.
    /// Unreachable on the forwarded-user-token path (kept for the trait's
    /// shared error surface); a rejected user token is [`Self::UserToken`].
    #[error("credential delegation failed")]
    Delegation,
    /// The caller's forwarded user token was rejected by NyxID (expired,
    /// revoked, or otherwise refused). A client-side credential problem, so it
    /// surfaces as a 401 — not the 503 reserved for a missing/broken proxy.
    /// Credential-free: the token never appears here.
    #[error("user token rejected")]
    UserToken,
    /// The linked-connections listing failed at NyxID.
    #[error("connection listing failed")]
    ConnectionListing,
    /// A transport-level failure reaching NyxID (credential-free text).
    #[error("proxy transport error: {0}")]
    Transport(String),
}

impl From<ProxyError> for AppError {
    /// Delegation/listing failures are a 503 (the credential proxy is the
    /// missing dependency); a transport failure is likewise a 503. A rejected
    /// user token is a 401 — the caller's credential is the problem, not the
    /// proxy's availability. None of these carry a token, so the message is safe.
    fn from(err: ProxyError) -> Self {
        match err {
            ProxyError::Delegation => {
                AppError::Unavailable("credential proxy delegation failed".to_string())
            }
            ProxyError::UserToken => {
                AppError::Unauthorized("github credential rejected".to_string())
            }
            ProxyError::ConnectionListing => {
                AppError::Unavailable("could not list linked GitHub accounts".to_string())
            }
            ProxyError::Transport(detail) => {
                tracing::debug!(detail = %detail, "github proxy transport error");
                AppError::Unavailable("credential proxy unreachable".to_string())
            }
        }
    }
}

/// Production [`GithubProxy`] backed by NyxID. Holds the NyxID client and the
/// caller's forwarded user token; built once per inbound request via
/// [`NyxIdGithubProxy::from_context`].
pub struct NyxIdGithubProxy {
    client: NyxIdClient,
    user_token: SecretString,
}

impl NyxIdGithubProxy {
    /// Build the proxy for one inbound request: resolve the NyxID client from
    /// the authorizer and capture the caller's forwarded user token, which is
    /// presented directly to NyxID's credential-injection proxy (no RFC 8693
    /// exchange — the agreed owner-only model acts AS the caller).
    ///
    /// A missing NyxID client is a 503; "headers mode" (no bearer forwarded)
    /// is a 401 via [`AuthContext::require_user_token`] — there is no degraded
    /// path meaningful for GitHub access.
    pub async fn from_context(
        authz: &Authorizer,
        ctx: &AuthContext,
    ) -> Result<NyxIdGithubProxy, AppError> {
        let nyxid = authz
            .nyxid()
            .ok_or_else(|| AppError::Unavailable("credential proxy not configured".to_string()))?;
        let user_token = ctx.require_user_token()?.clone();
        Ok(NyxIdGithubProxy {
            client: nyxid.clone(),
            user_token,
        })
    }

    /// Map a NyxID error onto a credential-free [`ProxyError`].
    ///
    /// On the forwarded-user-token path a rejected token surfaces as
    /// [`NyxIdError::UserTokenRejected`] (a 401-class [`ProxyError::UserToken`]),
    /// raised both by the api-key mint and by the connections listing, since both
    /// mean the caller's credential was refused — not a proxy outage.
    fn map_nyxid(err: NyxIdError, listing: bool) -> ProxyError {
        match err {
            NyxIdError::UserTokenRejected => ProxyError::UserToken,
            NyxIdError::Http(detail) | NyxIdError::Malformed(detail) => {
                if listing {
                    ProxyError::ConnectionListing
                } else {
                    ProxyError::Transport(detail)
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl GithubProxy for NyxIdGithubProxy {
    async fn accounts(&self) -> Result<Vec<GithubConnection>, ProxyError> {
        self.client
            .github_connections_user(&self.user_token)
            .await
            .map_err(|e| Self::map_nyxid(e, true))
    }

    async fn request(
        &self,
        account_selector: &str,
        method: Method,
        path_and_query: &str,
        body: Option<serde_json::Value>,
    ) -> Result<ProxyResponse, ProxyError> {
        // `account_selector` is the connection's routing handle resolved by the
        // service; per-account routing (the `_nyxid_via` selector) lives in the
        // nyxid seam, so we hand it a `GithubConnection` carrying that handle.
        let connection = GithubConnection {
            connection_id: account_selector.to_string(),
            login: account_selector.to_string(),
            primary: false,
        };
        // The forwarded-user-token helper takes the body as raw bytes; serialize
        // the optional JSON here so the seam stays JSON-shaped for callers.
        let body_bytes = match body {
            Some(value) => Some(
                serde_json::to_vec(&value)
                    .map_err(|e| ProxyError::Transport(format!("encode body: {e}")))?,
            ),
            None => None,
        };
        let response = self
            .client
            .proxy_github_user_for(
                &self.user_token,
                &connection,
                method,
                path_and_query,
                body_bytes,
            )
            .await
            .map_err(|e| Self::map_nyxid(e, false))?;
        // `proxy_github_user_for` returns a buffered `nyxid::ProxyResponse`
        // (status: StatusCode, headers, body: Vec<u8>); adapt it to the hub's
        // transport-agnostic `ProxyResponse`.
        Ok(ProxyResponse {
            status: response.status.as_u16(),
            headers: response.headers,
            body: Bytes::from(response.body),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_error_delegation_maps_to_503() {
        let err: AppError = ProxyError::Delegation.into();
        assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
    }

    #[test]
    fn proxy_error_listing_maps_to_503() {
        let err: AppError = ProxyError::ConnectionListing.into();
        assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
    }

    #[test]
    fn proxy_error_user_token_maps_to_401() {
        let err: AppError = ProxyError::UserToken.into();
        assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
    }

    #[test]
    fn map_nyxid_user_token_rejected_is_user_token() {
        // A forwarded user token refused by NyxID is a 401-class credential
        // rejection, NOT the 503 "delegation failed". The same refusal arises on
        // both the request and the connections-listing paths.
        let rejected = NyxIdGithubProxy::map_nyxid(NyxIdError::UserTokenRejected, false);
        assert!(matches!(rejected, ProxyError::UserToken));
        let listing = NyxIdGithubProxy::map_nyxid(NyxIdError::UserTokenRejected, true);
        assert!(matches!(listing, ProxyError::UserToken));
    }

    #[test]
    fn map_nyxid_http_is_listing_or_transport() {
        let listing = NyxIdGithubProxy::map_nyxid(NyxIdError::Http("boom".to_string()), true);
        assert!(matches!(listing, ProxyError::ConnectionListing));
        let transport = NyxIdGithubProxy::map_nyxid(NyxIdError::Http("boom".to_string()), false);
        assert!(matches!(transport, ProxyError::Transport(_)));
    }
}
