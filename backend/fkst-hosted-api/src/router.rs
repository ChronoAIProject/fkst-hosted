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

use crate::routes;
use crate::state::AppState;

/// Permissive CORS for v1 local development only.
// TODO(frontend issue): tighten CORS to the real origin.
fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
}

/// Build the application router.
///
/// Layer ordering (tower applies the first layer outermost):
/// 1. `SetRequestIdLayer` — assigns an `x-request-id` (uuid) when absent, so
///    the id exists before tracing reads it.
/// 2. `TraceLayer` — structured request/response spans.
/// 3. `PropagateRequestIdLayer` — copies the request id onto the response.
/// 4. `cors_layer` — permissive CORS for v1 local dev.
/// 5. `TimeoutLayer` — per-request timeout (`408 Request Timeout` on expiry;
///    `TimeoutLayer::new` is deprecated in tower-http 0.6.11, so the same 408
///    behavior is selected explicitly via `with_status_code`).
pub fn build_router(state: AppState) -> Router {
    let timeout = Duration::from_secs(state.config.request_timeout_secs);
    Router::new()
        .route("/health", get(routes::health::health))
        // The literal /api/v1/health route coexists with the /api/v1 nest:
        // axum nesting registers the inner routes individually (no catch-all),
        // so /api/v1/health keeps answering (asserted by integration test).
        .route("/api/v1/health", get(routes::health::health))
        .nest("/api/v1", routes::packages::router())
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
        )
}
