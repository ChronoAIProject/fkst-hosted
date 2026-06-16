//! The worker-local HTTP server: a single liveness endpoint so Kubernetes can
//! probe the worker. It NEVER touches MongoDB — liveness is process-only.

use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

/// `GET /health` -> `200 {"status":"ok","role":"worker"}`. No store, no I/O.
async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "role": "worker" }))
}

/// The minimal worker-local router (health only). The controller->worker control
/// path is heartbeat-piggybacked, so no inbound control route is needed here.
pub fn health_router() -> Router {
    Router::new().route("/health", get(health))
}

/// Resolve when SIGTERM (k8s pod termination) or Ctrl-C arrives — shared
/// shutdown semantics with the control-plane.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install SIGINT handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_returns_ok_worker() {
        let app = health_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["role"], "worker");
    }
}
