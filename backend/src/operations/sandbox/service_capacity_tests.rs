//! Unit tests for the pipeline's CAPACITY and OUTAGE half.
//!
//! Four failures, four distinct operator stories: an incomplete source read, a
//! backend that failed or ran long, a fleet larger than a configured ceiling, and
//! an authorization projection that cannot answer. None of them may leak an
//! upstream detail or a count.

use std::time::Duration;

use super::super::test_support::{
    item, mixed_fleet, run_for, scope, viewer, Fixture, ALICE, GRACE, MINE, THEIRS,
};
use super::*;
use crate::session_backend::inventory::{BoundedInventoryWarning, InventoryWarningCode};
use crate::session_backend::test_support::FakeSessionBackend;

/// A clipped page walk means the fleet read was INCOMPLETE, so no answer derived
/// from it may claim to be the complete matching set.
#[tokio::test]
async fn a_truncated_source_walk_is_a_capacity_failure_for_every_caller() {
    for (who, global) in [(ALICE, false), (GRACE, true)] {
        let error = run_for(
            who,
            global,
            SandboxFilters::default(),
            FakeSessionBackend::default()
                .with_inventory(mixed_fleet())
                .with_inventory_warnings(vec![BoundedInventoryWarning::snapshot(
                    InventoryWarningCode::SourceTruncated,
                )]),
        )
        .await
        .expect_err("an incomplete read cannot be served");
        assert!(matches!(error, AppError::SandboxInventoryTooLarge(_)));
    }
}

#[tokio::test]
async fn a_backend_failure_and_a_timeout_are_the_same_bounded_unavailable() {
    for backend in [
        FakeSessionBackend::default().with_inventory_error(),
        FakeSessionBackend::default()
            .with_inventory(mixed_fleet())
            .with_inventory_delay(Duration::from_millis(1_500)),
    ] {
        let error = run_for(ALICE, false, SandboxFilters::default(), backend)
            .await
            .expect_err("the read fails");
        let AppError::SandboxInventoryUnavailable(message) = error else {
            panic!("expected a bounded unavailable");
        };
        assert!(!message.contains("scripted"), "{message}");
    }
}

/// The source ceiling is the backend's own refusal; it must reach the caller as a
/// capacity failure carrying no count.
#[tokio::test]
async fn the_source_ceiling_answers_too_large_without_a_count() {
    let error = run_for(
        ALICE,
        false,
        SandboxFilters::default(),
        FakeSessionBackend::default().with_inventory_too_large(4_242),
    )
    .await
    .expect_err("an oversize fleet cannot be served");
    let AppError::SandboxInventoryTooLarge(message) = error else {
        panic!("expected a capacity failure");
    };
    assert!(!message.contains("4242"), "{message}");
    assert!(!message.contains("4,242"), "{message}");
}

/// The PUBLIC ceiling counts authorized rows only, so a huge hidden fleet cannot
/// fail a caller whose own result is small.
#[tokio::test]
async fn the_result_ceiling_counts_authorized_rows_only() {
    let mut fleet: Vec<RuntimeInventoryItem> = (0..50)
        .map(|index| item(&format!("hidden-{index:02}"), Some(THEIRS)))
        .collect();
    fleet.push(item("mine-1", Some(MINE)));
    fleet.push(item("mine-2", Some(MINE)));

    let fixture = Fixture::new();
    let access = fixture.access.clone();
    let alice = viewer(ALICE, &access);
    let alice_scope = scope(&alice, false);
    let filters = SandboxFilters::default();
    let backend = FakeSessionBackend::default().with_inventory(fleet.clone());
    let inventory = run(
        &backend,
        &fixture.request(&alice, &alice_scope, &filters, 3),
    )
    .await
    .expect("two authorized rows are well inside a ceiling of three");
    assert_eq!(inventory.items.len(), 2);

    // The same ceiling DOES fail an administrator whose own result is the fleet.
    let grace = viewer(GRACE, &access);
    let grace_scope = scope(&grace, true);
    let backend = FakeSessionBackend::default().with_inventory(fleet);
    let error = run(
        &backend,
        &fixture.request(&grace, &grace_scope, &filters, 3),
    )
    .await
    .expect_err("fifty-two rows exceed a ceiling of three");
    assert!(matches!(error, AppError::SandboxInventoryTooLarge(_)));
}

/// A cold projection blocks `accessible` and leaves `all` untouched: registry
/// health and runtime health are independent failures.
#[tokio::test]
async fn a_cold_projection_blocks_accessible_but_not_the_global_fleet_view() {
    let fixture = Fixture {
        registry: SessionAccessRegistry::new(true),
        ..Fixture::new()
    };
    let access = fixture.access.clone();
    let filters = SandboxFilters::default();

    let alice = viewer(ALICE, &access);
    let alice_scope = scope(&alice, false);
    let backend = FakeSessionBackend::default().with_inventory(mixed_fleet());
    let error = run(
        &backend,
        &fixture.request(&alice, &alice_scope, &filters, 5_000),
    )
    .await
    .expect_err("a cold projection cannot authorize a row");
    assert!(matches!(error, AppError::SessionVisibilityUnavailable(_)));

    let grace = viewer(GRACE, &access);
    let grace_scope = scope(&grace, true);
    let backend = FakeSessionBackend::default().with_inventory(mixed_fleet());
    let inventory = run(
        &backend,
        &fixture.request(&grace, &grace_scope, &filters, 5_000),
    )
    .await
    .expect("the global fleet view needs no session context");
    assert_eq!(inventory.items.len(), 5);
}
