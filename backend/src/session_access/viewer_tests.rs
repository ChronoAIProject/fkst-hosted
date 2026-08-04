//! Unit tests for the authenticated viewer, the global-admin gate, and scope
//! resolution.

use super::*;
use crate::session_access::metrics::ScopeOutcome;
use crate::session_access::registry::SessionAccessRegistry;
use crate::session_access::test_support::{app_state, denylist, policy_with_admins};
use axum::http::Request;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn user(id: i64, login: &str) -> GithubUser {
    GithubUser {
        login: login.to_string(),
        id,
    }
}

fn viewer(id: i64, login: &str, access: &AccessPolicy) -> AuthenticatedViewer {
    AuthenticatedViewer::new(user(id, login), access)
}

// ---- role derivation --------------------------------------------------------

#[test]
fn global_admin_matches_by_numeric_id_and_case_insensitive_login() {
    let by_login = policy_with_admins("@Deploy-Admin");
    assert!(viewer(500, "deploy-admin", &by_login).is_global_admin());
    assert!(!viewer(500, "someone-else", &by_login).is_global_admin());

    let by_id = policy_with_admins("4242");
    assert!(
        viewer(4242, "renamed-since", &by_id).is_global_admin(),
        "numeric ids stay rename-safe"
    );
    assert!(
        !viewer(999, "4242", &by_id).is_global_admin(),
        "a login that looks numeric must not match an id entry"
    );
}

#[test]
fn a_regular_viewer_has_no_admin_role() {
    let access = policy_with_admins("grace");
    let regular = viewer(101, "alice", &access);
    assert!(!regular.is_global_admin());
    assert_eq!(regular.id(), 101);
    assert_eq!(regular.login(), "alice");
}

#[test]
fn a_global_admin_listed_as_blocked_keeps_the_role() {
    // Preserves the existing AccessPolicy precedence: the two operator-owned
    // lists resolve in the admin's favour, and this milestone does not change it.
    let access = denylist("grace", "grace");
    assert!(viewer(107, "grace", &access).is_global_admin());
}

// ---- scope resolution -------------------------------------------------------

#[test]
fn a_regular_viewer_defaults_to_personal_scope_and_may_ask_for_it() {
    let access = policy_with_admins("grace");
    let regular = viewer(101, "alice", &access);
    for request in [
        ScopeRequest::new(None),
        ScopeRequest::new(Some(RequestedScope::Personal)),
    ] {
        let scope = regular.resolve_scope(request).expect("personal scope");
        match &scope {
            ViewerScope::Mine(personal) => {
                assert_eq!(personal.viewer_id(), 101);
                assert_eq!(personal.viewer_login(), "alice");
            }
            other => panic!("expected personal scope, got {other:?}"),
        }
        assert!(!scope.is_global());
    }
}

#[test]
fn a_regular_viewer_is_refused_the_global_scope() {
    let access = policy_with_admins("grace");
    let regular = viewer(101, "alice", &access);
    assert_eq!(
        regular.resolve_scope(ScopeRequest::new(Some(RequestedScope::Global))),
        Err(ScopeDenialReason::GlobalScope)
    );
}

#[test]
fn a_regular_viewer_is_refused_a_cross_actor_filter_in_any_scope() {
    let access = policy_with_admins("grace");
    let regular = viewer(101, "alice", &access);
    for requested in [None, Some(RequestedScope::Personal)] {
        assert_eq!(
            regular.resolve_scope(ScopeRequest::new(requested).with_cross_actor_filter()),
            Err(ScopeDenialReason::CrossActorFilter),
            "a cross-actor filter is a global-only capability"
        );
    }
}

#[test]
fn a_global_admin_defaults_to_global_scope_and_may_select_personal() {
    let access = policy_with_admins("grace");
    let admin = viewer(107, "grace", &access);
    let default = admin
        .resolve_scope(ScopeRequest::new(None))
        .expect("global");
    assert!(default.is_global());
    assert_eq!(default.identity_id(), 107);

    let personal = admin
        .resolve_scope(ScopeRequest::new(Some(RequestedScope::Personal)))
        .expect("an admin may exercise the personal semantics");
    assert!(!personal.is_global());

    admin
        .resolve_scope(ScopeRequest::new(None).with_cross_actor_filter())
        .expect("cross-actor filters are an administrator capability");
}

#[test]
fn the_route_wrapper_records_bounded_metrics_and_maps_the_stable_403() {
    let access = policy_with_admins("grace");
    let metrics = ScopeMetrics::new();
    let regular = viewer(101, "alice", &access);

    resolve_operations_scope(&regular, ScopeRequest::new(None), &metrics).expect("personal");
    let denied = resolve_operations_scope(
        &regular,
        ScopeRequest::new(Some(RequestedScope::Global)),
        &metrics,
    )
    .expect_err("a regular caller may not select the global scope");
    assert!(matches!(denied, AppError::ScopeForbidden(_)), "{denied:?}");
    assert!(
        !format!("{denied}").contains("grace"),
        "a denial must not reveal who is configured as an administrator"
    );

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.count(ScopeOutcome::MineDefault), 1);
    assert_eq!(snapshot.count(ScopeOutcome::AllForbidden), 1);
}

// ---- extractors -------------------------------------------------------------

fn state_with(base_url: &str, access: AccessPolicy) -> AppState {
    app_state(base_url, access, SessionAccessRegistry::new(false))
}

fn parts_with_auth(header: Option<&str>) -> Parts {
    let mut builder = Request::builder().uri("/api/v1/operations/activity");
    if let Some(value) = header {
        builder = builder.header(axum::http::header::AUTHORIZATION, value);
    }
    builder
        .body(axum::body::Body::empty())
        .expect("request builds")
        .into_parts()
        .0
}

async fn mock_user(login: &str, id: i64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "login": login, "id": id })),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn the_viewer_extractor_admits_a_regular_user_and_records_audit_identity() {
    let server = mock_user("alice", 101).await;
    let state = state_with(&server.uri(), policy_with_admins("grace"));
    let mut parts = parts_with_auth(Some("Bearer gho_viewer_token"));
    let slot = crate::audit::AuditIdentitySlot::new();
    parts.extensions.insert(slot.clone());

    let extracted = AuthenticatedViewer::from_request_parts(&mut parts, &state)
        .await
        .expect("a regular user is not rejected merely for reaching operations");
    assert_eq!(extracted.id(), 101);
    assert!(!extracted.is_global_admin());

    let identity = slot
        .get()
        .expect("the extractor recorded a verified identity");
    assert_eq!(identity.actor_id(), Some(101));
    let rendered = format!("{identity:?}");
    assert!(
        !rendered.contains("gho_viewer_token"),
        "the source token must never enter the request extensions: {rendered}"
    );
}

#[tokio::test]
async fn the_viewer_extractor_keeps_the_canonical_401_and_403() {
    let state = state_with("http://127.0.0.1:1", policy_with_admins(""));
    let mut parts = parts_with_auth(None);
    let err = AuthenticatedViewer::from_request_parts(&mut parts, &state)
        .await
        .expect_err("missing identity");
    assert!(matches!(err, AppError::Unauthorized(_)), "{err:?}");

    let server = mock_user("mallory", 999).await;
    let access =
        AccessPolicy::from_vars(&[("FKST_ACCESS_ALLOWED_USERS".to_string(), "alice".to_string())])
            .expect("allowlist parses");
    let state = state_with(&server.uri(), access);
    let mut parts = parts_with_auth(Some("Bearer gho_unlisted"));
    let err = AuthenticatedViewer::from_request_parts(&mut parts, &state)
        .await
        .expect_err("the deployment access model still rejects before scope resolution");
    assert!(matches!(err, AppError::Forbidden(_)), "{err:?}");
}

#[tokio::test]
async fn the_global_admin_extractor_is_a_strict_gate() {
    let server = mock_user("grace", 107).await;
    let state = state_with(&server.uri(), policy_with_admins("grace"));
    let mut parts = parts_with_auth(Some("Bearer gho_admin_token"));
    let admin = GlobalAdmin::from_request_parts(&mut parts, &state)
        .await
        .expect("a configured administrator passes");
    assert_eq!(admin.user().id, 107);

    let server = mock_user("alice", 101).await;
    let state = state_with(&server.uri(), policy_with_admins("grace"));
    let mut parts = parts_with_auth(Some("Bearer gho_regular_token"));
    let err = GlobalAdmin::from_request_parts(&mut parts, &state)
        .await
        .expect_err("a regular user is refused an admin-only route");
    assert!(matches!(err, AppError::Forbidden(_)), "{err:?}");
    assert!(
        !format!("{err}").contains("grace"),
        "the refusal must not name the configured administrators"
    );
}

#[tokio::test]
async fn a_renamed_admin_keeps_the_role_through_the_extractor() {
    let server = mock_user("grace-renamed", 107).await;
    let state = state_with(&server.uri(), policy_with_admins("107"));
    let mut parts = parts_with_auth(Some("Bearer gho_renamed_admin"));
    let extracted = AuthenticatedViewer::from_request_parts(&mut parts, &state)
        .await
        .expect("verified");
    assert!(
        extracted.is_global_admin(),
        "the numeric entry survives the rename"
    );
}
