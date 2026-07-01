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

/// Seconds added beyond `env.validate_deadline_secs` for the route-scoped PUT
/// timeout. It must comfortably exceed the validator's own internal backstop
/// (`deadline + WAIT_BUFFER(30)`) plus pod-setup and log-read overhead, so the
/// descriptive `422` (a timed-out verdict) wins the race over a bare `408`.
const ENV_PUT_TIMEOUT_BUFFER_SECS: u64 = 60;

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
    let short_timeout = Duration::from_secs(state.config.request_timeout_secs);
    // The named-environment PUT runs the isolated install-validation pod for up to
    // `env.validate_deadline_secs` — far beyond the short global timeout. So the
    // environments surface carries its OWN, much longer timeout; every OTHER route
    // keeps the short one. See `ENV_PUT_TIMEOUT_BUFFER_SECS`.
    let env_timeout = Duration::from_secs(
        u64::try_from(state.config.env.validate_deadline_secs)
            .unwrap_or(0)
            .saturating_add(ENV_PUT_TIMEOUT_BUFFER_SECS),
    );

    // The named-environment REST API (issue #338) under `/api/v1`. It is open at
    // the app layer: identity is the per-request GitHub token verified by the
    // `GithubUser` extractor, not middleware. Session query/stop are NOT a REST
    // surface — a session is controlled solely through its GitHub issue (the
    // `/status` + `/stop` issue comments, authorized by sender == issue author).
    let api_routes = routes::environments::router();

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

    // The route-scoped timeout is the reason the timeout is NOT applied in the
    // global ServiceBuilder below: axum's `.layer()` only wraps the routes present
    // when it is called. We give `top` the SHORT timeout, give the `/api/v1`
    // environments subtree its OWN long timeout, and only then merge the two — so
    // the environments PUT escapes the short global timeout while everything else
    // is still bounded by it.
    let top = top.layer(TimeoutLayer::with_status_code(
        StatusCode::REQUEST_TIMEOUT,
        short_timeout,
    ));
    let env =
        OpenApiRouter::new()
            .nest("/api/v1", api_routes)
            .layer(TimeoutLayer::with_status_code(
                StatusCode::REQUEST_TIMEOUT,
                env_timeout,
            ));

    // Merge the two independently-timed subtrees, then split the assembled router
    // from its OpenAPI document. The spec is rendered + served by a top-level
    // `/openapi.json` route, merged back onto the concrete axum router.
    let (router, spec) = top.merge(env).split_for_parts();

    Ok(router
        .merge(openapi::spec_route(spec)?)
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http())
                .layer(PropagateRequestIdLayer::x_request_id())
                .layer(cors_layer()),
        ))
}
