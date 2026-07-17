//! Shared canvas handler-test fixtures: a pre-verified viewer identity, the
//! bearer header, an [`AppState`] pointed at a wiremock server, a real
//! [`GithubAppTokens`] with a throwaway RSA key, and the App-token mint mocks.

use axum::http::HeaderMap;
use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::config::Config;
use crate::github_app::{GithubAppConfig, GithubAppTokens};
use crate::github_identity::GithubUser;
use crate::state::AppState;

pub(crate) fn auth_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer user-token".parse().unwrap(),
    );
    headers
}

pub(crate) fn viewer_user() -> GithubUser {
    GithubUser {
        login: "shining".to_string(),
        id: 9,
    }
}

/// A real [`GithubAppTokens`] pointed at the mock server (fresh throwaway RSA
/// key — the mock never verifies the JWT).
pub(crate) fn test_app(api_base: &str) -> GithubAppTokens {
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;
    let mut rng = rand::rngs::OsRng;
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa key");
    let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");
    GithubAppTokens::new(&GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(pem.to_string()),
        app_slug: Some("fkst-test".to_string()),
        webhook_secret: None,
        api_base: api_base.to_string(),
    })
    .expect("app tokens")
}

pub(crate) fn test_state(server_uri: &str, github_app: Option<GithubAppTokens>) -> AppState {
    let config = Config {
        github_api_base_url: server_uri.to_string(),
        ..Config::default()
    };
    AppState {
        config,
        github_app,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        log_registry: Default::default(),
    }
}

/// Mount the App-token mint pair (`…/installation` + `…/access_tokens`) for a repo.
pub(crate) async fn mount_app_token(server: &MockServer, owner: &str, name: &str, inst_id: i64) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/{owner}/{name}/installation")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": inst_id })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/app/installations/{inst_id}/access_tokens")))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "token": "ghs_test_installation_token",
            "expires_at": "2099-01-01T00:00:00Z"
        })))
        .mount(server)
        .await;
}
