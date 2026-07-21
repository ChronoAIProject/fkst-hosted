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
    let election_enabled = u8::from(recovery.leader_election_enabled);
    let leader = u8::from(recovery.leader);
    let leader_ready = u8::from(recovery.leader_ready);
    let leader_routing_ready = u8::from(recovery.leader_routing_ready);
    let state = if !recovery.leader_election_enabled {
        "disabled"
    } else if !recovery.leader {
        "follower"
    } else if recovery.degraded {
        "degraded"
    } else if recovery.ready {
        "ready"
    } else {
        "recovering"
    };
    let mut body = format!(
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
         fkst_startup_resync_last_success_timestamp_seconds {}\n\
         # HELP fkst_leader_election_enabled 1 when Kubernetes Lease election gates reconcile work.\n\
         # TYPE fkst_leader_election_enabled gauge\n\
         fkst_leader_election_enabled {election_enabled}\n\
         # HELP fkst_leader 1 when this process currently holds the Lease.\n\
         # TYPE fkst_leader gauge\n\
         fkst_leader {leader}\n\
         # HELP fkst_leader_ready 1 when this holder completed its acquisition resync.\n\
         # TYPE fkst_leader_ready gauge\n\
         fkst_leader_ready {leader_ready}\n\
         # HELP fkst_leader_routing_ready 1 when this holder is the only Service-selected replica.\n\
         # TYPE fkst_leader_routing_ready gauge\n\
         fkst_leader_routing_ready {leader_routing_ready}\n\
         # HELP fkst_leader_state Current bounded leader lifecycle state.\n\
         # TYPE fkst_leader_state gauge\n\
         fkst_leader_state{{state=\"disabled\"}} {}\n\
         fkst_leader_state{{state=\"follower\"}} {}\n\
         fkst_leader_state{{state=\"recovering\"}} {}\n\
         fkst_leader_state{{state=\"ready\"}} {}\n\
         fkst_leader_state{{state=\"degraded\"}} {}\n\
         # HELP fkst_leader_transitions_total Process-local leadership acquisitions and losses.\n\
         # TYPE fkst_leader_transitions_total counter\n\
         fkst_leader_transitions_total{{transition=\"acquired\"}} {}\n\
         fkst_leader_transitions_total{{transition=\"lost\"}} {}\n\
         # HELP fkst_leader_lease_failures_total Lease operation failures by bounded operation.\n\
         # TYPE fkst_leader_lease_failures_total counter\n\
         fkst_leader_lease_failures_total{{operation=\"acquire\"}} {}\n\
         fkst_leader_lease_failures_total{{operation=\"renew\"}} {}\n\
         fkst_leader_lease_failures_total{{operation=\"conflict\"}} {}\n\
         # HELP fkst_leader_routing_failures_total Failed attempts to publish or withdraw the leader Service label.\n\
         # TYPE fkst_leader_routing_failures_total counter\n\
         fkst_leader_routing_failures_total {}\n\
         # HELP fkst_leader_observed_lease_transitions Durable transition count last read from the Lease.\n\
         # TYPE fkst_leader_observed_lease_transitions gauge\n\
         fkst_leader_observed_lease_transitions {}\n\
         # HELP fkst_leader_last_successful_renew_timestamp_seconds Unix timestamp of this process's last confirmed renewal.\n\
         # TYPE fkst_leader_last_successful_renew_timestamp_seconds gauge\n\
         fkst_leader_last_successful_renew_timestamp_seconds {}\n\
         # HELP fkst_leader_last_successful_resync_timestamp_seconds Unix timestamp of this leader generation's last complete resync.\n\
         # TYPE fkst_leader_last_successful_resync_timestamp_seconds gauge\n\
         fkst_leader_last_successful_resync_timestamp_seconds {}\n",
        recovery.attempts.success,
        recovery.attempts.partial,
        recovery.attempts.failure,
        recovery.last_duration_seconds,
        recovery.last_repositories_enqueued,
        recovery.last_success_timestamp_seconds,
        u8::from(state == "disabled"),
        u8::from(state == "follower"),
        u8::from(state == "recovering"),
        u8::from(state == "ready"),
        u8::from(state == "degraded"),
        recovery.leader_acquisitions,
        recovery.leader_losses,
        recovery.leader_acquire_failures,
        recovery.leader_renew_failures,
        recovery.leader_conflicts,
        recovery.leader_routing_failures,
        recovery.observed_lease_transitions,
        recovery.last_successful_leader_renew_timestamp_seconds,
        recovery.last_successful_leader_resync_timestamp_seconds,
    );
    if let Some(identity) = &recovery.leader_identity {
        body.push_str(&format!(
            "# HELP fkst_leader_identity_info Identity of this configured contender.\n\
             # TYPE fkst_leader_identity_info gauge\n\
             fkst_leader_identity_info{{identity=\"{}\"}} 1\n",
            prometheus_label(identity)
        ));
    }
    if let Some(holder) = &recovery.observed_holder_identity {
        body.push_str(&format!(
            "# HELP fkst_leader_observed_holder_info Last holder observed in the Lease.\n\
             # TYPE fkst_leader_observed_holder_info gauge\n\
             fkst_leader_observed_holder_info{{identity=\"{}\"}} 1\n",
            prometheus_label(holder)
        ));
    }
    body
}

fn prometheus_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
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
        assert!(body.contains("fkst_leader_state{state=\"disabled\"} 1"));
        assert!(body.contains("fkst_leader_election_enabled 0"));
    }

    #[test]
    fn renders_leader_identity_transitions_and_failure_metrics() {
        let monitor = crate::recovery::RecoveryMonitor::new(true);
        monitor.enable_leader_election("pod-\\\"a".to_string());
        monitor.record_leader_acquired(7);
        monitor.record_leader_api_failure(true);
        monitor.record_attempt(
            crate::recovery::ResyncResult::Success,
            std::time::Duration::from_millis(10),
            2,
        );
        monitor.record_leader_routing(true);
        let body = render_metrics(&monitor.snapshot());
        assert!(body.contains("fkst_leader 1"));
        assert!(body.contains("fkst_leader_ready 1"));
        assert!(body.contains("fkst_leader_state{state=\"ready\"} 1"));
        assert!(body.contains("fkst_leader_transitions_total{transition=\"acquired\"} 1"));
        assert!(body.contains("fkst_leader_lease_failures_total{operation=\"renew\"} 1"));
        assert!(body.contains("fkst_leader_observed_lease_transitions 7"));
        assert!(body.contains("identity=\"pod-\\\\\\\"a\""));
    }
}
