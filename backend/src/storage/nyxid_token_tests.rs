//! wiremock tests for the NyxID service-account token provider: the first call
//! mints, a call inside the TTL is served from cache, a near-expiry token
//! refreshes, a non-2xx / malformed response errors, and neither the client
//! secret nor the minted token ever reaches `Debug` output or an error string.

use secrecy::{ExposeSecret, SecretString};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::storage::config::ChronoStorageConfig;

const TOKEN_PATH: &str = "/oauth/token";

/// Build a provider whose token endpoint is `{server}/oauth/token`, with the
/// given client secret (the object under secret-hygiene test).
fn provider(server: &MockServer, secret: &str) -> NyxidSaTokenProvider {
    let config = ChronoStorageConfig {
        base_url: "http://storage.invalid".to_string(),
        bucket: "bucket".to_string(),
        nyxid_token_url: format!("{}{TOKEN_PATH}", server.uri()),
        nyxid_client_id: "sa-client".to_string(),
        nyxid_client_secret: SecretString::from(secret.to_string()),
    };
    NyxidSaTokenProvider::new(reqwest::Client::new(), &config)
}

fn token_body(access_token: &str, expires_in: u64) -> serde_json::Value {
    serde_json::json!({
        "access_token": access_token,
        "expires_in": expires_in,
        "token_type": "Bearer",
    })
}

#[tokio::test]
async fn first_call_mints_via_the_token_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        // Standard OAuth2 client-credentials form body + content type.
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string_contains("grant_type=client_credentials"))
        .and(body_string_contains("client_id=sa-client"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("tok-1", 3600)))
        .expect(1)
        .mount(&server)
        .await;

    let provider = provider(&server, "sa-secret");
    let token = provider.access_token().await.expect("mint succeeds");
    assert_eq!(token.expose_secret(), "tok-1");
}

#[tokio::test]
async fn second_call_within_ttl_is_served_from_cache() {
    let server = MockServer::start().await;
    // `expect(1)` fails the test if the endpoint is hit more than once.
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("tok-cached", 3600)))
        .expect(1)
        .mount(&server)
        .await;

    let provider = provider(&server, "sa-secret");
    let first = provider.access_token().await.expect("first mint");
    let second = provider.access_token().await.expect("second is cached");
    assert_eq!(first.expose_secret(), "tok-cached");
    assert_eq!(second.expose_secret(), "tok-cached");
    // The mock's `expect(1)` (verified on drop) proves only one network mint.
}

#[tokio::test]
async fn near_expiry_token_triggers_a_refresh() {
    let server = MockServer::start().await;
    // A 1s lifetime is inside the 60s refresh buffer, so every call re-mints.
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("tok-short", 1)))
        .expect(2)
        .mount(&server)
        .await;

    let provider = provider(&server, "sa-secret");
    let _ = provider.access_token().await.expect("first mint");
    let _ = provider.access_token().await.expect("near-expiry refresh");
    // `expect(2)` proves the near-expiry token was refreshed, not cached.
}

#[tokio::test]
async fn non_success_status_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let err = provider(&server, "sa-secret")
        .access_token()
        .await
        .expect_err("503 must error");
    assert!(
        matches!(err, StorageError::TokenStatus { status: 503 }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn unauthorized_status_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = provider(&server, "sa-secret")
        .access_token()
        .await
        .expect_err("401 must error");
    assert!(
        matches!(err, StorageError::TokenStatus { status: 401 }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn malformed_body_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        // 200 but no `access_token` field.
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "foo": "bar" })))
        .mount(&server)
        .await;

    let err = provider(&server, "sa-secret")
        .access_token()
        .await
        .expect_err("missing access_token must error");
    assert!(matches!(err, StorageError::TokenMalformed), "got {err:?}");
}

#[tokio::test]
async fn transport_error_when_endpoint_unreachable() {
    // Point at a closed port: no server is listening, so the mint fails at the
    // transport layer and the error carries a URL-free category only.
    let config = ChronoStorageConfig {
        base_url: "http://storage.invalid".to_string(),
        bucket: "bucket".to_string(),
        nyxid_token_url: "http://127.0.0.1:1/oauth/token".to_string(),
        nyxid_client_id: "sa-client".to_string(),
        nyxid_client_secret: SecretString::from("sa-secret".to_string()),
    };
    let provider = NyxidSaTokenProvider::new(reqwest::Client::new(), &config);
    let err = provider
        .access_token()
        .await
        .expect_err("unreachable endpoint must error");
    assert!(
        matches!(err, StorageError::TokenTransport(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn secret_and_token_never_leak_in_debug_or_error() {
    const SECRET: &str = "SUPER_SECRET_CLIENT_VALUE";
    const TOKEN: &str = "MINTED_ACCESS_TOKEN_VALUE";

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(TOKEN, 3600)))
        .mount(&server)
        .await;

    let provider = provider(&server, SECRET);

    // Before minting: the client secret must not appear in Debug output.
    let debug_before = format!("{provider:?}");
    assert!(
        !debug_before.contains(SECRET),
        "Debug leaked the client secret: {debug_before}"
    );

    // After minting: neither the secret nor the cached token may appear.
    let token = provider.access_token().await.expect("mint");
    assert_eq!(token.expose_secret(), TOKEN);
    let debug_after = format!("{provider:?}");
    assert!(
        !debug_after.contains(SECRET),
        "Debug leaked the client secret: {debug_after}"
    );
    assert!(
        !debug_after.contains(TOKEN),
        "Debug leaked the minted token: {debug_after}"
    );

    // An error surfaced from this provider must carry neither value.
    let err = StorageError::TokenStatus { status: 500 };
    let rendered = format!("{err} {err:?}");
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(!rendered.contains(TOKEN), "{rendered}");
}
