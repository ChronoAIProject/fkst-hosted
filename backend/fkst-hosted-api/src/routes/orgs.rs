//! Organizations HTTP API: `GET /api/v1/orgs`.
//!
//! Surfaces the caller's NyxID org membership so clients can discover the
//! `org_id` values they may scope packages, goals, and sessions to. Pure web
//! edge: the membership lookup and its TTL cache live in [`crate::nyxid`].

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::AuthContext;
use crate::error::AppError;
use crate::state::AppState;

/// Response item for `GET /api/v1/orgs`. The route owns the wire shape (as the
/// other list endpoints do) so it can grow `name`/`role` without coupling to
/// the NyxID client's internal summary type.
#[derive(Debug, Serialize)]
pub struct OrgView {
    pub id: String,
}

/// `GET /api/v1/orgs`: list the organizations the caller belongs to, sourced
/// from NyxID with the caller's own delegated bearer token (so a user only
/// ever sees their own orgs).
///
/// Owner-only mode (NyxID not configured) returns `200 []` — consistent with
/// `authz.visible_org_ids` returning empty, never a 503. A NyxID outage is
/// fail-closed: `503 Unavailable`, never a silent empty list.
async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<Vec<OrgView>>, AppError> {
    let Some(client) = state.authz.nyxid() else {
        tracing::debug!(user_id = %ctx.user_id, "orgs listed (nyxid disabled -> [])");
        return Ok(Json(Vec::new()));
    };

    let orgs = client
        .user_orgs(&ctx.user_id, &ctx.raw_token)
        .await
        .map_err(|error| {
            tracing::error!(
                user_id = %ctx.user_id,
                error = %error,
                "nyxid user-orgs lookup failed"
            );
            AppError::Unavailable("authorization service unavailable".to_string())
        })?;

    tracing::debug!(user_id = %ctx.user_id, count = orgs.len(), "orgs listed");
    Ok(Json(
        orgs.into_iter().map(|o| OrgView { id: o.id }).collect(),
    ))
}

/// Org routes, nested under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new().route("/orgs", get(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_view_serializes_to_id_object() {
        let view = OrgView {
            id: "org_alpha".to_string(),
        };
        let body = serde_json::to_value(&view).unwrap();
        assert_eq!(body, serde_json::json!({ "id": "org_alpha" }));
    }
}
