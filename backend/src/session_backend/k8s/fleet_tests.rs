//! Unit tests for the fleet projection: `pod_to_handle`, `repo_key_from_pod`, and
//! `trigger_issue_from_pod` (the trigger-issue reader relocated verbatim from
//! `k8s/health_scrape_tests.rs`). The live LIST needs a cluster and is live-verified;
//! here we pin the pure pod → [`SessionHandle`] mappings.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::*;

fn pod_with_annotations(pairs: &[(&str, &str)]) -> Pod {
    let annotations = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<BTreeMap<_, _>>();
    Pod {
        metadata: ObjectMeta {
            annotations: Some(annotations),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A fully-stamped substrate-session pod for repo `acme/site`, installation 42,
/// session `sess-1`, trigger issue 7.
fn full_pod() -> Pod {
    let labels = BTreeMap::from([(SESSION_ID_LABEL.to_string(), "sess-1".to_string())]);
    let annotations = BTreeMap::from([
        (ANNOTATION_OWNER.to_string(), "acme".to_string()),
        (ANNOTATION_REPO.to_string(), "site".to_string()),
        (ANNOTATION_INSTALLATION.to_string(), "42".to_string()),
        (ANNOTATION_TRIGGER_ISSUE.to_string(), "7".to_string()),
    ]);
    Pod {
        metadata: ObjectMeta {
            labels: Some(labels),
            annotations: Some(annotations),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn pod_to_handle_projects_every_field() {
    let handle = pod_to_handle(&full_pod()).expect("projects");
    assert_eq!(handle.session_id, "sess-1");
    assert_eq!(handle.installation_id, 42);
    assert_eq!(handle.repo.owner, "acme");
    assert_eq!(handle.repo.name, "site");
    assert_eq!(handle.trigger_issue, Some(7));
}

#[test]
fn pod_to_handle_is_none_without_a_session_id_label() {
    let mut pod = full_pod();
    pod.metadata.labels = Some(BTreeMap::new());
    assert!(pod_to_handle(&pod).is_none());
}

#[test]
fn pod_to_handle_is_none_without_a_repo_key() {
    let mut pod = full_pod();
    // Drop the installation annotation → the repo key no longer resolves.
    pod.metadata.annotations = Some(BTreeMap::from([
        (ANNOTATION_OWNER.to_string(), "acme".to_string()),
        (ANNOTATION_REPO.to_string(), "site".to_string()),
    ]));
    assert!(pod_to_handle(&pod).is_none());
}

#[test]
fn pod_to_handle_tolerates_a_missing_trigger_issue() {
    let mut pod = full_pod();
    pod.metadata.annotations = Some(BTreeMap::from([
        (ANNOTATION_OWNER.to_string(), "acme".to_string()),
        (ANNOTATION_REPO.to_string(), "site".to_string()),
        (ANNOTATION_INSTALLATION.to_string(), "42".to_string()),
    ]));
    let handle = pod_to_handle(&pod).expect("projects without a trigger issue");
    assert_eq!(handle.trigger_issue, None);
}

#[test]
fn repo_key_from_pod_reads_owner_repo_and_installation() {
    let (installation, repo) = repo_key_from_pod(&full_pod()).expect("resolves");
    assert_eq!(installation, 42);
    assert_eq!(repo.owner, "acme");
    assert_eq!(repo.name, "site");
}

#[test]
fn repo_key_from_pod_is_none_when_an_annotation_is_missing_or_unparseable() {
    assert!(repo_key_from_pod(&pod_with_annotations(&[])).is_none());
    assert!(repo_key_from_pod(&pod_with_annotations(&[
        (ANNOTATION_OWNER, "acme"),
        (ANNOTATION_REPO, "site"),
        (ANNOTATION_INSTALLATION, "not-a-number"),
    ]))
    .is_none());
}

#[test]
fn trigger_issue_reads_the_stamped_annotation() {
    let pod = pod_with_annotations(&[(ANNOTATION_TRIGGER_ISSUE, "123")]);
    assert_eq!(trigger_issue_from_pod(&pod), Some(123));
}

#[test]
fn trigger_issue_is_none_when_missing_zero_or_unparseable() {
    assert_eq!(trigger_issue_from_pod(&pod_with_annotations(&[])), None);
    assert_eq!(
        trigger_issue_from_pod(&pod_with_annotations(&[(ANNOTATION_TRIGGER_ISSUE, "0")])),
        None
    );
    assert_eq!(
        trigger_issue_from_pod(&pod_with_annotations(&[(ANNOTATION_TRIGGER_ISSUE, "nan")])),
        None
    );
}
