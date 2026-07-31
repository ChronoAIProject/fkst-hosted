//! The authenticated `/api/v1/operations/*` surface.
//!
//! Every route here is available to EVERY admitted user, not just administrators:
//! the authorization is row-level and server-side, so "may I open this page" and
//! "which rows may I see" are two different questions with two different answers
//! (epic `AUTH-01`). Only selecting the global scope, naming another actor, or
//! reaching for an unauthorized session is refused.
//!
//! [`activity`] serves the historical trace; issue #5675 adds the live sandbox
//! inventory alongside it.

pub mod activity;
pub mod dto;
pub mod query;

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// The operations router, merged into the `/api/v1` subtree.
pub fn router() -> OpenApiRouter<AppState> {
    activity::router()
}
