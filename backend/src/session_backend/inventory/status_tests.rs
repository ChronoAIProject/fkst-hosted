//! The two mapping tables, exercised value by value — including the cases where
//! guessing would be actively harmful (`Terminated` is not success; a drained Pod
//! is not `Running`).

use super::*;

#[test]
fn kubernetes_maps_every_documented_phase() {
    let cases = [
        (Some("Pending"), RuntimeInventoryStatus::Pending),
        (Some("Running"), RuntimeInventoryStatus::Running),
        (Some("Succeeded"), RuntimeInventoryStatus::Succeeded),
        (Some("Failed"), RuntimeInventoryStatus::Failed),
    ];
    for (phase, expected) in cases {
        assert_eq!(
            RuntimeInventoryStatus::from_kubernetes(phase, false),
            expected,
            "phase {phase:?}"
        );
    }
}

#[test]
fn kubernetes_maps_absent_unknown_and_future_phases_to_unknown() {
    for phase in [None, Some("Unknown"), Some("Rebooting")] {
        assert_eq!(
            RuntimeInventoryStatus::from_kubernetes(phase, false),
            RuntimeInventoryStatus::Unknown,
            "phase {phase:?}"
        );
    }
}

#[test]
fn kubernetes_deletion_timestamp_beats_every_phase() {
    // A drained pod still reports Running; showing it as healthy would hide the
    // single most operationally interesting fact about it.
    for phase in [
        None,
        Some("Pending"),
        Some("Running"),
        Some("Succeeded"),
        Some("Failed"),
        Some("Unknown"),
    ] {
        assert_eq!(
            RuntimeInventoryStatus::from_kubernetes(phase, true),
            RuntimeInventoryStatus::Terminating,
            "phase {phase:?}"
        );
    }
}

#[test]
fn opensandbox_maps_every_documented_state() {
    let cases = [
        ("Pending", RuntimeInventoryStatus::Pending),
        ("Running", RuntimeInventoryStatus::Running),
        ("Paused", RuntimeInventoryStatus::Paused),
        ("Pausing", RuntimeInventoryStatus::Transitioning),
        ("Resuming", RuntimeInventoryStatus::Transitioning),
        ("Stopping", RuntimeInventoryStatus::Terminating),
        ("Terminated", RuntimeInventoryStatus::Terminated),
        ("Failed", RuntimeInventoryStatus::Failed),
    ];
    for (state, expected) in cases {
        assert_eq!(
            RuntimeInventoryStatus::from_opensandbox(state),
            expected,
            "state {state}"
        );
    }
}

#[test]
fn opensandbox_terminated_is_never_reported_as_success() {
    // The lifecycle API says the sandbox stopped existing, NOT that its work
    // succeeded. Inventing a success verdict is the worst kind of wrong here.
    assert_ne!(
        RuntimeInventoryStatus::from_opensandbox("Terminated"),
        RuntimeInventoryStatus::Succeeded
    );
}

#[test]
fn opensandbox_future_state_maps_to_unknown() {
    for state in ["Hibernating", "", "running"] {
        assert_eq!(
            RuntimeInventoryStatus::from_opensandbox(state),
            RuntimeInventoryStatus::Unknown,
            "state {state}"
        );
    }
}

#[test]
fn the_wire_spelling_round_trips_for_every_variant() {
    for status in RuntimeInventoryStatus::ALL {
        assert_eq!(RuntimeInventoryStatus::parse(status.as_str()), Some(status));
    }
    assert_eq!(RuntimeInventoryStatus::ALL.len(), 9);
}

#[test]
fn an_unrecognized_spelling_does_not_parse() {
    for value in ["", "RUNNING", "gone", "unknown_legacy"] {
        assert_eq!(RuntimeInventoryStatus::parse(value), None, "value {value}");
    }
}

#[test]
fn the_default_is_unknown() {
    assert_eq!(
        RuntimeInventoryStatus::default(),
        RuntimeInventoryStatus::Unknown
    );
}
