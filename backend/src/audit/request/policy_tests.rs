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
        .filter(|operation| !operation.policy.is_audited())
        .map(|operation| (operation.operation_id, operation.policy))
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
    // `operations_list_sandboxes` is still RESERVED: its DTO is reviewed, but its
    // route does not exist, so the live table must not name it.
    assert_eq!(policy_for("operations_list_sandboxes"), None);
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

/// The pairing that makes the table an [`AuditOperation`] rather than a string
/// list: an audited operation must have decided what its record may contain.
#[test]
fn every_audited_operation_declares_a_safe_argument_policy() {
    for operation in OPERATION_POLICIES {
        if operation.policy.is_audited() {
            assert_ne!(
                operation.arguments,
                ArgumentsPolicy::NotRecorded,
                "{} is audited but declares no safe-argument policy",
                operation.operation_id
            );
        } else {
            assert_eq!(
                operation.arguments,
                ArgumentsPolicy::NotRecorded,
                "{} is excluded, so it may never declare arguments",
                operation.operation_id
            );
        }
    }
}

/// One DTO per operation, in both directions: a second policy on one operation
/// would make the recorded shape depend on which call site ran last.
#[test]
fn no_dto_is_shared_between_two_operations() {
    let mut seen = BTreeSet::new();
    for operation in OPERATION_POLICIES
        .iter()
        .chain(RESERVED_ARGUMENT_POLICIES.iter())
    {
        if let Some(spec) = operation.arguments.spec() {
            assert!(
                seen.insert(spec.dto),
                "{} reuses the DTO {} another operation already owns",
                operation.operation_id,
                spec.dto
            );
        }
    }
}

/// The default status is what classifies a request rejected before its safe
/// parse could run. Getting it wrong makes an authentication failure look like
/// an endpoint that simply has no arguments.
#[test]
fn the_default_status_distinguishes_unavailable_from_not_applicable() {
    assert_eq!(
        default_arguments_status("canvas_create_session"),
        ArgumentsParseStatus::Unavailable
    );
    assert_eq!(
        default_arguments_status("list_user_environment_profiles"),
        ArgumentsParseStatus::NotApplicable
    );
    assert_eq!(
        default_arguments_status("health"),
        ArgumentsParseStatus::NotApplicable
    );
    // The `<unmatched>` sentinel has no argument contract to have run.
    assert_eq!(
        default_arguments_status("<unmatched>"),
        ArgumentsParseStatus::NotApplicable
    );
}

/// The reserved entries are exactly the operations whose ROUTES do not exist yet,
/// and they stay OUT of the live table until they do. `operations_list_activity`
/// graduated with its route (issue #5672); `operations_list_sandboxes` follows in
/// #5675.
#[test]
fn the_reserved_operations_are_declared_but_not_yet_live() {
    let reserved: Vec<&str> = RESERVED_ARGUMENT_POLICIES
        .iter()
        .map(|operation| operation.operation_id)
        .collect();
    assert_eq!(reserved, vec!["operations_list_sandboxes"]);
    for operation in RESERVED_ARGUMENT_POLICIES {
        assert!(operation.policy.is_audited());
        assert!(operation.arguments.spec().is_some());
        assert_eq!(
            operation_for(operation.operation_id),
            None,
            "{} must not be live until its route exists",
            operation.operation_id
        );
    }
}
