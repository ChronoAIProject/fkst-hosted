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
    let request = normalize(&params(None)).expect("normalizes");
    assert_eq!(request.requested_scope, None);
    assert_eq!(request.filters, SandboxFilters::default());
}

#[test]
fn the_route_vocabulary_maps_onto_the_shared_scope_request() {
    assert_eq!(
        normalize(&params(Some("accessible")))
            .expect("normalizes")
            .requested_scope,
        Some(RequestedScope::Personal)
    );
    assert_eq!(
        normalize(&params(Some(" all ")))
            .expect("normalizes")
            .requested_scope,
        Some(RequestedScope::Global)
    );
}

/// The activity endpoint's `mine` is deliberately NOT accepted here: two closed
/// vocabularies that silently accept each other's words stop being closed.
#[test]
fn an_unknown_scope_is_rejected_by_name() {
    for value in ["mine", "everything", ""] {
        let error = normalize(&params(Some(value))).expect_err("rejected");
        assert!(format!("{error}").contains("scope must be accessible or all"));
    }
}

#[test]
fn every_filter_is_normalized_into_its_stored_form() {
    let request = normalize(&SandboxQueryParams {
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
    let filters = &request.filters;
    assert_eq!(filters.status, Some(RuntimeInventoryStatus::Running));
    assert_eq!(filters.backend, Some(RuntimeBackendKind::OpenSandbox));
    assert_eq!(filters.creator_id, Some(101));
    assert_eq!(filters.creator_login.as_deref(), Some("Alice"));
    assert_eq!(filters.repo_full_name.as_deref(), Some("acme/site"));
    assert_eq!(request.session_id(), Some("sess-alice"));
    assert_eq!(filters.trigger_issue, Some(7));
    assert_eq!(
        filters.attribution_source,
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
        let error = normalize(&params).expect_err("rejected");
        assert!(format!("{error}").contains(expected), "{error}");
    }
}

/// An exact unauthorized/nonexistent session probe must be indistinguishable, so
/// the rejection cannot echo the id that failed.
#[test]
fn a_rejection_never_echoes_the_value_that_failed() {
    let error = normalize(&SandboxQueryParams {
        session_id: Some("canary-session/../escape".to_string()),
        ..SandboxQueryParams::default()
    })
    .expect_err("rejected");
    assert!(!format!("{error}").contains("canary-session"), "{error}");
}
