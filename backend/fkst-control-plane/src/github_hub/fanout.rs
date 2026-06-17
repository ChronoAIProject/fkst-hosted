//! Multi-account issue aggregation: the resilient fan-out that queries every
//! linked GitHub account concurrently and merges the results.
//!
//! Resilience contract: once the fan-out executes (i.e. the connections listing
//! succeeded), the response is ALWAYS 200 — a per-account failure or timeout is
//! recorded as an `error` object on that account's [`AccountIssues`], never a
//! transport-level error. Only `proxy.accounts()` / delegation failure bubbles
//! up as a 503.
//!
//! Concurrency uses [`tokio::task::JoinSet`]: each account is one spawned task
//! that shares an `Arc<P>` clone of the proxy, so a slow account never stalls
//! the others and each task is `'static` as `JoinSet::spawn` requires.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use tokio::task::JoinSet;

use crate::error::AppError;
use crate::github_hub::service::{
    classify, has_next_page, rate_limit_view, upstream_to_account_error,
};
use crate::github_hub::types::{
    issue_view, AccountError, AccountIssues, BodyMode, IssueView, IssuesEnvelope, RateLimitView,
};
use crate::github_hub::GithubProxy;
use crate::nyxid::GithubConnection;

/// Per-account upstream request budget. A slow account must not stall the
/// whole aggregate, so each fan-out task is wrapped in this timeout.
const PER_ACCOUNT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default and clamp bounds for the page size.
const DEFAULT_PER_PAGE: u32 = 30;
const MIN_PER_PAGE: u32 = 1;
const MAX_PER_PAGE: u32 = 50;

/// Validated parameters for an aggregate-issues request.
#[derive(Debug, Clone)]
pub struct AggregateParams {
    /// Optional case-insensitive login filter; `None` means all accounts.
    pub accounts: Option<Vec<String>>,
    /// GitHub `filter` (`assigned` | `created` | ...); defaults to `assigned`.
    pub filter: String,
    /// GitHub `state` (`open` | `closed` | `all`); defaults to `open`.
    pub state: String,
    /// Label names to AND-filter on (each URL-encoded individually).
    pub labels: Vec<String>,
    /// 1-based page (clamped to >= 1).
    pub page: u32,
    /// Page size (clamped to 1..=50, default 30).
    pub per_page: u32,
}

impl AggregateParams {
    /// Resolve `per_page` to its clamped value.
    fn clamped_per_page(&self) -> u32 {
        let n = if self.per_page == 0 {
            DEFAULT_PER_PAGE
        } else {
            self.per_page
        };
        n.clamp(MIN_PER_PAGE, MAX_PER_PAGE)
    }

    /// Resolve `page` to at least 1.
    fn clamped_page(&self) -> u32 {
        self.page.max(1)
    }

    /// Build the GitHub user-issues path + query for one account:
    /// `/issues?filter=..&state=..&per_page=..&page=..[&labels=a,b]`, with each
    /// label URL-encoded individually and joined by an (encoded) comma.
    fn issues_path(&self) -> String {
        let mut path = format!(
            "/issues?filter={}&state={}&per_page={}&page={}",
            encode(&self.filter),
            encode(&self.state),
            self.clamped_per_page(),
            self.clamped_page(),
        );
        if !self.labels.is_empty() {
            let labels = self
                .labels
                .iter()
                .map(|l| encode(l))
                .collect::<Vec<_>>()
                .join("%2C"); // encoded comma
            path.push_str(&format!("&labels={labels}"));
        }
        path
    }
}

/// Percent-encode a query-component value (RFC 3986 unreserved set kept). A
/// tiny confined helper so no extra dependency is pulled in.
fn encode(value: &str) -> String {
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

/// Aggregate the caller's issues across every linked GitHub account (subject to
/// the optional login filter), concurrently and resilient to per-account
/// failure. See the module docs for the 200-always contract.
///
/// `proxy` is shared (`Arc`) so each JoinSet task can hold its own `'static`
/// clone; the production proxy is cheap to clone (it is `Arc`-backed inside).
pub async fn aggregate_issues<P>(
    proxy: Arc<P>,
    params: AggregateParams,
) -> Result<IssuesEnvelope, AppError>
where
    P: GithubProxy + 'static,
{
    // Resolving connections is the only thing that can 503 the whole request.
    let connections = proxy.accounts().await?;
    let selected = filter_accounts(connections, params.accounts.as_deref());

    if selected.is_empty() {
        return Ok(IssuesEnvelope { results: vec![] });
    }

    let path = params.issues_path();
    let page = params.clamped_page();
    let per_page = params.clamped_per_page();

    // One spawned task per account; each result is tagged with its index so
    // the merged output preserves the resolved-account order.
    let mut set: JoinSet<(usize, AccountIssues)> = JoinSet::new();
    for (idx, connection) in selected.into_iter().enumerate() {
        let proxy = Arc::clone(&proxy);
        let path = path.clone();
        set.spawn(async move {
            let outcome = tokio::time::timeout(
                PER_ACCOUNT_TIMEOUT,
                query_account(&*proxy, &connection, &path),
            )
            .await;
            (idx, account_result(&connection, page, per_page, outcome))
        });
    }

    let mut indexed: Vec<(usize, AccountIssues)> = Vec::with_capacity(set.len());
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(pair) => indexed.push(pair),
            Err(join_err) => {
                // A panicked/cancelled task: do not fail the whole aggregate.
                // The account is lost from this response; log without detail.
                tracing::error!(error = %join_err, "github fan-out task failed");
            }
        }
    }
    indexed.sort_by_key(|(idx, _)| *idx);
    let results = indexed.into_iter().map(|(_, item)| item).collect();
    Ok(IssuesEnvelope { results })
}

/// Filter the resolved connections by the optional case-insensitive login set.
fn filter_accounts(
    connections: Vec<GithubConnection>,
    accounts: Option<&[String]>,
) -> Vec<GithubConnection> {
    match accounts {
        None => connections,
        Some(filter) => connections
            .into_iter()
            .filter(|c| {
                filter
                    .iter()
                    .any(|wanted| wanted.eq_ignore_ascii_case(&c.login))
            })
            .collect(),
    }
}

/// Query one account's issues page; returns the mapped issues + paging/rate
/// data, or an upstream-classified [`AccountError`].
async fn query_account<P>(
    proxy: &P,
    connection: &GithubConnection,
    path: &str,
) -> Result<AccountPage, AccountError>
where
    P: GithubProxy + ?Sized,
{
    let response = proxy
        .request(&connection.connection_id, Method::GET, path, None)
        .await
        .map_err(|e| AccountError {
            kind: "network".to_string(),
            // The proxy seam errors are credential-free; surface their Display.
            message: AppError::from(e).to_string(),
            retry_after_secs: None,
        })?;

    if !(200..300).contains(&response.status) {
        let upstream = classify(response.status, &response.headers, &response.body);
        return Err(upstream_to_account_error(upstream));
    }

    let value: serde_json::Value =
        serde_json::from_slice(&response.body).map_err(|e| AccountError {
            kind: "upstream".to_string(),
            message: format!("malformed github response: {e}"),
            retry_after_secs: None,
        })?;

    // In list responses the body is suppressed; the repository is derived from
    // each item's `repository_url` (the default is empty here since an
    // aggregate spans many repos).
    let issues = value
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|item| issue_view(item, &connection.login, "", BodyMode::Suppress))
                .collect::<Vec<IssueView>>()
        })
        .unwrap_or_default();

    Ok(AccountPage {
        issues,
        has_more: has_next_page(&response.headers),
        rate_limit: rate_limit_view(&response.headers),
    })
}

/// A successful per-account page.
struct AccountPage {
    issues: Vec<IssueView>,
    has_more: bool,
    rate_limit: Option<RateLimitView>,
}

/// Combine a per-account outcome (timeout vs result) into an [`AccountIssues`].
fn account_result(
    connection: &GithubConnection,
    page: u32,
    per_page: u32,
    outcome: Result<Result<AccountPage, AccountError>, tokio::time::error::Elapsed>,
) -> AccountIssues {
    match outcome {
        Ok(Ok(account_page)) => AccountIssues {
            account: connection.login.clone(),
            issues: account_page.issues,
            page,
            per_page,
            has_more: account_page.has_more,
            rate_limit: account_page.rate_limit,
            error: None,
        },
        Ok(Err(error)) => AccountIssues {
            account: connection.login.clone(),
            issues: vec![],
            page,
            per_page,
            has_more: false,
            rate_limit: None,
            error: Some(error),
        },
        Err(_elapsed) => AccountIssues {
            account: connection.login.clone(),
            issues: vec![],
            page,
            per_page,
            has_more: false,
            rate_limit: None,
            error: Some(AccountError {
                kind: "network".to_string(),
                message: "github request timed out".to_string(),
                retry_after_secs: None,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> AggregateParams {
        AggregateParams {
            accounts: None,
            filter: "assigned".to_string(),
            state: "open".to_string(),
            labels: vec![],
            page: 1,
            per_page: 30,
        }
    }

    #[test]
    fn issues_path_has_filter_state_paging() {
        let path = params().issues_path();
        assert!(path.contains("filter=assigned"), "{path}");
        assert!(path.contains("state=open"), "{path}");
        assert!(path.contains("per_page=30"), "{path}");
        assert!(path.contains("page=1"), "{path}");
        assert!(!path.contains("labels="), "no labels expected: {path}");
    }

    #[test]
    fn issues_path_encodes_labels_individually() {
        let mut p = params();
        p.labels = vec!["help wanted".to_string(), "p1".to_string()];
        let path = p.issues_path();
        // "help wanted" -> "help%20wanted"; comma -> "%2C".
        assert!(path.contains("labels=help%20wanted%2Cp1"), "{path}");
    }

    #[test]
    fn per_page_is_clamped_to_50() {
        let mut p = params();
        p.per_page = 999;
        assert!(p.issues_path().contains("per_page=50"));
    }

    #[test]
    fn per_page_zero_defaults_to_30() {
        let mut p = params();
        p.per_page = 0;
        assert!(p.issues_path().contains("per_page=30"));
    }

    #[test]
    fn page_clamped_to_at_least_1() {
        let mut p = params();
        p.page = 0;
        assert!(p.issues_path().contains("page=1"));
    }

    #[test]
    fn filter_accounts_is_case_insensitive() {
        let conns = vec![
            GithubConnection {
                connection_id: "c1".into(),
                login: "OctoCat".into(),
                primary: true,
            },
            GithubConnection {
                connection_id: "c2".into(),
                login: "Hubber".into(),
                primary: false,
            },
        ];
        let filtered = filter_accounts(conns, Some(&["octocat".to_string()]));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].login, "OctoCat");
    }

    #[test]
    fn timeout_outcome_records_network_error() {
        let conn = GithubConnection {
            connection_id: "c1".into(),
            login: "octocat".into(),
            primary: true,
        };
        let elapsed = make_elapsed();
        let result = account_result(&conn, 1, 30, Err(elapsed));
        let error = result.error.expect("error");
        assert_eq!(error.kind, "network");
        assert!(result.issues.is_empty());
    }

    /// Produce a `tokio::time::error::Elapsed` deterministically (it has no
    /// public constructor): time out an always-pending future immediately.
    fn make_elapsed() -> tokio::time::error::Elapsed {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("rt");
        rt.block_on(async {
            tokio::time::timeout(Duration::from_millis(0), std::future::pending::<()>())
                .await
                .expect_err("must elapse")
        })
    }
}
