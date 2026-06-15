//! GitHub API transport: the `GithubApi` trait and its `HttpGithubApi` implementation.
//!
//! Two endpoints are used:
//!   1. `GET {base}/repos/{owner}/{repo}/installation` -- resolve the installation
//!      covering a repo (404 = not installed).
//!   2. `POST {base}/app/installations/{id}/access_tokens` -- mint a 1-hour
//!      installation token scoped to specific repos and a permissions subset.
//!
//! HTTP client patterns mirror `src/journal/github.rs`: injected `api_base`,
//! 20s timeout, user-agent `fkst-hosted-api`, rate-limit / auth disambiguation.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use super::GithubAppError;

/// Request timeout for every GitHub API call.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Opaque installation ID resolved from the GitHub API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InstallationId(pub u64);

/// Permission subset requested for an installation token. Values are "read" or
/// "write". Omitted fields mean "no permission requested".
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize)]
pub struct TokenPermissions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issues: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_requests: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub administration: Option<String>,
}

/// Token creation request body.
#[derive(Serialize)]
pub struct InstallationTokenRequest {
    /// Bare repo names (NOT `owner/name`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<String>,
    /// Permission subset; `None` requests the installation's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<TokenPermissions>,
}

/// A minted installation token with its expiry.
pub struct InstallationToken {
    pub token: SecretString,
    pub expires_at: SystemTime,
}

// Hand-written: the token must never appear in Debug.
impl fmt::Debug for InstallationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallationToken")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Abstract GitHub API transport. `HttpGithubApi` is the production impl;
/// tests inject a fake.
#[async_trait]
pub trait GithubApi: Send + Sync {
    /// `GET {base}/repos/{owner}/{repo}/installation` with Bearer = app JWT.
    /// 404 -> [`GithubAppError::NotInstalled`].
    async fn installation_for_repo(
        &self,
        app_jwt: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<InstallationId, GithubAppError>;

    /// `POST {base}/app/installations/{id}/access_tokens`.
    /// 404 -> [`GithubAppError::InstallationGone`].
    /// 422 -> [`GithubAppError::TokenRequestRejected`].
    async fn create_installation_token(
        &self,
        app_jwt: &SecretString,
        id: InstallationId,
        req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError>;
}

/// Production HTTP transport backed by reqwest.
pub struct HttpGithubApi {
    api_base: String,
    client: reqwest::Client,
}

impl fmt::Debug for HttpGithubApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpGithubApi")
            .field("api_base", &self.api_base)
            .finish()
    }
}

impl HttpGithubApi {
    pub fn new(api_base: &str) -> Result<Self, GithubAppError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("fkst-hosted-api")
            .build()
            .map_err(|e| GithubAppError::Http(format!("client build: {e}")))?;
        Ok(Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            client,
        })
    }
}

/// Seconds until the rate-limit reset, from `retry-after` or
/// `x-ratelimit-reset`. Defaults to 60s when unparseable.
fn reset_seconds(headers: &reqwest::header::HeaderMap) -> u64 {
    if let Some(retry_after) = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        return retry_after;
    }
    if let Some(reset_epoch) = headers
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        return reset_epoch.saturating_sub(now);
    }
    60
}

/// True when a 403 carries rate-limit evidence.
fn is_rate_limited(headers: &reqwest::header::HeaderMap) -> bool {
    let remaining_zero = headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "0")
        .unwrap_or(false);
    remaining_zero || headers.contains_key("retry-after")
}

#[async_trait]
impl GithubApi for HttpGithubApi {
    async fn installation_for_repo(
        &self,
        app_jwt: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}/installation", self.api_base);
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(app_jwt.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("installation_for_repo: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(GithubAppError::NotInstalled {
                owner_repo: format!("{owner}/{repo}"),
                install_url: None,
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(GithubAppError::AppAuth);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            if is_rate_limited(response.headers()) {
                return Err(GithubAppError::RateLimited(reset_seconds(
                    response.headers(),
                )));
            }
            return Err(GithubAppError::AppAuth);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "installation_for_repo status {status}: {body}"
            )));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("installation_for_repo body: {e}")))?;
        let id = body["id"]
            .as_u64()
            .ok_or_else(|| GithubAppError::Http("installation_for_repo: missing id".to_string()))?;
        Ok(InstallationId(id))
    }

    async fn create_installation_token(
        &self,
        app_jwt: &SecretString,
        id: InstallationId,
        req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        let url = format!("{}/app/installations/{}/access_tokens", self.api_base, id.0);
        let response = self
            .client
            .post(&url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(app_jwt.expose_secret())
            .json(req)
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_installation_token: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(GithubAppError::InstallationGone {
                owner_repo: String::new(),
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(GithubAppError::AppAuth);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            if is_rate_limited(response.headers()) {
                return Err(GithubAppError::RateLimited(reset_seconds(
                    response.headers(),
                )));
            }
            return Err(GithubAppError::AppAuth);
        }
        if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::TokenRequestRejected(body));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "create_installation_token status {status}: {body}"
            )));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_installation_token body: {e}")))?;

        let token_str = body["token"]
            .as_str()
            .ok_or_else(|| {
                GithubAppError::Http("create_installation_token: missing token".to_string())
            })?
            .to_string();

        let expires_str = body["expires_at"].as_str().ok_or_else(|| {
            GithubAppError::Http("create_installation_token: missing expires_at".to_string())
        })?;

        let expires_dt = bson::DateTime::parse_rfc3339_str(expires_str).map_err(|e| {
            GithubAppError::Http(format!("create_installation_token: bad expires_at: {e}"))
        })?;

        let expires_at = SystemTime::UNIX_EPOCH
            + std::time::Duration::from_millis(expires_dt.timestamp_millis() as u64);

        Ok(InstallationToken {
            token: SecretString::from(token_str),
            expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const APP_JWT: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.test.payload";

    fn api(server_uri: &str) -> HttpGithubApi {
        HttpGithubApi::new(server_uri).expect("api client")
    }

    fn jwt() -> SecretString {
        SecretString::from(APP_JWT.to_string())
    }

    // ---- installation_for_repo -----------------------------------------------

    #[tokio::test]
    async fn installation_lookup_sends_bearer_on_correct_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/site/installation"))
            .and(header(
                "authorization",
                format!("Bearer {APP_JWT}").as_str(),
            ))
            .and(header("accept", "application/vnd.github+json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 99999 })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let id = api(&server.uri())
            .installation_for_repo(&jwt(), "acme", "site")
            .await
            .expect("ok");
        assert_eq!(id, InstallationId(99999));
    }

    #[tokio::test]
    async fn installation_404_is_not_installed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let err = api(&server.uri())
            .installation_for_repo(&jwt(), "acme", "site")
            .await
            .expect_err("must fail");
        match err {
            GithubAppError::NotInstalled { owner_repo, .. } => {
                assert_eq!(owner_repo, "acme/site");
            }
            other => panic!("expected NotInstalled, got {other:?}"),
        }
    }

    // ---- create_installation_token -------------------------------------------

    #[tokio::test]
    async fn token_mint_posts_bare_repo_names_and_permissions() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/app/installations/42/access_tokens"))
            .and(header(
                "authorization",
                format!("Bearer {APP_JWT}").as_str(),
            ))
            .and(body_partial_json(serde_json::json!({
                "repositories": ["site"],
                "permissions": { "contents": "write", "issues": "read" }
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "token": "ghs_testtoken123",
                "expires_at": "2026-06-12T12:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let result = api(&server.uri())
            .create_installation_token(
                &jwt(),
                InstallationId(42),
                &InstallationTokenRequest {
                    repositories: vec!["site".to_string()],
                    permissions: Some(TokenPermissions {
                        contents: Some("write".to_string()),
                        issues: Some("read".to_string()),
                        ..TokenPermissions::default()
                    }),
                },
            )
            .await
            .expect("ok");

        assert_eq!(result.token.expose_secret(), "ghs_testtoken123");
    }

    #[tokio::test]
    async fn token_mint_serializes_admin_and_pull_requests() {
        // Issue #110: the elevated session permission set must reach GitHub in
        // the request body. Assert the serialized `permissions` object carries
        // `administration:write` and `pull_requests:write` (alongside the
        // existing contents/issues writes).
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/app/installations/7/access_tokens"))
            .and(body_partial_json(serde_json::json!({
                "permissions": {
                    "contents": "write",
                    "pull_requests": "write",
                    "issues": "write",
                    "administration": "write"
                }
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "token": "ghs_admintoken",
                "expires_at": "2026-06-12T12:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let result = api(&server.uri())
            .create_installation_token(
                &jwt(),
                InstallationId(7),
                &InstallationTokenRequest {
                    repositories: vec!["site".to_string()],
                    permissions: Some(TokenPermissions {
                        contents: Some("write".to_string()),
                        pull_requests: Some("write".to_string()),
                        issues: Some("write".to_string()),
                        administration: Some("write".to_string()),
                        metadata: None,
                    }),
                },
            )
            .await
            .expect("ok");

        assert_eq!(result.token.expose_secret(), "ghs_admintoken");
    }

    #[tokio::test]
    async fn token_mint_parses_expires_at() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "token": "ghs_xyz",
                "expires_at": "2026-06-12T13:00:00Z"
            })))
            .mount(&server)
            .await;

        let result = api(&server.uri())
            .create_installation_token(
                &jwt(),
                InstallationId(1),
                &InstallationTokenRequest {
                    repositories: vec![],
                    permissions: None,
                },
            )
            .await
            .expect("ok");

        // Verify that expires_at was parsed (non-zero SystemTime).
        assert!(result.expires_at > SystemTime::UNIX_EPOCH);
    }

    #[tokio::test]
    async fn token_mint_404_is_installation_gone() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let err = api(&server.uri())
            .create_installation_token(
                &jwt(),
                InstallationId(1),
                &InstallationTokenRequest {
                    repositories: vec![],
                    permissions: None,
                },
            )
            .await
            .expect_err("must fail");
        assert!(
            matches!(err, GithubAppError::InstallationGone { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn token_mint_422_is_token_request_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(422)
                    .set_body_json(serde_json::json!({ "message": "permission not granted" })),
            )
            .mount(&server)
            .await;

        let err = api(&server.uri())
            .create_installation_token(
                &jwt(),
                InstallationId(1),
                &InstallationTokenRequest {
                    repositories: vec![],
                    permissions: None,
                },
            )
            .await
            .expect_err("must fail");
        match err {
            GithubAppError::TokenRequestRejected(detail) => {
                assert!(detail.contains("permission not granted"), "got {detail}");
            }
            other => panic!("expected TokenRequestRejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn token_mint_401_is_app_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = api(&server.uri())
            .create_installation_token(
                &jwt(),
                InstallationId(1),
                &InstallationTokenRequest {
                    repositories: vec![],
                    permissions: None,
                },
            )
            .await
            .expect_err("must fail");
        assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
    }

    #[tokio::test]
    async fn token_mint_plain_403_is_app_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let err = api(&server.uri())
            .create_installation_token(
                &jwt(),
                InstallationId(1),
                &InstallationTokenRequest {
                    repositories: vec![],
                    permissions: None,
                },
            )
            .await
            .expect_err("must fail");
        assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
    }

    #[tokio::test]
    async fn token_mint_403_with_rate_headers_is_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("retry-after", "45"),
            )
            .mount(&server)
            .await;

        let err = api(&server.uri())
            .create_installation_token(
                &jwt(),
                InstallationId(1),
                &InstallationTokenRequest {
                    repositories: vec![],
                    permissions: None,
                },
            )
            .await
            .expect_err("must fail");
        match err {
            GithubAppError::RateLimited(secs) => assert_eq!(secs, 45),
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn installation_401_is_app_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = api(&server.uri())
            .installation_for_repo(&jwt(), "a", "b")
            .await
            .expect_err("must fail");
        assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
    }

    #[tokio::test]
    async fn installation_plain_403_is_app_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let err = api(&server.uri())
            .installation_for_repo(&jwt(), "a", "b")
            .await
            .expect_err("must fail");
        assert!(matches!(err, GithubAppError::AppAuth), "got {err:?}");
    }

    #[tokio::test]
    async fn installation_403_with_rate_headers_is_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("x-ratelimit-reset", "9999999999"),
            )
            .mount(&server)
            .await;

        let err = api(&server.uri())
            .installation_for_repo(&jwt(), "a", "b")
            .await
            .expect_err("must fail");
        assert!(matches!(err, GithubAppError::RateLimited(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn installation_token_debug_never_shows_token() {
        let token = InstallationToken {
            token: SecretString::from("ghs_supersecret".to_string()),
            expires_at: SystemTime::UNIX_EPOCH,
        };
        let debug = format!("{token:?}");
        assert!(!debug.contains("ghs_supersecret"), "token leaked");
        assert!(debug.contains("<redacted>"));
    }
}
