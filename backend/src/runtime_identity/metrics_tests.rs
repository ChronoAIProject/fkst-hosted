//! Counter tests. The interesting property is not that a counter increments —
//! it is that every label tuple is finite and that two tuples never share a slot.

use super::*;

#[test]
fn identity_operations_are_counted_per_backend_and_result() {
    let telemetry = RuntimeTelemetry::new();
    telemetry.record_identity(
        RuntimeBackendKind::Kubernetes,
        IdentityOperationResult::Backfilled,
    );
    telemetry.record_identity(
        RuntimeBackendKind::Kubernetes,
        IdentityOperationResult::Backfilled,
    );
    telemetry.record_identity(
        RuntimeBackendKind::OpenSandbox,
        IdentityOperationResult::Conflict,
    );

    let snapshot = telemetry.snapshot();
    assert_eq!(
        snapshot.identity(
            RuntimeBackendKind::Kubernetes,
            IdentityOperationResult::Backfilled
        ),
        2
    );
    assert_eq!(
        snapshot.identity(
            RuntimeBackendKind::OpenSandbox,
            IdentityOperationResult::Conflict
        ),
        1
    );
    assert_eq!(
        snapshot.identity(
            RuntimeBackendKind::OpenSandbox,
            IdentityOperationResult::Backfilled
        ),
        0,
        "the two backends must not share a counter slot"
    );
}

#[test]
fn lifecycle_events_are_counted_per_backend_action_and_result() {
    let telemetry = RuntimeTelemetry::new();
    telemetry.record_lifecycle(
        RuntimeBackendKind::Kubernetes,
        LifecycleAction::Created,
        LifecycleEmitResult::Emitted,
    );
    telemetry.record_lifecycle(
        RuntimeBackendKind::Kubernetes,
        LifecycleAction::Created,
        LifecycleEmitResult::Dropped,
    );
    telemetry.record_lifecycle(
        RuntimeBackendKind::Kubernetes,
        LifecycleAction::Deleted,
        LifecycleEmitResult::Emitted,
    );

    let snapshot = telemetry.snapshot();
    assert_eq!(
        snapshot.lifecycle(
            RuntimeBackendKind::Kubernetes,
            LifecycleAction::Created,
            LifecycleEmitResult::Emitted
        ),
        1
    );
    assert_eq!(
        snapshot.lifecycle(
            RuntimeBackendKind::Kubernetes,
            LifecycleAction::Created,
            LifecycleEmitResult::Dropped
        ),
        1,
        "a lifecycle event lost to a full queue is a visible hole, not a silent one"
    );
    assert_eq!(
        snapshot.lifecycle(
            RuntimeBackendKind::OpenSandbox,
            LifecycleAction::Created,
            LifecycleEmitResult::Emitted
        ),
        0
    );
}

#[test]
fn every_action_and_result_has_a_distinct_counter_slot() {
    // A duplicated dense index would silently merge two series, which is exactly
    // the kind of bug a metric cannot self-report.
    let telemetry = RuntimeTelemetry::new();
    for action in LifecycleAction::ALL {
        telemetry.record_lifecycle(
            RuntimeBackendKind::Kubernetes,
            action,
            LifecycleEmitResult::Emitted,
        );
    }
    let snapshot = telemetry.snapshot();
    for action in LifecycleAction::ALL {
        assert_eq!(
            snapshot.lifecycle(
                RuntimeBackendKind::Kubernetes,
                action,
                LifecycleEmitResult::Emitted
            ),
            1,
            "{} shares a slot with another action",
            action.as_str()
        );
    }

    let telemetry = RuntimeTelemetry::new();
    for result in IdentityOperationResult::ALL {
        telemetry.record_identity(RuntimeBackendKind::OpenSandbox, result);
    }
    let snapshot = telemetry.snapshot();
    for result in IdentityOperationResult::ALL {
        assert_eq!(
            snapshot.identity(RuntimeBackendKind::OpenSandbox, result),
            1,
            "{} shares a slot with another result",
            result.as_str()
        );
    }
}

#[test]
fn the_outcome_conversion_preserves_the_bounded_result_name() {
    for (outcome, expected) in [
        (super::super::RuntimeIdentityOutcome::Unchanged, "unchanged"),
        (
            super::super::RuntimeIdentityOutcome::Backfilled,
            "backfilled",
        ),
        (super::super::RuntimeIdentityOutcome::Conflict, "conflict"),
        (super::super::RuntimeIdentityOutcome::NotFound, "not_found"),
    ] {
        assert_eq!(IdentityOperationResult::from(outcome).as_str(), expected);
    }
}

#[test]
fn a_default_snapshot_reads_zero_for_every_series() {
    let snapshot = RuntimeTelemetrySnapshot::default();
    for backend in RuntimeBackendKind::ALL {
        for result in IdentityOperationResult::ALL {
            assert_eq!(snapshot.identity(backend, result), 0);
        }
        for action in LifecycleAction::ALL {
            for result in LifecycleEmitResult::ALL {
                assert_eq!(snapshot.lifecycle(backend, action, result), 0);
            }
        }
    }
}
