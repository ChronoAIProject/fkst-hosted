//! The audit-coverage guard: every operation in the generated OpenAPI document
//! must carry exactly one EXPLICIT audit policy.
//!
//! This is the test that makes a new endpoint fail CI until someone decides
//! whether it is audited AND what its record may contain. It reads the live
//! document (with both conditionally mounted operations enabled) rather than a
//! checked-in list, so it tracks the code the way the spec itself does.
//!
//! The guard fires for five distinct mistakes:
//!
//! - an audited operation with no safe-argument DTO;
//! - two DTOs mapped onto one operation (or one DTO onto two);
//! - a DTO whose allowlist is not the documented one;
//! - a route reaching for a generic/raw capture helper instead of the typed one;
//! - drift in an operation id without the table following it.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::audit::request::policy::{
    arguments_policy_for, declared_operation_ids, policy_for, ArgumentsPolicy, ExclusionReason,
    OperationPolicy, OPERATION_POLICIES, RESERVED_ARGUMENT_POLICIES,
};
use fkst_control_plane::config::Config;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::{empty_self_router, AppState};
use http_body_util::BodyExt;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use tower::ServiceExt;

/// The eight OpenAPI operation keys a path item can carry.
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Build the router with EVERY conditionally mounted operation present, so the
/// document is the complete surface rather than one deployment's subset.
fn full_surface_router() -> axum::Router {
    let chat_config = fkst_control_plane::chat::config::from_vars(&[
        ("FKST_CHAT_ENABLED".to_string(), "true".to_string()),
        (
            "FKST_LLM_BASE_URL".to_string(),
            "https://llm.example/v1".to_string(),
        ),
        ("FKST_LLM_API_KEY".to_string(), "dummy-chat-key".to_string()),
        ("FKST_LLM_MODEL".to_string(), "dummy-model".to_string()),
    ])
    .expect("chat config parses")
    .expect("chat config is enabled");

    build_router(AppState {
        config: Config::default(),
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: Some(secrecy::SecretString::from(
            "audit-policy-test-secret".to_string(),
        )),
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        operations: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: empty_self_router(),
        chat: Some(std::sync::Arc::new(
            fkst_control_plane::chat::ChatRuntime::from_config(chat_config),
        )),
        audit: Default::default(),
    })
    .expect("the router must build; a build failure here IS the coverage guard firing")
}

/// `operationId -> "METHOD path"` for every operation in the served document.
async fn live_operations() -> BTreeMap<String, String> {
    let response = full_surface_router()
        .oneshot(
            Request::get("/openapi.json")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let spec: Value = serde_json::from_slice(&bytes).expect("valid JSON");

    let mut operations = BTreeMap::new();
    for (path, item) in spec["paths"].as_object().expect("paths object") {
        for method in METHODS {
            let Some(operation) = item.get(method) else {
                continue;
            };
            let operation_id = operation["operationId"]
                .as_str()
                .unwrap_or_else(|| {
                    panic!("{} {path} declares no operationId", method.to_uppercase())
                })
                .to_string();
            let previous = operations.insert(
                operation_id.clone(),
                format!("{} {path}", method.to_uppercase()),
            );
            assert!(
                previous.is_none(),
                "operationId {operation_id} is declared twice"
            );
        }
    }
    operations
}

/// The operations in `surface` that carry no explicit policy.
///
/// Split out of the guard so the guard's FAILURE direction is testable: with the
/// check written inline over `live_operations()`, a synthetic unpoliced operation
/// could not be injected, and nothing proved the assertion would actually fire.
/// A guard whose negative direction is never exercised is indistinguishable from
/// a guard that silently stopped working.
fn unpoliced(surface: &BTreeMap<String, String>) -> Vec<String> {
    surface
        .iter()
        .filter(|(operation_id, _)| policy_for(operation_id).is_none())
        .map(|(operation_id, route)| format!("{operation_id} ({route})"))
        .collect()
}

/// The guard itself: adding a product endpoint without an audit policy fails
/// here (and, because the catalog is built at router assembly, at startup too).
#[tokio::test]
async fn every_documented_operation_has_exactly_one_explicit_policy() {
    let unpoliced = unpoliced(&live_operations().await);
    assert!(
        unpoliced.is_empty(),
        "these operations have no explicit audit policy; add an Audited or \
         Excluded entry to audit::request::policy::OPERATION_POLICIES: {unpoliced:?}"
    );
}

/// The same guard, driven the other way: a synthetic operation that nobody gave
/// a policy MUST be reported.
///
/// The issue asks for exactly this ("adding a synthetic operation without policy
/// fails the guard"), and it cannot be shown by the positive test — a green run
/// there is equally consistent with "everything is policed" and with "the check
/// examines nothing". Injecting the operation into the surface map, rather than
/// into the router, keeps the proof deterministic and costs no fake endpoint.
#[tokio::test]
async fn a_synthetic_operation_without_a_policy_fails_the_guard() {
    let mut surface = live_operations().await;
    assert!(
        unpoliced(&surface).is_empty(),
        "the real surface must be clean before the injection means anything"
    );
    surface.insert(
        "synthetic_operation_nobody_policed".to_string(),
        "GET /api/v1/synthetic".to_string(),
    );
    let reported = unpoliced(&surface);
    assert_eq!(
        reported,
        vec!["synthetic_operation_nobody_policed (GET /api/v1/synthetic)".to_string()],
        "the coverage guard did not fire on an unpoliced operation"
    );
}

/// Pin the surface size so a silently REMOVED operation is noticed too — the
/// policy table would otherwise keep a stale entry forever.
#[tokio::test]
async fn the_audited_surface_is_the_expected_size_and_shape() {
    let operations = live_operations().await;
    // 29 before milestone #22: the scoped activity query (issue #5672) added
    // `operations_list_activity` and the live sandbox inventory (#5675) added
    // `operations_list_sandboxes`.
    assert_eq!(
        operations.len(),
        31,
        "the full surface changed; update this baseline deliberately: {:?}",
        operations.keys().collect::<Vec<_>>()
    );

    let excluded: BTreeSet<&str> = operations
        .keys()
        .filter(|id| policy_for(id) != Some(OperationPolicy::Audited))
        .map(String::as_str)
        .collect();
    assert_eq!(
        excluded,
        BTreeSet::from(["health", "readiness", "metrics"]),
        "only probe and scrape traffic may be excluded from the audit trail"
    );
    for probe in ["health", "readiness"] {
        assert_eq!(
            policy_for(probe),
            Some(OperationPolicy::Excluded(ExclusionReason::Probe))
        );
    }
    assert_eq!(
        policy_for("metrics"),
        Some(OperationPolicy::Excluded(ExclusionReason::Scrape))
    );
}

/// The two conditionally mounted operations are audited like any other product
/// call, and must be present in the full-surface document.
#[tokio::test]
async fn the_conditionally_mounted_operations_are_audited() {
    let operations = live_operations().await;
    for (operation_id, expected_route) in [
        ("github_app_webhook", "POST /api/v1/github/app/webhook"),
        ("chat_turn", "POST /api/v1/chat"),
    ] {
        assert_eq!(
            operations.get(operation_id).map(String::as_str),
            Some(expected_route),
            "{operation_id} must be in the full-surface document"
        );
        assert_eq!(policy_for(operation_id), Some(OperationPolicy::Audited));
    }
}

/// A stale table entry is as much a bug as a missing one: it hides the fact that
/// an operation disappeared.
#[tokio::test]
async fn the_policy_table_names_no_operation_the_document_lacks() {
    let operations = live_operations().await;
    let stale: Vec<&str> = declared_operation_ids()
        .filter(|id| !operations.contains_key(*id))
        .collect();
    assert!(
        stale.is_empty(),
        "these policy entries no longer match any documented operation: {stale:?}"
    );
}

/// A deployment with the conditional features off must still build — the catalog
/// tolerates an absent operation, it only rejects an undeclared one.
#[tokio::test]
async fn a_minimal_deployment_still_builds_its_catalog() {
    let router = build_router(AppState {
        config: Config::default(),
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        operations: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: empty_self_router(),
        chat: None,
        audit: Default::default(),
    })
    .expect("a minimal deployment must still build");
    let response = router
        .oneshot(
            Request::get("/health")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
}

/// The safe-argument half of the guard: an audited operation must have decided
/// what its record may contain, not merely that it has one.
#[tokio::test]
async fn every_audited_operation_has_exactly_one_named_safe_argument_policy() {
    let operations = live_operations().await;
    let mut missing = Vec::new();
    for (operation_id, route) in &operations {
        if policy_for(operation_id) != Some(OperationPolicy::Audited) {
            continue;
        }
        match arguments_policy_for(operation_id) {
            Some(ArgumentsPolicy::Safe(_)) | Some(ArgumentsPolicy::None) => {}
            _ => missing.push(format!("{operation_id} ({route})")),
        }
    }
    assert!(
        missing.is_empty(),
        "these audited operations declare no safe-argument policy; give each an \
         ArgumentsPolicy::Safe(..) naming its DTO, or ArgumentsPolicy::None when it \
         genuinely takes no arguments: {missing:?}"
    );
}

/// One DTO, one operation — in both directions. A shared DTO would make the
/// recorded shape depend on which call site ran last, and a second policy on one
/// operation would make the allowlist ambiguous.
#[tokio::test]
async fn no_dto_is_mapped_onto_more_than_one_operation() {
    let mut owners: BTreeMap<&str, &str> = BTreeMap::new();
    for operation in OPERATION_POLICIES.iter().chain(RESERVED_ARGUMENT_POLICIES) {
        let Some(spec) = operation.arguments.spec() else {
            continue;
        };
        if let Some(previous) = owners.insert(spec.dto, operation.operation_id) {
            panic!(
                "the DTO {} is mapped onto both {previous} and {}",
                spec.dto, operation.operation_id
            );
        }
    }
    assert!(!owners.is_empty(), "the table declares no DTOs at all");
}

/// Every declared allowlist is non-empty, snake_case, and free of the property
/// names that are forbidden everywhere on this surface.
#[tokio::test]
async fn no_declared_allowlist_names_a_forbidden_property() {
    const FORBIDDEN: &[&str] = &[
        "token",
        "access_token",
        "refresh_token",
        "authorization",
        "cookie",
        "code",
        "state",
        "signature",
        "body",
        "title",
        "description",
        "message",
        "path",
        "query",
        "url",
        "content",
        "cursor",
    ];
    for operation in OPERATION_POLICIES.iter().chain(RESERVED_ARGUMENT_POLICIES) {
        let Some(spec) = operation.arguments.spec() else {
            continue;
        };
        assert!(
            !spec.fields.is_empty(),
            "{} declares an empty allowlist",
            operation.operation_id
        );
        for field in spec.fields {
            assert!(
                !FORBIDDEN.contains(field),
                "{} declares the forbidden property {field}",
                operation.operation_id
            );
            assert!(
                field
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{} declares the non-snake_case property {field}",
                operation.operation_id
            );
        }
    }
}

/// A route must not reach past the typed contract into the raw context API.
///
/// `AuditRequestContext::record_arguments` accepts an arbitrary property map, so
/// a handler calling it directly could record anything at all — which is exactly
/// the "generic/raw JSON capture helper" the argument contract exists to
/// prevent. The check is a source scan because the alternative (making the
/// method unreachable) would also block the `arguments` module that legitimately
/// owns it.
#[test]
fn no_route_uses_a_raw_or_generic_argument_capture_helper() {
    const FORBIDDEN: &[(&str, &str)] = &[
        (
            "record_arguments(",
            "call crate::audit::arguments::record/record_safe with a typed DTO instead",
        ),
        (
            "ArgumentsParseStatus",
            "the parse status comes from the DTO or the audited extractor, never a route",
        ),
        (
            "serde_json::to_value(&req",
            "a request DTO is never serialized into an audit record",
        ),
    ];
    let routes = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes");
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    let mut stack = vec![routes];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("routes directory is readable") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("route source is readable");
            scanned += 1;
            for (needle, why) in FORBIDDEN {
                if source.contains(needle) {
                    offenders.push(format!("{}: {needle} — {why}", path.display()));
                }
            }
        }
    }
    assert!(scanned > 10, "the route scan found almost nothing to read");
    assert!(offenders.is_empty(), "{offenders:#?}");
}
