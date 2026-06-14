//! NyxID service client: service-token cache, org-role lookups, RFC 8693
//! token exchange, and GitHub credential-injection proxy helper.
//!
//! Design rules (mirroring `journal/github.rs`):
//! - All secrets live in `secrecy::SecretString`: exposed only at
//!   request-build time, never captured into `Debug`/`Display` of the
//!   client, any error variant, or any log line.
//! - The `api_base` is injectable for wiremock testing.
//! - HTTP timeout is 15 s; token expiry buffer is 60 s.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};

// ---- Constants ----

/// OAuth token endpoint path (service credentials + token exchange).
pub const TOKEN_PATH: &str = "/oauth/token";

/// Org list endpoint (caller's own bearer token).
pub const ORGS_PATH: &str = "/api/v1/orgs";

/// Users endpoint (service-account lookups).
pub const USERS_PATH: &str = "/api/v1/users";

/// GitHub credential-injection proxy path.
///
/// **UNVERIFIED — confirm against NyxID main.** Kept in one constant so
/// the exact route shape is isolated from the rest of the codebase.
pub const GITHUB_PROXY_PATH: &str = "/api/v1/proxy/github";

/// Endpoint that lists the user's linked GitHub connections (one per linked
/// GitHub account) under the caller's delegated token.
///
/// **UNVERIFIED — confirm against NyxID main; confined here.** NyxID `main`
/// today exposes `GET /api/v1/keys` (per-`UserService` instances, addressed by
/// id/slug/label) and `GET /api/v1/connections` (one row per service) — neither
/// yet projects a per-github-login `{connection_id, login, primary}` shape; the
/// multi-connection github listing is still a NyxID draft. This constant + the
/// [`GithubConnection`] deserialize mapping are the ONLY places that change when
/// that listing ships; the wiremock tests pin the contract regardless.
pub const GITHUB_CONNECTIONS_PATH: &str = "/api/v1/connections?provider=github";

/// NyxID-internal query selector that pins a proxied request to one specific
/// linked credential instance (verified `_nyxid_via` mechanism on NyxID main:
/// it routes by user-service / connection id and is stripped before the request
/// is forwarded to GitHub). Confined here so per-account routing lives in one
/// place.
const NYXID_VIA_PARAM: &str = "_nyxid_via";

/// Per-request HTTP timeout.
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Service-token refresh buffer: start refreshing this many seconds
/// before the actual expiry so a race does not serve an expired token.
const TOKEN_EXPIRY_BUFFER: Duration = Duration::from_secs(60);

// ---- Error type ----

/// NyxID integration errors. No variant carries secrets or credentials.
#[derive(Debug, thiserror::Error)]
pub enum NyxIdError {
    /// Service-account credentials were rejected by NyxID.
    #[error("nyxid service credentials rejected")]
    ServiceAuth,
    /// HTTP transport error (credential-free text).
    #[error("nyxid http error: {0}")]
    Http(String),
    /// NyxID returned an unexpected or malformed response.
    #[error("nyxid response malformed: {0}")]
    Malformed(String),
    /// RFC 8693 token exchange was rejected by NyxID.
    #[error("token exchange rejected: {0}")]
    ExchangeRejected(String),
}

// ---- DTOs ----

/// Organization role, matching NyxID's lowercase serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrgRole {
    Admin,
    Member,
    Viewer,
}

/// One membership entry from NyxID's `GET /api/v1/orgs/{id}/members`.
#[derive(Debug, Clone, Deserialize)]
pub struct OrgMember {
    pub membership_id: String,
    pub user_id: String,
    pub role: OrgRole,
    #[serde(default)]
    pub scope_source: Option<String>,
    #[serde(default)]
    pub revoked_at: Option<String>,
}

/// Org summary returned by `GET /api/v1/orgs`. Tolerant: unknown fields
/// are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct OrgSummary {
    pub id: String,
}

/// One linked GitHub account, as projected from NyxID's connections listing.
///
/// Tolerant by design: unknown NyxID fields are ignored and `primary` defaults
/// to `false` when NyxID omits it, so the type survives NyxID field drift while
/// the wiremock tests pin the contract. See [`GITHUB_CONNECTIONS_PATH`] for the
/// confined, UNVERIFIED route assumption.
#[derive(Debug, Clone, Deserialize)]
pub struct GithubConnection {
    /// Opaque connection identifier; fed verbatim to NyxID's `_nyxid_via`
    /// selector to target this account on a proxied request.
    pub connection_id: String,
    /// The GitHub login (username) the connection is authorized as.
    pub login: String,
    /// Whether this is the user's primary GitHub connection.
    #[serde(default)]
    pub primary: bool,
}

/// Delegated token obtained via RFC 8693 token exchange.
pub struct DelegatedToken {
    pub access_token: SecretString,
    pub expires_in: u64,
}

impl fmt::Debug for DelegatedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DelegatedToken")
            .field("access_token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

// ---- Cached entries ----

/// Cached service token + its absolute expiry instant.
struct CachedToken {
    token: SecretString,
    expires_at: Instant,
}

/// Cached org-role for a (org_id, user_id) pair.
struct CachedRole {
    role: Option<OrgRole>,
    expires_at: Instant,
}

/// Cached user-orgs list for a user_id.
struct CachedOrgs {
    orgs: Vec<OrgSummary>,
    expires_at: Instant,
}

// ---- Client ----

/// Inner state behind `Arc`.
struct Inner {
    base_url: String,
    client_id: String,
    client_secret: SecretString,
    http: reqwest::Client,
    /// Service-token cache.
    token_cache: RwLock<Option<CachedToken>>,
    /// Single-flight lock for service-token refresh.
    token_flight: Mutex<()>,
    /// Org-role cache keyed by (org_id, user_id).
    role_cache: RwLock<HashMap<(String, String), CachedRole>>,
    /// User-orgs cache keyed by user_id.
    orgs_cache: RwLock<HashMap<String, CachedOrgs>>,
    /// TTL for org-role and user-orgs caches.
    cache_ttl: Duration,
}

/// NyxID service client: service-token management, org-role lookups,
/// token exchange, and GitHub proxy helper. Cheaply cloneable (`Arc`-backed).
#[derive(Clone)]
pub struct NyxIdClient {
    inner: Arc<Inner>,
}

impl fmt::Debug for NyxIdClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NyxIdClient")
            .field("base_url", &self.inner.base_url)
            .field("client_id", &self.inner.client_id)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

/// Reduce a reqwest error to a credential-free string.
fn http_err(context: &str, err: reqwest::Error) -> NyxIdError {
    NyxIdError::Http(format!("{context}: {err}"))
}

/// Percent-encode a value for use as a URL query-component (RFC 3986): keep the
/// unreserved set `A-Za-z0-9-._~`, escape everything else. A tiny confined
/// helper so no extra dependency is needed to encode the `_nyxid_via` value.
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Append the `_nyxid_via=<connection_id>` selector to a GitHub proxy path,
/// choosing `&` vs `?` based on whether the path already has a query string.
/// The connection id is URL-encoded so arbitrary ids stay well-formed.
fn with_via_selector(github_path: &str, connection_id: &str) -> String {
    let separator = if github_path.contains('?') { '&' } else { '?' };
    format!(
        "{github_path}{separator}{NYXID_VIA_PARAM}={}",
        encode_query_value(connection_id)
    )
}

impl NyxIdClient {
    /// Build a new NyxIdClient. `base_url` is the NyxID issuer base (injectable
    /// for wiremock testing). `cache_ttl` controls how long org-role and
    /// user-orgs results are cached.
    pub fn new(
        base_url: &str,
        client_id: String,
        client_secret: SecretString,
        cache_ttl: Duration,
    ) -> Result<Self, NyxIdError> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("fkst-hosted-api")
            .build()
            .map_err(|e| http_err("client build", e))?;
        Ok(Self {
            inner: Arc::new(Inner {
                base_url: base_url.trim_end_matches('/').to_string(),
                client_id,
                client_secret,
                http,
                token_cache: RwLock::new(None),
                token_flight: Mutex::new(()),
                role_cache: RwLock::new(HashMap::new()),
                orgs_cache: RwLock::new(HashMap::new()),
                cache_ttl,
            }),
        })
    }

    /// Obtain a valid service token (client-credentials grant). Cached;
    /// refreshed 60 s before expiry. Single-flight: concurrent callers
    /// share one refresh.
    pub async fn service_token(&self) -> Result<SecretString, NyxIdError> {
        // Fast path: check the cache.
        {
            let cache = self.inner.token_cache.read().await;
            if let Some(cached) = cache.as_ref() {
                if cached.expires_at > Instant::now() + TOKEN_EXPIRY_BUFFER {
                    return Ok(cached.token.clone());
                }
            }
        }
        // Slow path: single-flight refresh.
        let _guard = self.inner.token_flight.lock().await;
        // Double-check after acquiring the lock.
        {
            let cache = self.inner.token_cache.read().await;
            if let Some(cached) = cache.as_ref() {
                if cached.expires_at > Instant::now() + TOKEN_EXPIRY_BUFFER {
                    return Ok(cached.token.clone());
                }
            }
        }
        let url = format!("{}{}", self.inner.base_url, TOKEN_PATH);
        let response = self
            .inner
            .http
            .post(&url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.inner.client_id),
                ("client_secret", self.inner.client_secret.expose_secret()),
            ])
            .send()
            .await
            .map_err(|e| http_err("service token", e))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            tracing::error!(status = %status, "nyxid service credentials rejected");
            return Err(NyxIdError::ServiceAuth);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!(status = %status, "nyxid service token request failed");
            return Err(NyxIdError::Http(format!(
                "service token status {status}: {}",
                // Body text may carry NyxID error detail, but never our
                // credentials — those were in the form body, not the response.
                &body[..body.len().min(200)]
            )));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u64,
        }
        let body: TokenResponse = response
            .json()
            .await
            .map_err(|e| NyxIdError::Malformed(format!("service token body: {e}")))?;

        let expires_at = Instant::now() + Duration::from_secs(body.expires_in);
        let token = SecretString::from(body.access_token);
        let clone = token.clone();
        {
            let mut cache = self.inner.token_cache.write().await;
            *cache = Some(CachedToken { token, expires_at });
        }
        tracing::debug!("service token refreshed");
        Ok(clone)
    }

    /// Look up the effective org role for `user_id` in `org_id`.
    /// Revoked memberships are filtered. Returns `Ok(None)` when the user
    /// is not a member. TTL-cached.
    pub async fn org_role(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<Option<OrgRole>, NyxIdError> {
        let key = (org_id.to_string(), user_id.to_string());
        // Check cache.
        {
            let cache = self.inner.role_cache.read().await;
            if let Some(cached) = cache.get(&key) {
                if cached.expires_at > Instant::now() {
                    return Ok(cached.role);
                }
            }
        }
        // Fetch via service account.
        let token = self.service_token().await?;
        let url = format!("{}{ORGS_PATH}/{org_id}/members", self.inner.base_url);
        let response = self
            .inner
            .http
            .get(&url)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| http_err("org members", e))?;

        if !response.status().is_success() {
            let status = response.status();
            tracing::error!(
                org_id,
                user_id,
                status = %status,
                "nyxid org members request failed"
            );
            return Err(NyxIdError::Http(format!("org members status {status}")));
        }

        let members: Vec<OrgMember> = response
            .json()
            .await
            .map_err(|e| NyxIdError::Malformed(format!("org members body: {e}")))?;

        // Find the user's active (non-revoked) membership. If there are
        // multiple active memberships, pick the highest-privilege one.
        let role = members
            .iter()
            .filter(|m| m.user_id == user_id && m.revoked_at.is_none())
            .map(|m| m.role)
            .max_by_key(|r| match r {
                OrgRole::Admin => 2,
                OrgRole::Member => 1,
                OrgRole::Viewer => 0,
            });

        let expires_at = Instant::now() + self.inner.cache_ttl;
        {
            let mut cache = self.inner.role_cache.write().await;
            cache.insert(key, CachedRole { role, expires_at });
        }
        Ok(role)
    }

    /// List the orgs the calling user belongs to. Uses the caller's OWN
    /// bearer token (forwarded to NyxID). TTL-cached per user.
    pub async fn user_orgs(
        &self,
        user_id: &str,
        user_token: &SecretString,
    ) -> Result<Vec<OrgSummary>, NyxIdError> {
        // Check cache.
        {
            let cache = self.inner.orgs_cache.read().await;
            if let Some(cached) = cache.get(user_id) {
                if cached.expires_at > Instant::now() {
                    return Ok(cached.orgs.clone());
                }
            }
        }
        let url = format!("{}{ORGS_PATH}", self.inner.base_url);
        let response = self
            .inner
            .http
            .get(&url)
            .bearer_auth(user_token.expose_secret())
            .send()
            .await
            .map_err(|e| http_err("user orgs", e))?;

        if !response.status().is_success() {
            let status = response.status();
            tracing::error!(
                user_id,
                status = %status,
                "nyxid user orgs request failed"
            );
            return Err(NyxIdError::Http(format!("user orgs status {status}")));
        }

        // NyxID returns the orgs array directly. Tolerant deserialization
        // ignores unknown fields per OrgSummary.
        let orgs: Vec<OrgSummary> = response
            .json()
            .await
            .map_err(|e| NyxIdError::Malformed(format!("user orgs body: {e}")))?;

        let expires_at = Instant::now() + self.inner.cache_ttl;
        {
            let mut cache = self.inner.orgs_cache.write().await;
            cache.insert(
                user_id.to_string(),
                CachedOrgs {
                    orgs: orgs.clone(),
                    expires_at,
                },
            );
        }
        Ok(orgs)
    }

    /// Exchange an inbound user token for a delegated token (RFC 8693).
    /// The delegated token carries an `act` claim and is used to call
    /// NyxID's credential-injection proxy.
    pub async fn exchange_token(
        &self,
        subject_token: &SecretString,
    ) -> Result<DelegatedToken, NyxIdError> {
        let url = format!("{}{}", self.inner.base_url, TOKEN_PATH);
        let response = self
            .inner
            .http
            .post(&url)
            .form(&[
                (
                    "grant_type",
                    "urn:ietf:params:oauth:grant-type:token-exchange",
                ),
                ("subject_token", subject_token.expose_secret()),
                (
                    "subject_token_type",
                    "urn:ietf:params:oauth:token-type:access_token",
                ),
                ("client_id", &self.inner.client_id),
                ("client_secret", self.inner.client_secret.expose_secret()),
            ])
            .send()
            .await
            .map_err(|e| http_err("token exchange", e))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let body = response.text().await.unwrap_or_default();
            return Err(NyxIdError::ExchangeRejected(body));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(NyxIdError::Http(format!(
                "token exchange status {status}: {}",
                &body[..body.len().min(200)]
            )));
        }

        #[derive(Deserialize)]
        struct ExchangeResponse {
            access_token: String,
            expires_in: u64,
        }
        let body: ExchangeResponse = response
            .json()
            .await
            .map_err(|e| NyxIdError::Malformed(format!("exchange body: {e}")))?;

        Ok(DelegatedToken {
            access_token: SecretString::from(body.access_token),
            expires_in: body.expires_in,
        })
    }

    /// Proxy a request to GitHub through NyxID's credential-injection
    /// proxy using a delegated token. The proxy injects the user's GitHub
    /// credential; fkst-hosted never sees the raw GitHub token.
    pub async fn proxy_github(
        &self,
        delegated: &DelegatedToken,
        method: reqwest::Method,
        github_path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response, NyxIdError> {
        let url = format!(
            "{}{}{}",
            self.inner.base_url, GITHUB_PROXY_PATH, github_path
        );
        let mut request = self
            .inner
            .http
            .request(method, &url)
            .bearer_auth(delegated.access_token.expose_secret());
        if let Some(json_body) = body {
            request = request.json(&json_body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| http_err("github proxy", e))?;
        Ok(response)
    }

    /// Proxy a GitHub request pinned to ONE linked GitHub account.
    ///
    /// Appends NyxID's verified `_nyxid_via=<connection_id>` selector to the
    /// proxied path (URL-encoded; joined with `&` when `github_path` already
    /// carries a query string, `?` otherwise) and delegates to the unchanged
    /// [`proxy_github`]. NyxID strips this selector before forwarding to GitHub.
    /// All per-account routing is confined to this helper.
    pub async fn proxy_github_for(
        &self,
        delegated: &DelegatedToken,
        connection: &GithubConnection,
        method: reqwest::Method,
        github_path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response, NyxIdError> {
        let routed = with_via_selector(github_path, &connection.connection_id);
        self.proxy_github(delegated, method, &routed, body).await
    }

    /// List the caller's linked GitHub connections via NyxID, using the
    /// delegated bearer. Maps NyxID's response into [`GithubConnection`]s.
    ///
    /// See [`GITHUB_CONNECTIONS_PATH`] for the confined, UNVERIFIED route
    /// assumption. No credentials appear in any error.
    pub async fn github_connections(
        &self,
        delegated: &DelegatedToken,
    ) -> Result<Vec<GithubConnection>, NyxIdError> {
        let url = format!("{}{}", self.inner.base_url, GITHUB_CONNECTIONS_PATH);
        let response = self
            .inner
            .http
            .get(&url)
            .bearer_auth(delegated.access_token.expose_secret())
            .send()
            .await
            .map_err(|e| http_err("github connections", e))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let body = response.text().await.unwrap_or_default();
            return Err(NyxIdError::ExchangeRejected(body));
        }
        if !status.is_success() {
            tracing::error!(status = %status, "nyxid github connections request failed");
            return Err(NyxIdError::Http(format!(
                "github connections status {status}"
            )));
        }

        let connections: Vec<GithubConnection> = response
            .json()
            .await
            .map_err(|e| NyxIdError::Malformed(format!("github connections body: {e}")))?;
        Ok(connections)
    }

    /// Check whether a user exists in NyxID via service-account lookup.
    /// Returns `Ok(true)` when the user is found, `Ok(false)` when not.
    /// Uses `GET /api/v1/users/{user_id}` via the service token.
    pub async fn user_exists(&self, user_id: &str) -> Result<bool, NyxIdError> {
        let token = self.service_token().await?;
        let url = format!("{}{USERS_PATH}/{user_id}", self.inner.base_url);
        let response = self
            .inner
            .http
            .get(&url)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| http_err("user exists", e))?;

        match response.status() {
            s if s.is_success() => Ok(true),
            reqwest::StatusCode::NOT_FOUND => Ok(false),
            status => {
                tracing::error!(
                    user_id,
                    status = %status,
                    "nyxid user lookup failed"
                );
                Err(NyxIdError::Http(format!("user exists status {status}")))
            }
        }
    }

    /// Check whether an organization exists in NyxID via service-account lookup.
    /// Returns `Ok(true)` when the org is found, `Ok(false)` when not.
    /// Uses `GET /api/v1/orgs/{org_id}` via the service token.
    pub async fn org_exists(&self, org_id: &str) -> Result<bool, NyxIdError> {
        let token = self.service_token().await?;
        let url = format!("{}{ORGS_PATH}/{org_id}", self.inner.base_url);
        let response = self
            .inner
            .http
            .get(&url)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| http_err("org exists", e))?;

        match response.status() {
            s if s.is_success() => Ok(true),
            reqwest::StatusCode::NOT_FOUND => Ok(false),
            status => {
                tracing::error!(
                    org_id,
                    status = %status,
                    "nyxid org lookup failed"
                );
                Err(NyxIdError::Http(format!("org exists status {status}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const CLIENT_ID: &str = "sa_test_client";
    const CLIENT_SECRET: &str = "sas_supersecret_value_12345";

    fn test_client(server_uri: &str) -> NyxIdClient {
        NyxIdClient::new(
            server_uri,
            CLIENT_ID.to_string(),
            SecretString::from(CLIENT_SECRET.to_string()),
            Duration::from_secs(30),
        )
        .expect("client build")
    }

    // ---- service_token ----

    #[tokio::test]
    async fn service_token_sends_client_credentials_and_caches() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .and(body_string_contains("grant_type=client_credentials"))
            .and(body_string_contains(format!("client_id={CLIENT_ID}")))
            .and(body_string_contains(format!(
                "client_secret={CLIENT_SECRET}"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "svc_tok_1",
                "token_type": "Bearer",
                "expires_in": 3600,
                "scope": "read"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let tok1 = client.service_token().await.expect("first call");
        assert_eq!(tok1.expose_secret(), "svc_tok_1");
        // Second call should be served from cache (expect(1) above).
        let tok2 = client.service_token().await.expect("cached call");
        assert_eq!(tok2.expose_secret(), "svc_tok_1");
    }

    #[tokio::test]
    async fn service_token_rejected_credentials_is_service_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let err = test_client(&server.uri())
            .service_token()
            .await
            .expect_err("must fail");
        assert!(matches!(err, NyxIdError::ServiceAuth), "got {err:?}");
    }

    // ---- org_role ----

    #[tokio::test]
    async fn org_role_filters_revoked_and_caches() {
        let server = MockServer::start().await;
        // Service token mock.
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "svc_tok", "token_type": "Bearer", "expires_in": 3600, "scope": "read"
            })))
            .mount(&server)
            .await;
        // Members mock for u1 — expect exactly 1 call (cached on second).
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs/org-1/members"))
            .and(header("authorization", "Bearer svc_tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "membership_id": "m1", "user_id": "u1", "role": "viewer", "revoked_at": "2026-01-01T00:00:00Z" },
                { "membership_id": "m2", "user_id": "u1", "role": "member" }
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let role = client.org_role("org-1", "u1").await.expect("role");
        assert_eq!(role, Some(OrgRole::Member), "revoked viewer filtered");

        // Cached call — no additional HTTP request.
        let role2 = client.org_role("org-1", "u1").await.expect("cached");
        assert_eq!(role2, Some(OrgRole::Member));
    }

    // ---- user_orgs ----

    #[tokio::test]
    async fn user_orgs_forwards_caller_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(ORGS_PATH))
            .and(header("authorization", "Bearer user_tok_123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": "org-a" }, { "id": "org-b", "extra_field": "ignored" }
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let user_token = SecretString::from("user_tok_123".to_string());
        let orgs = client.user_orgs("u1", &user_token).await.expect("orgs");
        assert_eq!(orgs.len(), 2);
        assert_eq!(orgs[0].id, "org-a");
        assert_eq!(orgs[1].id, "org-b");

        // Cached.
        let orgs2 = client.user_orgs("u1", &user_token).await.expect("cached");
        assert_eq!(orgs2.len(), 2);
    }

    // ---- exchange_token ----

    #[tokio::test]
    async fn exchange_token_sends_rfc8693_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .and(body_string_contains(
                "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange",
            ))
            .and(body_string_contains("subject_token=user_subj"))
            .and(body_string_contains(
                "subject_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "delegated_tok", "expires_in": 300, "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let subject = SecretString::from("user_subj".to_string());
        let delegated = client.exchange_token(&subject).await.expect("exchange");
        assert_eq!(delegated.access_token.expose_secret(), "delegated_tok");
        assert_eq!(delegated.expires_in, 300);
    }

    #[tokio::test]
    async fn exchange_token_rejection_is_typed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad subject token"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let subject = SecretString::from("bad".to_string());
        let err = client
            .exchange_token(&subject)
            .await
            .expect_err("must fail");
        assert!(
            matches!(err, NyxIdError::ExchangeRejected(ref s) if s.contains("bad subject token")),
            "got {err:?}"
        );
    }

    // ---- proxy_github ----

    #[tokio::test]
    async fn proxy_github_hits_the_single_path_constant() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("{GITHUB_PROXY_PATH}/repos/owner/repo")))
            .and(header("authorization", "Bearer delegated_tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let delegated = DelegatedToken {
            access_token: SecretString::from("delegated_tok".to_string()),
            expires_in: 300,
        };
        let resp = client
            .proxy_github(&delegated, reqwest::Method::GET, "/repos/owner/repo", None)
            .await
            .expect("proxy");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }

    // ---- github_connections ----

    fn delegated_token(value: &str) -> DelegatedToken {
        DelegatedToken {
            access_token: SecretString::from(value.to_string()),
            expires_in: 300,
        }
    }

    #[tokio::test]
    async fn github_connections_lists_linked_accounts() {
        let server = MockServer::start().await;
        // The path constant carries a query string; match on the path prefix.
        Mock::given(method("GET"))
            .and(path("/api/v1/connections"))
            .and(header("authorization", "Bearer delegated_tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "connection_id": "c1", "login": "octocat", "primary": true },
                { "connection_id": "c2", "login": "hubber", "extra": "ignored" }
            ])))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let connections = client
            .github_connections(&delegated_token("delegated_tok"))
            .await
            .expect("connections");
        assert_eq!(connections.len(), 2);
        assert_eq!(connections[0].login, "octocat");
        assert!(connections[0].primary);
        // `primary` defaults to false when omitted (tolerant deserialize).
        assert_eq!(connections[1].connection_id, "c2");
        assert!(!connections[1].primary);
    }

    #[tokio::test]
    async fn github_connections_rejection_is_typed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/connections"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client
            .github_connections(&delegated_token("delegated_tok"))
            .await
            .expect_err("must fail");
        assert!(
            matches!(err, NyxIdError::ExchangeRejected(_)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn github_connections_secret_never_appears_in_error_or_debug() {
        // Point at nothing to trigger a transport error carrying the URL.
        let client = test_client("http://127.0.0.1:1");
        let secret = "delegated_super_secret_tok";
        let err = client
            .github_connections(&delegated_token(secret))
            .await
            .expect_err("unreachable");
        assert!(!format!("{err}").contains(secret), "Display leaked secret");
        assert!(!format!("{err:?}").contains(secret), "Debug leaked secret");
    }

    #[test]
    fn with_via_selector_uses_question_mark_then_ampersand() {
        assert_eq!(with_via_selector("/issues", "c1"), "/issues?_nyxid_via=c1");
        assert_eq!(
            with_via_selector("/issues?state=open", "c1"),
            "/issues?state=open&_nyxid_via=c1"
        );
        // The connection id is URL-encoded.
        assert_eq!(
            with_via_selector("/issues", "a b/c"),
            "/issues?_nyxid_via=a%20b%2Fc"
        );
    }

    #[tokio::test]
    async fn proxy_github_for_appends_via_selector() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("{GITHUB_PROXY_PATH}/issues")))
            .and(wiremock::matchers::query_param("_nyxid_via", "c-42"))
            .and(wiremock::matchers::query_param("state", "open"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let connection = GithubConnection {
            connection_id: "c-42".to_string(),
            login: "octocat".to_string(),
            primary: true,
        };
        let resp = client
            .proxy_github_for(
                &delegated_token("delegated_tok"),
                &connection,
                reqwest::Method::GET,
                "/issues?state=open",
                None,
            )
            .await
            .expect("proxy");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }

    // ---- secret hygiene ----

    #[tokio::test]
    async fn no_error_variant_or_debug_ever_contains_the_secret() {
        let unreachable = NyxIdClient::new(
            "http://127.0.0.1:1",
            CLIENT_ID.to_string(),
            SecretString::from(CLIENT_SECRET.to_string()),
            Duration::from_secs(30),
        )
        .expect("client");

        let live_err = unreachable.service_token().await.expect_err("unreachable");

        let errors: Vec<NyxIdError> = vec![
            live_err,
            NyxIdError::ServiceAuth,
            NyxIdError::Http("status 500".to_string()),
            NyxIdError::Malformed("bad json".to_string()),
            NyxIdError::ExchangeRejected("denied".to_string()),
        ];
        for err in &errors {
            let display = format!("{err}");
            let debug = format!("{err:?}");
            assert!(
                !display.contains(CLIENT_SECRET),
                "Display leaked: {display}"
            );
            assert!(!debug.contains(CLIENT_SECRET), "Debug leaked: {debug}");
        }

        let client_debug = format!("{unreachable:?}");
        assert!(!client_debug.contains(CLIENT_SECRET), "client Debug leaked");
        assert!(client_debug.contains("<redacted>"));
    }
}
