//! The shared projection: identity round-trip, metadata state, correlation
//! parsing, and the promise that a managed runtime is never dropped.

use std::collections::BTreeMap;

use crate::runtime_identity::{
    stamp_pairs, AttributionSource, ObservedRuntimeIdentity, RuntimeIdentityMetadata,
    IDENTITY_SCHEMA_VERSION, K8S_IDENTITY_KEYS, SOURCE_BACKFILLED_CURRENT_TRIGGER,
};

use super::*;

fn ts(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .expect("rfc3339")
        .with_timezone(&Utc)
}

fn policy() -> RuntimeLifetimePolicy {
    RuntimeLifetimePolicy {
        max_lifetime_seconds: 0,
        minimum_lifetime_seconds: 120,
        idle_grace_seconds: 300,
        max_items: 5000,
        max_warnings: 256,
    }
}

/// A complete launch stamp, STAMPED and then READ BACK through the shared key
/// module, so the test exercises the real round trip rather than a hand-written
/// approximation of what a stamp looks like.
fn launch_identity() -> ObservedRuntimeIdentity {
    let identity = RuntimeIdentityMetadata::new(Some(11), "alice", 22, "carol");
    let stamped: BTreeMap<String, String> = stamp_pairs(&K8S_IDENTITY_KEYS, &identity)
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    crate::runtime_identity::read(&K8S_IDENTITY_KEYS, &stamped)
}

fn complete_facts() -> RawRuntimeFacts {
    RawRuntimeFacts {
        runtime_id: "fkst-sess-abc".to_string(),
        runtime_name: Some("fkst-sess-abc".to_string()),
        runtime_uid: Some("uid-1".to_string()),
        backend_location: Some("chronoai-fkst".to_string()),
        session_id: Some("abc".to_string()),
        managed: true,
        identity: launch_identity(),
        owner: Some("acme".to_string()),
        repo: Some("site".to_string()),
        installation_id_raw: Some("900".to_string()),
        trigger_issue_raw: Some("7".to_string()),
        status: RuntimeInventoryStatus::Running,
        raw_status: "Running".to_string(),
        created_at: Some(ts("2026-07-01T11:00:00Z")),
        ..RawRuntimeFacts::default()
    }
}

fn build(facts: RawRuntimeFacts, warnings: &mut WarningSink) -> RuntimeInventoryItem {
    build_item(
        facts,
        RuntimeBackendKind::Kubernetes,
        ts("2026-07-01T12:00:00Z"),
        &policy(),
        warnings,
    )
}

#[test]
fn a_fully_stamped_runtime_is_complete_with_launch_attribution() {
    let mut warnings = WarningSink::default();
    let item = build(complete_facts(), &mut warnings);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Complete);
    assert_eq!(item.attribution_source, AttributionSource::LaunchMetadata);
    assert_eq!(item.creator_id, Some(11));
    assert_eq!(item.creator_login.as_deref(), Some("alice"));
    assert_eq!(item.trigger_author_id, Some(22));
    assert_eq!(item.trigger_author_login.as_deref(), Some("carol"));
    assert_eq!(item.repo_full_name.as_deref(), Some("acme/site"));
    assert_eq!(item.installation_id, Some(900));
    assert_eq!(item.trigger_issue, Some(7));
    assert_eq!(item.age_seconds, Some(3600));
    assert!(warnings.is_empty(), "{:?}", warnings.into_warnings());
}

#[test]
fn a_backfilled_stamp_reports_backfilled_provenance() {
    let mut facts = complete_facts();
    facts.identity.source = Some(SOURCE_BACKFILLED_CURRENT_TRIGGER.to_string());
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(
        item.attribution_source,
        AttributionSource::BackfilledCurrentTrigger
    );
    // A backfilled stamp is still a contract stamp, so the row stays complete.
    assert_eq!(item.metadata_state, RuntimeMetadataState::Complete);
}

#[test]
fn an_unstamped_legacy_runtime_is_retained_as_unknown_legacy() {
    let mut facts = complete_facts();
    facts.identity = ObservedRuntimeIdentity::default();
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.attribution_source, AttributionSource::UnknownLegacy);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Partial);
    assert_eq!(item.creator_id, None);
    // Retained, not dropped: an orphan invisible to a global admin is worse.
    assert_eq!(item.session_id.as_deref(), Some("abc"));
}

#[test]
fn a_partial_stamp_is_partial_metadata() {
    let mut facts = complete_facts();
    facts.identity = ObservedRuntimeIdentity {
        schema_version: Some(IDENTITY_SCHEMA_VERSION.to_string()),
        creator_login: Some("alice".to_string()),
        ..ObservedRuntimeIdentity::default()
    };
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.attribution_source, AttributionSource::PartialMetadata);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Partial);
}

#[test]
fn a_conflicting_stamp_round_trips_its_conflict_state() {
    // Read back from a real metadata map carrying the durable conflict marker,
    // not a hand-set flag: the point of the marker is that a reader holding only
    // the runtime can reach this state at all.
    let identity = RuntimeIdentityMetadata::new(Some(11), "alice", 22, "carol");
    let mut stamped: BTreeMap<String, String> = stamp_pairs(&K8S_IDENTITY_KEYS, &identity)
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    stamped.insert(
        K8S_IDENTITY_KEYS.conflict.to_string(),
        "creator-id".to_string(),
    );

    let mut facts = complete_facts();
    facts.identity = crate::runtime_identity::read(&K8S_IDENTITY_KEYS, &stamped);
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.attribution_source, AttributionSource::Conflict);
    // Disputed attribution is not a COMPLETE stamp: something about it is known
    // to be wrong, even though nothing is malformed.
    assert_eq!(item.metadata_state, RuntimeMetadataState::Partial);
    // Reported verbatim — a conflict is surfaced, never healed.
    assert_eq!(item.creator_id, Some(11));
    let codes: Vec<_> = warnings.into_warnings().iter().map(|w| w.code).collect();
    assert!(
        codes.contains(&InventoryWarningCode::AttributionConflict),
        "{codes:?}"
    );
}

#[test]
fn a_malformed_identity_id_is_malformed_not_merely_partial() {
    // A corrupted stamp must never be mistaken for the legitimate
    // "assignee-derived creator has no id" state.
    let mut facts = complete_facts();
    facts.identity.malformed = true;
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Malformed);
    assert!(warnings
        .into_warnings()
        .iter()
        .any(|w| w.code == InventoryWarningCode::MalformedIdentity));
}

#[test]
fn an_unparseable_correlation_id_is_malformed_and_warned() {
    let mut facts = complete_facts();
    facts.installation_id_raw = Some("nine hundred".to_string());
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.installation_id, None);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Malformed);
    let codes: Vec<_> = warnings.into_warnings().iter().map(|w| w.code).collect();
    assert!(
        codes.contains(&InventoryWarningCode::MalformedCorrelation),
        "{codes:?}"
    );
}

#[test]
fn an_absent_correlation_id_is_partial_not_malformed() {
    let mut facts = complete_facts();
    facts.installation_id_raw = None;
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.installation_id, None);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Partial);
    assert!(warnings.is_empty());
}

#[test]
fn a_zero_trigger_issue_is_the_unknown_sentinel_not_issue_zero() {
    let mut facts = complete_facts();
    facts.trigger_issue_raw = Some("0".to_string());
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.trigger_issue, None);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Partial);
}

#[test]
fn an_orphan_without_a_session_id_is_returned_with_a_warning() {
    let mut facts = complete_facts();
    facts.session_id = None;
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.session_id, None);
    assert_eq!(item.runtime_id, "fkst-sess-abc");
    assert_eq!(item.metadata_state, RuntimeMetadataState::Partial);
    let warnings = warnings.into_warnings();
    let orphan = warnings
        .iter()
        .find(|w| w.code == InventoryWarningCode::MissingSessionId)
        .expect("orphan warning");
    assert_eq!(orphan.runtime_id.as_deref(), Some("fkst-sess-abc"));
    assert_eq!(orphan.session_id, None);
}

#[test]
fn only_half_a_repository_yields_no_full_name() {
    let mut facts = complete_facts();
    facts.repo = None;
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.repo_full_name, None);
}

#[test]
fn an_unknown_status_is_warned_and_keeps_its_raw_value() {
    let mut facts = complete_facts();
    facts.status = RuntimeInventoryStatus::Unknown;
    facts.raw_status = "Hibernating".to_string();
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.status, RuntimeInventoryStatus::Unknown);
    assert_eq!(item.raw_status, "Hibernating");
    assert!(warnings
        .into_warnings()
        .iter()
        .any(|w| w.code == InventoryWarningCode::UnknownStatus));
}

#[test]
fn a_malformed_creation_timestamp_is_distinguished_from_an_absent_one() {
    let mut malformed = complete_facts();
    malformed.created_at = None;
    malformed.created_at_malformed = true;
    let mut warnings = WarningSink::default();
    let item = build(malformed, &mut warnings);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Malformed);
    let codes: Vec<_> = warnings.into_warnings().iter().map(|w| w.code).collect();
    assert!(codes.contains(&InventoryWarningCode::MalformedCreatedAt));
    assert!(codes.contains(&InventoryWarningCode::MissingCreatedAt));

    let mut absent = complete_facts();
    absent.created_at = None;
    let mut warnings = WarningSink::default();
    let item = build(absent, &mut warnings);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Partial);
    assert_eq!(item.age_seconds, None);
}

#[test]
fn a_malformed_last_pending_marker_is_warned_and_idle_falls_back_to_creation() {
    let mut facts = complete_facts();
    facts.last_pending_malformed = true;
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    assert_eq!(item.last_pending_at, None);
    assert_eq!(item.idle_for_seconds, item.age_seconds);
    assert!(warnings
        .into_warnings()
        .iter()
        .any(|w| w.code == InventoryWarningCode::MalformedLastPending));
}

#[test]
fn status_reason_and_message_are_bounded_and_redacted() {
    let mut facts = complete_facts();
    facts.status_reason = Some("Crash\nLoopBackOff".to_string());
    facts.status_message = Some(format!(
        "pull from https://u:p@reg.example.com/i?sig=abc failed {}",
        "z".repeat(2000)
    ));
    let mut warnings = WarningSink::default();
    let item = build(facts, &mut warnings);
    let reason = item.status_reason.expect("reason");
    assert_eq!(reason, "Crash LoopBackOff");
    let message = item.status_message.expect("message");
    assert!(message.len() <= MAX_STATUS_MESSAGE_BYTES);
    assert!(!message.contains("u:p@"), "{message}");
    assert!(!message.contains("sig=abc"), "{message}");
}

#[test]
fn a_backend_that_reports_no_restart_count_stays_null_not_zero() {
    let mut facts = complete_facts();
    facts.restart_count = None;
    let mut warnings = WarningSink::default();
    assert_eq!(build(facts, &mut warnings).restart_count, None);
}

#[test]
fn a_drifted_managed_marker_is_reported_rather_than_assumed() {
    let mut facts = complete_facts();
    facts.managed = false;
    let mut warnings = WarningSink::default();
    assert!(!build(facts, &mut warnings).managed);
}
