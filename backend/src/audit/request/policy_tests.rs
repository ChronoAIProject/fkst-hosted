//! Unit tests for the explicit per-operation audit policy table.

use super::*;
use std::collections::BTreeSet;

#[test]
fn every_operation_id_appears_exactly_once() {
    let mut seen = BTreeSet::new();
    for id in declared_operation_ids() {
        assert!(seen.insert(id), "duplicate policy entry for {id}");
    }
    assert_eq!(seen.len(), OPERATION_POLICIES.len());
}

/// The exclusion list is the security-relevant half of the table: widening it is
/// how audit coverage quietly disappears, so it is pinned exactly.
#[test]
fn only_the_probe_scrape_and_contract_traffic_is_excluded() {
    let excluded: Vec<_> = OPERATION_POLICIES
        .iter()
        .filter(|(_, policy)| !policy.is_audited())
        .map(|(id, policy)| (*id, *policy))
        .collect();
    assert_eq!(
        excluded,
        vec![
            ("health", OperationPolicy::Excluded(ExclusionReason::Probe)),
            (
                "readiness",
                OperationPolicy::Excluded(ExclusionReason::Probe)
            ),
            (
                "metrics",
                OperationPolicy::Excluded(ExclusionReason::Scrape)
            ),
        ]
    );
}

#[test]
fn the_product_surface_is_audited_including_webhook_chat_and_oauth() {
    for audited in [
        "github_app_webhook",
        "chat_turn",
        "github_login_callback",
        "session_logs_oauth_callback",
        "canvas_overview",
        "observe_session",
    ] {
        assert_eq!(
            policy_for(audited),
            Some(OperationPolicy::Audited),
            "{audited} must be audited"
        );
    }
}

#[test]
fn an_unknown_operation_has_no_policy() {
    assert_eq!(policy_for("operations_list_activity"), None);
    assert_eq!(policy_for(""), None);
}

#[test]
fn the_contract_document_is_the_only_excluded_undocumented_route() {
    assert_eq!(
        undocumented_route_policy("GET", "/openapi.json"),
        Some(OperationPolicy::Excluded(ExclusionReason::Contract))
    );
    // A different method on the same path is NOT pre-excluded: it reaches the
    // 405 path and is audited like any other undocumented call.
    assert_eq!(undocumented_route_policy("POST", "/openapi.json"), None);
    assert_eq!(undocumented_route_policy("GET", "/health"), None);
}

#[test]
fn exclusion_reasons_render_bounded_labels() {
    for (reason, expected) in [
        (ExclusionReason::Probe, "probe"),
        (ExclusionReason::Scrape, "scrape"),
        (ExclusionReason::Contract, "contract"),
        (ExclusionReason::CorsPreflight, "cors_preflight"),
    ] {
        assert_eq!(reason.as_str(), expected);
        assert_eq!(reason.to_string(), expected);
    }
}
