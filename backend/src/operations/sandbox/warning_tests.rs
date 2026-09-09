//! Unit tests for the public warning projection.

use super::*;
use std::collections::BTreeSet;

#[test]
fn every_internal_code_except_source_truncation_has_a_public_code() {
    let internal = [
        InventoryWarningCode::MissingSessionId,
        InventoryWarningCode::MalformedCorrelation,
        InventoryWarningCode::MalformedIdentity,
        InventoryWarningCode::AttributionConflict,
        InventoryWarningCode::MissingCreatedAt,
        InventoryWarningCode::MalformedCreatedAt,
        InventoryWarningCode::MalformedLastPending,
        InventoryWarningCode::ClockSkew,
        InventoryWarningCode::LifetimeOverflow,
        InventoryWarningCode::UnknownStatus,
        InventoryWarningCode::WarningsTruncated,
    ];
    for code in internal {
        assert!(
            public_code(code).is_some(),
            "{code} has no public projection"
        );
    }
    assert_eq!(
        public_code(InventoryWarningCode::SourceTruncated),
        None,
        "a clipped page walk is an incompleteness FAILURE the service answers \
         with 503, not a warning a client can act on"
    );
}

#[test]
fn every_public_code_renders_a_distinct_stable_wire_value() {
    let rendered: BTreeSet<&str> = SandboxWarningCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect();
    assert_eq!(rendered.len(), SandboxWarningCode::ALL.len());
    for code in SandboxWarningCode::ALL {
        assert_eq!(format!("{code}"), code.as_str());
        assert!(
            !code.as_str().is_empty()
                && code
                    .as_str()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
        );
    }
}

/// Normalization is what makes the response field deterministic: the same set of
/// codes always renders in the same order, whatever order they were collected in.
#[test]
fn normalization_sorts_and_deduplicates_into_the_fixed_order() {
    let normalized = normalize(vec![
        SandboxWarningCode::UnknownStatus,
        SandboxWarningCode::MissingCreatedAt,
        SandboxWarningCode::UnknownStatus,
        SandboxWarningCode::AttributionConflict,
    ]);
    assert_eq!(
        normalized,
        vec![
            SandboxWarningCode::AttributionConflict,
            SandboxWarningCode::MissingCreatedAt,
            SandboxWarningCode::UnknownStatus,
        ]
    );
    assert_eq!(normalize(Vec::new()), Vec::new());
}

/// The declared order is the enum's own, so `ALL` is a faithful rendering order.
#[test]
fn the_declared_order_is_the_sort_order() {
    let mut sorted = SandboxWarningCode::ALL.to_vec();
    sorted.sort_unstable();
    assert_eq!(sorted, SandboxWarningCode::ALL.to_vec());
}
