//! GitHub issues hub: aggregate a user's GitHub issues across ALL their linked
//! GitHub accounts and run single-target issue operations, reaching GitHub ONLY
//! through NyxID's credential-injection proxy with an RFC 8693 delegated token.
//!
//! The [`GithubProxy`] trait is the single seam: every byte of GitHub access
//! goes through it, so there is zero direct GitHub HTTP in the hub. The
//! production [`NyxIdGithubProxy`] routes through [`crate::nyxid`]; tests swap
//! in a wiremock-backed NyxID and exercise the real trait impl.
//!
//! Security: issue bodies and tokens are NEVER logged — only counts and sizes.

pub mod fanout;
pub mod service;
pub mod types;

use axum::body::Bytes;
use reqwest::header::HeaderMap;
use reqwest::Method;

use crate::auth::AuthContext;
use crate::authz::Authorizer;
use crate::error::AppError;
use crate::nyxid::{DelegatedToken, GithubConnection, NyxIdClient, NyxIdError};

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
    #[error("credential delegation failed")]
    Delegation,
    /// The linked-connections listing failed at NyxID.
    #[error("connection listing failed")]
    ConnectionListing,
    /// A transport-level failure reaching NyxID (credential-free text).
    #[error("proxy transport error: {0}")]
    Transport(String),
}

impl From<ProxyError> for AppError {
    /// Delegation/listing failures are a 503 (the credential proxy is the
    /// missing dependency); a transport failure is likewise a 503. None of
    /// these carry a token, so the message is safe.
    fn from(err: ProxyError) -> Self {
        match err {
            ProxyError::Delegation => {
                AppError::Unavailable("credential proxy delegation failed".to_string())
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
/// per-request delegated token; built once per inbound request via
/// [`NyxIdGithubProxy::from_context`].
pub struct NyxIdGithubProxy {
    client: NyxIdClient,
    delegated: DelegatedToken,
}

impl NyxIdGithubProxy {
    /// Build the proxy for one inbound request: resolve the NyxID client from
    /// the authorizer, then exchange the caller's token for a delegated token.
    ///
    /// Mirrors `routes/goals.rs::create_new_repo`: a missing NyxID client is a
    /// 503, and a rejected/failed exchange maps via [`crate::goals::CreateRepoError`]
    /// onto the appropriate `AppError` (401/503) without leaking the token.
    pub async fn from_context(
        authz: &Authorizer,
        ctx: &AuthContext,
    ) -> Result<NyxIdGithubProxy, AppError> {
        let nyxid = authz
            .nyxid()
            .ok_or_else(|| AppError::Unavailable("credential proxy not configured".to_string()))?;
        let delegated = nyxid
            .exchange_token(&ctx.raw_token)
            .await
            .map_err(crate::goals::CreateRepoError::from)?;
        Ok(NyxIdGithubProxy {
            client: nyxid.clone(),
            delegated,
        })
    }

    /// Map a NyxID error onto a credential-free [`ProxyError`].
    fn map_nyxid(err: NyxIdError, listing: bool) -> ProxyError {
        match err {
            NyxIdError::ExchangeRejected(_) => ProxyError::Delegation,
            NyxIdError::Http(detail) | NyxIdError::Malformed(detail) => {
                if listing {
                    ProxyError::ConnectionListing
                } else {
                    ProxyError::Transport(detail)
                }
            }
            NyxIdError::ServiceAuth => ProxyError::Delegation,
            // The github-proxy path never mints an agent key, so this variant
            // cannot arise here; map it to a credential-rejection for safety.
            NyxIdError::UserTokenRejected => ProxyError::Delegation,
        }
    }
}

#[async_trait::async_trait]
impl GithubProxy for NyxIdGithubProxy {
    async fn accounts(&self) -> Result<Vec<GithubConnection>, ProxyError> {
        self.client
            .github_connections(&self.delegated)
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
        let response = self
            .client
            .proxy_github_for(&self.delegated, &connection, method, path_and_query, body)
            .await
            .map_err(|e| Self::map_nyxid(e, false))?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|e| ProxyError::Transport(format!("read body: {e}")))?;
        Ok(ProxyResponse {
            status,
            headers,
            body,
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
    fn map_nyxid_exchange_rejected_is_delegation() {
        let err =
            NyxIdGithubProxy::map_nyxid(NyxIdError::ExchangeRejected("denied".to_string()), false);
        assert!(matches!(err, ProxyError::Delegation));
    }

    #[test]
    fn map_nyxid_http_is_listing_or_transport() {
        let listing = NyxIdGithubProxy::map_nyxid(NyxIdError::Http("boom".to_string()), true);
        assert!(matches!(listing, ProxyError::ConnectionListing));
        let transport = NyxIdGithubProxy::map_nyxid(NyxIdError::Http("boom".to_string()), false);
        assert!(matches!(transport, ProxyError::Transport(_)));
    }
}
