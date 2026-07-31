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
use crate::audit::AuditHandle;
use crate::log_config::LogConfig;
use crate::storage::ChronoStorageClient;

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
async fn oauth_callback_happy_path_streams_the_bundle_as_a_download() {
    // One mock server serves BOTH the OAuth token exchange and `/user`; storage is a
    // second. A valid signed state + code round-trips to the bundle streamed as an
    // attachment THROUGH the control plane (not a 302 to storage) — so a browser on a
    // different machine than the cluster saves it reliably without reaching S3 itself.
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
        broader_oauth_client_id: None,
        broader_oauth_client_secret: None,
        oauth_base_url: gh.uri(),
        frontend_url: None,
    };
    let st = state(gh.uri(), Some(storage), log, registry(&[]));
    let signed = super::oauth::sign_state(b"oauth-secret", SESSION_ID);

    let response = get(
        st,
        &format!("/api/v1/logs/oauth/callback?code=abc&state={signed}"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let cd = response
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .expect("content-disposition present")
        .to_str()
        .unwrap();
    assert!(cd.starts_with("attachment;"), "must be an attachment: {cd}");
    assert!(
        cd.contains(&format!("fkst-logs-{SESSION_ID}.tar.gz")),
        "attachment filename carries the session id: {cd}"
    );
    let ct = response
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/gzip");
    assert_eq!(body_bytes(response).await, BUNDLE_BYTES);
}

// ---- The recorded outcome of the flow ---------------------------------------

/// A GitHub mock serving BOTH the OAuth token exchange and `/user`.
async fn oauth_github(login: &str, id: i64) -> MockServer {
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
            "login": login,
            "id": id,
        })))
        .mount(&gh)
        .await;
    gh
}

/// Drive one full callback and return `(status, recorded result)`.
async fn callback_outcome(
    gh: &MockServer,
    storage: Option<Arc<ChronoStorageClient>>,
) -> (StatusCode, serde_json::Value) {
    let log = LogConfig {
        admins: vec![],
        public_base_url: Some("https://fkst.example".to_string()),
        oauth_client_id: Some("Iv1.clientid".to_string()),
        oauth_client_secret: Some(SecretString::from("oauth-secret".to_string())),
        broader_oauth_client_id: None,
        broader_oauth_client_secret: None,
        oauth_base_url: gh.uri(),
        frontend_url: None,
    };
    let mut st = state(gh.uri(), storage, log, registry(&[]));
    let (audit, sink) = AuditHandle::recording();
    st.audit = audit;
    let signed = super::oauth::sign_state(b"oauth-secret", SESSION_ID);
    let response = get(
        st,
        &format!("/api/v1/logs/oauth/callback?code=abc&state={signed}"),
        None,
    )
    .await;
    let status = response.status();
    let event = sink
        .events()
        .into_iter()
        .find(|event| event.operation_id == "session_logs_oauth_callback")
        .expect("the callback recorded a terminal event");
    let result = event
        .arguments
        .get("result")
        .cloned()
        .expect("the callback record carries its result");
    (status, result)
}

/// A caller the session does not authorize is `denied`, not `success`: the
/// record's `result` is a dashboard facet, and "success" on a refused log
/// download would read as though the bundle had been handed over.
#[tokio::test]
async fn a_refused_callback_records_denied() {
    let gh = oauth_github("mallory", AUTHOR_ID + 1).await;
    let (storage, _s) = storage_server(true).await;
    let (status, result) = callback_outcome(&gh, Some(storage)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(result, serde_json::json!("denied"));
}

/// An authorized caller whose bundle cannot be read is `upstream_error` — the
/// flow reached its dependency and the dependency is what failed.
#[tokio::test]
async fn a_callback_whose_bundle_is_missing_records_an_upstream_error() {
    let gh = oauth_github("alice", AUTHOR_ID).await;
    let (storage, _s) = storage_server(false).await;
    let (status, result) = callback_outcome(&gh, Some(storage)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(result, serde_json::json!("upstream_error"));
}

/// …and the served download still says `success`, so the two cases above are a
/// distinction rather than a blanket rename.
#[tokio::test]
async fn a_served_callback_records_success() {
    let gh = oauth_github("alice", AUTHOR_ID).await;
    let (storage, _s) = storage_server(true).await;
    let (status, result) = callback_outcome(&gh, Some(storage)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result, serde_json::json!("success"));
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
