//! The hub engine: the upstream-status classifier, account resolution, the
//! single-target issue operations, and the entry point to the multi-account
//! aggregate fan-out (implemented in [`crate::github_hub::fanout`]).
//!
//! Generic over `P: GithubProxy` so the engine is decoupled from NyxID; the
//! production wiring injects [`crate::github_hub::NyxIdGithubProxy`], tests a
//! wiremock-backed one. Issue bodies are NEVER logged here — only counts/sizes.

use reqwest::header::HeaderMap;
use reqwest::Method;

use crate::error::AppError;
use crate::github_hub::types::{
    comment_view, issue_view, AccountError, AccountView, BodyMode, CommentView, IssueView,
    RateLimitView,
};
use crate::github_hub::{GithubProxy, ProxyResponse};

/// Classification of a GitHub upstream status. One source of truth shared by
/// the single-target and fan-out paths, reusing the journal client's
/// rate-limit detection so both layers agree on what "rate limited" means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upstream {
    /// 404 — the resource does not exist.
    NotFound,
    /// 401, or 403 without rate-limit evidence — authorization failed.
    Auth,
    /// 422 — semantic rejection; carries GitHub's first error message.
    Unprocessable(String),
    /// 403/429 with rate-limit evidence — carries the retry delay (seconds).
    RateLimited(u64),
    /// Any other non-success status — an upstream provider error.
    Upstream,
}

/// Classify a GitHub upstream status + headers + body into an [`Upstream`].
///
/// Reuses [`crate::journal::github`]'s `is_rate_limited` / `reset_seconds` so a
/// 403/429 is disambiguated identically to the journal client. The `body` is
/// consulted only for 422 (to surface GitHub's first validation message).
pub fn classify(status: u16, headers: &HeaderMap, body: &[u8]) -> Upstream {
    use crate::journal::github::{is_rate_limited, reset_seconds};

    match status {
        404 => Upstream::NotFound,
        401 => Upstream::Auth,
        429 => Upstream::RateLimited(reset_seconds(headers)),
        403 => {
            if is_rate_limited(headers) {
                Upstream::RateLimited(reset_seconds(headers))
            } else {
                Upstream::Auth
            }
        }
        422 => Upstream::Unprocessable(first_error_message(body)),
        _ => Upstream::Upstream,
    }
}

/// Surface GitHub's first error message from a 422 body
/// (`{"message": "...", "errors": [{"message": "..."}]}`), falling back to the
/// top-level `message`, then to a terse default. Never echoes the raw body.
fn first_error_message(body: &[u8]) -> String {
    let parsed: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return "github rejected the request".to_string(),
    };
    if let Some(msg) = parsed
        .get("errors")
        .and_then(|e| e.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("message"))
        .and_then(|m| m.as_str())
    {
        return msg.to_string();
    }
    parsed
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("github rejected the request")
        .to_string()
}

/// Map an [`Upstream`] onto the unified [`AppError`] for SINGLE-TARGET ops.
pub fn upstream_to_app_error(upstream: Upstream) -> AppError {
    match upstream {
        Upstream::NotFound => AppError::NotFound("github resource not found".to_string()),
        Upstream::Auth => AppError::Forbidden("github authorization failed".to_string()),
        Upstream::Unprocessable(message) => AppError::Unprocessable(message),
        Upstream::RateLimited(secs) => AppError::RateLimited {
            message: "github rate limited; retry later".to_string(),
            retry_after_secs: secs,
        },
        Upstream::Upstream => AppError::Upstream("github returned an unexpected error".to_string()),
    }
}

/// Map an [`Upstream`] onto a per-account [`AccountError`] for the FAN-OUT path.
pub fn upstream_to_account_error(upstream: Upstream) -> AccountError {
    match upstream {
        Upstream::NotFound => AccountError {
            kind: "upstream".to_string(),
            message: "github resource not found".to_string(),
            retry_after_secs: None,
        },
        Upstream::Auth => AccountError {
            kind: "auth".to_string(),
            message: "github authorization failed".to_string(),
            retry_after_secs: None,
        },
        Upstream::Unprocessable(message) => AccountError {
            kind: "upstream".to_string(),
            message,
            retry_after_secs: None,
        },
        Upstream::RateLimited(secs) => AccountError {
            kind: "rate_limited".to_string(),
            message: "github rate limited; retry later".to_string(),
            retry_after_secs: Some(secs),
        },
        Upstream::Upstream => AccountError {
            kind: "upstream".to_string(),
            message: "github returned an unexpected error".to_string(),
            retry_after_secs: None,
        },
    }
}

/// Extract a [`RateLimitView`] from response headers when both
/// `x-ratelimit-remaining` and `x-ratelimit-reset` are present and numeric.
pub fn rate_limit_view(headers: &HeaderMap) -> Option<RateLimitView> {
    let remaining = header_i64(headers, "x-ratelimit-remaining")?;
    let reset_epoch = header_i64(headers, "x-ratelimit-reset")?;
    Some(RateLimitView {
        remaining,
        reset_epoch,
    })
}

fn header_i64(headers: &HeaderMap, name: &str) -> Option<i64> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
}

/// Parse the `Link` header for a `rel="next"` relation (RFC 5988) to decide
/// whether more pages exist.
pub fn has_next_page(headers: &HeaderMap) -> bool {
    headers
        .get(reqwest::header::LINK)
        .and_then(|v| v.to_str().ok())
        .map(link_has_next)
        .unwrap_or(false)
}

/// True when a `Link` header value contains a `rel="next"` relation.
fn link_has_next(link: &str) -> bool {
    link.split(',').any(|segment| {
        segment
            .split(';')
            .any(|param| param.trim() == "rel=\"next\"")
    })
}

/// List the caller's linked GitHub accounts.
pub async fn list_accounts(proxy: &dyn GithubProxy) -> Result<Vec<AccountView>, AppError> {
    let connections = proxy.accounts().await?;
    tracing::info!(account_count = connections.len(), "listed github accounts");
    Ok(connections.into_iter().map(AccountView::from).collect())
}

/// Resolve the connection to target for a single-target op.
///
/// - exactly one linked account → that account (the `account` hint, if given,
///   must match it case-insensitively or it is a 422);
/// - several accounts + a matching `account` login → that account;
/// - several accounts + no `account` → 422 (the caller must disambiguate);
/// - an `account` that matches no linked login → 422.
pub async fn resolve_account(
    proxy: &dyn GithubProxy,
    account: Option<&str>,
) -> Result<crate::nyxid::GithubConnection, AppError> {
    let connections = proxy.accounts().await?;
    if connections.is_empty() {
        return Err(AppError::Unprocessable(
            "no GitHub accounts are linked".to_string(),
        ));
    }
    match account {
        Some(login) => connections
            .into_iter()
            .find(|c| c.login.eq_ignore_ascii_case(login))
            .ok_or_else(|| {
                AppError::Unprocessable(format!("no linked GitHub account named {login}"))
            }),
        None => {
            if connections.len() == 1 {
                Ok(connections.into_iter().next().expect("len checked"))
            } else {
                Err(AppError::Unprocessable(
                    "multiple GitHub accounts linked; specify account".to_string(),
                ))
            }
        }
    }
}

/// A validated `owner/repo` reference for a single-target op. Both segments are
/// non-empty; this keeps malformed path params out of the upstream URL.
pub struct RepoRef<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
}

impl<'a> RepoRef<'a> {
    /// Validate and build a repo reference, rejecting empty segments (400).
    pub fn new(owner: &'a str, repo: &'a str) -> Result<Self, AppError> {
        if owner.trim().is_empty() || repo.trim().is_empty() {
            return Err(AppError::Validation(
                "owner and repo must be non-empty".to_string(),
            ));
        }
        Ok(Self { owner, repo })
    }

    /// `"owner/repo"` for response attribution.
    fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

/// Run a single-target proxied request, classifying any non-2xx onto AppError,
/// and return the parsed success body as JSON.
async fn single_target(
    proxy: &dyn GithubProxy,
    selector: &str,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<ProxyResponse, AppError> {
    let response = proxy.request(selector, method, path, body).await?;
    if (200..300).contains(&response.status) {
        return Ok(response);
    }
    Err(upstream_to_app_error(classify(
        response.status,
        &response.headers,
        &response.body,
    )))
}

/// Parse a proxied success body into a `serde_json::Value`.
fn parse_json(body: &[u8]) -> Result<serde_json::Value, AppError> {
    serde_json::from_slice(body)
        .map_err(|e| AppError::Upstream(format!("malformed github response: {e}")))
}

// ---- Single-target operations --------------------------------------------

/// `POST /repos/{owner}/{repo}/issues`. Returns the created [`IssueView`]
/// (with body) and copies any rate-limit headers through (via the handler).
pub async fn create_issue(
    proxy: &dyn GithubProxy,
    repo: &RepoRef<'_>,
    account: Option<&str>,
    body: serde_json::Value,
) -> Result<(IssueView, Option<RateLimitView>), AppError> {
    let connection = resolve_account(proxy, account).await?;
    let path = format!("/repos/{}/{}/issues", repo.owner, repo.repo);
    let response = single_target(
        proxy,
        &connection.connection_id,
        Method::POST,
        &path,
        Some(body),
    )
    .await?;
    let rate_limit = rate_limit_view(&response.headers);
    let value = parse_json(&response.body)?;
    let view = issue_view(&value, &connection.login, &repo.slug(), BodyMode::Include);
    tracing::info!(
        repo = %repo.slug(),
        issue_number = view.number,
        "created github issue"
    );
    Ok((view, rate_limit))
}

/// `GET /repos/{owner}/{repo}/issues/{number}`. Body populated.
pub async fn get_issue(
    proxy: &dyn GithubProxy,
    repo: &RepoRef<'_>,
    number: u64,
    account: Option<&str>,
) -> Result<IssueView, AppError> {
    let connection = resolve_account(proxy, account).await?;
    let path = format!("/repos/{}/{}/issues/{number}", repo.owner, repo.repo);
    let response =
        single_target(proxy, &connection.connection_id, Method::GET, &path, None).await?;
    let value = parse_json(&response.body)?;
    Ok(issue_view(
        &value,
        &connection.login,
        &repo.slug(),
        BodyMode::Include,
    ))
}

/// `PATCH /repos/{owner}/{repo}/issues/{number}`. Returns the updated issue.
pub async fn patch_issue(
    proxy: &dyn GithubProxy,
    repo: &RepoRef<'_>,
    number: u64,
    account: Option<&str>,
    body: serde_json::Value,
) -> Result<IssueView, AppError> {
    let connection = resolve_account(proxy, account).await?;
    let path = format!("/repos/{}/{}/issues/{number}", repo.owner, repo.repo);
    let response = single_target(
        proxy,
        &connection.connection_id,
        Method::PATCH,
        &path,
        Some(body),
    )
    .await?;
    let value = parse_json(&response.body)?;
    tracing::info!(repo = %repo.slug(), issue_number = number, "patched github issue");
    Ok(issue_view(
        &value,
        &connection.login,
        &repo.slug(),
        BodyMode::Include,
    ))
}

/// `GET /repos/{owner}/{repo}/issues/{number}/comments`.
pub async fn list_comments(
    proxy: &dyn GithubProxy,
    repo: &RepoRef<'_>,
    number: u64,
    account: Option<&str>,
    page: u32,
    per_page: u32,
) -> Result<Vec<CommentView>, AppError> {
    let connection = resolve_account(proxy, account).await?;
    let per_page = per_page.clamp(1, 50);
    let page = page.max(1);
    let path = format!(
        "/repos/{}/{}/issues/{number}/comments?per_page={per_page}&page={page}",
        repo.owner, repo.repo
    );
    let response =
        single_target(proxy, &connection.connection_id, Method::GET, &path, None).await?;
    let value = parse_json(&response.body)?;
    let comments = value
        .as_array()
        .map(|arr| arr.iter().map(comment_view).collect())
        .unwrap_or_default();
    Ok(comments)
}

/// `POST /repos/{owner}/{repo}/issues/{number}/comments`. Returns the comment.
pub async fn create_comment(
    proxy: &dyn GithubProxy,
    repo: &RepoRef<'_>,
    number: u64,
    account: Option<&str>,
    body: serde_json::Value,
) -> Result<CommentView, AppError> {
    let connection = resolve_account(proxy, account).await?;
    let path = format!(
        "/repos/{}/{}/issues/{number}/comments",
        repo.owner, repo.repo
    );
    let response = single_target(
        proxy,
        &connection.connection_id,
        Method::POST,
        &path,
        Some(body),
    )
    .await?;
    let value = parse_json(&response.body)?;
    tracing::info!(repo = %repo.slug(), issue_number = number, "created github comment");
    Ok(comment_view(&value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[test]
    fn classify_404_is_not_found() {
        assert_eq!(classify(404, &HeaderMap::new(), b""), Upstream::NotFound);
    }

    #[test]
    fn classify_401_is_auth() {
        assert_eq!(classify(401, &HeaderMap::new(), b""), Upstream::Auth);
    }

    #[test]
    fn classify_403_without_ratelimit_is_auth() {
        assert_eq!(classify(403, &HeaderMap::new(), b""), Upstream::Auth);
    }

    #[test]
    fn classify_403_with_remaining_zero_is_rate_limited() {
        let h = headers_with(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "0")]);
        assert!(matches!(classify(403, &h, b""), Upstream::RateLimited(_)));
    }

    #[test]
    fn classify_429_is_rate_limited_with_retry_after() {
        let h = headers_with(&[("retry-after", "30")]);
        assert_eq!(classify(429, &h, b""), Upstream::RateLimited(30));
    }

    #[test]
    fn classify_422_surfaces_first_error_message() {
        let body =
            br#"{"message":"Validation Failed","errors":[{"message":"label does not exist"}]}"#;
        match classify(422, &HeaderMap::new(), body) {
            Upstream::Unprocessable(msg) => assert_eq!(msg, "label does not exist"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn classify_500_is_upstream() {
        assert_eq!(classify(500, &HeaderMap::new(), b""), Upstream::Upstream);
    }

    #[test]
    fn upstream_rate_limited_maps_to_429_app_error() {
        let err = upstream_to_app_error(Upstream::RateLimited(42));
        match err {
            AppError::RateLimited {
                retry_after_secs, ..
            } => assert_eq!(retry_after_secs, 42),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn upstream_to_account_error_rate_limited_kind() {
        let err = upstream_to_account_error(Upstream::RateLimited(9));
        assert_eq!(err.kind, "rate_limited");
        assert_eq!(err.retry_after_secs, Some(9));
    }

    #[test]
    fn has_next_page_detects_rel_next() {
        let h = headers_with(&[(
            "link",
            "<https://api.github.com/x?page=2>; rel=\"next\", <https://api.github.com/x?page=5>; rel=\"last\"",
        )]);
        assert!(has_next_page(&h));
    }

    #[test]
    fn has_next_page_false_without_next() {
        let h = headers_with(&[("link", "<https://api.github.com/x?page=1>; rel=\"prev\"")]);
        assert!(!has_next_page(&h));
    }

    #[test]
    fn rate_limit_view_reads_both_headers() {
        let h = headers_with(&[
            ("x-ratelimit-remaining", "12"),
            ("x-ratelimit-reset", "1700"),
        ]);
        let view = rate_limit_view(&h).expect("present");
        assert_eq!(view.remaining, 12);
        assert_eq!(view.reset_epoch, 1700);
    }

    #[test]
    fn repo_ref_rejects_empty() {
        assert!(RepoRef::new("", "x").is_err());
        assert!(RepoRef::new("x", "  ").is_err());
        assert!(RepoRef::new("acme", "site").is_ok());
    }
}
