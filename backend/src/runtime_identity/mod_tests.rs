//! Unit tests for the attribution value type: login normalization (which is what
//! keeps a Kubernetes annotation and an OpenSandbox metadata value the same
//! string), and the attribution-source classification a viewer is shown.

use super::*;

fn stamped(
    schema: bool,
    creator_id: Option<i64>,
    creator_login: Option<&str>,
) -> ObservedRuntimeIdentity {
    ObservedRuntimeIdentity {
        schema_version: schema.then(|| "1".to_string()),
        creator_id,
        creator_login: creator_login.map(str::to_string),
        trigger_author_id: Some(9),
        trigger_author_login: Some("octocat".to_string()),
        source: Some(SOURCE_LAUNCH_METADATA.to_string()),
        conflicting: false,
        malformed: false,
    }
}

#[test]
fn an_app_author_login_normalizes_to_a_label_safe_slug() {
    // `[` and `]` are not valid Kubernetes label-value characters, so an
    // un-normalized App login would make every seeded session's sandbox create
    // fail. Both GitHub renderings collapse to the same slug.
    assert_eq!(normalize_identity_login("fkst-cloud[bot]"), "fkst-cloud");
    assert_eq!(normalize_identity_login("app/fkst-cloud"), "fkst-cloud");
    assert_eq!(normalize_identity_login("  Octocat  "), "octocat");
}

#[test]
fn a_human_login_normalizes_to_a_stable_comparable_form() {
    // Case folding matches the repository's existing identity comparison, so a
    // stamp and a later comparison can never disagree on the same person.
    assert_eq!(normalize_identity_login("Shining"), "shining");
    assert_eq!(normalize_identity_login(""), "");
}

#[test]
fn metadata_construction_normalizes_both_logins_and_preserves_a_missing_creator_id() {
    let identity = RuntimeIdentityMetadata::new(None, "Alice", 77, "fkst-cloud[bot]");
    assert_eq!(identity.creator_login, "alice");
    assert_eq!(identity.trigger_author_login, "fkst-cloud");
    assert_eq!(identity.trigger_author_id, 77);
    assert_eq!(
        identity.creator_id, None,
        "an assignee-derived creator keeps its explicitly missing id; the trigger author's id is never borrowed"
    );
}

#[test]
fn a_complete_stamp_reads_as_launch_metadata_with_or_without_a_creator_id() {
    assert_eq!(
        stamped(true, Some(1), Some("alice")).attribution_source(),
        AttributionSource::LaunchMetadata
    );
    // The assignee-derived case is COMPLETE, not partial: the missing id is the
    // recorded fact, not a gap.
    assert_eq!(
        stamped(true, None, Some("alice")).attribution_source(),
        AttributionSource::LaunchMetadata
    );
}

#[test]
fn a_backfilled_stamp_never_claims_launch_provenance() {
    // The two write identical attribution keys, so only the durable marker can
    // separate them — and unlike the reconciler's in-memory knowledge, the
    // marker survives a restart.
    let mut observed = stamped(true, Some(1), Some("alice"));
    observed.source = Some(SOURCE_BACKFILLED_CURRENT_TRIGGER.to_string());
    assert_eq!(
        observed.attribution_source(),
        AttributionSource::BackfilledCurrentTrigger
    );
}

#[test]
fn a_stamp_with_no_marker_at_all_reads_as_launch_metadata() {
    // Only a launch writer can produce that shape: the backfill path has always
    // written a marker.
    let mut observed = stamped(true, Some(1), Some("alice"));
    observed.source = None;
    assert_eq!(
        observed.attribution_source(),
        AttributionSource::LaunchMetadata
    );
}

#[test]
fn an_unstamped_runtime_reads_as_unknown_legacy() {
    assert_eq!(
        ObservedRuntimeIdentity::default().attribution_source(),
        AttributionSource::UnknownLegacy
    );
    assert!(ObservedRuntimeIdentity::default().is_empty());
}

#[test]
fn a_half_stamped_runtime_reads_as_partial_metadata() {
    // Schema present but the creator login missing: something was written, but
    // not by a control plane that knew the whole contract.
    assert_eq!(
        stamped(true, Some(1), None).attribution_source(),
        AttributionSource::PartialMetadata
    );
    // Values present with no schema key at all: a legacy or foreign writer.
    assert_eq!(
        stamped(false, Some(1), Some("alice")).attribution_source(),
        AttributionSource::PartialMetadata
    );
}

#[test]
fn a_malformed_id_never_reads_as_a_complete_launch_stamp() {
    let mut observed = stamped(true, None, Some("alice"));
    observed.malformed = true;
    assert_eq!(
        observed.attribution_source(),
        AttributionSource::PartialMetadata,
        "a corrupted id must not be mistaken for the legitimate assignee-derived missing id"
    );
}

#[test]
fn a_recorded_disagreement_wins_over_every_other_classification() {
    let mut observed = stamped(true, Some(1), Some("alice"));
    observed.conflicting = true;
    assert_eq!(
        observed.attribution_source(),
        AttributionSource::Conflict,
        "a conflict is never hidden behind an otherwise complete-looking stamp"
    );
}

#[test]
fn every_bounded_label_string_is_stable() {
    // These strings are metric label values and PostHog property values; a
    // rename is a dashboard break, so they are pinned.
    assert_eq!(RuntimeBackendKind::Kubernetes.as_str(), "kubernetes");
    assert_eq!(RuntimeBackendKind::OpenSandbox.as_str(), "opensandbox");
    assert_eq!(
        AttributionSource::BackfilledCurrentTrigger.as_str(),
        "backfilled_current_trigger"
    );
    assert_eq!(AttributionSource::UnknownLegacy.as_str(), "unknown_legacy");
    assert_eq!(RuntimeIdentityOutcome::NotFound.as_str(), "not_found");
    // Dense indices must stay unique, or two backends would share a counter.
    let indices: Vec<usize> = RuntimeBackendKind::ALL.iter().map(|b| b.index()).collect();
    assert_eq!(indices, vec![0, 1]);
}
