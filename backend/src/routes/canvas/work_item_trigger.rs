//! The work-item endpoint's own trigger-issue read.
//!
//! Split out of [`super`] so the handler file stays within the source line
//! budget, and because this is a self-contained concern: one GitHub call, the
//! shape it decodes into, and the error mapping that turns its failures into the
//! API surface. The handler consumes only [`FetchedTrigger`].
//!
//! It exists at all because the sibling [`DashboardGithub::get_issue`]
//! deliberately drops the issue BODY (stop-session's pre-flight needs only
//! labels) while this endpoint must parse the session's work label and
//! collaborators out of it.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::dashboard::DashboardGithub;

/// The trigger issue as this endpoint reads it: the body (to parse the session's
/// work label + collaborators out of), the label names (to prove it really is a
/// trigger), whether the "issue" is actually a pull request (GitHub's issues API
/// serves PRs too), and the author/assignee metadata used to resolve the same
/// effective creator as the reconciler.
pub(super) struct FetchedTrigger {
    pub(super) body: String,
    pub(super) labels: Vec<String>,
    pub(super) state: String,
    pub(super) is_pull_request: bool,
    /// The trigger author's immutable numeric GitHub id.
    pub(super) author_id: i64,
    /// The trigger author's login (the App bot for seeded triggers).
    pub(super) author_login: String,
    /// Trigger assignee logins. Exactly one identifies the creator when the App
    /// bot authored the trigger.
    pub(super) assignees: Vec<String>,
}

/// Pull GitHub's own `message` out of an error body without leaking anything
/// else; falls back to the bare status.
async fn github_message(response: reqwest::Response) -> String {
    let status = response.status();
    response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|message| message.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("github returned status {status}"))
}

/// Map a failed trigger read onto the API error surface. GitHub answers 404 for
/// both "no such issue" and "no access" (anti-enumeration), so both surface as
/// not-found here — the same contract stop-session's pre-flight uses.
fn trigger_read_error(status: reqwest::StatusCode, message: String) -> AppError {
    match status.as_u16() {
        401 => AppError::Unauthorized(format!("github rejected the token: {message}")),
        403 => AppError::Forbidden(format!("GitHub refused the read: {message}")),
        404 => AppError::NotFound(format!("github get_issue: {message}")),
        _ => AppError::Unavailable(format!("github get_issue returned status {status}")),
    }
}

impl DashboardGithub {
    /// `GET /repos/{owner}/{repo}/issues/{number}` (user token) returning the
    /// full BODY — the work-item endpoint parses the session's work label out of
    /// it. The sibling [`DashboardGithub::get_issue`] deliberately drops the
    /// body (stop-session's pre-flight needs only labels), so this endpoint owns
    /// the body-bearing read it uniquely requires.
    pub(super) async fn fetch_trigger(
        &self,
        user_token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<FetchedTrigger, AppError> {
        let url = format!("{}/repos/{owner}/{repo}/issues/{number}", self.api_base);
        let response = self
            .client
            .get(&url)
            .bearer_auth(user_token.expose_secret())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "github get-trigger transport error");
                AppError::Unavailable("github get-issue request failed".to_string())
            })?;
        let status = response.status();
        if status.is_success() {
            #[derive(Deserialize)]
            struct RawLabel {
                name: String,
            }
            #[derive(Deserialize)]
            struct RawUser {
                id: i64,
                login: String,
            }
            #[derive(Deserialize)]
            struct RawAssignee {
                login: String,
            }
            #[derive(Deserialize)]
            struct RawIssue {
                /// GitHub sends `"body": null` for a body-less issue.
                #[serde(default)]
                body: Option<String>,
                #[serde(default)]
                labels: Vec<RawLabel>,
                #[serde(default)]
                state: String,
                /// Present only when this "issue" is actually a PR.
                pull_request: Option<serde_json::Value>,
                /// The trigger author; required on GitHub issue responses.
                user: RawUser,
                #[serde(default)]
                assignees: Vec<RawAssignee>,
            }
            let raw: RawIssue = response.json().await.map_err(|e| {
                tracing::warn!(error = %e, "github get-trigger response did not parse");
                AppError::Upstream("github get-issue response was malformed".to_string())
            })?;
            return Ok(FetchedTrigger {
                body: raw.body.unwrap_or_default(),
                labels: raw.labels.into_iter().map(|label| label.name).collect(),
                state: raw.state,
                is_pull_request: raw.pull_request.is_some(),
                author_id: raw.user.id,
                author_login: raw.user.login,
                assignees: raw
                    .assignees
                    .into_iter()
                    .map(|assignee| assignee.login)
                    .collect(),
            });
        }
        Err(trigger_read_error(status, github_message(response).await))
    }
}
