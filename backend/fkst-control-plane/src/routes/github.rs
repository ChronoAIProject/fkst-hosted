//! HTTP edge for the GitHub issues hub, nested under `/api/v1/github`.
//!
//! Every handler builds a per-request [`NyxIdGithubProxy`] from the existing
//! `state.authz.nyxid()` + the caller's token (no AppState/main wiring), then
//! delegates to the [`crate::github_hub::service`] / `fanout` engine. GitHub is
//! reached ONLY through that proxy seam.
//!
//! Bodies and tokens are NEVER logged; the DTOs deny unknown fields so client
//! typos fail loudly.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::auth::AuthContext;
use crate::authz::permissions::{self, require_permission};
use crate::error::AppError;
use crate::github_hub::fanout::{aggregate_issues, AggregateParams};
use crate::github_hub::service::{
    create_comment, create_issue, get_issue, list_accounts, list_comments, patch_issue, RepoRef,
};
use crate::github_hub::types::{
    AccountView, CommentView, IssueView, IssuesEnvelope, RateLimitView,
};
use crate::github_hub::NyxIdGithubProxy;
use crate::routes::extract::AppJson;
use crate::state::AppState;

/// Build the per-request NyxID-backed proxy. A missing credential proxy is a
/// 503; a rejected token exchange is mapped (401/503) without leaking the token.
async fn build_proxy(state: &AppState, ctx: &AuthContext) -> Result<NyxIdGithubProxy, AppError> {
    NyxIdGithubProxy::from_context(&state.authz, ctx).await
}

// ---- DTOs -----------------------------------------------------------------

/// Query for `GET /github/issues`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssuesQuery {
    /// Comma-separated GitHub logins to restrict the fan-out to.
    #[serde(default)]
    accounts: Option<String>,
    #[serde(default = "default_filter")]
    filter: String,
    #[serde(default = "default_state")]
    state: String,
    /// Comma-separated label names.
    #[serde(default)]
    labels: Option<String>,
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default = "default_per_page")]
    per_page: u32,
}

fn default_filter() -> String {
    "assigned".to_string()
}
fn default_state() -> String {
    "open".to_string()
}
fn default_page() -> u32 {
    1
}
fn default_per_page() -> u32 {
    30
}

/// Split a comma-separated list into trimmed, non-empty items.
fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Query carrying only an optional `account` selector.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountQuery {
    #[serde(default)]
    account: Option<String>,
}

/// Query for paginated comment listing.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentsQuery {
    #[serde(default)]
    account: Option<String>,
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default = "default_per_page")]
    per_page: u32,
}

/// Body for `POST /github/repos/{owner}/{repo}/issues`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateIssueBody {
    title: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    #[serde(default)]
    assignees: Option<Vec<String>>,
    #[serde(default)]
    account: Option<String>,
}

/// Body for `PATCH /github/repos/{owner}/{repo}/issues/{number}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchIssueBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    #[serde(default)]
    assignees: Option<Vec<String>>,
    #[serde(default)]
    account: Option<String>,
}

/// Body for `POST /github/repos/{owner}/{repo}/issues/{number}/comments`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCommentBody {
    body: String,
    #[serde(default)]
    account: Option<String>,
}

/// Parse a path `number` segment into a positive issue number (else 400).
fn parse_number(raw: &str) -> Result<u64, AppError> {
    raw.parse::<u64>()
        .map_err(|_| AppError::Validation(format!("invalid issue number: {raw}")))
}

// ---- Handlers -------------------------------------------------------------

/// `GET /github/accounts`.
async fn accounts(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<AccountView>>, AppError> {
    require_permission(&ctx, permissions::GITHUB_READ)?;
    let proxy = build_proxy(&state, &ctx).await?;
    let accounts = list_accounts(&proxy).await?;
    Ok(Json(accounts))
}

/// `GET /github/issues` — the resilient multi-account aggregate.
async fn issues(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(query): Query<IssuesQuery>,
) -> Result<Json<IssuesEnvelope>, AppError> {
    require_permission(&ctx, permissions::GITHUB_READ)?;
    let proxy = Arc::new(build_proxy(&state, &ctx).await?);
    let params = AggregateParams {
        accounts: query.accounts.as_deref().map(split_csv),
        filter: query.filter,
        state: query.state,
        labels: query.labels.as_deref().map(split_csv).unwrap_or_default(),
        page: query.page,
        per_page: query.per_page,
    };
    let envelope = aggregate_issues(proxy, params).await?;
    Ok(Json(envelope))
}

/// Build the GitHub issue create/patch JSON body from optional fields, omitting
/// absent ones so a PATCH only changes what the caller supplied.
fn issue_mutation_body(
    title: Option<String>,
    body: Option<String>,
    state: Option<String>,
    labels: Option<Vec<String>>,
    assignees: Option<Vec<String>>,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if let Some(title) = title {
        map.insert("title".to_string(), serde_json::Value::String(title));
    }
    if let Some(body) = body {
        map.insert("body".to_string(), serde_json::Value::String(body));
    }
    if let Some(state) = state {
        map.insert("state".to_string(), serde_json::Value::String(state));
    }
    if let Some(labels) = labels {
        map.insert("labels".to_string(), serde_json::json!(labels));
    }
    if let Some(assignees) = assignees {
        map.insert("assignees".to_string(), serde_json::json!(assignees));
    }
    serde_json::Value::Object(map)
}

/// `POST /github/repos/{owner}/{repo}/issues`.
///
/// On success returns 201 with the created [`IssueView`] and copies GitHub's
/// `x-ratelimit-remaining` / `x-ratelimit-reset` through as response headers so
/// callers can pace their writes.
async fn create_issue_handler(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path((owner, repo)): Path<(String, String)>,
    AppJson(req): AppJson<CreateIssueBody>,
) -> Result<Response, AppError> {
    require_permission(&ctx, permissions::GITHUB_WRITE)?;
    let repo_ref = RepoRef::new(&owner, &repo)?;
    if req.title.trim().is_empty() {
        return Err(AppError::Validation(
            "issue title must not be empty".to_string(),
        ));
    }
    let proxy = build_proxy(&state, &ctx).await?;
    let body = issue_mutation_body(Some(req.title), req.body, None, req.labels, req.assignees);
    let (view, rate_limit) = create_issue(&proxy, &repo_ref, req.account.as_deref(), body).await?;
    let mut response = (StatusCode::CREATED, Json(view)).into_response();
    copy_rate_limit_headers(&mut response, rate_limit.as_ref());
    Ok(response)
}

/// Copy a [`RateLimitView`] onto a response as `x-ratelimit-*` headers.
fn copy_rate_limit_headers(response: &mut Response, rate_limit: Option<&RateLimitView>) {
    if let Some(rl) = rate_limit {
        if let Ok(value) = HeaderValue::from_str(&rl.remaining.to_string()) {
            response
                .headers_mut()
                .insert("x-ratelimit-remaining", value);
        }
        if let Ok(value) = HeaderValue::from_str(&rl.reset_epoch.to_string()) {
            response.headers_mut().insert("x-ratelimit-reset", value);
        }
    }
}

/// `PATCH /github/repos/{owner}/{repo}/issues/{number}`.
async fn patch_issue_handler(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path((owner, repo, number)): Path<(String, String, String)>,
    AppJson(req): AppJson<PatchIssueBody>,
) -> Result<Json<IssueView>, AppError> {
    require_permission(&ctx, permissions::GITHUB_WRITE)?;
    let repo_ref = RepoRef::new(&owner, &repo)?;
    let number = parse_number(&number)?;
    let proxy = build_proxy(&state, &ctx).await?;
    let body = issue_mutation_body(req.title, req.body, req.state, req.labels, req.assignees);
    let view = patch_issue(&proxy, &repo_ref, number, req.account.as_deref(), body).await?;
    Ok(Json(view))
}

/// `GET /github/repos/{owner}/{repo}/issues/{number}` (body populated).
async fn get_issue_handler(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path((owner, repo, number)): Path<(String, String, String)>,
    Query(query): Query<AccountQuery>,
) -> Result<Json<IssueView>, AppError> {
    require_permission(&ctx, permissions::GITHUB_READ)?;
    let repo_ref = RepoRef::new(&owner, &repo)?;
    let number = parse_number(&number)?;
    let proxy = build_proxy(&state, &ctx).await?;
    let view = get_issue(&proxy, &repo_ref, number, query.account.as_deref()).await?;
    Ok(Json(view))
}

/// `GET /github/repos/{owner}/{repo}/issues/{number}/comments`.
async fn list_comments_handler(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path((owner, repo, number)): Path<(String, String, String)>,
    Query(query): Query<CommentsQuery>,
) -> Result<Json<Vec<CommentView>>, AppError> {
    require_permission(&ctx, permissions::GITHUB_READ)?;
    let repo_ref = RepoRef::new(&owner, &repo)?;
    let number = parse_number(&number)?;
    let proxy = build_proxy(&state, &ctx).await?;
    let comments = list_comments(
        &proxy,
        &repo_ref,
        number,
        query.account.as_deref(),
        query.page,
        query.per_page,
    )
    .await?;
    Ok(Json(comments))
}

/// `POST /github/repos/{owner}/{repo}/issues/{number}/comments`.
async fn create_comment_handler(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path((owner, repo, number)): Path<(String, String, String)>,
    AppJson(req): AppJson<CreateCommentBody>,
) -> Result<(StatusCode, Json<CommentView>), AppError> {
    require_permission(&ctx, permissions::GITHUB_WRITE)?;
    let repo_ref = RepoRef::new(&owner, &repo)?;
    let number = parse_number(&number)?;
    if req.body.trim().is_empty() {
        return Err(AppError::Validation(
            "comment body must not be empty".to_string(),
        ));
    }
    let proxy = build_proxy(&state, &ctx).await?;
    let body = serde_json::json!({ "body": req.body });
    let comment = create_comment(&proxy, &repo_ref, number, req.account.as_deref(), body).await?;
    Ok((StatusCode::CREATED, Json(comment)))
}

// ---- Router ---------------------------------------------------------------

/// GitHub issues-hub routes, nested under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/github/accounts", get(accounts))
        .route("/github/issues", get(issues))
        .route(
            "/github/repos/:owner/:repo/issues",
            post(create_issue_handler),
        )
        .route(
            "/github/repos/:owner/:repo/issues/:number",
            get(get_issue_handler).patch(patch_issue_handler),
        )
        .route(
            "/github/repos/:owner/:repo/issues/:number/comments",
            get(list_comments_handler).post(create_comment_handler),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_csv_trims_and_drops_empty() {
        assert_eq!(split_csv("a, b ,,c"), vec!["a", "b", "c"]);
        assert!(split_csv("  ,, ").is_empty());
    }

    #[test]
    fn parse_number_rejects_non_numeric() {
        assert!(parse_number("abc").is_err());
        assert_eq!(parse_number("42").expect("ok"), 42);
    }

    #[test]
    fn issue_mutation_body_omits_absent_fields() {
        let body = issue_mutation_body(Some("t".into()), None, Some("closed".into()), None, None);
        assert_eq!(body["title"], "t");
        assert_eq!(body["state"], "closed");
        assert!(body.get("body").is_none());
        assert!(body.get("labels").is_none());
    }
}
