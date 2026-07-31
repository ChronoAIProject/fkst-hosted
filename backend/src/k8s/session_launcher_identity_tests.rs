//! Durable creator/trigger attribution on a session Pod (issue #5673).
//!
//! Two properties carry the whole contract: the stamp lands as ANNOTATIONS (not
//! labels, which are what the reconciler selects on) and it never leaks into the
//! pod environment, where a session or its packages could read and act on it.

use k8s_openapi::api::core::v1::Pod;

use super::tests::{config, spec};
use super::{build_session_pod, session_env_pairs};

/// Read a pod annotation, or fail with the key that was missing.
fn annotation<'a>(pod: &'a Pod, key: &str) -> Option<&'a str> {
    pod.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(key))
        .map(String::as_str)
}

#[test]
fn a_session_pod_is_stamped_with_the_identity_annotations_and_its_provenance() {
    let pod = build_session_pod(&spec(), &config()).expect("pod builds");
    let keys = crate::runtime_identity::K8S_IDENTITY_KEYS;
    assert_eq!(annotation(&pod, keys.schema), Some("1"));
    assert_eq!(annotation(&pod, keys.creator_id), Some("4242"));
    assert_eq!(annotation(&pod, keys.creator_login), Some("author-login"));
    assert_eq!(annotation(&pod, keys.trigger_author_id), Some("4242"));
    assert_eq!(
        annotation(&pod, keys.trigger_author_login),
        Some("author-login")
    );
    assert_eq!(
        annotation(&pod, keys.source),
        Some("launch_metadata"),
        "only a launch writer may claim launch provenance; a later backfill stamps \
         `backfilled_current_trigger` instead"
    );
}

#[test]
fn attribution_is_stamped_as_annotations_and_never_as_labels() {
    // Labels are what the reconciler SELECTS on; attribution is what it
    // DISPLAYS. Promoting it would add one apiserver label-index value per
    // creator for a query nobody issues.
    let pod = build_session_pod(&spec(), &config()).expect("pod builds");
    let labels = pod.metadata.labels.as_ref().expect("labels");
    for key in crate::runtime_identity::K8S_IDENTITY_KEYS.all() {
        assert!(!labels.contains_key(key), "{key} leaked into the selector");
    }
    for value in labels.values() {
        assert!(value != "author-login" && value != "4242", "{value}");
    }
}

#[test]
fn an_assignee_derived_creator_omits_the_creator_id_annotation() {
    let mut spec = spec();
    spec.creator_id = None;
    spec.creator_login = "assignee".to_string();
    let pod = build_session_pod(&spec, &config()).expect("pod builds");
    let keys = crate::runtime_identity::K8S_IDENTITY_KEYS;
    assert_eq!(annotation(&pod, keys.creator_id), None);
    assert_eq!(annotation(&pod, keys.creator_login), Some("assignee"));
    assert_eq!(
        annotation(&pod, keys.trigger_author_id),
        Some("4242"),
        "the trigger author's id is never reused as the creator's"
    );
}

#[test]
fn an_app_authored_trigger_stamps_a_normalized_author_login() {
    let mut spec = spec();
    spec.trigger_author_login = "fkst-cloud[bot]".to_string();
    let pod = build_session_pod(&spec, &config()).expect("pod builds");
    assert_eq!(
        annotation(
            &pod,
            crate::runtime_identity::K8S_IDENTITY_KEYS.trigger_author_login
        ),
        Some("fkst-cloud"),
        "the same value an OpenSandbox metadata stamp must accept"
    );
}

#[test]
fn identity_never_reaches_the_pod_environment() {
    // A creator id is attribution, not configuration: it must not appear as an
    // env var the session or its packages could read or act on.
    let rendered = session_env_pairs(&spec(), &config());
    for (key, value) in &rendered {
        assert!(
            !key.contains("CREATOR_ID") && !key.contains("TRIGGER_AUTHOR"),
            "{key} leaked identity into the pod env"
        );
        assert!(value != "4242", "{key} carries the raw creator id");
    }
}
