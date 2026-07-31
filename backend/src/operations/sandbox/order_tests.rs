//! Unit tests for the documented ordering.
//!
//! The load-bearing property is not "this exact sequence" but "the sequence does
//! not depend on anything outside the row", which is what makes a hidden runtime
//! unable to move an authorized one.

use super::super::test_support::{instant, item, with_status};
use super::*;

fn sorted(mut items: Vec<RuntimeInventoryItem>) -> Vec<String> {
    items.sort_by(compare);
    items.into_iter().map(|item| item.runtime_id).collect()
}

#[test]
fn active_and_problem_states_sort_before_settled_terminal_ones() {
    let ordered: Vec<&str> = [
        RuntimeInventoryStatus::Failed,
        RuntimeInventoryStatus::Pending,
        RuntimeInventoryStatus::Running,
        RuntimeInventoryStatus::Transitioning,
        RuntimeInventoryStatus::Paused,
        RuntimeInventoryStatus::Terminating,
        RuntimeInventoryStatus::Unknown,
        RuntimeInventoryStatus::Succeeded,
        RuntimeInventoryStatus::Terminated,
    ]
    .iter()
    .map(|status| status.as_str())
    .collect();
    // The rank is strictly increasing in the documented order, which is the
    // whole claim the table in the module docs makes.
    let ranks: Vec<u8> = [
        RuntimeInventoryStatus::Failed,
        RuntimeInventoryStatus::Pending,
        RuntimeInventoryStatus::Running,
        RuntimeInventoryStatus::Transitioning,
        RuntimeInventoryStatus::Paused,
        RuntimeInventoryStatus::Terminating,
        RuntimeInventoryStatus::Unknown,
        RuntimeInventoryStatus::Succeeded,
        RuntimeInventoryStatus::Terminated,
    ]
    .into_iter()
    .map(status_rank)
    .collect();
    assert!(
        ranks.windows(2).all(|pair| pair[0] < pair[1]),
        "{ordered:?} -> {ranks:?}"
    );
}

#[test]
fn every_status_has_a_distinct_rank() {
    let mut ranks: Vec<u8> = RuntimeInventoryStatus::ALL
        .into_iter()
        .map(status_rank)
        .collect();
    ranks.sort_unstable();
    ranks.dedup();
    assert_eq!(ranks.len(), RuntimeInventoryStatus::ALL.len());
}

#[test]
fn within_one_state_the_newest_runtime_sorts_first_and_nulls_last() {
    let newest = RuntimeInventoryItem {
        created_at: Some(instant(14, 0)),
        ..item("b-newest", Some("sess-a"))
    };
    let oldest = RuntimeInventoryItem {
        created_at: Some(instant(10, 0)),
        ..item("a-oldest", Some("sess-a"))
    };
    let undated = RuntimeInventoryItem {
        created_at: None,
        ..item("a-undated", Some("sess-a"))
    };
    assert_eq!(
        sorted(vec![undated, oldest, newest]),
        vec!["b-newest", "a-oldest", "a-undated"],
        "a runtime with no creation timestamp is the least informative row, so it \
         must not displace a live one"
    );
}

#[test]
fn the_runtime_id_breaks_a_remaining_tie() {
    let left = RuntimeInventoryItem {
        created_at: Some(instant(12, 0)),
        ..item("zzz", Some("sess-a"))
    };
    let right = RuntimeInventoryItem {
        created_at: Some(instant(12, 0)),
        ..item("aaa", Some("sess-a"))
    };
    assert_eq!(sorted(vec![left, right]), vec!["aaa", "zzz"]);
}

/// The comparator reads no index, no neighbour, and no snapshot — so the order of
/// the authorized subset is identical whatever else the fleet contained.
#[test]
fn the_order_of_a_subset_is_independent_of_the_rows_removed_from_it() {
    let mine = vec![
        with_status("mine-1", Some("sess-a"), RuntimeInventoryStatus::Running),
        with_status("mine-2", Some("sess-a"), RuntimeInventoryStatus::Failed),
        with_status("mine-3", Some("sess-a"), RuntimeInventoryStatus::Terminated),
    ];
    let expected = sorted(mine.clone());

    let mut mixed = vec![
        with_status("hidden-1", Some("sess-x"), RuntimeInventoryStatus::Failed),
        with_status("hidden-2", Some("sess-x"), RuntimeInventoryStatus::Running),
    ];
    mixed.extend(mine);
    mixed.sort_by(compare);
    let after_removal: Vec<String> = mixed
        .into_iter()
        .filter(|item| item.runtime_id.starts_with("mine-"))
        .map(|item| item.runtime_id)
        .collect();
    assert_eq!(after_removal, expected);
}
