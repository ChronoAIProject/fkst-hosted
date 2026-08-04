//! Unit tests for route normalization and the verified operation catalog.

use super::*;
use utoipa::openapi::path::{HttpMethod, Operation, Paths};
use utoipa::openapi::OpenApi;

/// Build a document from `(method, path, operationId)` triples.
fn doc(entries: &[(HttpMethod, &str, Option<&str>)]) -> OpenApi {
    let mut paths = Paths::new();
    for (method, path, operation_id) in entries {
        let mut operation = Operation::new();
        operation.operation_id = operation_id.map(str::to_string);
        paths.add_path_operation(*path, vec![method.clone()], operation);
    }
    let mut document = OpenApi::new(utoipa::openapi::Info::new("test", "0"), Paths::new());
    document.paths = paths;
    document
}

fn catalog(entries: &[(HttpMethod, &str, Option<&str>)]) -> OperationCatalog {
    OperationCatalog::from_openapi(&doc(entries)).expect("catalog builds")
}

#[test]
fn normalizes_axum_parameters_into_openapi_templates() {
    assert_eq!(normalize_matched_path("/health"), "/health");
    assert_eq!(
        normalize_matched_path("/api/v1/logs/:session_id"),
        "/api/v1/logs/{session_id}"
    );
    assert_eq!(
        normalize_matched_path("/api/v1/repos/:owner/:name/sessions/:issue_number/outcomes"),
        "/api/v1/repos/{owner}/{name}/sessions/{issue_number}/outcomes"
    );
    assert_eq!(normalize_matched_path("/files/*rest"), "/files/{rest}");
}

#[test]
fn resolves_an_audited_operation_from_the_matched_template() {
    let catalog = catalog(&[(HttpMethod::Get, "/api/v1/overview", Some("canvas_overview"))]);
    let decision = catalog.resolve(&Method::GET, Some("/api/v1/overview"));
    match decision {
        RouteDecision::Record {
            operation_id,
            route_template,
        } => {
            assert_eq!(&*operation_id, "canvas_overview");
            assert_eq!(&*route_template, "/api/v1/overview");
        }
        other => panic!("expected a recorded operation, got {other:?}"),
    }
}

#[test]
fn resolves_a_parameterized_nested_template_including_the_api_prefix() {
    let catalog = catalog(&[(
        HttpMethod::Get,
        "/api/v1/logs/{session_id}/manifest",
        Some("session_log_manifest"),
    )]);
    match catalog.resolve(&Method::GET, Some("/api/v1/logs/:session_id/manifest")) {
        RouteDecision::Record {
            operation_id,
            route_template,
        } => {
            assert_eq!(&*operation_id, "session_log_manifest");
            assert_eq!(&*route_template, "/api/v1/logs/{session_id}/manifest");
        }
        other => panic!("expected a recorded operation, got {other:?}"),
    }
}

#[test]
fn excluded_operations_are_skipped_with_their_reason() {
    let catalog = catalog(&[
        (HttpMethod::Get, "/health", Some("health")),
        (HttpMethod::Get, "/metrics", Some("metrics")),
    ]);
    assert_eq!(
        catalog.resolve(&Method::GET, Some("/health")),
        RouteDecision::Skip(ExclusionReason::Probe)
    );
    assert_eq!(
        catalog.resolve(&Method::GET, Some("/metrics")),
        RouteDecision::Skip(ExclusionReason::Scrape)
    );
}

#[test]
fn the_contract_document_and_cors_preflights_are_skipped() {
    let catalog = catalog(&[(HttpMethod::Get, "/health", Some("health"))]);
    assert_eq!(
        catalog.resolve(&Method::GET, Some("/openapi.json")),
        RouteDecision::Skip(ExclusionReason::Contract)
    );
    // A preflight is answered before any handler, whatever it targets.
    assert_eq!(
        catalog.resolve(&Method::OPTIONS, Some("/health")),
        RouteDecision::Skip(ExclusionReason::CorsPreflight)
    );
    assert_eq!(
        catalog.resolve(&Method::OPTIONS, None),
        RouteDecision::Skip(ExclusionReason::CorsPreflight)
    );
}

/// The redaction rule that matters most: an unrouted path may carry an OAuth
/// `code`/`state`, so neither it nor anything derived from it is recorded.
#[test]
fn an_unmatched_path_records_only_the_sentinels() {
    let catalog = catalog(&[(HttpMethod::Get, "/health", Some("health"))]);
    match catalog.resolve(&Method::GET, None) {
        RouteDecision::Record {
            operation_id,
            route_template,
        } => {
            assert_eq!(&*operation_id, "<unmatched>");
            assert_eq!(&*route_template, "<unmatched>");
        }
        other => panic!("an unmatched path must still be recorded, got {other:?}"),
    }
}

/// A matched path whose method has no documented operation (axum's `405`): the
/// template is a documented constant, so it is kept, but the operation is not
/// invented.
#[test]
fn a_matched_path_without_an_operation_keeps_the_template() {
    let catalog = catalog(&[(HttpMethod::Get, "/api/v1/overview", Some("canvas_overview"))]);
    match catalog.resolve(&Method::POST, Some("/api/v1/overview")) {
        RouteDecision::Record {
            operation_id,
            route_template,
        } => {
            assert_eq!(&*operation_id, "<unmatched>");
            assert_eq!(&*route_template, "/api/v1/overview");
        }
        other => panic!("expected a recorded unmatched operation, got {other:?}"),
    }
}

/// axum serves HEAD from the GET handler, so a HEAD probe must inherit the GET
/// operation's policy — otherwise a HEAD-based uptime check against `/health`
/// would pump exactly the noise the exclusions exist to keep out.
#[test]
fn a_head_request_inherits_the_get_operations_policy() {
    let catalog = catalog(&[
        (HttpMethod::Get, "/health", Some("health")),
        (HttpMethod::Get, "/api/v1/overview", Some("canvas_overview")),
    ]);
    assert_eq!(
        catalog.resolve(&Method::HEAD, Some("/health")),
        RouteDecision::Skip(ExclusionReason::Probe)
    );
    // The undocumented-route table is keyed the same way.
    assert_eq!(
        catalog.resolve(&Method::HEAD, Some("/openapi.json")),
        RouteDecision::Skip(ExclusionReason::Contract)
    );
    // An audited GET stays audited under HEAD, recorded as the same operation.
    match catalog.resolve(&Method::HEAD, Some("/api/v1/overview")) {
        RouteDecision::Record { operation_id, .. } => {
            assert_eq!(&*operation_id, "canvas_overview")
        }
        other => panic!("expected the GET operation, got {other:?}"),
    }
}

/// The fall-back is HEAD-only and one-directional: no other method may borrow a
/// GET operation's identity or its exclusion.
#[test]
fn no_other_method_borrows_the_get_operations_policy() {
    let catalog = catalog(&[(HttpMethod::Get, "/health", Some("health"))]);
    for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
        match catalog.resolve(&method, Some("/health")) {
            RouteDecision::Record { operation_id, .. } => {
                assert_eq!(&*operation_id, "<unmatched>", "{method}")
            }
            other => panic!("{method} must not inherit the GET policy, got {other:?}"),
        }
    }
}

#[test]
fn an_operation_without_an_operation_id_fails_the_build() {
    let error = OperationCatalog::from_openapi(&doc(&[(HttpMethod::Get, "/thing", None)]))
        .expect_err("a missing operationId must fail the build");
    assert_eq!(
        error,
        CatalogError::MissingOperationId {
            method: "GET",
            path: "/thing".to_string()
        }
    );
}

#[test]
fn a_duplicate_operation_id_fails_the_build() {
    let error = OperationCatalog::from_openapi(&doc(&[
        (HttpMethod::Get, "/a", Some("health")),
        (HttpMethod::Get, "/b", Some("health")),
    ]))
    .expect_err("a duplicate operationId must fail the build");
    assert_eq!(
        error,
        CatalogError::DuplicateOperationId {
            operation_id: "health".to_string()
        }
    );
}

#[test]
fn an_operation_without_an_explicit_policy_fails_the_build() {
    let error =
        OperationCatalog::from_openapi(&doc(&[(HttpMethod::Get, "/new", Some("brand_new_thing"))]))
            .expect_err("an unpoliced operation must fail the build");
    assert_eq!(
        error,
        CatalogError::UnpolicedOperation {
            operation_id: "brand_new_thing".to_string()
        }
    );
}

/// The OTHER half of the decision, guarded on its own: an operation that is
/// recorded but never chose an argument boundary must fail the build too.
///
/// The real table cannot express this — [`audited`] always pairs `Audited` with
/// a named DTO — so the declaration is written out by hand here. Without this
/// case the branch would be code nobody has ever seen execute.
#[test]
fn an_audited_operation_without_an_argument_policy_fails_the_build() {
    const NO_ARGUMENT_POLICY: &[AuditOperation] = &[AuditOperation {
        operation_id: "audited_but_undecided",
        policy: OperationPolicy::Audited,
        arguments: ArgumentsPolicy::NotRecorded,
    }];
    let error = OperationCatalog::from_openapi_with(
        &doc(&[(HttpMethod::Get, "/new", Some("audited_but_undecided"))]),
        NO_ARGUMENT_POLICY,
    )
    .expect_err("an audited operation with no argument policy must fail the build");
    assert_eq!(
        error,
        CatalogError::MissingArgumentPolicy {
            operation_id: "audited_but_undecided".to_string()
        }
    );
    // The message has to name the fix, because it is read by whoever added the
    // endpoint, in CI, with no other context.
    let rendered = error.to_string();
    assert!(rendered.contains("audited_but_undecided"), "{rendered}");
    assert!(rendered.contains("ArgumentsPolicy::Safe"), "{rendered}");
}

/// The same declaration, with the argument half supplied, builds — so the case
/// above fails for the stated reason and not because the fixture was broken.
#[test]
fn the_same_operation_builds_once_it_declares_an_argument_policy() {
    const DECIDED: &[AuditOperation] = &[AuditOperation {
        operation_id: "audited_but_undecided",
        policy: OperationPolicy::Audited,
        arguments: ArgumentsPolicy::None,
    }];
    let catalog = OperationCatalog::from_openapi_with(
        &doc(&[(HttpMethod::Get, "/new", Some("audited_but_undecided"))]),
        DECIDED,
    )
    .expect("both halves of the decision exist");
    assert_eq!(catalog.len(), 1);
}

#[test]
fn lookup_reports_the_declared_operations() {
    let catalog = catalog(&[
        (HttpMethod::Get, "/health", Some("health")),
        (HttpMethod::Get, "/ready", Some("readiness")),
    ]);
    assert_eq!(catalog.len(), 2);
    assert!(!catalog.is_empty());
    let mut ids: Vec<_> = catalog.operation_ids().collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["health", "readiness"]);
    assert!(catalog.lookup(&Method::GET, "/health").is_some());
    assert!(catalog.lookup(&Method::POST, "/health").is_none());
    assert!(catalog.lookup(&Method::GET, "/nope").is_none());
}
