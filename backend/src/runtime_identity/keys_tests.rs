//! Round-trip tests for both key sets.
//!
//! These are the drift guard the issue asks for: the exact key STRINGS are
//! pinned (a rename silently orphans every already-stamped runtime), and every
//! stamp is proved to read back through the same key set unchanged.

use super::*;

fn identity(creator_id: Option<i64>) -> RuntimeIdentityMetadata {
    RuntimeIdentityMetadata::new(creator_id, "Alice", 77, "fkst-cloud[bot]")
}

fn stamped(keys: &IdentityKeys, identity: &RuntimeIdentityMetadata) -> BTreeMap<String, String> {
    stamp_pairs(keys, identity)
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

#[test]
fn the_kubernetes_annotation_keys_are_the_documented_ones() {
    // Pinned verbatim: these strings live on runtimes that outlive this process,
    // so a rename orphans attribution that can never be recovered.
    assert_eq!(
        K8S_IDENTITY_KEYS.all(),
        [
            "fkst.chrono-ai.fun/identity-schema-version",
            "fkst.chrono-ai.fun/creator-id",
            "fkst.chrono-ai.fun/creator-login",
            "fkst.chrono-ai.fun/trigger-author-id",
            "fkst.chrono-ai.fun/trigger-author-login",
        ]
    );
}

#[test]
fn the_opensandbox_metadata_keys_are_the_documented_ones() {
    assert_eq!(
        OSB_IDENTITY_KEYS.all(),
        [
            "fkst-identity-schema",
            "fkst-creator-id",
            "fkst-creator-login",
            "fkst-trigger-author-id",
            "fkst-trigger-author-login",
        ]
    );
}

#[test]
fn a_human_creator_round_trips_through_both_key_sets() {
    for keys in [&K8S_IDENTITY_KEYS, &OSB_IDENTITY_KEYS] {
        let identity = identity(Some(4242));
        let metadata = stamped(keys, &identity);
        let observed = read(keys, &metadata);

        assert_eq!(observed.schema_version.as_deref(), Some("1"));
        assert_eq!(observed.creator_id, Some(4242));
        assert_eq!(observed.creator_login.as_deref(), Some("alice"));
        assert_eq!(observed.trigger_author_id, Some(77));
        assert_eq!(observed.trigger_author_login.as_deref(), Some("fkst-cloud"));
        assert!(!observed.malformed);
    }
}

#[test]
fn an_assignee_derived_creator_round_trips_with_the_id_key_absent() {
    for keys in [&K8S_IDENTITY_KEYS, &OSB_IDENTITY_KEYS] {
        let metadata = stamped(keys, &identity(None));
        assert!(
            !metadata.contains_key(keys.creator_id),
            "an unavailable creator id is an ABSENT key, never an empty or placeholder value"
        );
        let observed = read(keys, &metadata);
        assert_eq!(observed.creator_id, None);
        assert_eq!(observed.creator_login.as_deref(), Some("alice"));
        assert_eq!(
            observed.trigger_author_id,
            Some(77),
            "the trigger author's id is stamped in its own key and never doubles as the creator's"
        );
    }
}

#[test]
fn the_two_key_sets_stamp_identical_values_under_different_names() {
    // The whole point of one renderer: swapping a session's runtime backend must
    // not change what its attribution SAYS.
    let identity = identity(Some(1));
    let k8s = stamped(&K8S_IDENTITY_KEYS, &identity);
    let osb = stamped(&OSB_IDENTITY_KEYS, &identity);
    let k8s_values: Vec<&String> = k8s.values().collect();
    let osb_values: Vec<&String> = osb.values().collect();
    assert_eq!(k8s.len(), osb.len());
    assert_eq!(
        k8s_values.iter().collect::<std::collections::BTreeSet<_>>(),
        osb_values.iter().collect::<std::collections::BTreeSet<_>>()
    );
}

#[test]
fn a_non_numeric_id_reads_as_malformed_rather_than_absent() {
    let mut metadata = stamped(&K8S_IDENTITY_KEYS, &identity(Some(1)));
    metadata.insert(
        K8S_IDENTITY_KEYS.creator_id.to_string(),
        "not-a-number".to_string(),
    );
    let observed = read(&K8S_IDENTITY_KEYS, &metadata);
    assert_eq!(observed.creator_id, None);
    assert!(
        observed.malformed,
        "a corrupted id must be distinguishable from the legitimate missing one"
    );
}

#[test]
fn a_blank_login_reads_as_absent() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        K8S_IDENTITY_KEYS.creator_login.to_string(),
        "   ".to_string(),
    );
    let observed = read(&K8S_IDENTITY_KEYS, &metadata);
    assert_eq!(observed.creator_login, None);
    assert!(observed.is_empty());
}

#[test]
fn an_empty_login_is_never_stamped() {
    // A blank login would be an invalid Kubernetes label value AND a claim to
    // know something we do not.
    let identity = RuntimeIdentityMetadata::new(Some(1), "", 77, "");
    let metadata = stamped(&OSB_IDENTITY_KEYS, &identity);
    assert!(!metadata.contains_key(OSB_IDENTITY_KEYS.creator_login));
    assert!(!metadata.contains_key(OSB_IDENTITY_KEYS.trigger_author_login));
    assert_eq!(metadata[OSB_IDENTITY_KEYS.schema], IDENTITY_SCHEMA_VERSION);
    assert_eq!(metadata[OSB_IDENTITY_KEYS.trigger_author_id], "77");
}

#[test]
fn every_stamped_value_is_a_valid_kubernetes_label_value() {
    // OpenSandbox metadata values obey the label-value contract, so a stamp that
    // violated it would fail the create. Normalization is what makes even an App
    // author login safe.
    let metadata = stamped(&OSB_IDENTITY_KEYS, &identity(Some(4242)));
    for value in metadata.values() {
        assert!(!value.is_empty() && value.len() <= 63, "{value}");
        let bytes = value.as_bytes();
        assert!(bytes[0].is_ascii_alphanumeric(), "{value}");
        assert!(bytes[bytes.len() - 1].is_ascii_alphanumeric(), "{value}");
        assert!(
            bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
            "{value}"
        );
    }
}
