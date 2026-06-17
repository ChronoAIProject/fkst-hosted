//! Serialize-only projections for the GitHub issues hub and the pure mapping
//! from GitHub's issue/comment JSON onto them. No I/O lives here.
//!
//! Mapping rules:
//! - `IssueView.body` is populated ONLY on a single-issue GET; in list
//!   responses it is `None` (GitHub returns the body in lists too, but the hub
//!   suppresses it to keep list payloads small and to never aggregate bodies).
//! - `repository` is `"owner/name"`, derived from the GitHub `repository_url`
//!   (`.../repos/{owner}/{name}`) when present, else from the caller's request
//!   context (single-target ops know the repo from the path).
//! - `labels` is the list of `labels[].name`; `assignees` is `assignees[].login`.

use serde::Serialize;
use utoipa::ToSchema;

/// One linked GitHub account, projected for the hub's `/accounts` response.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct AccountView {
    pub connection_id: String,
    pub login: String,
    pub primary: bool,
}

impl From<crate::nyxid::GithubConnection> for AccountView {
    fn from(c: crate::nyxid::GithubConnection) -> Self {
        Self {
            connection_id: c.connection_id,
            login: c.login,
            primary: c.primary,
        }
    }
}

/// A GitHub issue projected for the hub. `account` is the GitHub login the
/// issue was fetched under (so a merged list stays attributable per account).
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct IssueView {
    pub account: String,
    pub repository: String,
    pub number: i64,
    pub id: i64,
    pub title: String,
    /// Populated only on single-issue GET; `None` in list responses.
    pub body: Option<String>,
    pub state: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub comments: i64,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Rate-limit snapshot copied through from GitHub response headers when present.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct RateLimitView {
    pub remaining: i64,
    pub reset_epoch: i64,
}

/// A per-account failure inside a fan-out (never a transport-level 5xx of the
/// hub itself). Serialized as the `error` object on [`AccountIssues`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct AccountError {
    /// One of `"rate_limited" | "auth" | "upstream" | "network"`.
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
}

/// The result of querying ONE account during an aggregate fan-out. Either
/// `issues` (possibly empty) succeeded, or `error` records why that account
/// failed — the overall fan-out is still a 200.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct AccountIssues {
    pub account: String,
    pub issues: Vec<IssueView>,
    pub page: u32,
    pub per_page: u32,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AccountError>,
}

/// Top-level aggregate response: one [`AccountIssues`] per resolved account.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct IssuesEnvelope {
    pub results: Vec<AccountIssues>,
}

/// A GitHub issue comment projected for the hub.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct CommentView {
    pub id: i64,
    pub user: String,
    pub body: String,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Whether a body field should be carried through when mapping an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    /// Single-issue GET: include the body.
    Include,
    /// List item: suppress the body.
    Suppress,
}

/// Derive `"owner/name"` from a GitHub `repository_url`
/// (`https://api.github.com/repos/{owner}/{name}`), falling back to `default`
/// (the request-context repo) when absent or unparseable.
fn repository_from(value: &serde_json::Value, default: &str) -> String {
    value
        .get("repository_url")
        .and_then(|v| v.as_str())
        .and_then(|url| {
            let idx = url.find("/repos/")?;
            let tail = &url[idx + "/repos/".len()..];
            // Keep exactly "owner/name" (first two path segments).
            let mut parts = tail.split('/');
            let owner = parts.next()?;
            let name = parts.next()?;
            if owner.is_empty() || name.is_empty() {
                return None;
            }
            Some(format!("{owner}/{name}"))
        })
        .unwrap_or_else(|| default.to_string())
}

/// Map a single GitHub issue JSON object onto an [`IssueView`].
///
/// `account` is the login the issue was fetched under; `default_repo` is the
/// request-context `"owner/name"` used when the JSON lacks `repository_url`.
/// Missing scalars degrade to safe defaults (empty string / 0) rather than
/// failing the whole response — GitHub always supplies them, but a tolerant
/// projection keeps one malformed item from sinking an aggregate.
pub fn issue_view(
    value: &serde_json::Value,
    account: &str,
    default_repo: &str,
    body_mode: BodyMode,
) -> IssueView {
    let body = match body_mode {
        BodyMode::Include => value
            .get("body")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        BodyMode::Suppress => None,
    };

    let labels = value
        .get("labels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let assignees = value
        .get("assignees")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("login").and_then(|l| l.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    IssueView {
        account: account.to_string(),
        repository: repository_from(value, default_repo),
        number: value.get("number").and_then(|v| v.as_i64()).unwrap_or(0),
        id: value.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
        title: str_field(value, "title"),
        body,
        state: str_field(value, "state"),
        labels,
        assignees,
        comments: value.get("comments").and_then(|v| v.as_i64()).unwrap_or(0),
        html_url: str_field(value, "html_url"),
        created_at: str_field(value, "created_at"),
        updated_at: str_field(value, "updated_at"),
    }
}

/// Map a single GitHub issue-comment JSON object onto a [`CommentView`].
pub fn comment_view(value: &serde_json::Value) -> CommentView {
    CommentView {
        id: value.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
        user: value
            .get("user")
            .and_then(|u| u.get("login"))
            .and_then(|l| l.as_str())
            .unwrap_or_default()
            .to_string(),
        body: str_field(value, "body"),
        html_url: str_field(value, "html_url"),
        created_at: str_field(value, "created_at"),
        updated_at: str_field(value, "updated_at"),
    }
}

/// Read a string field, defaulting to `""` when absent or non-string.
fn str_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_issue() -> serde_json::Value {
        json!({
            "id": 1001,
            "number": 7,
            "title": "Fix the thing",
            "body": "a detailed body",
            "state": "open",
            "comments": 3,
            "html_url": "https://github.com/acme/site/issues/7",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
            "repository_url": "https://api.github.com/repos/acme/site",
            "labels": [ {"name": "bug"}, {"name": "p1"} ],
            "assignees": [ {"login": "octocat"}, {"login": "hubber"} ]
        })
    }

    #[test]
    fn list_mapping_suppresses_body() {
        let view = issue_view(
            &sample_issue(),
            "octocat",
            "fallback/repo",
            BodyMode::Suppress,
        );
        assert_eq!(view.body, None, "body must be null in list mapping");
        assert_eq!(view.repository, "acme/site");
        assert_eq!(view.labels, vec!["bug", "p1"]);
        assert_eq!(view.assignees, vec!["octocat", "hubber"]);
        assert_eq!(view.number, 7);
        assert_eq!(view.account, "octocat");
    }

    #[test]
    fn single_mapping_includes_body() {
        let view = issue_view(
            &sample_issue(),
            "octocat",
            "fallback/repo",
            BodyMode::Include,
        );
        assert_eq!(view.body.as_deref(), Some("a detailed body"));
    }

    #[test]
    fn repository_falls_back_to_default_when_url_missing() {
        let value = json!({ "number": 1 });
        let view = issue_view(&value, "octocat", "owner/name", BodyMode::Suppress);
        assert_eq!(view.repository, "owner/name");
    }

    #[test]
    fn account_error_kind_serializes() {
        let err = AccountError {
            kind: "rate_limited".to_string(),
            message: "slow down".to_string(),
            retry_after_secs: Some(30),
        };
        let json = serde_json::to_value(&err).expect("serialize");
        assert_eq!(json["kind"], "rate_limited");
        assert_eq!(json["retry_after_secs"], 30);
    }

    #[test]
    fn account_error_omits_retry_after_when_none() {
        let err = AccountError {
            kind: "auth".to_string(),
            message: "nope".to_string(),
            retry_after_secs: None,
        };
        let json = serde_json::to_value(&err).expect("serialize");
        assert!(json.get("retry_after_secs").is_none());
    }

    #[test]
    fn comment_mapping_reads_user_login() {
        let value = json!({
            "id": 55,
            "user": { "login": "octocat" },
            "body": "looks good",
            "html_url": "https://github.com/acme/site/issues/7#issuecomment-55",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        });
        let view = comment_view(&value);
        assert_eq!(view.id, 55);
        assert_eq!(view.user, "octocat");
        assert_eq!(view.body, "looks good");
    }

    #[test]
    fn account_view_from_connection() {
        let c = crate::nyxid::GithubConnection {
            connection_id: "c1".to_string(),
            login: "octocat".to_string(),
            primary: true,
        };
        let view = AccountView::from(c);
        assert_eq!(view.connection_id, "c1");
        assert!(view.primary);
    }
}
