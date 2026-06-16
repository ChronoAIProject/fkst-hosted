//! HTTP route handlers.

pub mod catalog;
pub mod extract;
pub mod github;
pub mod github_app_webhook;
pub mod goals;
pub mod health;
pub mod sessions;

use crate::error::AppError;

/// Render a stored BSON datetime (always UTC) as RFC3339 with a `Z` suffix.
/// A formatting failure means a corrupt stored timestamp: a 500, never a 4xx.
pub(crate) fn rfc3339(ts: bson::DateTime) -> Result<String, AppError> {
    ts.try_to_rfc3339_string()
        .map_err(|error| AppError::Internal(anyhow::anyhow!("invalid stored timestamp: {error}")))
}
