//! Unit tests for parameter normalization.
//!
//! Normalization is the ONLY step that runs before any authorization decision, so
//! the properties asserted here are: it is total (every accepted value has one
//! normalized form), it is pure (no state, no registry, no backend), and it never
//! echoes a value it refused.

use super::*;
use crate::runtime_identity::{AttributionSource, RuntimeBackendKind};
use crate::session_backend::inventory::RuntimeInventoryStatus;

fn params(scope: Option<&str>) -> SandboxQueryParams {
    SandboxQueryParams {
        scope: scope.map(str::to_string),
        ..SandboxQueryParams::default()
    }
}

#[test]
fn an_omitted_scope_stays_unstated_so_the_server_resolves_the_default() {
    assert_eq!(requested_scope(&params(None)).expect("normalizes"), None);
    assert_eq!(
        filters(&params(None)).expect("normalizes"),
        SandboxFilters::default()
    );
}

#[test]
fn the_route_vocabulary_maps_onto_the_shared_scope_request() {
    assert_eq!(
        requested_scope(&params(Some("accessible"))).expect("normalizes"),
        Some(RequestedScope::Personal)
    );
    assert_eq!(
        requested_scope(&params(Some(" all "))).expect("normalizes"),
        Some(RequestedScope::Global)
    );
}

/// The two halves are independent entry points, which is what lets the route
/// decide the scope before it validates anything else: a malformed filter must
/// not make the scope question unanswerable.
#[test]
fn a_malformed_filter_does_not_prevent_the_scope_from_resolving() {
    let hostile = SandboxQueryParams {
        scope: Some("all".to_string()),
        status: Some("melted".to_string()),
        ..SandboxQueryParams::default()
    };
    assert_eq!(
        requested_scope(&hostile).expect("the scope word is well formed"),
        Some(RequestedScope::Global)
    );
    assert!(filters(&hostile).is_err());
}

/// The activity endpoint's `mine` is deliberately NOT accepted here: two closed
/// vocabularies that silently accept each other's words stop being closed.
#[test]
fn an_unknown_scope_is_rejected_by_name() {
    for value in ["mine", "everything", ""] {
        let error = requested_scope(&params(Some(value))).expect_err("rejected");
        assert!(format!("{error}").contains("scope must be accessible or all"));
    }
}

#[test]
fn every_filter_is_normalized_into_its_stored_form() {
    let normalized = filters(&SandboxQueryParams {
        scope: Some("accessible".to_string()),
        status: Some("running".to_string()),
        backend: Some("opensandbox".to_string()),
        creator_id: Some(101),
        creator_login: Some("@Alice".to_string()),
        repo_full_name: Some("acme/site".to_string()),
        session_id: Some("sess-alice".to_string()),
        trigger_issue: Some(7),
        attribution_source: Some("unknown_legacy".to_string()),
    })
    .expect("normalizes");
    assert_eq!(normalized.status, Some(RuntimeInventoryStatus::Running));
    assert_eq!(normalized.backend, Some(RuntimeBackendKind::OpenSandbox));
    assert_eq!(normalized.creator_id, Some(101));
    assert_eq!(normalized.creator_login.as_deref(), Some("Alice"));
    assert_eq!(normalized.repo_full_name.as_deref(), Some("acme/site"));
    assert_eq!(normalized.session_id(), Some("sess-alice"));
    assert_eq!(normalized.trigger_issue, Some(7));
    assert_eq!(
        normalized.attribution_source,
        Some(AttributionSource::UnknownLegacy)
    );
}

#[test]
fn every_invalid_filter_is_rejected_and_names_its_own_parameter() {
    let cases: Vec<(SandboxQueryParams, &str)> = vec![
        (
            SandboxQueryParams {
                status: Some("melted".to_string()),
                ..SandboxQueryParams::default()
            },
            "status",
        ),
        (
            SandboxQueryParams {
                backend: Some("nomad".to_string()),
                ..SandboxQueryParams::default()
            },
            "backend",
        ),
        (
            SandboxQueryParams {
                creator_id: Some(-1),
                ..SandboxQueryParams::default()
            },
            "creator_id",
        ),
        (
            SandboxQueryParams {
                creator_login: Some("not a login".to_string()),
                ..SandboxQueryParams::default()
            },
            "creator_login",
        ),
        (
            SandboxQueryParams {
                repo_full_name: Some("acme".to_string()),
                ..SandboxQueryParams::default()
            },
            "repo_full_name",
        ),
        (
            SandboxQueryParams {
                session_id: Some("sess/../etc".to_string()),
                ..SandboxQueryParams::default()
            },
            "session_id",
        ),
        (
            SandboxQueryParams {
                trigger_issue: Some(0),
                ..SandboxQueryParams::default()
            },
            "trigger_issue",
        ),
        (
            SandboxQueryParams {
                attribution_source: Some("vibes".to_string()),
                ..SandboxQueryParams::default()
            },
            "attribution_source",
        ),
    ];
    for (params, expected) in cases {
        let error = filters(&params).expect_err("rejected");
        assert!(format!("{error}").contains(expected), "{error}");
    }
}

/// An exact unauthorized/nonexistent session probe must be indistinguishable, so
/// the rejection cannot echo the id that failed.
#[test]
fn a_rejection_never_echoes_the_value_that_failed() {
    let error = filters(&SandboxQueryParams {
        session_id: Some("canary-session/../escape".to_string()),
        ..SandboxQueryParams::default()
    })
    .expect_err("rejected");
    assert!(!format!("{error}").contains("canary-session"), "{error}");
}
