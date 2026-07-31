//! The immutability guarantee, exercised case by case: all-missing, partially
//! missing, already complete, conflicting, and the assignee-derived missing id
//! that must never be filled from the trigger author.

use super::super::keys::{read, stamp_pairs, K8S_IDENTITY_KEYS, OSB_IDENTITY_KEYS};
use super::*;

fn identity(creator_id: Option<i64>) -> RuntimeIdentityMetadata {
    RuntimeIdentityMetadata::new(creator_id, "alice", 77, "octocat")
}

fn complete_stamp(identity: &RuntimeIdentityMetadata) -> BTreeMap<String, String> {
    stamp_pairs(&K8S_IDENTITY_KEYS, identity)
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

#[test]
fn an_unstamped_runtime_backfills_every_key_the_registration_can_supply() {
    let identity = identity(Some(4242));
    let plan = plan(&K8S_IDENTITY_KEYS, &BTreeMap::new(), &identity);
    let IdentityPlan::Backfill(pairs) = plan else {
        panic!("an empty runtime must backfill, got {plan:?}");
    };
    assert_eq!(pairs.len(), 6);
    let filled: BTreeMap<String, String> = pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    // Everything a launch stamp writes, EXCEPT that the provenance marker tells
    // the truth about where the values came from: the trigger as it reads now.
    let mut expected = complete_stamp(&identity);
    expected.insert(
        K8S_IDENTITY_KEYS.source.to_string(),
        SOURCE_BACKFILLED_CURRENT_TRIGGER.to_string(),
    );
    assert_eq!(filled, expected);
}

#[test]
fn a_backfill_never_rewrites_an_existing_provenance_marker() {
    // A runtime stamped at launch that later gains one absent key keeps saying
    // `launch_metadata`: that is where its stamp originated, and overwriting it
    // would be the same silent rewrite the conflict rule forbids for attribution.
    let identity = identity(Some(4242));
    let mut metadata = complete_stamp(&identity);
    metadata.remove(K8S_IDENTITY_KEYS.creator_id);

    let IdentityPlan::Backfill(pairs) = plan(&K8S_IDENTITY_KEYS, &metadata, &identity) else {
        panic!("a missing creator id must backfill");
    };
    let keys: Vec<&str> = pairs.iter().map(|(key, _)| *key).collect();
    assert_eq!(keys, vec![K8S_IDENTITY_KEYS.creator_id]);
}

#[test]
fn an_already_complete_runtime_is_never_patched() {
    let identity = identity(Some(4242));
    assert_eq!(
        plan(&K8S_IDENTITY_KEYS, &complete_stamp(&identity), &identity),
        IdentityPlan::Complete,
        "a settled runtime must produce no write at all, on every sweep, forever"
    );
}

#[test]
fn a_partially_stamped_runtime_backfills_only_the_absent_keys() {
    let identity = identity(Some(4242));
    let mut metadata = complete_stamp(&identity);
    metadata.remove(K8S_IDENTITY_KEYS.creator_login);
    metadata.remove(K8S_IDENTITY_KEYS.trigger_author_login);

    let IdentityPlan::Backfill(pairs) = plan(&K8S_IDENTITY_KEYS, &metadata, &identity) else {
        panic!("a partial stamp must backfill the gaps");
    };
    let keys: Vec<&str> = pairs.iter().map(|(key, _)| *key).collect();
    assert_eq!(
        keys,
        vec![
            K8S_IDENTITY_KEYS.creator_login,
            K8S_IDENTITY_KEYS.trigger_author_login
        ],
        "only the absent keys are written; the present ones are left exactly as they are"
    );
}

#[test]
fn a_differing_creator_id_is_a_conflict_and_writes_nothing() {
    let mut metadata = complete_stamp(&identity(Some(4242)));
    metadata.remove(K8S_IDENTITY_KEYS.creator_login);
    // The runtime was launched for a different person. Filling in the missing
    // login here would produce a half-old, half-new attribution.
    let plan = plan(&K8S_IDENTITY_KEYS, &metadata, &identity(Some(9999)));
    assert_eq!(
        plan,
        IdentityPlan::Conflict {
            key: K8S_IDENTITY_KEYS.creator_id
        }
    );
}

#[test]
fn a_differing_login_is_a_conflict_but_a_case_variant_is_not() {
    let identity = identity(Some(1));
    let mut metadata = complete_stamp(&identity);

    metadata.insert(
        K8S_IDENTITY_KEYS.creator_login.to_string(),
        "ALICE".to_string(),
    );
    assert_eq!(
        plan(&K8S_IDENTITY_KEYS, &metadata, &identity),
        IdentityPlan::Complete,
        "GitHub logins are case-insensitive; a case-only difference is the same person"
    );

    metadata.insert(
        K8S_IDENTITY_KEYS.creator_login.to_string(),
        "mallory".to_string(),
    );
    assert_eq!(
        plan(&K8S_IDENTITY_KEYS, &metadata, &identity),
        IdentityPlan::Conflict {
            key: K8S_IDENTITY_KEYS.creator_login
        }
    );
}

#[test]
fn a_registration_without_a_creator_id_never_claims_the_runtimes() {
    // The runtime knows an id; the current (assignee-derived) registration does
    // not. Making no claim cannot conflict, and must not erase what is stamped.
    let metadata = complete_stamp(&identity(Some(4242)));
    assert_eq!(
        plan(&K8S_IDENTITY_KEYS, &metadata, &identity(None)),
        IdentityPlan::Complete
    );
}

#[test]
fn an_assignee_derived_session_never_borrows_the_trigger_author_id() {
    let identity = identity(None);
    let IdentityPlan::Backfill(pairs) = plan(&K8S_IDENTITY_KEYS, &BTreeMap::new(), &identity)
    else {
        panic!("an empty runtime must backfill");
    };
    let filled: BTreeMap<&str, String> = pairs.into_iter().collect();
    assert!(
        !filled.contains_key(K8S_IDENTITY_KEYS.creator_id),
        "the creator id stays explicitly missing"
    );
    assert_eq!(filled[K8S_IDENTITY_KEYS.trigger_author_id], "77");
}

#[test]
fn a_future_schema_version_is_tolerated_rather_than_reported_as_a_conflict() {
    let identity = identity(Some(1));
    let mut metadata = complete_stamp(&identity);
    metadata.insert(K8S_IDENTITY_KEYS.schema.to_string(), "2".to_string());
    assert_eq!(
        plan(&K8S_IDENTITY_KEYS, &metadata, &identity),
        IdentityPlan::Complete,
        "a stamp written by another release is not an attribution disagreement"
    );
}

#[test]
fn the_conflict_check_runs_before_any_backfill_is_reported() {
    // A runtime with one wrong value and several missing ones must yield a
    // conflict, not a partial write that mixes two attributions.
    let mut metadata = BTreeMap::new();
    metadata.insert(
        K8S_IDENTITY_KEYS.trigger_author_id.to_string(),
        "999".to_string(),
    );
    assert!(matches!(
        plan(&K8S_IDENTITY_KEYS, &metadata, &identity(Some(1))),
        IdentityPlan::Conflict { .. }
    ));
}

#[test]
fn the_plan_is_identical_for_both_key_sets() {
    let identity = identity(Some(4242));
    let osb: BTreeMap<String, String> = stamp_pairs(&OSB_IDENTITY_KEYS, &identity)
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    assert_eq!(
        plan(&OSB_IDENTITY_KEYS, &osb, &identity),
        IdentityPlan::Complete
    );
    assert!(matches!(
        plan(&OSB_IDENTITY_KEYS, &BTreeMap::new(), &identity),
        IdentityPlan::Backfill(_)
    ));
}

#[test]
fn is_settled_matches_the_full_plan_for_a_complete_stamp() {
    let identity = identity(Some(4242));
    let observed = read(&K8S_IDENTITY_KEYS, &complete_stamp(&identity));
    assert!(
        is_settled(&observed, &identity),
        "the reconcile fast path must agree with the authoritative plan"
    );
}

#[test]
fn is_settled_refuses_every_case_the_plan_would_act_on() {
    let identity = identity(Some(4242));
    // Nothing stamped at all.
    assert!(!is_settled(&ObservedRuntimeIdentity::default(), &identity));

    // A stamped value that disagrees: the fast path must not swallow it, or the
    // conflict would never be recorded.
    let mut observed = read(&K8S_IDENTITY_KEYS, &complete_stamp(&identity));
    observed.creator_id = Some(9999);
    assert!(!is_settled(&observed, &identity));

    // The registration gained an id the runtime lacks: a genuine backfill.
    let mut observed = read(&K8S_IDENTITY_KEYS, &complete_stamp(&identity));
    observed.creator_id = None;
    assert!(!is_settled(&observed, &identity));

    // A corrupted id.
    let mut observed = read(&K8S_IDENTITY_KEYS, &complete_stamp(&identity));
    observed.malformed = true;
    assert!(!is_settled(&observed, &identity));

    // No schema key: a legacy runtime, even if its other values happen to match.
    let mut observed = read(&K8S_IDENTITY_KEYS, &complete_stamp(&identity));
    observed.schema_version = None;
    assert!(!is_settled(&observed, &identity));
}

#[test]
fn is_settled_accepts_the_assignee_derived_missing_id() {
    let identity = identity(None);
    let observed = read(&K8S_IDENTITY_KEYS, &complete_stamp(&identity));
    assert!(
        is_settled(&observed, &identity),
        "a missing creator id on BOTH sides is settled, not a permanent backfill loop"
    );
}
