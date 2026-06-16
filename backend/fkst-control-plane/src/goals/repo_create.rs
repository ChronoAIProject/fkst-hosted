//! GitHub repository creation via the NyxID credential-injection proxy.
//!
//! This module provides a single function [`create_repo`] that:
//! 1. Builds a GitHub "create repository" API body.
//! 2. Proxies the request through NyxID so that fkst-hosted never sees the
//!    user's GitHub credential.
//! 3. Parses the 201 Created response to extract `owner.login` + `name`.
//! 4. Classifies errors into domain-typed variants for clean AppError mapping.

use super::repo_create_classify as classify;
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
    /// Authentication or authorization failure (GitHub 401/403) with no
    /// scope / SSO / org-policy signal — i.e. a genuinely bad or expired
    /// credential.
    #[error("github authorization failed: {0}")]
    AuthFailed(String),
    /// The linked GitHub token is missing the OAuth scope required to create
    /// repositories (`repo` / `public_repo`). GitHub signals this with a 403
    /// whose `X-Accepted-OAuth-Scopes` requires a scope absent from
    /// `X-OAuth-Scopes`, or a body mentioning the missing scope. This is a
    /// connection-configuration problem, not a transient auth failure, so it
    /// carries its own actionable variant rather than collapsing into
    /// [`CreateRepoError::AuthFailed`].
    #[error("github token missing required scope")]
    InsufficientScope,
    /// The org enforces SAML SSO and the proxied OAuth token is not
    /// SSO-authorized for it (GitHub 403 carrying an `X-GitHub-SSO` header).
    /// The optional `auth_url` is the authorization URL parsed from that
    /// header (it expires ~1h), forwarded to the user so they can authorize.
    #[error("github org requires SAML SSO authorization")]
    SsoUnauthorized {
        org: String,
        auth_url: Option<String>,
    },
    /// The org's policy forbids creating this repository (members/Apps cannot
    /// create repos, the OAuth app is not org-approved, or the requested
    /// visibility is disallowed for a non-owner — GitHub 403/422). Distinct
    /// from [`CreateRepoError::InsufficientScope`]: the token is fine, the
    /// org rules reject the operation. Carries an org-context message.
    #[error("github org policy blocks repository creation: {0}")]
    OrgPolicy(String),
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

    // Error classification. Capture the scope/SSO signal headers BEFORE the
    // body is consumed — `response.text()` takes the response by value, after
    // which `headers()` is no longer reachable. Classification rules live in
    // the sibling `repo_create_classify` module.
    let signals = classify::ScopeSignals::from_headers(response.headers());
    let error_body = response.text().await.unwrap_or_default();

    match status {
        reqwest::StatusCode::UNPROCESSABLE_ENTITY => {
            // GitHub returns 422 for name-taken, validation errors, and — under
            // an org — visibility/policy denials (e.g. a non-owner requesting a
            // private repo). Disambiguate before falling back to Upstream.
            if error_body.contains("already exists") {
                Err(CreateRepoError::NameTaken(spec.name.clone()))
            } else if let Some(org) = spec.org_login.as_deref() {
                Err(classify::classify_org_unprocessable(
                    status,
                    org,
                    spec.private,
                    &error_body,
                ))
            } else {
                Err(CreateRepoError::Upstream {
                    status: status.as_u16(),
                    message: truncate_error_body(&error_body),
                })
            }
        }
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            classify::classify_forbidden(
                status,
                spec.org_login.as_deref(),
                spec.private,
                &signals,
                &error_body,
            )
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
///
/// `pub(super)` so the sibling classification module can reuse it when building
/// the truncated `AuthFailed` / `Upstream` messages.
pub(super) fn truncate_error_body(body: &str) -> String {
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a NyxID client pointed at `uri`. Centralized so the wiremock tests
    /// stay focused on the GitHub response shape, not client boilerplate.
    fn test_client(uri: &str) -> crate::nyxid::NyxIdClient {
        crate::nyxid::NyxIdClient::new(
            uri,
            "api-github",
            "test_client".to_string(),
            secrecy::SecretString::from("test_secret".to_string()),
            std::time::Duration::from_secs(30),
        )
        .expect("client build")
    }

    fn delegated() -> DelegatedToken {
        DelegatedToken {
            access_token: secrecy::SecretString::from("delegated_tok".to_string()),
            expires_in: 300,
        }
    }

    /// A personal-account spec (no org) with the given name.
    fn personal_spec(name: &str) -> CreateRepoSpec {
        CreateRepoSpec {
            name: name.to_string(),
            private: false,
            description: None,
            org_login: None,
        }
    }

    /// An org spec for `org`, with the given name/visibility.
    fn org_spec(name: &str, org: &str, private: bool) -> CreateRepoSpec {
        CreateRepoSpec {
            name: name.to_string(),
            private,
            description: None,
            org_login: Some(org.to_string()),
        }
    }

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
        let server = MockServer::start().await;
        let client = test_client(&server.uri());

        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/proxy/{}{}",
                crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG,
                "/user/repos"
            )))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "owner": { "login": "testuser" },
                "name": "my-repo"
            })))
            .mount(&server)
            .await;

        let repo = create_repo(&client, &delegated(), &personal_spec("my-repo"))
            .await
            .expect("create should succeed");
        assert_eq!(repo.owner, "testuser");
        assert_eq!(repo.name, "my-repo");
    }

    #[tokio::test]
    async fn create_repo_org_uses_orgs_path() {
        let server = MockServer::start().await;
        let client = test_client(&server.uri());

        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/proxy/{}{}",
                crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG,
                "/orgs/acme/repos"
            )))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "owner": { "login": "acme" },
                "name": "org-repo"
            })))
            .mount(&server)
            .await;

        let repo = create_repo(&client, &delegated(), &org_spec("org-repo", "acme", true))
            .await
            .expect("create should succeed");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "org-repo");
    }

    #[tokio::test]
    async fn create_repo_422_name_taken_returns_name_taken() {
        let server = MockServer::start().await;
        let client = test_client(&server.uri());

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(422).set_body_string(
                r#"{"message":"Validation Failed","errors":[{"message":"name already exists on this account"}]}"#,
            ))
            .mount(&server)
            .await;

        let err = create_repo(&client, &delegated(), &personal_spec("taken-repo"))
            .await
            .expect_err("should fail");
        assert!(
            matches!(err, CreateRepoError::NameTaken(ref n) if n == "taken-repo"),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn create_repo_403_generic_returns_auth_failed() {
        // A 403 with no scope/SSO/policy signal stays a genuine auth failure.
        let server = MockServer::start().await;
        let client = test_client(&server.uri());

        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(403).set_body_string(r#"{"message":"Bad credentials"}"#),
            )
            .mount(&server)
            .await;

        let err = create_repo(&client, &delegated(), &personal_spec("forbidden-repo"))
            .await
            .expect_err("should fail");
        assert!(matches!(err, CreateRepoError::AuthFailed(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn create_repo_403_missing_scope_returns_insufficient_scope() {
        // GitHub signals a missing `repo` scope via the OAuth-scopes headers:
        // the endpoint accepts `repo` but the token only holds read scopes.
        let server = MockServer::start().await;
        let client = test_client(&server.uri());

        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("X-Accepted-OAuth-Scopes", "repo")
                    .insert_header("X-OAuth-Scopes", "read:user, user:email")
                    .set_body_string(r#"{"message":"Forbidden"}"#),
            )
            .mount(&server)
            .await;

        let err = create_repo(&client, &delegated(), &personal_spec("scope-repo"))
            .await
            .expect_err("should fail");
        assert!(
            matches!(err, CreateRepoError::InsufficientScope),
            "got {err:?}"
        );

        // And the handler renders it as a 422 carrying the actionable hint.
        let app_err: crate::error::AppError = err.into();
        assert!(
            matches!(app_err, crate::error::AppError::Unprocessable(ref m) if m.contains("`repo`")),
            "got {app_err:?}"
        );
    }

    #[tokio::test]
    async fn create_repo_403_sso_returns_sso_unauthorized_with_url() {
        // An org 403 carrying X-GitHub-SSO with an authorization URL.
        let server = MockServer::start().await;
        let client = test_client(&server.uri());
        let auth_url = "https://github.com/orgs/acme/sso?authorization_request=ABC123";

        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("X-GitHub-SSO", format!("required; url={auth_url}").as_str())
                    .set_body_string(
                        r#"{"message":"Resource protected by organization SAML enforcement."}"#,
                    ),
            )
            .mount(&server)
            .await;

        let err = create_repo(&client, &delegated(), &org_spec("sso-repo", "acme", false))
            .await
            .expect_err("should fail");
        match &err {
            CreateRepoError::SsoUnauthorized { org, auth_url: url } => {
                assert_eq!(org, "acme");
                assert_eq!(url.as_deref(), Some(auth_url));
            }
            other => panic!("expected SsoUnauthorized, got {other:?}"),
        }

        // The 422 surfaces the org name and the auth URL.
        let app_err: crate::error::AppError = err.into();
        match app_err {
            crate::error::AppError::Unprocessable(msg) => {
                assert!(msg.contains("acme"), "msg: {msg}");
                assert!(msg.contains(auth_url), "msg should surface auth url: {msg}");
            }
            other => panic!("expected Unprocessable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_repo_403_org_policy_returns_org_policy() {
        let server = MockServer::start().await;
        let client = test_client(&server.uri());

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403).set_body_string(
                r#"{"message":"Organization members are not permitted to create repositories"}"#,
            ))
            .mount(&server)
            .await;

        let err = create_repo(
            &client,
            &delegated(),
            &org_spec("policy-repo", "acme", false),
        )
        .await
        .expect_err("should fail");
        match err {
            CreateRepoError::OrgPolicy(ref msg) => assert!(msg.contains("acme"), "msg: {msg}"),
            other => panic!("expected OrgPolicy, got {other:?}"),
        }

        let app_err: crate::error::AppError = err.into();
        assert!(
            matches!(app_err, crate::error::AppError::Unprocessable(_)),
            "got {app_err:?}"
        );
    }

    #[tokio::test]
    async fn create_repo_422_org_visibility_denied_returns_org_policy() {
        // A non-owner requesting a private org repo gets a 422 visibility denial.
        let server = MockServer::start().await;
        let client = test_client(&server.uri());

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(422).set_body_string(
                r#"{"message":"Visibility can't be private for this organization"}"#,
            ))
            .mount(&server)
            .await;

        let err = create_repo(&client, &delegated(), &org_spec("vis-repo", "acme", true))
            .await
            .expect_err("should fail");
        assert!(matches!(err, CreateRepoError::OrgPolicy(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn create_repo_429_returns_rate_limited() {
        let server = MockServer::start().await;
        let client = test_client(&server.uri());

        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(429).set_body_string(r#"{"message":"Rate limit exceeded"}"#),
            )
            .mount(&server)
            .await;

        let err = create_repo(&client, &delegated(), &personal_spec("rate-limited-repo"))
            .await
            .expect_err("should fail");
        assert!(matches!(err, CreateRepoError::RateLimited), "got {err:?}");
    }

    #[tokio::test]
    async fn create_repo_201_missing_owner_login_returns_malformed() {
        let server = MockServer::start().await;
        let client = test_client(&server.uri());

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "name": "my-repo"
                // Missing owner.login
            })))
            .mount(&server)
            .await;

        let err = create_repo(&client, &delegated(), &personal_spec("my-repo"))
            .await
            .expect_err("should fail");
        assert!(matches!(err, CreateRepoError::Malformed(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn create_repo_nyxid_unavailable_returns_error() {
        // Point at nothing to trigger a transport error.
        let client = test_client("http://127.0.0.1:1");

        let err = create_repo(&client, &delegated(), &personal_spec("unreachable-repo"))
            .await
            .expect_err("should fail");
        assert!(
            matches!(err, CreateRepoError::NyxIdUnavailable(_)),
            "got {err:?}"
        );
    }
}
