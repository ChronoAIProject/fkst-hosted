//! Merge tests: ordering, deduplication, partial semantics, and the invariant
//! that a source outage can never look like "nothing happened".

use super::*;
use crate::operations::record::{ActivityRecord, ActivitySourceKind, DeliveryState};
use crate::operations::source::{SourceError, SourcePage};
use crate::operations::test_support::{
    all, api_record, authorized_session, lifecycle_record, mine,
};

const VIEWER_ID: i64 = 101;
const VIEWER: &str = "alice";
const SESSION: &str = "sess-alice";

fn page(records: Vec<ActivityRecord>) -> SourcePage {
    let raw_rows = records.len();
    SourcePage {
        records,
        raw_rows,
        row_errors: 0,
    }
}

fn ids(page: &MergedPage) -> Vec<&str> {
    page.items.iter().map(ActivityRecord::event_id).collect()
}

#[test]
fn records_from_both_sources_merge_newest_first() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let posthog = page(vec![
        api_record("ev-old", VIEWER_ID, 300, ActivitySourceKind::Posthog),
        api_record("ev-mid", VIEWER_ID, 200, ActivitySourceKind::Posthog),
    ]);
    let relay = page(vec![api_record(
        "ev-new",
        VIEWER_ID,
        100,
        ActivitySourceKind::Relay,
    )]);
    let merged = merge(&constraint, Some(Ok(posthog)), Some(Ok(relay)), 10, 11)
        .expect("both sources answered");
    assert_eq!(ids(&merged), vec!["ev-new", "ev-mid", "ev-old"]);
    assert!(!merged.status.partial);
    assert_eq!(merged.status.posthog, SourceHealth::Healthy);
    assert_eq!(merged.status.relay, SourceHealth::Healthy);
    assert!(merged.next_key.is_none(), "the page is not full");
}

/// Identical timestamps must still order deterministically, by event id.
#[test]
fn identical_timestamps_break_ties_on_the_event_id() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let posthog = page(vec![
        api_record("ev-a", VIEWER_ID, 100, ActivitySourceKind::Posthog),
        api_record("ev-c", VIEWER_ID, 100, ActivitySourceKind::Posthog),
        api_record("ev-b", VIEWER_ID, 100, ActivitySourceKind::Posthog),
    ]);
    let merged = merge(&constraint, Some(Ok(posthog)), None, 10, 11).expect("posthog answered");
    assert_eq!(ids(&merged), vec!["ev-c", "ev-b", "ev-a"]);
}

#[test]
fn a_duplicate_event_keeps_posthog_content_and_the_most_severe_delivery_state() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let mut relay_copy = api_record("ev-1", VIEWER_ID, 100, ActivitySourceKind::Relay);
    relay_copy.merge_delivery(DeliveryState::DeadLetter);
    let merged = merge(
        &constraint,
        Some(Ok(page(vec![api_record(
            "ev-1",
            VIEWER_ID,
            100,
            ActivitySourceKind::Posthog,
        )]))),
        Some(Ok(page(vec![relay_copy]))),
        10,
        11,
    )
    .expect("both answered");
    assert_eq!(merged.items.len(), 1);
    assert_eq!(merged.items[0].source(), ActivitySourceKind::Posthog);
    assert_eq!(merged.items[0].delivery_state(), DeliveryState::DeadLetter);
    assert_eq!(
        merged.duplicates, 1,
        "at-least-once delivery makes duplicates normal; they are counted, not \
         treated as an error"
    );
}

#[test]
fn posthog_unavailable_returns_authorized_relay_rows_marked_partial() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let merged = merge(
        &constraint,
        Some(Err(SourceError::Transient { kind: "timeout" })),
        Some(Ok(page(vec![api_record(
            "ev-1",
            VIEWER_ID,
            100,
            ActivitySourceKind::Relay,
        )]))),
        10,
        11,
    )
    .expect("the relay still answered");
    assert_eq!(ids(&merged), vec!["ev-1"]);
    assert!(merged.status.partial);
    assert_eq!(merged.status.posthog, SourceHealth::Unavailable);
    assert_eq!(
        merged.status.message_code,
        Some(message_codes::POSTHOG_UNAVAILABLE)
    );
}

#[test]
fn relay_unavailable_returns_authorized_posthog_history_marked_partial() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let merged = merge(
        &constraint,
        Some(Ok(page(vec![api_record(
            "ev-1",
            VIEWER_ID,
            100,
            ActivitySourceKind::Posthog,
        )]))),
        Some(Err(SourceError::Transient { kind: "connect" })),
        10,
        11,
    )
    .expect("posthog still answered");
    assert_eq!(ids(&merged), vec!["ev-1"]);
    assert!(merged.status.partial);
    assert_eq!(merged.status.relay, SourceHealth::Unavailable);
    assert_eq!(
        merged.status.message_code,
        Some(message_codes::RELAY_UNAVAILABLE)
    );
}

/// The invariant the whole partial contract exists for.
#[test]
fn neither_source_available_is_an_error_never_a_complete_empty_page() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let error = merge(
        &constraint,
        Some(Err(SourceError::Transient { kind: "timeout" })),
        Some(Err(SourceError::Upstream { kind: "auth" })),
        10,
        11,
    )
    .expect_err("an outage must never be rounded down to zero rows");
    assert!(
        error.is_upstream_fault(),
        "the actionable fault wins: {error:?}"
    );
}

#[test]
fn an_unconfigured_relay_is_not_an_outage() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let merged = merge(
        &constraint,
        Some(Ok(page(vec![api_record(
            "ev-1",
            VIEWER_ID,
            100,
            ActivitySourceKind::Posthog,
        )]))),
        None,
        10,
        11,
    )
    .expect("posthog alone is a complete answer");
    assert_eq!(merged.status.relay, SourceHealth::NotConfigured);
    assert!(!merged.status.partial);
    assert!(merged.status.message_code.is_none());
}

#[test]
fn a_full_page_yields_a_cursor_from_the_last_returned_row() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let records: Vec<_> = (0..3)
        .map(|i| {
            api_record(
                &format!("ev-{i}"),
                VIEWER_ID,
                i * 10,
                ActivitySourceKind::Posthog,
            )
        })
        .collect();
    let merged = merge(&constraint, Some(Ok(page(records))), None, 2, 3).expect("posthog answered");
    assert_eq!(merged.items.len(), 2, "at most `limit` rows are returned");
    let next = merged.next_key.expect("another page may exist");
    assert_eq!(next.event_id, merged.items[1].event_id());
    assert_eq!(next.timestamp, merged.items[1].sort_timestamp());
}

/// A dropped row still consumed a slot, so the has-more probe must be derived
/// from the RAW row count — otherwise one malformed record truncates a timeline.
#[test]
fn a_dropped_row_still_counts_towards_page_saturation() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let source_page = SourcePage {
        records: vec![
            api_record("ev-0", VIEWER_ID, 0, ActivitySourceKind::Posthog),
            api_record("ev-1", VIEWER_ID, 10, ActivitySourceKind::Posthog),
        ],
        raw_rows: 3,
        row_errors: 1,
    };
    let merged = merge(&constraint, Some(Ok(source_page)), None, 2, 3).expect("posthog answered");
    assert_eq!(merged.items.len(), 2);
    assert!(merged.next_key.is_some());
    assert_eq!(merged.row_errors, 1);
    assert!(merged.status.partial);
    assert_eq!(merged.status.posthog, SourceHealth::Degraded);
    assert_eq!(
        merged.status.message_code,
        Some(message_codes::ROWS_DROPPED)
    );
}

/// The defence-in-depth assertion: a source that regressed its predicate cannot
/// leak a foreign row through the merge, and the drop is operator telemetry only.
#[test]
fn a_record_contradicting_the_personal_constraint_is_dropped_defensively() {
    let constraint = mine(VIEWER_ID, VIEWER, None);
    let merged = merge(
        &constraint,
        Some(Ok(page(vec![
            api_record("ev-mine", VIEWER_ID, 10, ActivitySourceKind::Posthog),
            api_record("ev-theirs", 999, 20, ActivitySourceKind::Posthog),
        ]))),
        None,
        10,
        11,
    )
    .expect("posthog answered");
    assert_eq!(ids(&merged), vec!["ev-mine"]);
    assert_eq!(merged.constraint_violations, 1);
    assert_eq!(
        merged.row_errors, 0,
        "a constraint violation is NOT user-visible row-error metadata"
    );
    assert!(!merged.status.partial);
}

/// A lifecycle row is visible only through its authorized session, never through
/// an actor. A row for some other session must not survive the merge.
#[test]
fn a_lifecycle_row_for_another_session_is_dropped_in_personal_scope() {
    let session = authorized_session(SESSION, VIEWER_ID, VIEWER);
    let constraint = mine(VIEWER_ID, VIEWER, Some(session));
    let merged = merge(
        &constraint,
        Some(Ok(page(vec![
            lifecycle_record("ev-ok", SESSION, 10, ActivitySourceKind::Posthog),
            lifecycle_record(
                "ev-other",
                "sess-someone-else",
                5,
                ActivitySourceKind::Posthog,
            ),
        ]))),
        None,
        10,
        11,
    )
    .expect("posthog answered");
    assert_eq!(ids(&merged), vec!["ev-ok"]);
    assert_eq!(merged.constraint_violations, 1);
}

#[test]
fn the_global_scope_admits_every_actor_and_system_row() {
    let constraint = all(900, "root");
    let merged = merge(
        &constraint,
        Some(Ok(page(vec![
            api_record("ev-a", 101, 10, ActivitySourceKind::Posthog),
            api_record("ev-b", 202, 20, ActivitySourceKind::Posthog),
            lifecycle_record("ev-c", "sess-anything", 30, ActivitySourceKind::Posthog),
        ]))),
        None,
        10,
        11,
    )
    .expect("posthog answered");
    assert_eq!(ids(&merged), vec!["ev-a", "ev-b", "ev-c"]);
    assert_eq!(merged.constraint_violations, 0);
}
