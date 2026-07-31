//! The authenticated `/api/v1/operations/*` surface.
//!
//! Every route here is available to EVERY admitted user, not just administrators:
//! the authorization is row-level and server-side, so "may I open this page" and
//! "which rows may I see" are two different questions with two different answers
//! (epic `AUTH-01`). Only selecting the global scope, naming another actor, or
//! reaching for an unauthorized session is refused.
//!
//! [`activity`] serves the historical trace and [`sandboxes`] the live runtime
//! inventory. They are deliberately independent: a PostHog outage must never hide
//! live sandbox state, and a runtime-backend outage must never falsify history.

pub mod activity;
pub mod dto;
pub mod query;
pub mod sandbox_dto;
pub mod sandbox_query;
pub mod sandboxes;

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// The operations router, merged into the `/api/v1` subtree.
pub fn router() -> OpenApiRouter<AppState> {
    activity::router().merge(sandboxes::router())
}
