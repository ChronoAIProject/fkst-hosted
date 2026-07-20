//! Unit tests for the lifecycle verbs' PURE argument-assembly + pod → [`LivePod`]
//! projection (relocated from `reconcile/repo_tests.rs` and the pod effect assembly
//! in `reconcile/execute_tests.rs`, unchanged). The live LIST/patch/delete/create
//! wiring needs a cluster and is live-verified; here we cover the load-bearing pure
//! mappings the backend does off a sample `Pod`.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{Pod, PodStatus};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
use k8s_openapi::chrono::{DateTime, Utc};

use super::*;

fn ts(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .expect("rfc3339")
        .with_timezone(&Utc)
}

/// Build a substrate-session pod with the given phase/deletion + a full annotation
/// set for repo `acme/site`, session `sess-1`, trigger issue 7.
fn sample_pod(phase: Option<&str>, terminating: bool) -> Pod {
    let labels = BTreeMap::from([(SESSION_ID_LABEL.to_string(), "sess-1".to_string())]);
    let annotations = BTreeMap::from([
        (ANNOTATION_OWNER.to_string(), "acme".to_string()),
        (ANNOTATION_REPO.to_string(), "site".to_string()),
        (ANNOTATION_TRIGGER_ISSUE.to_string(), "7".to_string()),
        (ANNOTATION_CONFIG_HASH.to_string(), "hash-xyz".to_string()),
        (ANNOTATION_WORK_LABEL.to_string(), "fkst-run".to_string()),
        (
            ANNOTATION_LAST_PENDING_AT.to_string(),
            "2026-07-01T10:00:00+00:00".to_string(),
        ),
    ]);
    Pod {
        metadata: ObjectMeta {
            name: Some("fkst-sess-sess-1".to_string()),
            labels: Some(labels),
            annotations: Some(annotations),
            creation_timestamp: Some(Time(ts("2026-07-01T09:00:00Z"))),
            deletion_timestamp: terminating.then(|| Time(ts("2026-07-01T10:30:00Z"))),
            ..Default::default()
        },
        status: Some(PodStatus {
            phase: phase.map(str::to_string),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn maps_a_running_pod_to_a_live_pod() {
    let live = pod_to_live(&sample_pod(Some("Running"), false)).expect("maps");
    assert_eq!(live.session_id, "sess-1");
    assert_eq!(live.trigger_issue, 7);
    assert_eq!(live.liveness, PodLiveness::Live);
    assert_eq!(live.created_at, ts("2026-07-01T09:00:00Z"));
    assert_eq!(live.last_pending_at, Some(ts("2026-07-01T10:00:00Z")));
    assert_eq!(live.config_hash.as_deref(), Some("hash-xyz"));
    // The work-label annotation is carried so an orphaned pod can retire-notify.
    assert_eq!(live.work_labels, vec!["fkst-run".to_string()]);
}

#[test]
fn maps_a_comma_joined_work_label_annotation_to_the_full_set() {
    // A multi-label session (epic #594 I4) records its effective set comma-joined; the
    // projection splits it back so the planner can retire across every label.
    let mut pod = sample_pod(Some("Running"), false);
    pod.metadata
        .annotations
        .as_mut()
        .unwrap()
        .insert(ANNOTATION_WORK_LABEL.to_string(), "alpha,beta".to_string());
    let live = pod_to_live(&pod).expect("maps");
    assert_eq!(
        live.work_labels,
        vec!["alpha".to_string(), "beta".to_string()]
    );
}

#[test]
fn phase_projections_cover_the_matrix() {
    assert_eq!(
        phase_to_liveness(Some("Pending"), false),
        PodLiveness::Starting
    );
    assert_eq!(phase_to_liveness(Some("Running"), false), PodLiveness::Live);
    assert_eq!(
        phase_to_liveness(Some("Succeeded"), false),
        PodLiveness::Terminal
    );
    assert_eq!(
        phase_to_liveness(Some("Failed"), false),
        PodLiveness::Terminal
    );
    // Unknown / not-yet-set → Starting (not observed running).
    assert_eq!(
        phase_to_liveness(Some("Unknown"), false),
        PodLiveness::Starting
    );
    assert_eq!(phase_to_liveness(None, false), PodLiveness::Starting);
    // A set deletionTimestamp always wins.
    assert_eq!(
        phase_to_liveness(Some("Running"), true),
        PodLiveness::Terminating
    );
}

#[test]
fn a_deleting_pod_is_terminating_regardless_of_phase() {
    let live = pod_to_live(&sample_pod(Some("Running"), true)).expect("maps");
    assert_eq!(live.liveness, PodLiveness::Terminating);
}

#[test]
fn a_pod_without_a_session_id_label_is_skipped() {
    let mut pod = sample_pod(Some("Running"), false);
    pod.metadata.labels = Some(BTreeMap::new());
    assert!(pod_to_live(&pod).is_none(), "no session-id label → skipped");
}

#[test]
fn missing_last_pending_and_config_hash_map_to_none() {
    let mut pod = sample_pod(Some("Pending"), false);
    pod.metadata.annotations = Some(BTreeMap::from([
        (ANNOTATION_OWNER.to_string(), "acme".to_string()),
        (ANNOTATION_REPO.to_string(), "site".to_string()),
        (ANNOTATION_TRIGGER_ISSUE.to_string(), "7".to_string()),
    ]));
    let live = pod_to_live(&pod).expect("maps");
    assert_eq!(live.last_pending_at, None);
    assert_eq!(live.config_hash, None);
    // An older pod predating the work-label annotation carries no label to retire.
    assert!(live.work_labels.is_empty());
    assert_eq!(live.liveness, PodLiveness::Starting);
}

#[test]
fn repo_filter_matches_on_owner_and_name_annotations() {
    let pod = sample_pod(Some("Running"), false);
    assert!(pod_matches_repo(
        &pod,
        &RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string()
        }
    ));
    assert!(!pod_matches_repo(
        &pod,
        &RepoRef {
            owner: "acme".to_string(),
            name: "other".to_string()
        }
    ));
}

#[test]
fn kill_delete_params_carries_the_grace_period() {
    let params = kill_delete_params(60);
    assert_eq!(params.grace_period_seconds, Some(60));
    // A zero grace is legitimate (immediate SIGKILL) and must be honoured.
    assert_eq!(kill_delete_params(0).grace_period_seconds, Some(0));
}

#[test]
fn last_pending_patch_sets_the_annotation_key_to_now() {
    let now = DateTime::parse_from_rfc3339("2026-07-01T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let patch = last_pending_patch(now);
    let value = &patch["metadata"]["annotations"][ANNOTATION_LAST_PENDING_AT];
    assert_eq!(value.as_str().unwrap(), now.to_rfc3339());
}
