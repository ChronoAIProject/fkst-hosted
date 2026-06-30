//! Router composition with the cross-cutting tower-http middleware stack.

use std::time::Duration;

use axum::http::{Method, StatusCode};
use axum::Router;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::AppError;
use crate::openapi;
use crate::routes;
use crate::state::AppState;

/// Permissive CORS for v1 local development only.
// TODO(frontend issue): tighten CORS to the real origin.
fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers(Any)
}

/// Build the application router.
///
/// The public surface is assembled on a [`utoipa_axum`](utoipa_axum) router so
/// every `#[utoipa::path]` operation registered here also lands in the OpenAPI
/// document; the assembled spec is then split out and served verbatim at
/// `GET /openapi.json` (top-level, unauthenticated — see [`crate::openapi`]).
/// The control plane is API-only: there is no in-process session execution and
/// no fleet-only `/internal/v1/*` worker protocol (removed in the single
/// control-plane refactor).
///
/// There is no application-level authentication: the `/api/v1/*` routes are
/// open, read-only, and network-isolated. Identity is the HMAC-verified GitHub
/// webhook actor — the only authenticated inbound is the signature-verified
/// GitHub App webhook (which lives outside the `/api/v1` nest, like `/health`).
pub fn build_router(state: AppState) -> Result<Router, AppError> {
    let timeout = Duration::from_secs(state.config.request_timeout_secs);

    let api_routes = routes::sessions::router();

    // The GitHub App webhook (issue #108) is UNAUTHENTICATED at the app layer
    // but signature-verified inside the handler over the raw body. It lives at
    // the top level (like `/health`), outside the `/api/v1` nest. It is only
    // mounted when a webhook secret is configured; with no secret there is
    // nothing to verify against, so the route is absent and installation
    // resolution degrades to on-demand (a warning is logged). When unmounted it
    // is likewise absent from the generated spec (the spec reflects what is
    // actually served).
    let mut top = OpenApiRouter::with_openapi(openapi::api_doc())
        .routes(routes!(routes::health::health))
        // The literal /api/v1/health route coexists with the /api/v1 nest:
        // axum nesting registers the inner routes individually (no catch-all),
        // so /api/v1/health keeps answering (asserted by integration test).
        .routes(routes!(routes::health::health_v1))
        // `/metrics` (#144) is TOP-level like `/health`: it carries only counts
        // (no secret) and is served on the ClusterIP-only surface, so Prometheus
        // scrapes it directly.
        .merge(routes::metrics::router());
    if state.github_app_webhook_secret.is_some() {
        top = top.merge(routes::github_app_webhook::router());
        tracing::info!("github app webhook endpoint mounted (signature-verified, unauthenticated)");
    } else {
        tracing::warn!(
            "github app webhook secret not set (FKST_GITHUB_APP_WEBHOOK_SECRET); \
             webhook endpoint disabled — installation resolution degrades to on-demand"
        );
    }

    // Nest the (open, read-only) API under `/api/v1`, then split the assembled
    // router from its OpenAPI document. The spec is rendered + served by a
    // top-level `/openapi.json` route, merged back onto the concrete axum router.
    let (router, spec) = top.nest("/api/v1", api_routes).split_for_parts();

    Ok(router
        .merge(openapi::spec_route(spec)?)
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http())
                .layer(PropagateRequestIdLayer::x_request_id())
                .layer(cors_layer())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    timeout,
                )),
        ))
}
