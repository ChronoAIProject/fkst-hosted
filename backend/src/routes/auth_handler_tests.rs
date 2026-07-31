//! Handler tests for the frontend login callback and refresh.
//!
//! The focus is the identity-completion rule (epic `AUTH-02`): an exchanged or
//! refreshed OAuth credential only counts as a sign-in once `GET /user` names its
//! owner, and a `/user` failure is a stable authentication failure rather than an
//! invented actor.

use super::*;

use axum::http::header;
use wiremock::matchers::{method as http_method, path as http_path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audit::AuditIdentitySlot;
use crate::config::Config;
use crate::routes::logs::identity::clear_cache;
use crate::session_access::test_support::policy_with_admins;

/// A login-configured state whose OAuth host and GitHub API base are both `base`.
fn login_state(base: &str) -> AppState {
    let mut state = crate::session_access::test_support::app_state(
        base,
        policy_with_admins(""),
        Default::default(),
    );
    state.config = Config {
        github_api_base_url: base.to_string(),
        ..state.config
    };
    state.config.log.oauth_client_id = Some("Iv1.clientid".to_string());
    state.config.log.oauth_client_secret = Some(SecretString::from("oauth-secret".to_string()));
    state.config.log.oauth_base_url = base.to_string();
    state.config.log.public_base_url = Some("https://api.fkst.example".to_string());
    state.config.log.frontend_url = Some("https://app.fkst.example".to_string());
    state
}

/// Mock the token exchange/refresh endpoint with a fresh token set.
async fn mount_token_endpoint(server: &MockServer) {
    Mock::given(http_method("POST"))
        .and(http_path("/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "ghu_fresh_access",
            "refresh_token": "ghr_fresh_refresh",
            "expires_in": 28800,
            "token_type": "bearer"
        })))
        .mount(server)
        .await;
}

/// Mock `GET /user` with the given status/body.
async fn mount_user(server: &MockServer, ok: bool) {
    let response = if ok {
        ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({ "login": "octocat", "id": 583231 }))
    } else {
        ResponseTemplate::new(401)
    };
    Mock::given(http_method("GET"))
        .and(http_path("/user"))
        .respond_with(response)
        .mount(server)
        .await;
}

fn callback_query(state_param: String) -> LoginCallbackQuery {
    LoginCallbackQuery {
        code: Some("the-code".to_string()),
        state: Some(state_param),
        error: None,
        setup_action: None,
        installation_id: None,
    }
}

fn signed_login_state() -> String {
    oauth::sign_state(b"oauth-secret", &login_state_message())
}

#[tokio::test]
async fn a_successful_callback_verifies_the_identity_before_redirecting() {
    clear_cache();
    let server = MockServer::start().await;
    mount_token_endpoint(&server).await;
    mount_user(&server, true).await;

    let mut extensions = Extensions::new();
    let slot = AuditIdentitySlot::new();
    extensions.insert(slot.clone());

    let response = github_login_callback(
        State(login_state(&server.uri())),
        extensions,
        Query(callback_query(signed_login_state())),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = response
        .headers()
        .get(header::LOCATION)
        .expect("redirect")
        .to_str()
        .expect("ascii")
        .to_string();
    assert!(
        location.contains("#gh_token=ghu_fresh_access"),
        "{location}"
    );

    let identity = slot.get().expect("the callback recorded a verified actor");
    assert_eq!(identity.actor_id(), Some(583231));
    let rendered = format!("{identity:?}");
    assert!(!rendered.contains("ghu_fresh_access"), "{rendered}");
    assert!(!rendered.contains("ghr_fresh_refresh"), "{rendered}");
    assert!(!rendered.contains("the-code"), "{rendered}");
}

#[tokio::test]
async fn a_callback_whose_identity_check_fails_hands_the_spa_nothing() {
    clear_cache();
    let server = MockServer::start().await;
    mount_token_endpoint(&server).await;
    mount_user(&server, false).await;

    let response = github_login_callback(
        State(login_state(&server.uri())),
        Extensions::new(),
        Query(callback_query(signed_login_state())),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "an unverifiable identity is a stable authentication failure"
    );
    assert!(
        response.headers().get(header::LOCATION).is_none(),
        "no token may reach the frontend without a verified owner"
    );
}

#[tokio::test]
async fn a_tampered_state_never_reaches_the_exchange_or_the_identity_check() {
    // The mock server mounts NOTHING, so any outbound call would fail the test's
    // intent: a rejected state stays anonymous and exchanges nothing.
    let server = MockServer::start().await;
    let response = github_login_callback(
        State(login_state(&server.uri())),
        Extensions::new(),
        Query(callback_query("login:1700000000.deadbeef".to_string())),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(server
        .received_requests()
        .await
        .unwrap_or_default()
        .is_empty());
}

#[tokio::test]
async fn a_successful_refresh_is_attributed_to_a_verified_identity() {
    clear_cache();
    let server = MockServer::start().await;
    mount_token_endpoint(&server).await;
    mount_user(&server, true).await;

    let mut extensions = Extensions::new();
    let slot = AuditIdentitySlot::new();
    extensions.insert(slot.clone());

    let response = github_refresh_token(
        State(login_state(&server.uri())),
        extensions,
        crate::audit::arguments::AuditedJson(RefreshRequest {
            refresh_token: "ghr_old".to_string(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        slot.get().expect("recorded").actor_id(),
        Some(583231),
        "a refreshed session is attributed to the verified owner"
    );
}

#[tokio::test]
async fn a_refresh_whose_identity_check_fails_is_a_401() {
    clear_cache();
    let server = MockServer::start().await;
    mount_token_endpoint(&server).await;
    mount_user(&server, false).await;

    let response = github_refresh_token(
        State(login_state(&server.uri())),
        Extensions::new(),
        crate::audit::arguments::AuditedJson(RefreshRequest {
            refresh_token: "ghr_old".to_string(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
