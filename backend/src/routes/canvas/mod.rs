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
mod overview;
mod sessions;
#[cfg(test)]
pub(crate) mod test_support;
mod types;

pub use types::IssueDetail;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// The canvas router (nested under `/api/v1`). GitHub-token authenticated via
/// the `GithubUser` extractor (like the dashboard), so no documented security
/// scheme.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(overview::overview))
        .routes(routes!(sessions::repo_sessions))
}
