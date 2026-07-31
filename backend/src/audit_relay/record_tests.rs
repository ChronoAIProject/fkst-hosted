//! The closed storage vocabulary: stable spellings, total parsing, and the two
//! predicates the rest of the relay branches on.

use super::*;

#[test]
fn every_state_round_trips_through_its_wire_spelling() {
    for state in RecordState::ALL {
        assert_eq!(RecordState::parse(state.as_str()), Some(state));
    }
    assert_eq!(RecordState::parse("delivered"), None);
}

#[test]
fn every_record_kind_round_trips() {
    for kind in RelayRecordKind::ALL {
        assert_eq!(RelayRecordKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(RelayRecordKind::parse("something_else"), None);
}

#[test]
fn only_a_started_row_lacks_a_terminal_projection() {
    assert!(!RecordState::Started.has_terminal());
    for state in RecordState::ALL
        .into_iter()
        .filter(|s| *s != RecordState::Started)
    {
        assert!(state.has_terminal(), "{} must be terminal", state.as_str());
    }
}

#[test]
fn only_complete_and_incomplete_await_capture() {
    let awaiting: Vec<&str> = RecordState::ALL
        .into_iter()
        .filter(|state| state.awaits_capture())
        .map(RecordState::as_str)
        .collect();
    assert_eq!(awaiting, vec!["complete", "incomplete"]);
}

#[test]
fn acceptance_and_verification_are_distinct_delivery_states() {
    assert_eq!(
        RecordState::PosthogAccepted.delivery_state(),
        "accepted_pending_verification"
    );
    assert_eq!(
        RecordState::PosthogVerified.delivery_state(),
        "verified_in_posthog"
    );
    assert_ne!(
        RecordState::PosthogAccepted.delivery_state(),
        RecordState::PosthogVerified.delivery_state()
    );
}

#[test]
fn the_ingress_and_activity_vocabularies_are_separate() {
    assert_eq!(RelayRecordKind::ApiRequest.ingress(), "request");
    assert_eq!(RelayRecordKind::ApiRequest.as_str(), "api_request");
    assert_eq!(RelayRecordKind::SandboxLifecycle.ingress(), "lifecycle");
    assert_eq!(
        RelayRecordKind::SandboxLifecycle.as_str(),
        "sandbox_lifecycle"
    );
}
