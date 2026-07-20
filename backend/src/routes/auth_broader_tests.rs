//! Handler + helper tests for the broader-visibility OAuth connect flow
//! (`crate::routes::auth_broader`). The wiremock server plays GitHub's classic
//! `login/oauth/access_token` exchange; the authorize step needs no network.

use super::*;

use axum::http::header;
use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::config::Config;

/// An [`AppState`] whose log config has the broader pair + URLs set, with the OAuth
/// host pointed at `oauth_base` (a wiremock server for the exchange tests).
fn broader_state(oauth_base: &str) -> AppState {
    let mut config = Config::default();
    config.log.broader_oauth_client_id = Some("classic-id".to_string());
    config.log.broader_oauth_client_secret = Some(SecretString::from("classic-secret".to_string()));
    config.log.oauth_base_url = oauth_base.to_string();
    config.log.public_base_url = Some("https://api.fkst.example".to_string());
    config.log.frontend_url = Some("https://app.fkst.example".to_string());
    AppState {
        config,
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        log_registry: Default::default(),
        log_bundle_cache: Default::default(),
    }
}

/// An [`AppState`] with the broader flow UNconfigured (the default config).
fn unconfigured_state() -> AppState {
    AppState {
        config: Config::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        log_registry: Default::default(),
        log_bundle_cache: Default::default(),
    }
}

fn location(resp: &Response) -> String {
    resp.headers()
        .get(header::LOCATION)
        .expect("a redirect must carry a Location header")
        .to_str()
        .expect("location is valid ascii")
        .to_string()
}

// ---- pure helpers -----------------------------------------------------------

#[test]
fn broader_callback_redirect_uri_appends_the_path_and_trims_slash() {
    assert_eq!(
        broader_callback_redirect_uri("https://api.fkst.example/"),
        "https://api.fkst.example/api/v1/auth/github/broader/callback"
    );
    assert_eq!(
        broader_callback_redirect_uri("https://api.fkst.example"),
        "https://api.fkst.example/api/v1/auth/github/broader/callback"
    );
}

#[test]
fn broader_success_url_carries_the_token_in_the_fragment() {
    let url = broader_success_url(
        "https://app.fkst.example/",
        &SecretString::from("gho_classic"),
    );
    assert_eq!(url, "https://app.fkst.example/#broader_token=gho_classic");
    // The token is ONLY ever in the fragment, never a query string.
    assert!(
        !url.contains("?"),
        "token must not ride a query string: {url}"
    );
}

// ---- connect (authorize redirect) -------------------------------------------

#[tokio::test]
async fn connect_redirects_to_github_with_repo_read_org_scope() {
    let state = broader_state("https://github.com");
    let resp = github_broader(State(state)).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let loc = location(&resp);
    assert!(loc.starts_with("https://github.com/login/oauth/authorize?"));
    assert!(loc.contains("client_id=classic-id"), "{loc}");
    // scope=repo read:org, percent-encoded (space → + or %20, ':' → %3A).
    assert!(
        loc.contains("scope=repo+read%3Aorg") || loc.contains("scope=repo%20read%3Aorg"),
        "authorize URL must request repo + read:org: {loc}"
    );
    // The redirect_uri is the broader callback, percent-encoded.
    assert!(
        loc.contains("broader%2Fcallback"),
        "redirect_uri must be the broader callback: {loc}"
    );
}

#[tokio::test]
async fn connect_is_503_when_unconfigured() {
    let resp = github_broader(State(unconfigured_state())).await;
    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unconfigured broader flow must be inert (503)"
    );
}

// ---- callback (state verify + code exchange) --------------------------------

fn signed_broader_state() -> String {
    oauth::sign_state(b"classic-secret", &signed_state_message(BROADER_STATE_KIND))
}

#[tokio::test]
async fn callback_exchanges_code_and_redirects_with_broader_token_fragment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "gho_classic_token",
            "token_type": "bearer",
            "scope": "repo,read:org"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = broader_state(&server.uri());
    let query = BroaderCallbackQuery {
        code: Some("the-code".to_string()),
        state: Some(signed_broader_state()),
        error: None,
    };
    let resp = github_broader_callback(State(state), Query(query)).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let loc = location(&resp);
    assert_eq!(
        loc, "https://app.fkst.example#broader_token=gho_classic_token",
        "the broader token must ride the frontend fragment only"
    );
    // Belt-and-braces: the token is never in a query string.
    assert!(!loc.contains("?broader_token"), "{loc}");
}

#[tokio::test]
async fn callback_rejects_a_tampered_state() {
    // A syntactically-plausible state with a bogus signature must fail verification.
    let state = broader_state("https://github.com");
    let query = BroaderCallbackQuery {
        code: Some("the-code".to_string()),
        state: Some("broader:1700000000.deadbeef".to_string()),
        error: None,
    };
    let resp = github_broader_callback(State(state), Query(query)).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_rejects_missing_code_or_state() {
    let state = broader_state("https://github.com");
    let query = BroaderCallbackQuery {
        code: None,
        state: Some(signed_broader_state()),
        error: None,
    };
    let resp = github_broader_callback(State(state), Query(query)).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_bounces_to_the_frontend_on_user_denial() {
    let state = broader_state("https://github.com");
    let query = BroaderCallbackQuery {
        code: None,
        state: None,
        error: Some("access_denied".to_string()),
    };
    let resp = github_broader_callback(State(state), Query(query)).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(
        location(&resp),
        "https://app.fkst.example#broader_error=access_denied"
    );
}

#[tokio::test]
async fn callback_maps_a_rejected_exchange_to_401_html() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;

    let state = broader_state(&server.uri());
    let query = BroaderCallbackQuery {
        code: Some("bad-code".to_string()),
        state: Some(signed_broader_state()),
        error: None,
    };
    let resp = github_broader_callback(State(state), Query(query)).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn callback_is_503_when_unconfigured() {
    let query = BroaderCallbackQuery {
        code: Some("c".to_string()),
        state: Some("s".to_string()),
        error: None,
    };
    let resp = github_broader_callback(State(unconfigured_state()), Query(query)).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
