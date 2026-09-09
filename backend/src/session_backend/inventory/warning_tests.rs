//! The bounded warning sink: correlation is preserved, the ceiling holds, and a
//! full sink says so instead of silently dropping.

use super::*;

#[test]
fn a_pushed_warning_keeps_its_runtime_and_session_correlation() {
    let mut sink = WarningSink::default();
    sink.push(
        InventoryWarningCode::MalformedCorrelation,
        Some("fkst-sess-a"),
        Some("sess-a"),
    );
    assert_eq!(
        sink.into_warnings(),
        vec![BoundedInventoryWarning {
            code: InventoryWarningCode::MalformedCorrelation,
            runtime_id: Some("fkst-sess-a".to_string()),
            session_id: Some("sess-a".to_string()),
        }]
    );
}

#[test]
fn a_snapshot_warning_carries_no_correlation() {
    let mut sink = WarningSink::default();
    sink.push_snapshot(InventoryWarningCode::SourceTruncated);
    let warnings = sink.into_warnings();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].runtime_id, None);
    assert_eq!(warnings[0].session_id, None);
}

#[test]
fn an_orphan_warning_may_name_the_runtime_without_a_session() {
    let mut sink = WarningSink::default();
    sink.push(
        InventoryWarningCode::MissingSessionId,
        Some("fkst-sess-orphan"),
        None,
    );
    let warnings = sink.into_warnings();
    assert_eq!(warnings[0].runtime_id.as_deref(), Some("fkst-sess-orphan"));
    assert_eq!(warnings[0].session_id, None);
}

#[test]
fn the_sink_stops_at_its_ceiling_and_says_so() {
    let mut sink = WarningSink::new(4);
    for index in 0..50 {
        sink.push(
            InventoryWarningCode::ClockSkew,
            Some(&format!("runtime-{index}")),
            None,
        );
    }
    let warnings = sink.into_warnings();
    // Three real warnings plus the marker: never more than the ceiling.
    assert_eq!(warnings.len(), 4);
    assert_eq!(
        warnings.last().expect("marker").code,
        InventoryWarningCode::WarningsTruncated
    );
    // The marker appears exactly once no matter how many pushes overflowed.
    assert_eq!(
        warnings
            .iter()
            .filter(|w| w.code == InventoryWarningCode::WarningsTruncated)
            .count(),
        1
    );
}

#[test]
fn a_zero_ceiling_still_leaves_room_for_the_marker() {
    let mut sink = WarningSink::new(0);
    sink.push(InventoryWarningCode::ClockSkew, Some("r"), None);
    let warnings = sink.into_warnings();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, InventoryWarningCode::WarningsTruncated);
}

#[test]
fn a_fresh_sink_is_empty() {
    let sink = WarningSink::default();
    assert!(sink.is_empty());
    assert_eq!(sink.len(), 0);
}

#[test]
fn every_code_has_a_distinct_stable_spelling() {
    let codes = [
        InventoryWarningCode::MissingSessionId,
        InventoryWarningCode::MalformedCorrelation,
        InventoryWarningCode::MalformedIdentity,
        InventoryWarningCode::MissingCreatedAt,
        InventoryWarningCode::MalformedCreatedAt,
        InventoryWarningCode::MalformedLastPending,
        InventoryWarningCode::ClockSkew,
        InventoryWarningCode::LifetimeOverflow,
        InventoryWarningCode::UnknownStatus,
        InventoryWarningCode::SourceTruncated,
        InventoryWarningCode::WarningsTruncated,
    ];
    let mut spellings: Vec<&str> = codes.iter().map(|code| code.as_str()).collect();
    spellings.sort_unstable();
    let unique = spellings.len();
    spellings.dedup();
    assert_eq!(spellings.len(), unique, "codes must not share a spelling");
    // The Display impl is what a bounded metric/response label renders.
    assert_eq!(
        InventoryWarningCode::ClockSkew.to_string(),
        "clock_skew".to_string()
    );
}
