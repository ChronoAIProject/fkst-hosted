//! The pure Pod -> inventory-facts projection, over every documented shape:
//! phases, deletion precedence, restart summation, waiting/terminated detail,
//! transition selection, and the malformed/missing metadata cases that must be
//! REPRESENTED rather than dropped.

use k8s_openapi::api::core::v1::{
    ContainerState, ContainerStateRunning, ContainerStateTerminated, ContainerStateWaiting,
    PodCondition, PodStatus,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

use crate::k8s::session_launcher::ANNOTATION_LAST_PENDING_AT;
use crate::runtime_identity::AttributionSource;
use crate::session_backend::inventory::status::RuntimeInventoryStatus;

use super::inventory_test_fixtures::{container, sample_pod, ts};
use super::{facts_from_pod, UNNAMED_RUNTIME_ID};

#[test]
fn a_complete_pod_projects_every_correlation_and_attribution_fact() {
    let facts = facts_from_pod(&sample_pod(Some("Running"), false), "chronoai-fkst");
    assert_eq!(facts.runtime_id, "fkst-sess-sess-1");
    assert_eq!(facts.runtime_name.as_deref(), Some("fkst-sess-sess-1"));
    assert_eq!(facts.runtime_uid.as_deref(), Some("uid-1"));
    assert_eq!(facts.backend_location.as_deref(), Some("chronoai-fkst"));
    assert_eq!(facts.session_id.as_deref(), Some("sess-1"));
    assert!(facts.managed);
    assert_eq!(facts.owner.as_deref(), Some("acme"));
    assert_eq!(facts.repo.as_deref(), Some("site"));
    assert_eq!(facts.installation_id_raw.as_deref(), Some("900"));
    assert_eq!(facts.trigger_issue_raw.as_deref(), Some("7"));
    assert_eq!(facts.status, RuntimeInventoryStatus::Running);
    assert_eq!(facts.raw_status, "Running");
    assert_eq!(facts.created_at, Some(ts("2026-07-01T09:00:00Z")));
    assert_eq!(facts.last_pending_at, Some(ts("2026-07-01T11:30:00Z")));
    assert_eq!(facts.deletion_timestamp, None);
    assert_eq!(
        facts.identity.attribution_source(),
        AttributionSource::LaunchMetadata
    );
}

#[test]
fn every_phase_maps_and_deletion_wins() {
    for (phase, expected) in [
        (Some("Pending"), RuntimeInventoryStatus::Pending),
        (Some("Running"), RuntimeInventoryStatus::Running),
        (Some("Succeeded"), RuntimeInventoryStatus::Succeeded),
        (Some("Failed"), RuntimeInventoryStatus::Failed),
        (Some("Unknown"), RuntimeInventoryStatus::Unknown),
        (None, RuntimeInventoryStatus::Unknown),
    ] {
        let facts = facts_from_pod(&sample_pod(phase, false), "ns");
        assert_eq!(facts.status, expected, "phase {phase:?}");
        assert_eq!(facts.raw_status, phase.unwrap_or_default());

        let terminating = facts_from_pod(&sample_pod(phase, true), "ns");
        assert_eq!(terminating.status, RuntimeInventoryStatus::Terminating);
        // The raw phase is preserved independently of the normalized override.
        assert_eq!(terminating.raw_status, phase.unwrap_or_default());
        assert_eq!(
            terminating.deletion_timestamp,
            Some(ts("2026-07-01T11:45:00Z"))
        );
    }
}

#[test]
fn restart_counts_sum_across_app_and_init_containers() {
    let mut pod = sample_pod(Some("Running"), false);
    pod.status = Some(PodStatus {
        phase: Some("Running".to_string()),
        container_statuses: Some(vec![container("app", 3, None), container("side", 2, None)]),
        init_container_statuses: Some(vec![container("init", 4, None)]),
        ..Default::default()
    });
    assert_eq!(facts_from_pod(&pod, "ns").restart_count, Some(9));
}

#[test]
fn a_negative_restart_count_cannot_reduce_the_total() {
    // Impossible from a real kubelet; a hostile response must not be able to hide
    // restarts by contributing a negative summand.
    let mut pod = sample_pod(Some("Running"), false);
    pod.status = Some(PodStatus {
        phase: Some("Running".to_string()),
        container_statuses: Some(vec![
            container("app", 5, None),
            container("evil", -100, None),
        ]),
        ..Default::default()
    });
    assert_eq!(facts_from_pod(&pod, "ns").restart_count, Some(5));
}

#[test]
fn an_extreme_restart_count_saturates_instead_of_overflowing() {
    let mut pod = sample_pod(Some("Running"), false);
    pod.status = Some(PodStatus {
        phase: Some("Running".to_string()),
        container_statuses: Some(vec![
            container("a", i32::MAX, None),
            container("b", i32::MAX, None),
            container("c", i32::MAX, None),
        ]),
        ..Default::default()
    });
    let total = facts_from_pod(&pod, "ns").restart_count.expect("count");
    assert!(total >= i32::MAX as u32);
}

#[test]
fn a_waiting_container_supplies_the_operational_reason_and_message() {
    let mut pod = sample_pod(Some("Pending"), false);
    pod.status = Some(PodStatus {
        phase: Some("Pending".to_string()),
        reason: Some("PodPending".to_string()),
        container_statuses: Some(vec![container(
            "app",
            0,
            Some(ContainerState {
                waiting: Some(ContainerStateWaiting {
                    reason: Some("ImagePullBackOff".to_string()),
                    message: Some("Back-off pulling image".to_string()),
                }),
                ..Default::default()
            }),
        )]),
        ..Default::default()
    });
    let facts = facts_from_pod(&pod, "ns");
    // Container detail beats the pod-level summary: it is what explains the stall.
    assert_eq!(facts.status_reason.as_deref(), Some("ImagePullBackOff"));
    assert_eq!(
        facts.status_message.as_deref(),
        Some("Back-off pulling image")
    );
}

#[test]
fn a_terminated_container_supplies_reason_message_and_transition() {
    let mut pod = sample_pod(Some("Failed"), false);
    pod.status = Some(PodStatus {
        phase: Some("Failed".to_string()),
        container_statuses: Some(vec![container(
            "app",
            0,
            Some(ContainerState {
                terminated: Some(ContainerStateTerminated {
                    reason: Some("Error".to_string()),
                    message: Some("exit 1".to_string()),
                    started_at: Some(Time(ts("2026-07-01T09:01:00Z"))),
                    finished_at: Some(Time(ts("2026-07-01T10:15:00Z"))),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )]),
        ..Default::default()
    });
    let facts = facts_from_pod(&pod, "ns");
    assert_eq!(facts.status_reason.as_deref(), Some("Error"));
    assert_eq!(facts.status_message.as_deref(), Some("exit 1"));
    assert_eq!(facts.last_transition_at, Some(ts("2026-07-01T10:15:00Z")));
}

#[test]
fn the_pod_level_summary_is_used_when_no_container_explains_itself() {
    let mut pod = sample_pod(Some("Failed"), false);
    pod.status = Some(PodStatus {
        phase: Some("Failed".to_string()),
        reason: Some("Evicted".to_string()),
        message: Some("The node was low on resource".to_string()),
        container_statuses: Some(vec![container(
            "app",
            0,
            Some(ContainerState {
                running: Some(ContainerStateRunning {
                    started_at: Some(Time(ts("2026-07-01T09:05:00Z"))),
                }),
                ..Default::default()
            }),
        )]),
        ..Default::default()
    });
    let facts = facts_from_pod(&pod, "ns");
    assert_eq!(facts.status_reason.as_deref(), Some("Evicted"));
    assert_eq!(
        facts.status_message.as_deref(),
        Some("The node was low on resource")
    );
}

#[test]
fn the_latest_transition_is_the_max_over_conditions_and_container_state() {
    let mut pod = sample_pod(Some("Running"), false);
    pod.status = Some(PodStatus {
        phase: Some("Running".to_string()),
        conditions: Some(vec![
            PodCondition {
                type_: "Initialized".to_string(),
                status: "True".to_string(),
                last_transition_time: Some(Time(ts("2026-07-01T09:02:00Z"))),
                ..Default::default()
            },
            PodCondition {
                type_: "Ready".to_string(),
                status: "True".to_string(),
                last_transition_time: Some(Time(ts("2026-07-01T09:10:00Z"))),
                ..Default::default()
            },
        ]),
        container_statuses: Some(vec![container(
            "app",
            0,
            Some(ContainerState {
                running: Some(ContainerStateRunning {
                    started_at: Some(Time(ts("2026-07-01T09:06:00Z"))),
                }),
                ..Default::default()
            }),
        )]),
        ..Default::default()
    });
    assert_eq!(
        facts_from_pod(&pod, "ns").last_transition_at,
        Some(ts("2026-07-01T09:10:00Z"))
    );
}

#[test]
fn a_status_free_pod_reports_no_transition_rather_than_guessing() {
    let mut pod = sample_pod(None, false);
    pod.status = None;
    let facts = facts_from_pod(&pod, "ns");
    assert_eq!(facts.last_transition_at, None);
    assert_eq!(facts.restart_count, Some(0));
    assert_eq!(facts.status, RuntimeInventoryStatus::Unknown);
}

#[test]
fn a_missing_creation_timestamp_is_not_defaulted_to_now() {
    let mut pod = sample_pod(Some("Running"), false);
    pod.metadata.creation_timestamp = None;
    let facts = facts_from_pod(&pod, "ns");
    assert_eq!(facts.created_at, None);
    assert!(!facts.created_at_malformed);
}

#[test]
fn a_malformed_last_pending_annotation_is_flagged() {
    let mut pod = sample_pod(Some("Running"), false);
    pod.metadata
        .annotations
        .as_mut()
        .expect("annotations")
        .insert(
            ANNOTATION_LAST_PENDING_AT.to_string(),
            "yesterday".to_string(),
        );
    let facts = facts_from_pod(&pod, "ns");
    assert_eq!(facts.last_pending_at, None);
    assert!(facts.last_pending_malformed);
}

#[test]
fn an_absent_last_pending_annotation_is_not_malformed() {
    let mut pod = sample_pod(Some("Running"), false);
    pod.metadata
        .annotations
        .as_mut()
        .expect("annotations")
        .remove(ANNOTATION_LAST_PENDING_AT);
    let facts = facts_from_pod(&pod, "ns");
    assert_eq!(facts.last_pending_at, None);
    assert!(!facts.last_pending_malformed);
}

#[test]
fn an_orphan_pod_with_no_session_label_is_still_projected() {
    let mut pod = sample_pod(Some("Running"), false);
    pod.metadata.labels = None;
    pod.metadata.annotations = None;
    let facts = facts_from_pod(&pod, "ns");
    assert_eq!(facts.session_id, None);
    assert_eq!(facts.runtime_id, "fkst-sess-sess-1");
    assert!(facts.managed);
}

#[test]
fn a_nameless_pod_falls_back_to_its_uid_and_then_to_a_placeholder() {
    let mut pod = sample_pod(Some("Running"), false);
    pod.metadata.name = None;
    assert_eq!(facts_from_pod(&pod, "ns").runtime_id, "uid-1");
    pod.metadata.uid = None;
    assert_eq!(facts_from_pod(&pod, "ns").runtime_id, UNNAMED_RUNTIME_ID);
}
