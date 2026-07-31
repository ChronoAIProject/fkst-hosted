//! Unit tests for the normative pipeline's AUTHORIZATION half.
//!
//! These drive the service against a scripted backend so the ORDER of the
//! pipeline — authorize, then filter, then warn, then sort — is asserted
//! directly rather than inferred from an HTTP response. The capacity and outage
//! half lives in `service_capacity_tests.rs`.

use super::super::test_support::{
    ids, instant, lifetime, mixed_fleet, run_for, scope, viewer, with_status, Fixture, ALICE, ERIN,
    GRACE, MINE, THEIRS,
};
use super::*;
use crate::session_backend::inventory::{
    BoundedInventoryWarning, InventoryWarningCode, RuntimeInventoryStatus,
};
use crate::session_backend::test_support::FakeSessionBackend;

#[tokio::test]
async fn a_regular_caller_receives_only_their_authorized_rows_in_the_documented_order() {
    let inventory = run_for(
        ALICE,
        false,
        SandboxFilters::default(),
        FakeSessionBackend::default().with_inventory(mixed_fleet()),
    )
    .await
    .expect("the read succeeds");
    assert_eq!(
        ids(&inventory),
        vec!["mine-failed", "mine-running"],
        "failed sorts before running, and no hidden row survives"
    );
    assert_eq!(inventory.items.len(), 2);
}

#[tokio::test]
async fn a_global_admin_receives_the_complete_fleet_including_the_unattributable_rows() {
    let inventory = run_for(
        GRACE,
        true,
        SandboxFilters::default(),
        FakeSessionBackend::default().with_inventory(mixed_fleet()),
    )
    .await
    .expect("the read succeeds");
    assert_eq!(inventory.items.len(), 5);
    assert!(ids(&inventory).contains(&"hidden-orphan".to_string()));
}

/// The pipeline's central claim, asserted structurally: changing the hidden rows
/// cannot change ANYTHING about an authorized caller's result.
#[tokio::test]
async fn mutating_the_hidden_rows_changes_nothing_about_an_authorized_result() {
    let baseline = run_for(
        ALICE,
        false,
        SandboxFilters::default(),
        FakeSessionBackend::default().with_inventory(mixed_fleet()),
    )
    .await
    .expect("the read succeeds");

    // A completely different hidden population: more rows, different states,
    // different ids, different order, and warnings of their own.
    let mut mutated: Vec<RuntimeInventoryItem> = (0..40)
        .map(|index| {
            with_status(
                &format!("aaa-hidden-{index:02}"),
                Some(THEIRS),
                RuntimeInventoryStatus::Pending,
            )
        })
        .collect();
    mutated.push(with_status(
        "mine-running",
        Some(MINE),
        RuntimeInventoryStatus::Running,
    ));
    mutated.push(with_status(
        "mine-failed",
        Some(MINE),
        RuntimeInventoryStatus::Failed,
    ));
    let warnings: Vec<BoundedInventoryWarning> = (0..40)
        .map(|index| BoundedInventoryWarning {
            code: InventoryWarningCode::MalformedIdentity,
            runtime_id: Some(format!("aaa-hidden-{index:02}")),
            session_id: Some(THEIRS.to_string()),
        })
        .collect();

    let after = run_for(
        ALICE,
        false,
        SandboxFilters::default(),
        FakeSessionBackend::default()
            .with_inventory(mutated)
            .with_inventory_warnings(warnings),
    )
    .await
    .expect("the read succeeds");

    assert_eq!(ids(&after), ids(&baseline));
    assert_eq!(after.items.len(), baseline.items.len());
    assert_eq!(after.warning_codes, baseline.warning_codes);
    assert_eq!(
        after
            .items
            .iter()
            .map(|runtime| runtime.warning_codes.clone())
            .collect::<Vec<_>>(),
        baseline
            .items
            .iter()
            .map(|runtime| runtime.warning_codes.clone())
            .collect::<Vec<_>>()
    );
}

/// Filters run AFTER authorization, so naming another user's creator id narrows
/// to nothing rather than widening to their rows.
#[tokio::test]
async fn a_creator_filter_naming_another_user_returns_nothing_rather_than_their_rows() {
    let inventory = run_for(
        ALICE,
        false,
        SandboxFilters {
            creator_id: Some(999),
            ..SandboxFilters::default()
        },
        FakeSessionBackend::default().with_inventory(mixed_fleet()),
    )
    .await
    .expect("the read succeeds");
    assert!(inventory.items.is_empty());
}

#[tokio::test]
async fn a_session_filter_narrows_within_the_authorized_set() {
    let inventory = run_for(
        ALICE,
        false,
        SandboxFilters {
            session_id: Some(MINE.to_string()),
            status: Some(RuntimeInventoryStatus::Running),
            ..SandboxFilters::default()
        },
        FakeSessionBackend::default().with_inventory(mixed_fleet()),
    )
    .await
    .expect("the read succeeds");
    assert_eq!(ids(&inventory), vec!["mine-running"]);
}

/// An unrelated caller gets a complete, honest, EMPTY answer — no hint of the
/// fleet's size, and no error that would confirm anything exists.
#[tokio::test]
async fn an_unrelated_caller_receives_an_empty_snapshot() {
    let inventory = run_for(
        ERIN,
        false,
        SandboxFilters::default(),
        FakeSessionBackend::default().with_inventory(mixed_fleet()),
    )
    .await
    .expect("the read succeeds");
    assert!(inventory.items.is_empty());
    assert!(inventory.warning_codes.is_empty());
}

#[tokio::test]
async fn exactly_one_inventory_read_happens_per_request() {
    let fixture = Fixture::new();
    let access = fixture.access.clone();
    let viewer = viewer(ALICE, &access);
    let scope = scope(&viewer, false);
    let filters = SandboxFilters::default();
    let backend = FakeSessionBackend::default().with_inventory(mixed_fleet());
    run(&backend, &fixture.request(&viewer, &scope, &filters, 5_000))
        .await
        .expect("the read succeeds");
    let calls = backend.inventory_calls.lock().expect("not poisoned");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        lifetime(),
        "the configured policy is passed through"
    );
}

#[tokio::test]
async fn the_backends_own_observed_at_is_returned_verbatim() {
    let observed_at = instant(9, 30);
    let inventory = run_for(
        ALICE,
        false,
        SandboxFilters::default(),
        FakeSessionBackend::default()
            .with_inventory(mixed_fleet())
            .with_inventory_observed_at(observed_at),
    )
    .await
    .expect("the read succeeds");
    assert_eq!(inventory.observed_at, observed_at);
}

/// Warnings reach a caller only when they name a row that caller is receiving.
#[tokio::test]
async fn a_warning_about_a_hidden_row_never_reaches_a_regular_caller() {
    let inventory = run_for(
        ALICE,
        false,
        SandboxFilters::default(),
        FakeSessionBackend::default()
            .with_inventory(mixed_fleet())
            .with_inventory_warnings(vec![
                BoundedInventoryWarning {
                    code: InventoryWarningCode::AttributionConflict,
                    runtime_id: Some("mine-running".to_string()),
                    session_id: Some(MINE.to_string()),
                },
                BoundedInventoryWarning {
                    code: InventoryWarningCode::MalformedIdentity,
                    runtime_id: Some("hidden-failed".to_string()),
                    session_id: Some(THEIRS.to_string()),
                },
                BoundedInventoryWarning::snapshot(InventoryWarningCode::WarningsTruncated),
            ]),
    )
    .await
    .expect("the read succeeds");

    let running = inventory
        .items
        .iter()
        .find(|runtime| runtime.item.runtime_id == "mine-running")
        .expect("the visible row");
    assert_eq!(
        running.warning_codes,
        vec![SandboxWarningCode::AttributionConflict]
    );
    assert_eq!(
        inventory.warning_codes,
        vec![SandboxWarningCode::AttributionConflict],
        "the response summarizes the returned rows; a snapshot-scope code would \
         let a hidden runtime change a regular caller's answer"
    );
}

#[tokio::test]
async fn a_global_admin_additionally_receives_the_snapshot_scope_codes() {
    let inventory = run_for(
        GRACE,
        true,
        SandboxFilters::default(),
        FakeSessionBackend::default()
            .with_inventory(mixed_fleet())
            .with_inventory_warnings(vec![BoundedInventoryWarning::snapshot(
                InventoryWarningCode::WarningsTruncated,
            )]),
    )
    .await
    .expect("the read succeeds");
    assert_eq!(
        inventory.warning_codes,
        vec![SandboxWarningCode::WarningsIncomplete]
    );
}
