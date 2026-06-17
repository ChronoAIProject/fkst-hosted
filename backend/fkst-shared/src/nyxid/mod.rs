//! NyxID service client: forwarded-user-token org lookups, agent-key minting,
//! and the GitHub credential-injection proxy helpers.
//!
//! Owner-only credential model (#257): the client carries NO service account.
//! Every operation authenticates with the FORWARDED USER TOKEN (`bearer_auth`),
//! so a deploy needs only the NyxID base URL — no client credential.
//!
//! Design rules (mirroring `journal/github.rs`):
//! - All secrets live in `secrecy::SecretString`: exposed only at
//!   request-build time, never captured into `Debug`/`Display` of the
//!   client, any error variant, or any log line.
//! - The `api_base` is injectable for wiremock testing.
//! - HTTP timeout is 15 s.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::sync::RwLock;

// ---- Constants ----

/// Org list endpoint (caller's own bearer token).
pub const ORGS_PATH: &str = "/api/v1/orgs";

/// Agent API-key endpoint. `POST` mints a per-user agent key (`nyxid_ag_…`)
/// presenting the user's own bearer.
/// Verified against NyxID main v0.7.0 (`models/api_key.rs`, `key_service.rs`).
pub const API_KEYS_PATH: &str = "/api/v1/api-keys";

/// Default downstream-service slug for NyxID's GitHub credential-injection
/// proxy, used when `FKST_NYXID_GITHUB_PROXY_SLUG` is unset.
///
/// Verified against NyxID `main`/v0.7.0 (`backend/src/services/provider_service.rs`,
/// `DefaultServiceSeed` table): NyxID seeds its GitHub OAuth proxy under slug
/// `api-github` (`base_url = https://api.github.com`). The per-deployment value
/// is configurable; the forwarded-user-token proxy helpers build the generic
/// `/api/v1/proxy/s/{slug}` route from it, never reading a hardcoded route from
/// anywhere else in the codebase.
pub const DEFAULT_GITHUB_PROXY_SLUG: &str = "api-github";

/// Endpoint that lists the user's linked GitHub connections (one per linked
/// GitHub account) under the caller's delegated token.
///
/// **Draft contract — does NOT match NyxID main; confined here; consumed ONLY
/// by `github_hub` (NOT the ownership/authz path).** Verified against NyxID
/// `main` @ `bcaccc9` (v0.7.0): NyxID has NO per-github-login projection. The
/// real `GET /api/v1/connections` takes no `provider` param and returns
/// per-`UserService` rows (`service_id`/`service_name`/`has_credential`, no
/// `login`); the actual per-account listing is `GET /api/v1/providers/my-tokens`
/// → `{ "tokens": [...] }`, where the GitHub login lives in `metadata["username"]`
/// (may be absent) and there is no `connection_id`/`primary`; per-account routing
/// (`_nyxid_via`) keys on a `UserService` id from `/api/v1/keys`, not the token
/// id; and the identity JWT carries no github-login claim. This `{connection_id,
/// login, primary}` shape + path are therefore a wiremock-pinned DRAFT used only
/// by the github-issues hub. Correcting it to the verified `main` model is
/// tracked in #156. The object/ownership layer never reads this — goal/session
/// ownership is keyed on the NyxID `sub` (#142), so no sub↔login binding exists.
pub const GITHUB_CONNECTIONS_PATH: &str = "/api/v1/connections?provider=github";

/// NyxID-internal query selector that pins a proxied request to one specific
/// linked credential instance (verified `_nyxid_via` mechanism on NyxID main:
/// it routes by user-service / connection id and is stripped before the request
/// is forwarded to GitHub). Confined here so per-account routing lives in one
/// place.
const NYXID_VIA_PARAM: &str = "_nyxid_via";

/// Per-request HTTP timeout.
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

// ---- Error type ----

/// NyxID integration errors. No variant carries secrets or credentials.
#[derive(Debug, thiserror::Error)]
pub enum NyxIdError {
    /// HTTP transport error (credential-free text).
    #[error("nyxid http error: {0}")]
    Http(String),
    /// NyxID returned an unexpected or malformed response.
    #[error("nyxid response malformed: {0}")]
    Malformed(String),
    /// The user's first-party access token was rejected by NyxID (401/403):
    /// expired, revoked, or a delegated/service token a human-only route
    /// refuses. Produced both by the api-key mint and by the forwarded-user-token
    /// GitHub connections listing. Kept DISTINCT from the generic [`Self::Http`]
    /// so the session driver can surface the precise reason without ever
    /// logging the token itself.
    #[error("nyxid user token rejected")]
    UserTokenRejected,
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
/// confined DRAFT route/shape (does not match NyxID main; corrected in #156).
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

/// Buffered result of a generic NyxID proxy call ([`NyxIdClient::proxy_request`]).
///
/// Carries the upstream status, headers, and the fully-read body bytes. The
/// whole response is buffered (rather than streamed) so the proxy helper stays
/// transport-agnostic and trivially testable; the bodies fkst-hosted proxies
/// through Ornn are small JSON envelopes, never large package zips (those are
/// fetched DIRECTLY from chrono-storage, not through this proxy).
#[derive(Debug)]
pub struct ProxyResponse {
    /// Upstream HTTP status, surfaced verbatim so callers can pass Ornn's
    /// 4xx/5xx through as the authoritative result (fkst-hosted adds no
    /// permission logic of its own).
    pub status: reqwest::StatusCode,
    /// Upstream response headers (e.g. content-type), surfaced verbatim.
    pub headers: reqwest::header::HeaderMap,
    /// The fully-read response body. A `Vec<u8>` (not a `bytes::Bytes`) so the
    /// type needs no extra dependency; the proxied bodies are small JSON.
    pub body: Vec<u8>,
}

/// A freshly minted agent API key. NyxID returns the `full_key` (the
/// `nyxid_ag_…` value) exactly ONCE at creation; the `id` is the stable
/// handle. The full key is a secret: it is held in a `SecretString` and
/// redacted from `Debug` so it never lands in a log line.
pub struct CreatedKey {
    /// Stable key identifier.
    pub id: String,
    /// The one-time-visible `nyxid_ag_…` secret. NEVER log or persist this.
    pub full_key: SecretString,
}

impl fmt::Debug for CreatedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedKey")
            .field("id", &self.id)
            .field("full_key", &"<redacted>")
            .finish()
    }
}

// ---- Cached entries ----

/// Cached user-orgs list for a user_id.
struct CachedOrgs {
    orgs: Vec<OrgSummary>,
    expires_at: Instant,
}

// ---- Client ----

/// Inner state behind `Arc`.
struct Inner {
    base_url: String,
    /// The configured downstream-service slug NyxID resolves to inject the
    /// user's GitHub credential. The forwarded-user-token helpers
    /// ([`NyxIdClient::proxy_github_user`]) build the generic
    /// `/api/v1/proxy/s/{slug}` shape from this raw slug.
    github_proxy_slug: String,
    http: reqwest::Client,
    /// User-orgs cache keyed by user_id.
    orgs_cache: RwLock<HashMap<String, CachedOrgs>>,
    /// TTL for the user-orgs cache.
    cache_ttl: Duration,
}

/// NyxID service client (owner-only, #257): forwarded-user-token org lookups,
/// agent-key minting, and the GitHub proxy helpers. Cheaply cloneable
/// (`Arc`-backed); carries no service-account credential.
#[derive(Clone)]
pub struct NyxIdClient {
    inner: Arc<Inner>,
}

impl fmt::Debug for NyxIdClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NyxIdClient")
            .field("base_url", &self.inner.base_url)
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
    /// Build a new owner-only NyxIdClient (#257). `base_url` is the NyxID issuer
    /// base (injectable for wiremock testing). `github_proxy_slug` is the
    /// downstream-service slug NyxID resolves to inject the user's GitHub
    /// credential; the forwarded-user-token helpers build the generic
    /// `/api/v1/proxy/s/{slug}` route from it (default
    /// [`DEFAULT_GITHUB_PROXY_SLUG`]). `cache_ttl` controls how long user-orgs
    /// results are cached.
    ///
    /// The client carries NO service account: every operation authenticates with
    /// the forwarded user token (`bearer_auth`) — per-session key mint, the Ornn
    /// proxy, the github_hub connections lookups, and repo-create.
    pub fn new(
        base_url: &str,
        github_proxy_slug: &str,
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
                github_proxy_slug: github_proxy_slug.to_string(),
                http,
                orgs_cache: RwLock::new(HashMap::new()),
                cache_ttl,
            }),
        })
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

    /// Proxy a GitHub request through NyxID's credential-injection proxy
    /// presenting the user's OWN forwarded bearer token (no RFC 8693 exchange).
    ///
    /// Targets the generic `{base}/api/v1/proxy/s/{slug}{github_path}` shape —
    /// the same slug proxy [`proxy_request`] uses — verified reachable for the
    /// api-github slug under a forwarded user bearer (`GET …/s/api-github/user`
    /// returns the user profile). The configured GitHub-proxy slug names the
    /// downstream service; the user token is exposed only to build the
    /// `Authorization` header and is NEVER captured into any error, log line, or
    /// the returned value. The full response is buffered into a [`ProxyResponse`]
    /// (status/headers/body surfaced verbatim, no success-mapping) so callers
    /// pass GitHub's 4xx/5xx through as the authoritative result.
    pub async fn proxy_github_user(
        &self,
        user_token: &SecretString,
        method: reqwest::Method,
        github_path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<ProxyResponse, NyxIdError> {
        let url = format!(
            "{}/api/v1/proxy/s/{}{github_path}",
            self.inner.base_url, self.inner.github_proxy_slug
        );
        let mut request = self
            .inner
            .http
            .request(method, &url)
            .bearer_auth(user_token.expose_secret());
        if let Some(bytes) = body {
            request = request.body(bytes);
        }
        let response = request
            .send()
            .await
            .map_err(|e| http_err("github user proxy", e))?;

        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|e| http_err("github user proxy body", e))?
            .to_vec();
        Ok(ProxyResponse {
            status,
            headers,
            body,
        })
    }

    /// Proxy a forwarded-user-token GitHub request pinned to ONE linked GitHub
    /// account.
    ///
    /// Appends NyxID's verified `_nyxid_via=<connection_id>` selector to the
    /// proxied path (URL-encoded; joined with `&` when `github_path` already
    /// carries a query string, `?` otherwise) and delegates to the unchanged
    /// [`proxy_github_user`]. NyxID strips this selector before forwarding to
    /// GitHub. All per-account routing is confined to this helper.
    pub async fn proxy_github_user_for(
        &self,
        user_token: &SecretString,
        connection: &GithubConnection,
        method: reqwest::Method,
        github_path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<ProxyResponse, NyxIdError> {
        let routed = with_via_selector(github_path, &connection.connection_id);
        self.proxy_github_user(user_token, method, &routed, body)
            .await
    }

    /// Proxy an arbitrary request to a downstream service through NyxID's
    /// credential-injection proxy, presenting the user's OWN bearer token.
    ///
    /// Builds `{base}/api/v1/proxy/s/{slug}{path}` + `query`, sets
    /// `Authorization: Bearer <user_token>`, optionally attaches `body` (used
    /// for non-GET methods), and buffers the full response into a
    /// [`ProxyResponse`]. The slug-based `proxy/s/{slug}` shape is NyxID's
    /// generic credential proxy, so this one helper
    /// serves every slugged service — e.g. `ornn-api` for the Ornn skill
    /// registry (#114). The user token is exposed only to build the header and
    /// is NEVER captured into any error, log line, or the returned value.
    ///
    /// The status is surfaced verbatim (no success-mapping here): callers that
    /// front a downstream API are expected to pass its 4xx/5xx through as the
    /// authoritative result rather than reinterpret it.
    pub async fn proxy_request(
        &self,
        slug: &str,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        user_token: &SecretString,
        body: Option<Vec<u8>>,
    ) -> Result<ProxyResponse, NyxIdError> {
        let url = format!("{}/api/v1/proxy/s/{slug}{path}", self.inner.base_url);
        let mut request = self
            .inner
            .http
            .request(method, &url)
            .bearer_auth(user_token.expose_secret());
        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(bytes) = body {
            request = request.body(bytes);
        }
        let response = request
            .send()
            .await
            .map_err(|e| http_err("nyxid proxy request", e))?;

        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|e| http_err("nyxid proxy body", e))?
            .to_vec();
        Ok(ProxyResponse {
            status,
            headers,
            body,
        })
    }

    /// List the caller's linked GitHub connections via NyxID, using the user's
    /// OWN forwarded bearer token (no RFC 8693 exchange). Maps NyxID's
    /// response into [`GithubConnection`]s.
    ///
    /// See [`GITHUB_CONNECTIONS_PATH`] for the confined DRAFT route/shape (does
    /// not match NyxID main; corrected in #156). The user token is exposed only
    /// to build the bearer header and appears in no error. A 401/403 maps to the
    /// DISTINCT [`NyxIdError::UserTokenRejected`] (the forwarded user credential
    /// was refused), mirroring [`Self::mint_user_api_key`].
    pub async fn github_connections_user(
        &self,
        user_token: &SecretString,
    ) -> Result<Vec<GithubConnection>, NyxIdError> {
        let url = format!("{}{}", self.inner.base_url, GITHUB_CONNECTIONS_PATH);
        let response = self
            .inner
            .http
            .get(&url)
            .bearer_auth(user_token.expose_secret())
            .send()
            .await
            .map_err(|e| http_err("github user connections", e))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(NyxIdError::UserTokenRejected);
        }
        if !status.is_success() {
            tracing::error!(status = %status, "nyxid github user connections request failed");
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

    /// Mint a per-user agent API key on the caller's behalf by presenting the
    /// user's OWN first-party access token as the bearer (NyxID's mint route
    /// is human-only and binds the new key to that user). The body carries the
    /// key `name`, its `scopes`, and `allow_all_services`.
    ///
    /// `expires_at` is an optional RFC 3339 timestamp (e.g.
    /// `2026-04-01T00:00:00Z`) forwarded verbatim to NyxID's
    /// `CreateApiKeyRequest.expires_at` (verified against NyxID `main`:
    /// `handlers/api_keys.rs` parses RFC 3339 or `YYYY-MM-DD`). When `Some`, the
    /// minted key SELF-EXPIRES — the cleanup mechanism for per-session keys
    /// (#216), since NyxID's human-only key store rejects a service-account
    /// token on a user-minted key. `None` keeps the key non-expiring (used only
    /// where a caller deliberately opts out of TTL).
    ///
    /// Returns [`CreatedKey`] with the one-time-visible `full_key`. A 401/403
    /// maps to the DISTINCT [`NyxIdError::UserTokenRejected`] (the user token
    /// was expired/revoked/delegated) so the caller can fail the session with a
    /// precise reason. The token and the minted key are never logged.
    pub async fn mint_user_api_key(
        &self,
        raw_token: &SecretString,
        name: &str,
        scopes: &str,
        allow_all_services: bool,
        expires_at: Option<&str>,
    ) -> Result<CreatedKey, NyxIdError> {
        let url = format!("{}{API_KEYS_PATH}", self.inner.base_url);
        // Build the body, adding `expires_at` only when a TTL is requested so
        // the field is omitted (NyxID treats absent as non-expiring) otherwise.
        let mut payload = serde_json::json!({
            "name": name,
            "scopes": scopes,
            "allow_all_services": allow_all_services,
        });
        if let Some(expiry) = expires_at {
            payload["expires_at"] = serde_json::Value::String(expiry.to_string());
        }
        let response = self
            .inner
            .http
            .post(&url)
            .bearer_auth(raw_token.expose_secret())
            .json(&payload)
            .send()
            .await
            .map_err(|e| http_err("api-key mint", e))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // The body may carry NyxID's reason but never our token (it was a
            // request header, not the response). We discard it: the typed
            // variant already states the cause, and logging a rejected-auth
            // body risks echoing sensitive request context back into the logs.
            tracing::error!(status = %status, "nyxid rejected the user token for api-key mint");
            return Err(NyxIdError::UserTokenRejected);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!(status = %status, "nyxid api-key mint failed");
            return Err(NyxIdError::Http(format!(
                "api-key mint status {status}: {}",
                &body[..body.len().min(200)]
            )));
        }

        #[derive(Deserialize)]
        struct MintResponse {
            id: String,
            full_key: String,
        }
        let body: MintResponse = response
            .json()
            .await
            .map_err(|e| NyxIdError::Malformed(format!("api-key mint body: {e}")))?;

        Ok(CreatedKey {
            id: body.id,
            full_key: SecretString::from(body.full_key),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_client(server_uri: &str) -> NyxIdClient {
        client_with_slug(server_uri, DEFAULT_GITHUB_PROXY_SLUG)
    }

    /// Build a test client with an explicit GitHub-proxy slug so the proxy-path
    /// tests can assert both the default and an override route shape.
    fn client_with_slug(server_uri: &str, slug: &str) -> NyxIdClient {
        NyxIdClient::new(server_uri, slug, Duration::from_secs(30)).expect("client build")
    }

    // ---- owner-only client (#257) ----

    #[test]
    fn client_debug_carries_only_the_base_url() {
        // The owner-only client carries no credential; its Debug must surface
        // only the base URL and never any secret marker.
        let client = test_client("http://localhost");
        let debug = format!("{client:?}");
        assert!(debug.contains("base_url"), "got: {debug}");
        assert!(!debug.contains("<redacted>"), "got: {debug}");
        assert!(!debug.contains("client_secret"), "got: {debug}");
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

    // ---- proxy_request (generic slug proxy) ----

    #[tokio::test]
    async fn proxy_request_builds_slug_path_query_and_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/proxy/s/ornn-api/api/v1/skills/demo"))
            .and(header("authorization", "Bearer user_tok"))
            .and(wiremock::matchers::query_param("version", "1.2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(serde_json::json!({"data": {"name": "demo"}})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let token = SecretString::from("user_tok".to_string());
        let resp = client
            .proxy_request(
                "ornn-api",
                reqwest::Method::GET,
                "/api/v1/skills/demo",
                &[("version", "1.2")],
                &token,
                None,
            )
            .await
            .expect("proxy request");
        assert_eq!(resp.status, reqwest::StatusCode::OK);
        assert!(resp
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .contains("application/json"));
        assert!(!resp.body.is_empty());
    }

    #[tokio::test]
    async fn proxy_request_passes_upstream_status_through_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/proxy/s/ornn-api/api/v1/skills/private"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let token = SecretString::from("user_tok".to_string());
        let resp = client
            .proxy_request(
                "ornn-api",
                reqwest::Method::GET,
                "/api/v1/skills/private",
                &[],
                &token,
                None,
            )
            .await
            .expect("proxy request returns Ok even on 4xx");
        assert_eq!(resp.status, reqwest::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn proxy_request_never_leaks_the_user_token_in_errors() {
        // Point at nothing to force a transport error carrying the URL; the
        // user token must never appear in Display or Debug of the error.
        let client = test_client("http://127.0.0.1:1");
        let secret = "user_super_secret_proxy_token";
        let err = client
            .proxy_request(
                "ornn-api",
                reqwest::Method::GET,
                "/api/v1/skills/demo",
                &[],
                &SecretString::from(secret.to_string()),
                None,
            )
            .await
            .expect_err("unreachable");
        assert!(!format!("{err}").contains(secret), "Display leaked token");
        assert!(!format!("{err:?}").contains(secret), "Debug leaked token");
    }

    // ---- _nyxid_via selector ----

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

    // ---- proxy_github_user / proxy_github_user_for (forwarded user token) ----

    #[tokio::test]
    async fn proxy_github_user_hits_slug_proxy_with_user_bearer() {
        let server = MockServer::start().await;
        // Forwarded user token must hit the generic `/api/v1/proxy/s/{slug}`
        // shape (NOT the legacy `/api/v1/proxy/{slug}` path).
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/proxy/s/{DEFAULT_GITHUB_PROXY_SLUG}/user"
            )))
            .and(header("authorization", "Bearer user_tok"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(serde_json::json!({"login": "octocat"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let token = SecretString::from("user_tok".to_string());
        let resp = client
            .proxy_github_user(&token, reqwest::Method::GET, "/user", None)
            .await
            .expect("proxy github user");
        assert_eq!(resp.status, reqwest::StatusCode::OK);
        assert!(resp
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .contains("application/json"));
        assert!(!resp.body.is_empty());
    }

    #[tokio::test]
    async fn proxy_github_user_honours_an_override_slug() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/proxy/s/custom-gh-slug/user"))
            .and(header("authorization", "Bearer user_tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&server)
            .await;

        let client = client_with_slug(&server.uri(), "custom-gh-slug");
        let token = SecretString::from("user_tok".to_string());
        let resp = client
            .proxy_github_user(&token, reqwest::Method::GET, "/user", None)
            .await
            .expect("proxy github user");
        assert_eq!(resp.status, reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn proxy_github_user_passes_upstream_status_through_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/proxy/s/{DEFAULT_GITHUB_PROXY_SLUG}/user"
            )))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let token = SecretString::from("user_tok".to_string());
        let resp = client
            .proxy_github_user(&token, reqwest::Method::GET, "/user", None)
            .await
            .expect("proxy returns Ok even on 4xx");
        assert_eq!(resp.status, reqwest::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn proxy_github_user_for_appends_via_selector_under_user_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/proxy/s/{DEFAULT_GITHUB_PROXY_SLUG}/issues"
            )))
            .and(header("authorization", "Bearer user_tok"))
            .and(wiremock::matchers::query_param("_nyxid_via", "c-42"))
            .and(wiremock::matchers::query_param("state", "open"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let token = SecretString::from("user_tok".to_string());
        let connection = GithubConnection {
            connection_id: "c-42".to_string(),
            login: "octocat".to_string(),
            primary: true,
        };
        let resp = client
            .proxy_github_user_for(
                &token,
                &connection,
                reqwest::Method::GET,
                "/issues?state=open",
                None,
            )
            .await
            .expect("proxy github user for");
        assert_eq!(resp.status, reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn proxy_github_user_never_leaks_the_user_token_in_errors() {
        // Point at nothing to force a transport error carrying the URL; the
        // user token must never appear in Display or Debug of the error.
        let client = test_client("http://127.0.0.1:1");
        let secret = "user_super_secret_github_token";
        let err = client
            .proxy_github_user(
                &SecretString::from(secret.to_string()),
                reqwest::Method::GET,
                "/user",
                None,
            )
            .await
            .expect_err("unreachable");
        assert!(!format!("{err}").contains(secret), "Display leaked token");
        assert!(!format!("{err:?}").contains(secret), "Debug leaked token");
    }

    // ---- github_connections_user (forwarded user token) ----

    #[tokio::test]
    async fn github_connections_user_lists_accounts_under_user_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/connections"))
            .and(header("authorization", "Bearer user_tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "connection_id": "c1", "login": "octocat", "primary": true },
                { "connection_id": "c2", "login": "hubber", "extra": "ignored" }
            ])))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let token = SecretString::from("user_tok".to_string());
        let connections = client
            .github_connections_user(&token)
            .await
            .expect("connections");
        assert_eq!(connections.len(), 2);
        assert_eq!(connections[0].login, "octocat");
        assert!(connections[0].primary);
        assert_eq!(connections[1].connection_id, "c2");
        assert!(!connections[1].primary);
    }

    #[tokio::test]
    async fn github_connections_user_rejection_is_typed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/connections"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let token = SecretString::from("user_tok".to_string());
        let err = client
            .github_connections_user(&token)
            .await
            .expect_err("must fail");
        assert!(matches!(err, NyxIdError::UserTokenRejected), "got {err:?}");
    }

    #[tokio::test]
    async fn github_connections_user_secret_never_appears_in_error_or_debug() {
        // Point at nothing to trigger a transport error carrying the URL.
        let client = test_client("http://127.0.0.1:1");
        let secret = "user_super_secret_connections_tok";
        let err = client
            .github_connections_user(&SecretString::from(secret.to_string()))
            .await
            .expect_err("unreachable");
        assert!(!format!("{err}").contains(secret), "Display leaked secret");
        assert!(!format!("{err:?}").contains(secret), "Debug leaked secret");
    }

    // ---- mint_user_api_key ----

    #[tokio::test]
    async fn mint_user_api_key_posts_bearer_and_body_then_maps_created_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .and(header("authorization", "Bearer user_raw_tok"))
            .and(body_string_contains("fkst-session-abc"))
            .and(body_string_contains("proxy"))
            .and(body_string_contains("allow_all_services"))
            // The requested TTL must reach NyxID as `expires_at` (#216).
            .and(body_string_contains("expires_at"))
            .and(body_string_contains("2026-04-01T00:00:00Z"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "key-id-1",
                "full_key": "nyxid_ag_deadbeefdeadbeefdeadbeef",
                "name": "fkst-session-abc"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let raw = SecretString::from("user_raw_tok".to_string());
        let created = client
            .mint_user_api_key(
                &raw,
                "fkst-session-abc",
                "proxy",
                true,
                Some("2026-04-01T00:00:00Z"),
            )
            .await
            .expect("mint");
        assert_eq!(created.id, "key-id-1");
        assert_eq!(
            created.full_key.expose_secret(),
            "nyxid_ag_deadbeefdeadbeefdeadbeef"
        );
        // The minted key is a secret: its Debug must be redacted.
        let debug = format!("{created:?}");
        assert!(!debug.contains("nyxid_ag_"), "Debug leaked the full key");
        assert!(debug.contains("<redacted>"));
    }

    #[tokio::test]
    async fn mint_user_api_key_accepts_200_as_well_as_201() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "k2", "full_key": "nyxid_ag_00"
            })))
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let raw = SecretString::from("tok".to_string());
        let created = client
            .mint_user_api_key(&raw, "n", "proxy", true, None)
            .await
            .expect("mint 200");
        assert_eq!(created.id, "k2");
    }

    #[tokio::test]
    async fn mint_user_api_key_omits_expires_at_when_no_ttl_requested() {
        use wiremock::matchers::body_json;
        let server = MockServer::start().await;
        // With `expires_at = None` the field must NOT appear in the body so
        // NyxID treats the key as non-expiring; assert the exact JSON shape.
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .and(body_json(serde_json::json!({
                "name": "n",
                "scopes": "proxy",
                "allow_all_services": true,
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "k3", "full_key": "nyxid_ag_03"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let raw = SecretString::from("tok".to_string());
        let created = client
            .mint_user_api_key(&raw, "n", "proxy", true, None)
            .await
            .expect("mint without ttl");
        assert_eq!(created.id, "k3");
    }

    #[tokio::test]
    async fn mint_user_api_key_401_is_user_token_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .respond_with(ResponseTemplate::new(401).set_body_string("token expired"))
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let err = client
            .mint_user_api_key(
                &SecretString::from("bad".to_string()),
                "n",
                "proxy",
                true,
                None,
            )
            .await
            .expect_err("must fail");
        assert!(matches!(err, NyxIdError::UserTokenRejected), "got {err:?}");
    }

    #[tokio::test]
    async fn mint_user_api_key_403_is_user_token_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .respond_with(
                ResponseTemplate::new(403).set_body_string("delegated tokens cannot mint"),
            )
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let err = client
            .mint_user_api_key(
                &SecretString::from("delegated".to_string()),
                "n",
                "proxy",
                true,
                None,
            )
            .await
            .expect_err("must fail");
        assert!(matches!(err, NyxIdError::UserTokenRejected), "got {err:?}");
    }

    #[tokio::test]
    async fn mint_user_api_key_500_is_generic_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let client = test_client(&server.uri());
        let err = client
            .mint_user_api_key(
                &SecretString::from("tok".to_string()),
                "n",
                "proxy",
                true,
                None,
            )
            .await
            .expect_err("must fail");
        assert!(matches!(err, NyxIdError::Http(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn mint_user_api_key_never_leaks_the_raw_token_in_errors() {
        // Point at nothing to force a transport error carrying the URL; the
        // raw token must never appear in Display or Debug of the error.
        let client = test_client("http://127.0.0.1:1");
        let secret_tok = "user_raw_super_secret_token";
        let err = client
            .mint_user_api_key(
                &SecretString::from(secret_tok.to_string()),
                "n",
                "proxy",
                true,
                Some("2026-04-01T00:00:00Z"),
            )
            .await
            .expect_err("unreachable");
        assert!(
            !format!("{err}").contains(secret_tok),
            "Display leaked token"
        );
        assert!(
            !format!("{err:?}").contains(secret_tok),
            "Debug leaked token"
        );
    }

    // ---- secret hygiene ----

    #[tokio::test]
    async fn no_error_variant_or_debug_ever_contains_the_secret() {
        // The owner-only client carries no credential; the secret under test is
        // the forwarded USER token, which a live transport error must never echo.
        const USER_SECRET: &str = "user_super_secret_token_value_12345";
        let unreachable = test_client("http://127.0.0.1:1");

        let live_err = unreachable
            .mint_user_api_key(
                &SecretString::from(USER_SECRET.to_string()),
                "n",
                "proxy",
                true,
                None,
            )
            .await
            .expect_err("unreachable");

        let errors: Vec<NyxIdError> = vec![
            live_err,
            NyxIdError::Http("status 500".to_string()),
            NyxIdError::Malformed("bad json".to_string()),
            NyxIdError::UserTokenRejected,
        ];
        for err in &errors {
            let display = format!("{err}");
            let debug = format!("{err:?}");
            assert!(!display.contains(USER_SECRET), "Display leaked: {display}");
            assert!(!debug.contains(USER_SECRET), "Debug leaked: {debug}");
        }

        // The client Debug carries only the base URL — no secret, no marker.
        let client_debug = format!("{unreachable:?}");
        assert!(!client_debug.contains(USER_SECRET), "client Debug leaked");
        assert!(!client_debug.contains("<redacted>"));
    }
}
