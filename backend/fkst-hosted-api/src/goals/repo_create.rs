//! GitHub repository creation via the NyxID credential-injection proxy.
//!
//! This module provides a single function [`create_repo`] that:
//! 1. Builds a GitHub "create repository" API body.
//! 2. Proxies the request through NyxID so that fkst-hosted never sees the
//!    user's GitHub credential.
//! 3. Parses the 201 Created response to extract `owner.login` + `name`.
//! 4. Classifies errors into domain-typed variants for clean AppError mapping.

use crate::models::RepoRef;
use crate::nyxid::{DelegatedToken, NyxIdClient};

/// Specification for a new GitHub repository.
#[derive(Debug, Clone)]
pub struct CreateRepoSpec {
    /// Repository name (GitHub-normalized).
    pub name: String,
    /// Whether the repo should be private.
    pub private: bool,
    /// Optional description.
    pub description: Option<String>,
    /// If `Some`, create under this org; otherwise create under the
    /// authenticated user's personal account.
    pub org_login: Option<String>,
}

/// Domain-typed errors from the repo-creation flow.
#[derive(Debug, thiserror::Error)]
pub enum CreateRepoError {
    /// The repository name is already taken (GitHub 422).
    #[error("repository name already exists: {0}")]
    NameTaken(String),
    /// Authentication or authorization failure (GitHub 401/403).
    #[error("github authorization failed: {0}")]
    AuthFailed(String),
    /// Rate limited by GitHub (429).
    #[error("github rate limited")]
    RateLimited,
    /// NyxID is unavailable or returned an error.
    #[error("nyxid unavailable: {0}")]
    NyxIdUnavailable(String),
    /// NyxID token exchange was rejected.
    #[error("token exchange rejected: {0}")]
    ExchangeRejected(String),
    /// An upstream HTTP error with status and message.
    #[error("upstream error {status}: {message}")]
    Upstream { status: u16, message: String },
    /// The GitHub response was missing expected fields.
    #[error("malformed response: {0}")]
    Malformed(String),
}

/// Create a GitHub repository by proxying through NyxID.
///
/// The `delegated_token` must be obtained via
/// [`NyxIdClient::exchange_token`] before calling this function.
///
/// Returns a [`RepoRef`] with `owner` and `name` extracted from the 201
/// Created response body.
pub async fn create_repo(
    nyxid: &NyxIdClient,
    delegated_token: &DelegatedToken,
    spec: &CreateRepoSpec,
) -> Result<RepoRef, CreateRepoError> {
    // Build the GitHub API path.
    let github_path = match &spec.org_login {
        Some(org) => format!("/orgs/{org}/repos"),
        None => "/user/repos".to_string(),
    };

    // Build the JSON body per GitHub REST API spec.
    let mut body = serde_json::json!({
        "name": spec.name,
        "private": spec.private,
        "auto_init": true,
    });
    if let Some(ref desc) = spec.description {
        body["description"] = serde_json::Value::String(desc.clone());
    }

    tracing::info!(
        repo_name = %spec.name,
        org = ?spec.org_login,
        private = spec.private,
        "creating repository via NyxID GitHub proxy"
    );

    let response = nyxid
        .proxy_github(
            delegated_token,
            reqwest::Method::POST,
            &github_path,
            Some(body),
        )
        .await
        .map_err(|e| CreateRepoError::NyxIdUnavailable(e.to_string()))?;

    let status = response.status();

    // Classify error responses before consuming the body.
    if status == reqwest::StatusCode::CREATED {
        // Success path: parse owner.login + name.
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| CreateRepoError::Malformed(format!("failed to parse 201 body: {e}")))?;

        let owner = body
            .get("owner")
            .and_then(|o| o.get("login"))
            .and_then(|l| l.as_str())
            .ok_or_else(|| CreateRepoError::Malformed("response missing owner.login".to_string()))?
            .to_string();

        let name = body
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or_else(|| CreateRepoError::Malformed("response missing name".to_string()))?
            .to_string();

        tracing::info!(
            repo_owner = %owner,
            repo_name = %name,
            "repository created successfully"
        );

        return Ok(RepoRef { owner, name });
    }

    // Error classification.
    let error_body = response.text().await.unwrap_or_default();

    match status {
        reqwest::StatusCode::UNPROCESSABLE_ENTITY => {
            // GitHub returns 422 for name-taken and various validation errors.
            // Check if the body mentions "already exists".
            if error_body.contains("already exists") {
                Err(CreateRepoError::NameTaken(spec.name.clone()))
            } else {
                Err(CreateRepoError::Upstream {
                    status: status.as_u16(),
                    message: truncate_error_body(&error_body),
                })
            }
        }
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            tracing::warn!(
                status = %status,
                "github repo creation auth failure"
            );
            Err(CreateRepoError::AuthFailed(truncate_error_body(
                &error_body,
            )))
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            tracing::warn!("github rate limited during repo creation");
            Err(CreateRepoError::RateLimited)
        }
        _ => {
            tracing::error!(
                status = %status,
                "unexpected github repo creation error"
            );
            Err(CreateRepoError::Upstream {
                status: status.as_u16(),
                message: truncate_error_body(&error_body),
            })
        }
    }
}

/// Truncate error body to a safe length to avoid logging enormous payloads.
fn truncate_error_body(body: &str) -> String {
    const MAX_LEN: usize = 300;
    if body.len() <= MAX_LEN {
        body.to_string()
    } else {
        format!("{}...", &body[..MAX_LEN])
    }
}

/// Map NyxID errors to CreateRepoError for use in the trigger handler.
impl From<crate::nyxid::NyxIdError> for CreateRepoError {
    fn from(err: crate::nyxid::NyxIdError) -> Self {
        use crate::nyxid::NyxIdError;
        match err {
            NyxIdError::ExchangeRejected(detail) => CreateRepoError::ExchangeRejected(detail),
            other => CreateRepoError::NyxIdUnavailable(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_error_body_short_is_passthrough() {
        let msg = "short error";
        assert_eq!(truncate_error_body(msg), msg);
    }

    #[test]
    fn truncate_error_body_long_is_truncated() {
        let long = "x".repeat(500);
        let truncated = truncate_error_body(&long);
        assert!(truncated.len() <= 303, "should be ~300 + '...'");
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn create_repo_spec_fields_preserved() {
        let spec = CreateRepoSpec {
            name: "my-repo".to_string(),
            private: true,
            description: Some("a test repo".to_string()),
            org_login: Some("acme".to_string()),
        };
        assert_eq!(spec.name, "my-repo");
        assert!(spec.private);
        assert_eq!(spec.description.as_deref(), Some("a test repo"));
        assert_eq!(spec.org_login.as_deref(), Some("acme"));
    }

    #[tokio::test]
    async fn create_repo_personal_uses_user_repos_path() {
        let server = wiremock::MockServer::start().await;

        // Wire up a NyxID client pointed at the mock.
        let client = crate::nyxid::NyxIdClient::new(
            &server.uri(),
            "api-github",
            "test_client".to_string(),
            secrecy::SecretString::from("test_secret".to_string()),
            std::time::Duration::from_secs(30),
        )
        .expect("client build");

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path(format!(
                "/api/v1/proxy/{}{}",
                crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG,
                "/user/repos"
            )))
            .respond_with(
                wiremock::ResponseTemplate::new(201).set_body_json(serde_json::json!({
                    "owner": { "login": "testuser" },
                    "name": "my-repo"
                })),
            )
            .mount(&server)
            .await;

        let delegated = DelegatedToken {
            access_token: secrecy::SecretString::from("delegated_tok".to_string()),
            expires_in: 300,
        };
        let spec = CreateRepoSpec {
            name: "my-repo".to_string(),
            private: false,
            description: None,
            org_login: None,
        };

        let result = create_repo(&client, &delegated, &spec).await;
        let repo = result.expect("create should succeed");
        assert_eq!(repo.owner, "testuser");
        assert_eq!(repo.name, "my-repo");
    }

    #[tokio::test]
    async fn create_repo_org_uses_orgs_path() {
        let server = wiremock::MockServer::start().await;

        let client = crate::nyxid::NyxIdClient::new(
            &server.uri(),
            "api-github",
            "test_client".to_string(),
            secrecy::SecretString::from("test_secret".to_string()),
            std::time::Duration::from_secs(30),
        )
        .expect("client build");

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path(format!(
                "/api/v1/proxy/{}{}",
                crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG,
                "/orgs/acme/repos"
            )))
            .respond_with(
                wiremock::ResponseTemplate::new(201).set_body_json(serde_json::json!({
                    "owner": { "login": "acme" },
                    "name": "org-repo"
                })),
            )
            .mount(&server)
            .await;

        let delegated = DelegatedToken {
            access_token: secrecy::SecretString::from("delegated_tok".to_string()),
            expires_in: 300,
        };
        let spec = CreateRepoSpec {
            name: "org-repo".to_string(),
            private: true,
            description: Some("org repo".to_string()),
            org_login: Some("acme".to_string()),
        };

        let result = create_repo(&client, &delegated, &spec).await;
        let repo = result.expect("create should succeed");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "org-repo");
    }

    #[tokio::test]
    async fn create_repo_422_name_taken_returns_name_taken() {
        let server = wiremock::MockServer::start().await;

        let client = crate::nyxid::NyxIdClient::new(
            &server.uri(),
            "api-github",
            "test_client".to_string(),
            secrecy::SecretString::from("test_secret".to_string()),
            std::time::Duration::from_secs(30),
        )
        .expect("client build");

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(422).set_body_string(
                    r#"{"message":"Validation Failed","errors":[{"message":"name already exists on this account"}]}"#,
                ),
            )
            .mount(&server)
            .await;

        let delegated = DelegatedToken {
            access_token: secrecy::SecretString::from("delegated_tok".to_string()),
            expires_in: 300,
        };
        let spec = CreateRepoSpec {
            name: "taken-repo".to_string(),
            private: false,
            description: None,
            org_login: None,
        };

        let err = create_repo(&client, &delegated, &spec)
            .await
            .expect_err("should fail");
        assert!(
            matches!(err, CreateRepoError::NameTaken(ref n) if n == "taken-repo"),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn create_repo_403_returns_auth_failed() {
        let server = wiremock::MockServer::start().await;

        let client = crate::nyxid::NyxIdClient::new(
            &server.uri(),
            "api-github",
            "test_client".to_string(),
            secrecy::SecretString::from("test_secret".to_string()),
            std::time::Duration::from_secs(30),
        )
        .expect("client build");

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(403).set_body_string(r#"{"message":"Forbidden"}"#),
            )
            .mount(&server)
            .await;

        let delegated = DelegatedToken {
            access_token: secrecy::SecretString::from("delegated_tok".to_string()),
            expires_in: 300,
        };
        let spec = CreateRepoSpec {
            name: "forbidden-repo".to_string(),
            private: false,
            description: None,
            org_login: None,
        };

        let err = create_repo(&client, &delegated, &spec)
            .await
            .expect_err("should fail");
        assert!(matches!(err, CreateRepoError::AuthFailed(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn create_repo_429_returns_rate_limited() {
        let server = wiremock::MockServer::start().await;

        let client = crate::nyxid::NyxIdClient::new(
            &server.uri(),
            "api-github",
            "test_client".to_string(),
            secrecy::SecretString::from("test_secret".to_string()),
            std::time::Duration::from_secs(30),
        )
        .expect("client build");

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(429)
                    .set_body_string(r#"{"message":"Rate limit exceeded"}"#),
            )
            .mount(&server)
            .await;

        let delegated = DelegatedToken {
            access_token: secrecy::SecretString::from("delegated_tok".to_string()),
            expires_in: 300,
        };
        let spec = CreateRepoSpec {
            name: "rate-limited-repo".to_string(),
            private: false,
            description: None,
            org_login: None,
        };

        let err = create_repo(&client, &delegated, &spec)
            .await
            .expect_err("should fail");
        assert!(matches!(err, CreateRepoError::RateLimited), "got {err:?}");
    }

    #[tokio::test]
    async fn create_repo_201_missing_owner_login_returns_malformed() {
        let server = wiremock::MockServer::start().await;

        let client = crate::nyxid::NyxIdClient::new(
            &server.uri(),
            "api-github",
            "test_client".to_string(),
            secrecy::SecretString::from("test_secret".to_string()),
            std::time::Duration::from_secs(30),
        )
        .expect("client build");

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(201).set_body_json(serde_json::json!({
                    "name": "my-repo"
                    // Missing owner.login
                })),
            )
            .mount(&server)
            .await;

        let delegated = DelegatedToken {
            access_token: secrecy::SecretString::from("delegated_tok".to_string()),
            expires_in: 300,
        };
        let spec = CreateRepoSpec {
            name: "my-repo".to_string(),
            private: false,
            description: None,
            org_login: None,
        };

        let err = create_repo(&client, &delegated, &spec)
            .await
            .expect_err("should fail");
        assert!(matches!(err, CreateRepoError::Malformed(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn create_repo_nyxid_unavailable_returns_error() {
        // Point at nothing to trigger transport error.
        let client = crate::nyxid::NyxIdClient::new(
            "http://127.0.0.1:1",
            "api-github",
            "test_client".to_string(),
            secrecy::SecretString::from("test_secret".to_string()),
            std::time::Duration::from_secs(30),
        )
        .expect("client build");

        let delegated = DelegatedToken {
            access_token: secrecy::SecretString::from("delegated_tok".to_string()),
            expires_in: 300,
        };
        let spec = CreateRepoSpec {
            name: "unreachable-repo".to_string(),
            private: false,
            description: None,
            org_login: None,
        };

        let err = create_repo(&client, &delegated, &spec)
            .await
            .expect_err("should fail");
        assert!(
            matches!(err, CreateRepoError::NyxIdUnavailable(_)),
            "got {err:?}"
        );
    }
}
