//! Tests for the status-summary health read + the pure sandbox-state → pod-phase
//! taxonomy. The taxonomy cases run the SHARED pure evaluator
//! ([`crate::k8s::health_eval::evaluate_health`]) over the projected status, proving a
//! Failed sandbox degrades and a Terminated one reads clean — byte-for-byte with the
//! Kubernetes backend.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::k8s::health_eval::{evaluate_health, HealthVerdict, PodStatusSummary};
use crate::session_backend::opensandbox::dto::SandboxState;
use crate::session_backend::RuntimeStatus;

use super::super::backend_test_support::{
    backend, list_page, osb_config, sandbox_json, SESSION_ID,
};
use super::state_to_phase;

/// Adapt an OSB [`RuntimeStatus`] into the evaluator's summary exactly as the health
/// scrape does (a `None` restart count round-trips to `0`).
fn to_summary(status: &RuntimeStatus) -> PodStatusSummary {
    PodStatusSummary {
        phase: status.phase.clone(),
        restart_count: status
            .restart_count
            .map(|r| i32::try_from(r).unwrap_or(i32::MAX))
            .unwrap_or(0),
        waiting_reason: status.stall_reason.clone(),
    }
}

/// Resolve one sandbox in `state` (optionally with a status `reason`) and read its
/// projected [`RuntimeStatus`].
async fn status_for(state: &str, reason: Option<&str>) -> RuntimeStatus {
    let server = MockServer::start().await;
    let mut body = sandbox_json(
        "sbx-1",
        state,
        "2026-07-09T00:00:00Z",
        json!({ "fkst-session-id": SESSION_ID }),
    );
    if let Some(r) = reason {
        body["status"]["reason"] = json!(r);
    }
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([body]))))
        .mount(&server)
        .await;
    backend(&server.uri(), osb_config())
        .status_summary_impl(SESSION_ID)
        .await
        .expect("status")
}

#[test]
fn state_to_phase_matches_the_health_eval_vocabulary() {
    assert_eq!(state_to_phase(&SandboxState::Failed), "Failed");
    assert_eq!(state_to_phase(&SandboxState::Terminated), "Succeeded");
    assert_eq!(state_to_phase(&SandboxState::Running), "Running");
    for state in [
        SandboxState::Pending,
        SandboxState::Pausing,
        SandboxState::Paused,
        SandboxState::Resuming,
        SandboxState::Stopping,
    ] {
        assert_eq!(
            state_to_phase(&state),
            "Pending",
            "{state:?} maps to Pending"
        );
    }
    assert_eq!(
        state_to_phase(&SandboxState::Unknown("weird".to_string())),
        "Unknown"
    );
}

#[tokio::test]
async fn status_summary_reports_failed_as_a_degraded_phase() {
    let status = status_for("Failed", Some("OOMKilled")).await;
    assert_eq!(status.phase.as_deref(), Some("Failed"));
    // No pod-style restart signal (None → 0, so no false restart-degrade); the failure
    // reason is carried as the stall signal.
    assert_eq!(status.restart_count, None);
    assert_eq!(status.stall_reason.as_deref(), Some("OOMKilled"));
    // The SHARED pure evaluator degrades it — the taxonomy contract.
    assert!(matches!(
        evaluate_health(&to_summary(&status), &[]),
        HealthVerdict::Degraded { .. }
    ));
}

#[tokio::test]
async fn status_summary_reports_terminated_as_a_clean_succeeded_phase() {
    // A non-Failed state's reason is filtered out, so it can never false-degrade.
    let status = status_for("Terminated", Some("Completed")).await;
    assert_eq!(status.phase.as_deref(), Some("Succeeded"));
    assert_eq!(status.stall_reason, None);
    assert_eq!(
        evaluate_health(&to_summary(&status), &[]),
        HealthVerdict::Healthy
    );
}

#[tokio::test]
async fn status_summary_of_a_gone_session_is_the_default() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;
    let status = backend(&server.uri(), osb_config())
        .status_summary_impl(SESSION_ID)
        .await
        .expect("status");
    assert_eq!(status, RuntimeStatus::default());
}
