//! Router composition with the cross-cutting tower-http middleware stack.

use std::sync::Arc;
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
use crate::auth::verify::Verifier;
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
/// wrapped with the JWT verification middleware. Health endpoints (`/health`
/// and `/api/v1/health`) remain public by construction.
///
/// Returns `Err` if the verifier cannot be built (e.g. malformed JWKS URL).
pub fn build_router(state: AppState) -> Result<Router, AppError> {
    let timeout = Duration::from_secs(state.config.request_timeout_secs);

    let api_routes = routes::packages::router()
        .merge(routes::generate::router())
        .merge(routes::sessions::router())
        .merge(routes::goals::router())
        .merge(routes::github::router())
        .merge(routes::vault::router())
        .merge(routes::catalog::router());

    let api_routes = match &state.auth_mode {
        AuthMode::Enabled(settings) => {
            let verifier = Arc::new(Verifier::new(settings));
            tracing::info!(
                base_url = %settings.base_url,
                issuer = %settings.issuer,
                audience = %settings.audience,
                "JWT authentication enabled"
            );
            middleware::protect(api_routes, verifier)
        }
        AuthMode::Disabled => {
            tracing::warn!("AUTHENTICATION DISABLED — all /api/v1 routes are open");
            api_routes
        }
    };

    Ok(Router::new()
        .route("/health", get(routes::health::health))
        // The literal /api/v1/health route coexists with the /api/v1 nest:
        // axum nesting registers the inner routes individually (no catch-all),
        // so /api/v1/health keeps answering (asserted by integration test).
        .route("/api/v1/health", get(routes::health::health))
        .nest("/api/v1", api_routes)
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
