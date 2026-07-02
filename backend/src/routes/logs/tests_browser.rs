//! Browser-mode integration tests for the log-download endpoint: the no-Bearer OAuth
//! entry redirect, the callback's signed-state (CSRF) guard + a full callback happy
//! path, and a proof that the caller's token never reaches the logs. Fixtures live in
//! [`super::test_support`]; the API-mode suite lives in [`super::tests`].

use std::sync::{Arc, Mutex};

use axum::http::{header, StatusCode};
use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::test_support::*;
use crate::log_config::LogConfig;

#[tokio::test]
async fn no_bearer_with_oauth_configured_redirects_to_github() {
    let st = state(
        "https://api.github.test".to_string(),
        None,
        log_config(&[], true),
        registry(&[]),
    );
    let response = get(st, &format!("/api/v1/logs/{SESSION_ID}"), None).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let loc = location(&response);
    assert!(
        loc.starts_with("https://github.test/login/oauth/authorize?"),
        "unexpected location: {loc}"
    );
    assert!(loc.contains("client_id=Iv1.clientid"));
    // The signed state carries the session id (its prefix, before the HMAC).
    assert!(
        loc.contains(&format!("state={SESSION_ID}")),
        "state missing session id: {loc}"
    );
}

#[tokio::test]
async fn no_bearer_without_oauth_configured_is_503() {
    let st = state(
        "https://api.github.test".to_string(),
        None,
        log_config(&[], false),
        registry(&[]),
    );
    let response = get(st, &format!("/api/v1/logs/{SESSION_ID}"), None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn oauth_callback_rejects_a_tampered_state() {
    // A callback whose state fails the HMAC check is a 400 (CSRF/tamper guard),
    // never proceeding to a code exchange.
    let st = state(
        "https://api.github.test".to_string(),
        None,
        log_config(&[], true),
        registry(&[]),
    );
    let response = get(
        st,
        "/api/v1/logs/oauth/callback?code=abc&state=sess-abc-123.deadbeef",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // Browser mode renders HTML, not the JSON envelope.
    let ct = response
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        ct.starts_with("text/html"),
        "browser error must be HTML: {ct}"
    );
}

#[tokio::test]
async fn oauth_callback_happy_path_redirects_to_the_presigned_url() {
    // One mock server serves BOTH the OAuth token exchange and `/user`; storage is a
    // second. A valid signed state + code round-trips to a 302 at the presigned URL.
    let gh = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "user-token",
            "token_type": "bearer",
        })))
        .mount(&gh)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "login": "alice",
            "id": AUTHOR_ID,
        })))
        .mount(&gh)
        .await;
    let (storage, _s) = storage_server(true).await;

    // OAuth base + API base both point at `gh`; sign a valid state for SESSION_ID.
    let log = LogConfig {
        admins: vec![],
        public_base_url: Some("https://fkst.example".to_string()),
        oauth_client_id: Some("Iv1.clientid".to_string()),
        oauth_client_secret: Some(SecretString::from("oauth-secret".to_string())),
        oauth_base_url: gh.uri(),
    };
    let st = state(gh.uri(), Some(storage), log, registry(&[]));
    let signed = super::oauth::sign_state(b"oauth-secret", SESSION_ID);

    let response = get(
        st,
        &format!("/api/v1/logs/oauth/callback?code=abc&state={signed}"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(location(&response), PRESIGNED_URL);
}

// ---- Secret hygiene: the caller's token never reaches the logs --------------

/// A `MakeWriter` capturing every emitted log line into a shared buffer.
#[derive(Clone)]
struct BufWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for BufWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufWriter {
    type Writer = BufWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn the_caller_token_is_never_written_to_the_logs() {
    // A distinctive token that would be trivially greppable if it ever leaked.
    const SECRET_TOKEN: &str = "gho_SUPER_SECRET_LEAK_CANARY_9f2a";

    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let (storage, _s) = storage_server(true).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(BufWriter(buf.clone()))
        .with_max_level(tracing::Level::TRACE)
        .finish();

    // `#[tokio::test]` is a current-thread runtime, so this thread-local subscriber
    // captures every event the request emits (including the "authorized" info log).
    {
        let _guard = tracing::subscriber::set_default(subscriber);
        let response = get(
            st,
            &format!("/api/v1/logs/{SESSION_ID}"),
            Some(SECRET_TOKEN),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "the request must succeed so logging actually happens"
        );
    }

    let logs = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8 logs");
    assert!(
        !logs.contains(SECRET_TOKEN),
        "the caller's token leaked into the logs: {logs}"
    );
    assert!(
        !logs.to_lowercase().contains("authorization"),
        "the Authorization header must never be logged: {logs}"
    );
}
