//! `GET /metrics`: a minimal Prometheus liveness gauge for the control plane.
//!
//! v1 is datastore-free and a session IS a Kubernetes Job (not an in-memory
//! record), so there is nothing local to count. The endpoint emits a single
//! `fkst_up` gauge so a scraper has a stable liveness signal; richer per-session
//! metrics would come from the Kubernetes API, not here. Unauthenticated, like
//! `/health` — it carries no secret.

use axum::response::IntoResponse;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// The Prometheus text content type (version 0.0.4 exposition format).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Render the exposition body. Split out so it is unit-testable without an HTTP
/// request.
fn render_metrics() -> String {
    "# HELP fkst_up 1 when the control plane is serving.\n\
     # TYPE fkst_up gauge\n\
     fkst_up 1\n"
        .to_string()
}

/// `GET /metrics`: the control plane's liveness gauge. Unauthenticated.
#[utoipa::path(
    get,
    path = "/metrics",
    tag = "system",
    operation_id = "metrics",
    responses(
        (
            status = 200,
            description = "Prometheus text exposition (version 0.0.4)",
            content_type = "text/plain",
            body = String
        )
    )
)]
async fn metrics() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        render_metrics(),
    )
}

/// `/metrics` route, mounted at the TOP level (unauthenticated, like `/health`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(metrics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_up_gauge_in_prometheus_text() {
        let body = render_metrics();
        assert!(body.contains("# TYPE fkst_up gauge"));
        assert!(body.contains("\nfkst_up 1\n") || body.starts_with("fkst_up 1\n"));
    }
}
