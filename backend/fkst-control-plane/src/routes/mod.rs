//! HTTP route handlers.

pub mod admin_state;
pub mod catalog;
pub mod extract;
pub mod github;
pub mod github_app_webhook;
pub mod goals;
pub mod goals_submit;
pub mod goals_submit_dto;
pub mod health;
pub mod metrics;
pub mod repos;
pub mod repos_scaffold;
pub mod sessions;

use serde::Serialize;
use utoipa::ToSchema;

use crate::error::AppError;

/// GitHub repository reference (`owner/name`) echoed in goal and session
/// response bodies. Single shared definition so the goals and sessions
/// projections (which carried byte-identical copies) map to ONE OpenAPI schema.
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct RepoRefView {
    pub owner: String,
    pub name: String,
}

/// Render a stored BSON datetime (always UTC) as RFC3339 with a `Z` suffix.
/// A formatting failure means a corrupt stored timestamp: a 500, never a 4xx.
pub(crate) fn rfc3339(ts: bson::DateTime) -> Result<String, AppError> {
    ts.try_to_rfc3339_string()
        .map_err(|error| AppError::Internal(anyhow::anyhow!("invalid stored timestamp: {error}")))
}
