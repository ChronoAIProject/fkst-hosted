//! Record tests: the sort key, the ownership fields, and the severity ordering
//! that makes deduplication keep the more alarming delivery state.

use super::*;
use crate::operations::test_support::{anchor, api_record, lifecycle_record};

#[test]
fn the_sort_key_is_the_terminal_instant_of_each_contract() {
    let api = api_record("ev-1", 101, 30, ActivitySourceKind::Posthog);
    let lifecycle = lifecycle_record("ev-2", "sess-1", 10, ActivitySourceKind::Posthog);
    assert_eq!(
        api.sort_timestamp(),
        anchor() - k8s_openapi::chrono::Duration::seconds(30)
    );
    assert_eq!(
        lifecycle.sort_timestamp(),
        anchor() - k8s_openapi::chrono::Duration::seconds(10)
    );
    assert_eq!(api.event_id(), "ev-1");
    assert_eq!(lifecycle.event_id(), "ev-2");
}

#[test]
fn only_an_api_row_carries_an_owning_actor_id() {
    let api = api_record("ev-1", 101, 0, ActivitySourceKind::Posthog);
    let lifecycle = lifecycle_record("ev-2", "sess-1", 0, ActivitySourceKind::Posthog);
    assert_eq!(api.actor_id(), Some(101));
    assert!(!api.is_lifecycle());
    assert_eq!(
        lifecycle.actor_id(),
        None,
        "a system transition has no owning human, which is exactly why it needs \
         session authorization rather than an actor predicate"
    );
    assert!(lifecycle.is_lifecycle());
    assert_eq!(lifecycle.session_id(), Some("sess-1"));
}

/// Severity order is the whole contract of `merge_delivery`.
#[test]
fn delivery_states_are_ordered_by_severity() {
    assert!(DeliveryState::VerifiedInPosthog < DeliveryState::AcceptedPendingVerification);
    assert!(DeliveryState::AcceptedPendingVerification < DeliveryState::Queued);
    assert!(DeliveryState::Queued < DeliveryState::Incomplete);
    assert!(DeliveryState::Incomplete < DeliveryState::DeadLetter);
}

#[test]
fn merging_a_delivery_state_keeps_the_more_severe_one() {
    let mut record = api_record("ev-1", 101, 0, ActivitySourceKind::Posthog);
    assert_eq!(record.delivery_state(), DeliveryState::VerifiedInPosthog);

    record.merge_delivery(DeliveryState::DeadLetter);
    assert_eq!(
        record.delivery_state(),
        DeliveryState::DeadLetter,
        "a stuck delivery must not be erased by a verified copy"
    );

    record.merge_delivery(DeliveryState::VerifiedInPosthog);
    assert_eq!(
        record.delivery_state(),
        DeliveryState::DeadLetter,
        "merging is monotonic: severity never decreases"
    );
}

#[test]
fn the_source_and_state_wire_strings_are_the_documented_closed_set() {
    assert_eq!(ActivitySourceKind::Posthog.as_str(), "posthog");
    assert_eq!(ActivitySourceKind::Relay.as_str(), "relay");
    assert_eq!(
        DeliveryState::VerifiedInPosthog.as_str(),
        "verified_in_posthog"
    );
    assert_eq!(
        DeliveryState::AcceptedPendingVerification.as_str(),
        "accepted_pending_verification"
    );
    assert_eq!(DeliveryState::Queued.as_str(), "queued");
    assert_eq!(DeliveryState::Incomplete.as_str(), "incomplete");
    assert_eq!(DeliveryState::DeadLetter.as_str(), "dead_letter");
}
