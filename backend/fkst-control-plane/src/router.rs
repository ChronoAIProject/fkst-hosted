//! Router composition with the cross-cutting tower-http middleware stack.

use std::time::Duration;

use axum::http::{Method, StatusCode};
use axum::routing::get;
use axum::Router;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::auth::middleware;
use crate::auth::AuthMode;
use crate::error::AppError;
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
/// When authentication is enabled, all `/api/v1/*` routes (except health) are
/// wrapped with the proxy-trusted identity middleware (issue #113): it reads the
/// NyxID-injected identity headers — it does NOT verify a user token or fetch
/// JWKS. Health endpoints (`/health` and `/api/v1/health`) remain public by
/// construction.
pub fn build_router(state: AppState) -> Result<Router, AppError> {
    let timeout = Duration::from_secs(state.config.request_timeout_secs);

    let api_routes = routes::sessions::router()
        .merge(routes::goals::router())
        .merge(routes::github::router())
        .merge(routes::catalog::router())
        .merge(routes::repos::router());

    let api_routes = match &state.auth_mode {
        AuthMode::Enabled(settings) => {
            tracing::info!(
                base_url = %settings.base_url,
                "proxy-trusted identity enforced (decode-only; no JWKS, no user-token verify)"
            );
            middleware::protect(api_routes)
        }
        AuthMode::Disabled => {
            tracing::warn!("AUTHENTICATION DISABLED — all /api/v1 routes are open");
            api_routes
        }
    };

    // The GitHub App webhook (issue #108) is UNAUTHENTICATED — GitHub presents
    // no NyxID identity — but signature-verified inside the handler over the
    // raw body. It must therefore live OUTSIDE the `protect()` nest (like
    // `/health`). It is only mounted when a webhook secret is configured; with
    // no secret there is nothing to verify against, so the route is absent and
    // installation resolution degrades to on-demand (a warning is logged).
    let mut top = Router::new()
        .route("/health", get(routes::health::health))
        // The literal /api/v1/health route coexists with the /api/v1 nest:
        // axum nesting registers the inner routes individually (no catch-all),
        // so /api/v1/health keeps answering (asserted by integration test).
        .route("/api/v1/health", get(routes::health::health));
    if state.github_app_webhook_secret.is_some() {
        top = top.merge(routes::github_app_webhook::router());
        tracing::info!("github app webhook endpoint mounted (signature-verified, unauthenticated)");
    } else {
        tracing::warn!(
            "github app webhook secret not set (FKST_GITHUB_APP_WEBHOOK_SECRET); \
             webhook endpoint disabled — installation resolution degrades to on-demand"
        );
    }

    Ok(top.nest("/api/v1", api_routes).with_state(state).layer(
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

/// Merge the internal worker-protocol routes onto the top-level router (#134).
///
/// Internal routes live at `/internal/v1/*`, NOT under `/api/v1` and NOT behind
/// the NyxID proxy-trust middleware: they carry their own constant-time
/// shared-secret auth (inside `internal_router`). This keeps the public
/// `/api/v1` surface unchanged while exposing the fleet-only internal surface.
pub fn mount_internal(
    top: Router,
    registry: crate::controller::WorkerRegistry,
    auth: crate::controller::InternalAuth,
    heartbeat_interval_secs: u64,
    claims: std::sync::Arc<crate::controller::ClaimMap>,
    minter: Option<std::sync::Arc<dyn crate::controller::SessionTokenMinter>>,
    reassign: Option<std::sync::Arc<crate::controller::ReassignDriver>>,
) -> Router {
    top.merge(crate::controller::internal_router(
        registry,
        auth,
        heartbeat_interval_secs,
        claims,
        minter,
        reassign,
    ))
}
