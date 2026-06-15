//! Error classification for the GitHub repo-creation flow.
//!
//! This is the "business logic" half of [`crate::goals::repo_create`]: given a
//! non-201 GitHub response, decide which domain-typed [`CreateRepoError`] it
//! maps to. It is split from the request/response skeleton so the proxy plumbing
//! stays small and the classification rules can evolve (and be unit-tested as
//! pure functions) independently.
//!
//! The driving distinction is between a *genuine* credential failure
//! ([`CreateRepoError::AuthFailed`]) and the actionable, connection-shaped
//! failures GitHub signals via headers/body on the repo-creation path:
//! a missing OAuth scope, a SAML-SSO authorization gate, or an org policy.

use super::repo_create::{truncate_error_body, CreateRepoError};
use crate::models::RepoRef;

/// OAuth-scope and SSO signal headers extracted from a GitHub error response.
///
/// GitHub annotates token-scope failures with `X-Accepted-OAuth-Scopes` (the
/// scopes the endpoint accepts) and `X-OAuth-Scopes` (the scopes the token
/// actually carries); SAML-SSO failures arrive with `X-GitHub-SSO`. These are
/// captured into owned values so classification can run after the response body
/// is consumed (`reqwest::Response::text` takes the response by value).
pub(super) struct ScopeSignals {
    /// Scopes GitHub says the endpoint requires (`X-Accepted-OAuth-Scopes`).
    accepted_scopes: Option<String>,
    /// Scopes the proxied token carries (`X-OAuth-Scopes`).
    token_scopes: Option<String>,
    /// Raw `X-GitHub-SSO` header value, present on an SSO-unauthorized 403.
    sso_header: Option<String>,
}

impl ScopeSignals {
    pub(super) fn from_headers(headers: &reqwest::header::HeaderMap) -> Self {
        let read = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        ScopeSignals {
            accepted_scopes: read("x-accepted-oauth-scopes"),
            token_scopes: read("x-oauth-scopes"),
            sso_header: read("x-github-sso"),
        }
    }

    /// True when the endpoint requires a repo-creation scope the token lacks.
    ///
    /// GitHub may list several acceptable scopes (comma-separated); the token
    /// satisfies the requirement if it carries any one of them. A missing or
    /// empty `X-OAuth-Scopes` is treated as "carries nothing".
    fn indicates_missing_repo_scope(&self) -> bool {
        let Some(accepted) = self.accepted_scopes.as_deref() else {
            return false;
        };
        let accepted: Vec<&str> = split_scopes(accepted);
        // Only a scope failure when a repo-creation scope is among the accepted
        // set — otherwise the 403 is unrelated to `repo`.
        let wants_repo_scope = accepted.iter().any(|s| *s == "repo" || *s == "public_repo");
        if !wants_repo_scope {
            return false;
        }
        let held: Vec<&str> = self
            .token_scopes
            .as_deref()
            .map(split_scopes)
            .unwrap_or_default();
        // Missing iff the token holds NONE of the accepted scopes.
        !accepted.iter().any(|a| held.contains(a))
    }

    /// Parse the authorization URL from an `X-GitHub-SSO` header value.
    ///
    /// The header looks like `required; url=https://github.com/orgs/<org>/sso?...`.
    fn sso_auth_url(&self) -> Option<String> {
        let header = self.sso_header.as_deref()?;
        header.split(';').find_map(|part| {
            let part = part.trim();
            part.strip_prefix("url=").map(str::to_string)
        })
    }

    /// Whether GitHub flagged this response with an SSO gate.
    fn has_sso(&self) -> bool {
        self.sso_header.is_some()
    }
}

/// Split a GitHub OAuth-scopes header value into trimmed, non-empty scopes.
fn split_scopes(value: &str) -> Vec<&str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Classify a GitHub 401/403 on the repo-creation path into the most specific
/// actionable variant: SSO first (unambiguous via header), then a missing-scope
/// signal (header- or body-driven), then an org-policy denial, finally a
/// genuine credential failure.
///
/// `org_login` carries the org context (the variants embed the org name so the
/// `From<CreateRepoError> for AppError` mapping can render an actionable hint
/// without extra context); `private` shapes the org-policy message.
pub(super) fn classify_forbidden(
    status: reqwest::StatusCode,
    org_login: Option<&str>,
    private: bool,
    signals: &ScopeSignals,
    error_body: &str,
) -> Result<RepoRef, CreateRepoError> {
    // SAML SSO: the org enforces SSO and this token is not authorized for it.
    if signals.has_sso() {
        if let Some(org) = org_login {
            let auth_url = signals.sso_auth_url();
            tracing::warn!(
                org = %org,
                has_auth_url = auth_url.is_some(),
                "github org SSO authorization required"
            );
            return Err(CreateRepoError::SsoUnauthorized {
                org: org.to_string(),
                auth_url,
            });
        }
    }

    // Missing OAuth scope (e.g. token lacks `repo`): header-driven or, as a
    // fallback, a body that names the missing scope.
    if signals.indicates_missing_repo_scope() || body_indicates_missing_scope(error_body) {
        tracing::warn!(
            status = %status,
            "github repo creation rejected: linked token missing repo scope"
        );
        return Err(CreateRepoError::InsufficientScope);
    }

    // Under an org, a forbidden response with policy-shaped wording is an
    // org-policy denial rather than a bad credential.
    if let Some(org) = org_login {
        if is_org_policy_body(error_body) {
            tracing::warn!(org = %org, status = %status, "org policy denied repo creation");
            return Err(CreateRepoError::OrgPolicy(org_policy_message(org, private)));
        }
    }

    // Genuine bad/expired credential — no scope, SSO, or org-policy signal.
    tracing::warn!(status = %status, "github repo creation auth failure");
    Err(CreateRepoError::AuthFailed(truncate_error_body(error_body)))
}

/// Classify a GitHub 422 under an org into either an org-policy/visibility
/// denial or a generic upstream error. (Name-taken is handled by the caller
/// before this is reached.)
pub(super) fn classify_org_unprocessable(
    status: reqwest::StatusCode,
    org: &str,
    private: bool,
    error_body: &str,
) -> CreateRepoError {
    if is_org_policy_body(error_body) {
        tracing::warn!(org = %org, "org policy/visibility denied repo creation (422)");
        CreateRepoError::OrgPolicy(org_policy_message(org, private))
    } else {
        CreateRepoError::Upstream {
            status: status.as_u16(),
            message: truncate_error_body(error_body),
        }
    }
}

/// True when a GitHub error body names a missing OAuth scope. Used only as a
/// fallback when the `X-*-OAuth-Scopes` headers are absent (some proxies strip
/// them); the header path is authoritative.
fn body_indicates_missing_scope(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    (lower.contains("scope") && (lower.contains("missing") || lower.contains("require")))
        || lower.contains("must have the following scope")
}

/// True when a GitHub forbidden/unprocessable body is shaped like an
/// org-policy / visibility denial rather than a credential failure.
fn is_org_policy_body(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("policy")
        || lower.contains("not allowed")
        || lower.contains("visibility")
        || lower.contains("members are not permitted")
        || lower.contains("organization has enabled")
        || lower.contains("must be approved")
}

/// Build the client-facing org-policy message, distinguishing the common
/// "non-owner cannot create a private repo" case from a general denial.
fn org_policy_message(org: &str, private: bool) -> String {
    if private {
        format!(
            "the `{org}` organization's policy prevents creating this repository \
             (creating a private repo may require an owner, or org membership with \
             repo-creation permission)"
        )
    } else {
        format!(
            "the `{org}` organization's policy prevents creating this repository \
             (org membership with repo-creation permission may be required, or the \
             org/OAuth-app must approve it)"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(*k, HeaderValue::from_str(v).expect("valid header value"));
        }
        map
    }

    #[test]
    fn missing_repo_scope_detected_when_token_lacks_accepted_scope() {
        let signals = ScopeSignals::from_headers(&headers(&[
            ("x-accepted-oauth-scopes", "repo"),
            ("x-oauth-scopes", "read:user, user:email"),
        ]));
        assert!(signals.indicates_missing_repo_scope());
    }

    #[test]
    fn missing_repo_scope_false_when_token_has_repo() {
        let signals = ScopeSignals::from_headers(&headers(&[
            ("x-accepted-oauth-scopes", "repo, public_repo"),
            ("x-oauth-scopes", "read:user, repo"),
        ]));
        assert!(!signals.indicates_missing_repo_scope());
    }

    #[test]
    fn missing_repo_scope_false_when_endpoint_does_not_want_repo() {
        // A 403 unrelated to repo scope (accepted set has no repo scope).
        let signals = ScopeSignals::from_headers(&headers(&[
            ("x-accepted-oauth-scopes", "admin:org"),
            ("x-oauth-scopes", "read:user"),
        ]));
        assert!(!signals.indicates_missing_repo_scope());
    }

    #[test]
    fn missing_repo_scope_false_when_no_headers() {
        let signals = ScopeSignals::from_headers(&HeaderMap::new());
        assert!(!signals.indicates_missing_repo_scope());
    }

    #[test]
    fn sso_auth_url_parsed_from_header() {
        let signals = ScopeSignals::from_headers(&headers(&[(
            "x-github-sso",
            "required; url=https://github.com/orgs/acme/sso?authorization_request=ABC",
        )]));
        assert!(signals.has_sso());
        assert_eq!(
            signals.sso_auth_url().as_deref(),
            Some("https://github.com/orgs/acme/sso?authorization_request=ABC")
        );
    }

    #[test]
    fn sso_auth_url_none_when_header_has_no_url() {
        let signals = ScopeSignals::from_headers(&headers(&[("x-github-sso", "partial-results")]));
        assert!(signals.has_sso());
        assert_eq!(signals.sso_auth_url(), None);
    }

    #[test]
    fn classify_forbidden_sso_takes_precedence() {
        let signals = ScopeSignals::from_headers(&headers(&[(
            "x-github-sso",
            "required; url=https://github.com/orgs/acme/sso",
        )]));
        let err = classify_forbidden(
            reqwest::StatusCode::FORBIDDEN,
            Some("acme"),
            false,
            &signals,
            "",
        )
        .expect_err("should classify as error");
        match err {
            CreateRepoError::SsoUnauthorized { org, auth_url } => {
                assert_eq!(org, "acme");
                assert_eq!(
                    auth_url.as_deref(),
                    Some("https://github.com/orgs/acme/sso")
                );
            }
            other => panic!("expected SsoUnauthorized, got {other:?}"),
        }
    }

    #[test]
    fn classify_forbidden_missing_scope_via_header() {
        let signals = ScopeSignals::from_headers(&headers(&[
            ("x-accepted-oauth-scopes", "repo"),
            ("x-oauth-scopes", "read:user"),
        ]));
        let err = classify_forbidden(reqwest::StatusCode::FORBIDDEN, None, false, &signals, "")
            .expect_err("should classify as error");
        assert!(
            matches!(err, CreateRepoError::InsufficientScope),
            "got {err:?}"
        );
    }

    #[test]
    fn classify_forbidden_missing_scope_via_body_fallback() {
        let signals = ScopeSignals::from_headers(&HeaderMap::new());
        let body = r#"{"message":"Token must have the following scopes: repo"}"#;
        let err = classify_forbidden(reqwest::StatusCode::FORBIDDEN, None, false, &signals, body)
            .expect_err("should classify as error");
        assert!(
            matches!(err, CreateRepoError::InsufficientScope),
            "got {err:?}"
        );
    }

    #[test]
    fn classify_forbidden_org_policy() {
        let signals = ScopeSignals::from_headers(&HeaderMap::new());
        let body = r#"{"message":"Organization members are not permitted to create repositories"}"#;
        let err = classify_forbidden(
            reqwest::StatusCode::FORBIDDEN,
            Some("acme"),
            false,
            &signals,
            body,
        )
        .expect_err("should classify as error");
        match err {
            CreateRepoError::OrgPolicy(msg) => assert!(msg.contains("acme"), "msg: {msg}"),
            other => panic!("expected OrgPolicy, got {other:?}"),
        }
    }

    #[test]
    fn classify_forbidden_generic_is_auth_failed() {
        let signals = ScopeSignals::from_headers(&HeaderMap::new());
        let err = classify_forbidden(
            reqwest::StatusCode::FORBIDDEN,
            None,
            false,
            &signals,
            r#"{"message":"Bad credentials"}"#,
        )
        .expect_err("should classify as error");
        assert!(matches!(err, CreateRepoError::AuthFailed(_)), "got {err:?}");
    }

    #[test]
    fn classify_org_unprocessable_visibility_denial() {
        let body = r#"{"message":"Visibility can't be private for this organization"}"#;
        let err = classify_org_unprocessable(
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            "acme",
            true,
            body,
        );
        match err {
            CreateRepoError::OrgPolicy(msg) => assert!(msg.contains("acme"), "msg: {msg}"),
            other => panic!("expected OrgPolicy, got {other:?}"),
        }
    }

    #[test]
    fn classify_org_unprocessable_unknown_is_upstream() {
        let body = r#"{"message":"Validation Failed","errors":[]}"#;
        let err = classify_org_unprocessable(
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            "acme",
            false,
            body,
        );
        assert!(
            matches!(err, CreateRepoError::Upstream { .. }),
            "got {err:?}"
        );
    }
}
