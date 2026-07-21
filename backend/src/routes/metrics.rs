//! `GET /metrics`: process liveness plus low-cardinality recovery metrics.

use axum::extract::State;
use axum::response::IntoResponse;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::recovery::RecoverySnapshot;
use crate::state::AppState;

/// The Prometheus text content type (version 0.0.4 exposition format).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Render the exposition body. Split out so it is unit-testable without an HTTP
/// request.
fn render_metrics(recovery: &RecoverySnapshot) -> String {
    let complete = u8::from(recovery.startup_resync_complete);
    let ready = u8::from(recovery.ready);
    format!(
        "# HELP fkst_up 1 when the control plane is serving.\n\
         # TYPE fkst_up gauge\n\
         fkst_up 1\n\
         # HELP fkst_startup_resync_attempts_total Full-resync attempts by bounded result.\n\
         # TYPE fkst_startup_resync_attempts_total counter\n\
         fkst_startup_resync_attempts_total{{result=\"success\"}} {}\n\
         fkst_startup_resync_attempts_total{{result=\"partial\"}} {}\n\
         fkst_startup_resync_attempts_total{{result=\"failure\"}} {}\n\
         # HELP fkst_startup_resync_last_duration_seconds Duration of the last full-resync attempt.\n\
         # TYPE fkst_startup_resync_last_duration_seconds gauge\n\
         fkst_startup_resync_last_duration_seconds {}\n\
         # HELP fkst_startup_resync_complete 1 after the first complete full-resync pass, or when dispatch is disabled.\n\
         # TYPE fkst_startup_resync_complete gauge\n\
         fkst_startup_resync_complete {complete}\n\
         # HELP fkst_recovery_ready 1 when the latest full-resync pass is complete, or when dispatch is disabled.\n\
         # TYPE fkst_recovery_ready gauge\n\
         fkst_recovery_ready {ready}\n\
         # HELP fkst_startup_resync_last_repositories_enqueued Repositories enqueued by the last full-resync attempt.\n\
         # TYPE fkst_startup_resync_last_repositories_enqueued gauge\n\
         fkst_startup_resync_last_repositories_enqueued {}\n\
         # HELP fkst_startup_resync_last_success_timestamp_seconds Unix timestamp of the last complete full-resync pass.\n\
         # TYPE fkst_startup_resync_last_success_timestamp_seconds gauge\n\
         fkst_startup_resync_last_success_timestamp_seconds {}\n",
        recovery.attempts.success,
        recovery.attempts.partial,
        recovery.attempts.failure,
        recovery.last_duration_seconds,
        recovery.last_repositories_enqueued,
        recovery.last_success_timestamp_seconds,
    )
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
async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        render_metrics(&state.recovery.snapshot()),
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
    fn renders_liveness_and_bounded_recovery_metrics() {
        let monitor = crate::recovery::RecoveryMonitor::new(true);
        monitor.record_attempt(
            crate::recovery::ResyncResult::Partial,
            std::time::Duration::from_millis(250),
            3,
        );
        let body = render_metrics(&monitor.snapshot());
        assert!(body.contains("# TYPE fkst_up gauge"));
        assert!(body.contains("\nfkst_up 1\n") || body.starts_with("fkst_up 1\n"));
        assert!(body.contains("fkst_startup_resync_attempts_total{result=\"partial\"} 1"));
        assert!(body.contains("fkst_startup_resync_attempts_total{result=\"success\"} 0"));
        assert!(body.contains("fkst_startup_resync_last_duration_seconds 0.25"));
        assert!(body.contains("fkst_startup_resync_complete 0"));
        assert!(body.contains("fkst_recovery_ready 0"));
        assert!(body.contains("fkst_startup_resync_last_repositories_enqueued 3"));
    }
}
