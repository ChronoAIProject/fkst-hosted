//! NyxID connect-at-install HTTP surface (issue #297).
//!
//! `GET /api/v1/nyxid/connect?owner=<login>` redirects the owner's browser to a
//! NyxID OAuth consent for a broker binding; `GET /api/v1/nyxid/connect/callback`
//! receives the redirect back, exchanges the code for a durable `binding_id`, and
//! stores it per owner. Both are mounted at the top level (outside the proxy
//! auth nest) because the callback is a raw browser redirect carrying no
//! proxy-injected identity — the `state` value is the CSRF + identity binder.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::AppError;
use crate::nyxid_connect::{authorize_url, complete_callback, ConnectError};
use crate::state::AppState;

/// Header the NyxID proxy injects with the caller's user id (best-effort here).
const HEADER_USER_ID: &str = "X-NyxID-User-Id";

/// `/connect` query: which GitHub owner login this binding authorizes for.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ConnectQuery {
    owner: String,
}

/// Begin a NyxID broker-binding consent for `owner`: stash a CSRF `state` and
/// redirect to the NyxID authorize endpoint.
#[utoipa::path(
    get,
    path = "/api/v1/nyxid/connect",
    tag = "nyxid",
    operation_id = "nyxid_connect_begin",
    params(ConnectQuery),
    responses(
        (status = 302, description = "Redirect to the NyxID broker consent"),
        (status = 503, description = "NyxID connect is not configured", body = crate::error::ErrorEnvelope)
    )
)]
pub async fn connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ConnectQuery>,
) -> Result<Response, AppError> {
    let cfg = state
        .config
        .broker_client()
        .ok_or_else(|| AppError::Config("nyxid connect is not configured".to_string()))?;
    let base_url = state
        .nyxid_base_url()
        .ok_or_else(|| AppError::Config("auth (NyxID base URL) is not enabled".to_string()))?;
    let nyxid_user = headers
        .get(HEADER_USER_ID)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    let csrf = state.binding_store.begin_connect(&query.owner, nyxid_user);
    let url = authorize_url(&base_url, &cfg, &csrf);
    tracing::info!(owner = %query.owner, "nyxid connect: redirecting to consent");
    Ok(Redirect::to(&url).into_response())
}

/// `/connect/callback` query: the OAuth `code` + the `state` we issued.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CallbackQuery {
    code: String,
    state: String,
}

/// The callback success body.
#[derive(Debug, Serialize, ToSchema)]
pub struct CallbackResult {
    status: &'static str,
    owner: String,
}

/// Complete the consent: exchange the code for a durable binding and store it
/// under the owner the `state` stands for.
#[utoipa::path(
    get,
    path = "/api/v1/nyxid/connect/callback",
    tag = "nyxid",
    operation_id = "nyxid_connect_callback",
    params(CallbackQuery),
    responses(
        (status = 200, description = "Binding captured + stored", body = CallbackResult),
        (status = 400, description = "Unknown/expired state or exchange failure", body = crate::error::ErrorEnvelope)
    )
)]
pub async fn callback(
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
) -> Result<Json<CallbackResult>, AppError> {
    let cfg = state
        .config
        .broker_client()
        .ok_or_else(|| AppError::Config("nyxid connect is not configured".to_string()))?;
    let base_url = state
        .nyxid_base_url()
        .ok_or_else(|| AppError::Config("auth (NyxID base URL) is not enabled".to_string()))?;

    let http = reqwest::Client::new();
    let (owner, record) = complete_callback(
        &http,
        &base_url,
        &cfg,
        &state.binding_store,
        &query.code,
        &query.state,
    )
    .await
    .map_err(map_connect_error)?;

    state.binding_store.store_binding(&owner, record);
    tracing::info!(owner = %owner, "nyxid connect: broker binding stored");
    Ok(Json(CallbackResult {
        status: "connected",
        owner,
    }))
}

/// Map a [`ConnectError`] onto an [`AppError`] HTTP status.
fn map_connect_error(error: ConnectError) -> AppError {
    match error {
        ConnectError::NotConfigured => AppError::Config(error.to_string()),
        ConnectError::UnknownState
        | ConnectError::NoBinding
        | ConnectError::NoToken
        | ConnectError::Rejected(_) => AppError::Validation(error.to_string()),
        ConnectError::Transport(_) => AppError::Internal(anyhow::anyhow!(error.to_string())),
    }
}

/// The router for the NyxID connect endpoints (mounted at the top level).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(connect, callback))
}
