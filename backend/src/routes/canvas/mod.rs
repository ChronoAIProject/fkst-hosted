//! The canvas dashboard's live REST surface (stateless, GitHub-token
//! authenticated):
//!
//! - `GET  /api/v1/overview` — the whole-account canvas: every account
//!   (personal + orgs) with its repos, App-installation status, and live
//!   active-session/package counts.
//! - `GET  /api/v1/repos/{owner}/{name}/sessions` — one repo's sessions,
//!   scanned live (triggers, work issues, PRs, liveness).
//! - `POST /api/v1/repos/{owner}/{name}/sessions` — open a trigger issue AS
//!   the signed-in user (the human stays the session authz owner).
//! - `DELETE /api/v1/repos/{owner}/{name}/sessions/{issue_number}` — close the
//!   trigger issue AS the signed-in user (closing IS the stop/retire contract).
//!
//! Everything is computed from live GitHub reads plus the existing in-process
//! services (App tokens, the session backend) — no cache, no storage, no new
//! state. Auth mirrors the dashboard: the [`crate::github_identity::GithubUser`]
//! extractor verifies the caller, and the SAME bearer token drives the
//! user-scoped GitHub reads/writes.

mod github;
mod mutate;
mod outcomes;
mod overview;
// Broader-visibility enumeration-token resolution (issue #572): decides whether the
// overview enumerates with the App token or a caller-supplied broader-OAuth token.
mod overview_broader;
mod sessions;
#[cfg(test)]
pub(crate) mod test_support;
mod trigger_body;
mod types;
mod work_item;
mod work_projection;

pub use types::IssueDetail;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;
use crate::{
    github_app::listing::IssueSummary,
    models::RepoRef,
    reconcile::{effective_creator, parse_registration, CreatorResolution, SessionRegistration},
};

/// Parse a canvas-visible trigger with the same effective-creator attribution as
/// the reconciler. The canvas remains a read surface (the reconcile gate owns the
/// role decision), but registrations it projects must carry the same owner identity.
pub(super) fn parse_trigger_registration(
    installation_id: i64,
    repo: &RepoRef,
    issue: &IssueSummary,
    bot_login: Option<&str>,
) -> Result<SessionRegistration, (i64, String)> {
    match effective_creator(&issue.metadata(), bot_login) {
        CreatorResolution::Resolved(creator) => {
            parse_registration(installation_id, repo, issue, creator)
        }
        CreatorResolution::Unattributable { assignee_count, .. } => Err((
            issue.number,
            format!(
                "a bot-authored trigger must have exactly one assignee (found {assignee_count}) to attribute a session creator"
            ),
        )),
    }
}

/// The canvas router (nested under `/api/v1`). GitHub-token authenticated via
/// the `GithubUser` extractor (like the dashboard), so no documented security
/// scheme.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(overview::overview))
        .routes(routes!(sessions::repo_sessions, mutate::create_session))
        .routes(routes!(mutate::stop_session))
        .routes(routes!(work_item::create_work_item))
        .routes(routes!(outcomes::session_outcomes))
        .routes(routes!(outcomes::outcome_blob))
}
