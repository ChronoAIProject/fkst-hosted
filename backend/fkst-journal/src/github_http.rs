//! Shared HTTP plumbing for the GitHub journal client: credential-free error
//! reduction, rate-limit header parsing, and status classification.
//!
//! Kept separate from [`crate::github`] (the Contents-API record
//! path) and [`crate::comments`] (the issue-comment mirror) so both
//! reuse the same auth/rate-limit disambiguation, and so neither file grows
//! unwieldy.

use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::header::HeaderMap;
use reqwest::StatusCode;

use crate::JournalError;

/// Default GitHub REST API base (overridable for tests / GHE).
pub const DEFAULT_API_BASE: &str = "https://api.github.com";

/// Request timeout for every GitHub call.
pub(crate) const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Reduce a reqwest error to a credential-free string (reqwest never embeds
/// request headers in its messages; this keeps that invariant explicit).
///
/// Crate-visible so the sibling [`crate::comments`] module reuses the
/// same credential-free reduction.
pub(crate) fn http_err(context: &str, err: reqwest::Error) -> JournalError {
    JournalError::Http(format!("{context}: {err}"))
}

/// Seconds until the rate-limit reset, from `retry-after` (delta seconds) or
/// `x-ratelimit-reset` (epoch seconds). Defaults to 60s when unparseable.
///
/// Public so the control-plane's github-hub upstream classifier (a different
/// crate after the #151 extraction) reuses the same header parsing as the
/// journal client.
pub fn reset_seconds(headers: &HeaderMap) -> u64 {
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

/// True when a 403 carries rate-limit evidence (exhausted quota or an
/// explicit retry hint) rather than an auth refusal.
///
/// Public so the control-plane's github-hub upstream classifier (a different
/// crate after the #151 extraction) reuses the same rate-limit detection as the
/// journal client.
pub fn is_rate_limited(headers: &HeaderMap) -> bool {
    let remaining_zero = headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "0")
        .unwrap_or(false);
    remaining_zero || headers.contains_key("retry-after")
}

/// Map auth/rate-limit statuses to their dedicated variants; `None` for
/// everything else.
///
/// Crate-visible so the sibling [`crate::comments`] module reuses the
/// same status classification.
pub(crate) fn classify_status(status: StatusCode, headers: &HeaderMap) -> Option<JournalError> {
    match status {
        StatusCode::UNAUTHORIZED => Some(JournalError::GithubAuth),
        StatusCode::FORBIDDEN => {
            if is_rate_limited(headers) {
                Some(JournalError::GithubRateLimited(reset_seconds(headers)))
            } else {
                Some(JournalError::GithubAuth)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::github::ProgressRepo;
    use crate::model::ProgressRecord;
    use crate::JournalError;

    const TOKEN: &str = "ghp_supersecret_token_value_1234567890";

    fn repo(server_uri: &str, with_token: bool) -> ProgressRepo {
        let token = with_token.then(|| SecretString::from(TOKEN.to_string()));
        ProgressRepo::new(server_uri, "owner/name", "main", token).expect("client")
    }

    fn sample_record() -> ProgressRecord {
        ProgressRecord::new("rk", "demo", "fp", "2026-06-10T00:00:00Z".to_string())
    }

    // ---- 401/403 disambiguation -----------------------------------------------

    #[tokio::test]
    async fn forbidden_with_rate_headers_is_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("retry-after", "30"),
            )
            .mount(&server)
            .await;
        let err = repo(&server.uri(), true)
            .get_record("j.json")
            .await
            .expect_err("403 must fail");
        match err {
            JournalError::GithubRateLimited(secs) => assert_eq!(secs, 30),
            other => panic!("expected GithubRateLimited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forbidden_without_rate_headers_and_401_are_auth_failures() {
        for status in [401, 403] {
            let server = MockServer::start().await;
            Mock::given(method("PUT"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let err = repo(&server.uri(), true)
                .put_record("j.json", &sample_record(), None, "journal")
                .await
                .expect_err("auth must fail");
            assert!(
                matches!(err, JournalError::GithubAuth),
                "status {status}: got {err:?}"
            );
        }
    }

    // ---- secret hygiene -------------------------------------------------------------

    #[tokio::test]
    async fn no_error_variant_or_debug_ever_contains_the_token() {
        // Drive a real failing request so reqwest-derived errors are covered.
        let unreachable = ProgressRepo::new(
            "http://127.0.0.1:1",
            "owner/name",
            "main",
            Some(SecretString::from(TOKEN.to_string())),
        )
        .expect("client");
        let live_err = unreachable
            .get_record("j.json")
            .await
            .expect_err("unreachable");

        let errors: Vec<JournalError> = vec![
            live_err,
            JournalError::CasExhausted(5),
            JournalError::CasConflict,
            JournalError::RemoteMissing,
            JournalError::Fenced { got: 1, known: 2 },
            JournalError::GithubAuth,
            JournalError::GithubRateLimited(30),
            JournalError::UnsupportedSchema("x@2".to_string()),
            JournalError::Http("contents PUT status 500".to_string()),
            JournalError::Other(anyhow::anyhow!("wrapped context")),
        ];
        for err in &errors {
            let display = format!("{err}");
            let debug = format!("{err:?}");
            assert!(!display.contains(TOKEN), "Display leaked: {display}");
            assert!(!debug.contains(TOKEN), "Debug leaked: {debug}");
        }

        let repo_debug = format!("{unreachable:?}");
        assert!(!repo_debug.contains(TOKEN), "repo Debug leaked");
        assert!(repo_debug.contains("<redacted>"));
    }
}
