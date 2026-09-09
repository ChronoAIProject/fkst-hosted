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

// --- Durable creator/trigger attribution (issue #5673) -----------------------

#[test]
fn the_identity_merge_patch_touches_metadata_annotations_and_nothing_else() {
    // Attribution must never be able to restart a session, so the patch may not
    // name a single field outside `metadata.annotations`.
    let keys = crate::runtime_identity::K8S_IDENTITY_KEYS;
    let patch = identity_merge_patch(&[
        (keys.schema, "1".to_string()),
        (keys.creator_login, "alice".to_string()),
    ]);
    let object = patch.as_object().expect("an object patch");
    assert_eq!(object.len(), 1);
    let metadata = patch["metadata"].as_object().expect("metadata");
    assert_eq!(metadata.len(), 1);
    assert_eq!(patch["metadata"]["annotations"][keys.schema], "1");
    assert_eq!(
        patch["metadata"]["annotations"][keys.creator_login],
        "alice"
    );
    assert!(patch.get("spec").is_none());
}

#[test]
fn the_identity_merge_patch_carries_only_the_absent_keys() {
    // A JSON merge patch replaces every key it names, so naming a key that is
    // already present is how an accidental overwrite happens.
    let keys = crate::runtime_identity::K8S_IDENTITY_KEYS;
    let patch = identity_merge_patch(&[(keys.creator_id, "42".to_string())]);
    let annotations = patch["metadata"]["annotations"]
        .as_object()
        .expect("annotations");
    assert_eq!(annotations.len(), 1);
    assert!(!annotations.contains_key(keys.creator_login));
}

#[test]
fn a_pod_projection_recovers_its_identity_stamp() {
    let keys = crate::runtime_identity::K8S_IDENTITY_KEYS;
    let mut pod = sample_pod(Some("Running"), false);
    let annotations = pod.metadata.annotations.as_mut().expect("annotations");
    annotations.insert(keys.schema.to_string(), "1".to_string());
    annotations.insert(keys.creator_id.to_string(), "4242".to_string());
    annotations.insert(keys.creator_login.to_string(), "alice".to_string());
    annotations.insert(keys.trigger_author_id.to_string(), "77".to_string());
    annotations.insert(keys.trigger_author_login.to_string(), "octocat".to_string());

    let live = pod_to_live(&pod).expect("maps");
    assert_eq!(live.identity.creator_id, Some(4242));
    assert_eq!(live.identity.creator_login.as_deref(), Some("alice"));
    assert_eq!(
        live.identity.attribution_source(),
        crate::runtime_identity::AttributionSource::LaunchMetadata
    );
}

#[test]
fn a_pod_carrying_the_durable_conflict_marker_projects_a_conflict() {
    // The marker survives the process that wrote it, so a pass that never
    // compared this stamp against anything still reports the dispute.
    let keys = crate::runtime_identity::K8S_IDENTITY_KEYS;
    let mut pod = sample_pod(Some("Running"), false);
    let annotations = pod.metadata.annotations.as_mut().expect("annotations");
    annotations.insert(keys.schema.to_string(), "1".to_string());
    annotations.insert(keys.creator_id.to_string(), "4242".to_string());
    annotations.insert(keys.creator_login.to_string(), "alice".to_string());
    annotations.insert(keys.trigger_author_id.to_string(), "77".to_string());
    annotations.insert(keys.trigger_author_login.to_string(), "octocat".to_string());
    annotations.insert(keys.conflict.to_string(), "creator-id".to_string());

    let live = pod_to_live(&pod).expect("maps");
    assert!(live.identity.conflicting);
    assert_eq!(
        live.identity.attribution_source(),
        crate::runtime_identity::AttributionSource::Conflict
    );
}

#[test]
fn a_pod_predating_the_stamp_projects_an_unknown_legacy_identity() {
    let live = pod_to_live(&sample_pod(Some("Running"), false)).expect("maps");
    assert!(live.identity.is_empty());
    assert_eq!(
        live.identity.attribution_source(),
        crate::runtime_identity::AttributionSource::UnknownLegacy,
        "a legacy runtime is honestly unknown, never guessed from the repository"
    );
}
