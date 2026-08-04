//! Shared fixtures for the health-route suites, driving the REAL `build_router` with
//! wiremock standing in for GitHub `/user` and chrono-storage.
//!
//! The authorization behaviour is asserted to be equivalent to the log-download path
//! by reusing that suite's fixtures verbatim — the same registry, the same identity
//! mock, the same `AppState` builder — so the two cannot drift. Split out so each
//! suite file stays under the 500-line module cap.

use std::sync::Arc;

use secrecy::SecretString;
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::routes::logs::test_support::{
    github_user_ok, log_config, registry, state, AUTHOR_ID, SESSION_ID,
};
pub(super) use crate::session_backend::test_support::FakeSessionBackend;
use crate::session_backend::SessionBackend;
use crate::state::AppState;
use crate::storage::{ChronoStorageClient, ChronoStorageConfig};

pub(super) const REPORT_ID: &str =
    "chronoai-fkst-8f2c1d64-0a1b-4c2d-8e3f-0123456789ab-health-agent-status-report-20260730-141500";

pub(super) fn index_json(entries: Value) -> String {
    json!({ "schema": 1, "session_id": SESSION_ID, "reports": entries }).to_string()
}

pub(super) fn entry(id: &str, generated_at: &str, status: &str, interval: u64) -> Value {
    json!({
        "id": id,
        "key": format!("health/{SESSION_ID}/{id}.md"),
        "generated_at": generated_at,
        "expected_interval_secs": interval,
        "status": status,
        "headline": "a headline",
        "producer": "fkst-health@0.1.0",
    })
}

pub(super) fn report_body(body: &str) -> String {
    format!(
        "+++\n\
         fkst_health_report = 1\n\
         session_id = \"{SESSION_ID}\"\n\
         producer = \"fkst-health@0.1.0\"\n\
         generated_at = \"2026-07-30T14:15:00Z\"\n\
         window_start = \"2026-07-30T14:05:00Z\"\n\
         expected_interval_secs = 600\n\
         status = \"stalled\"\n\
         headline = \"a headline\"\n\
         confidence = \"high\"\n\
         \n\
         [[evidence]]\n\
         key = \"codex_runs_started\"\n\
         value = \"0\"\n\
         \n\
         [[work_items]]\n\
         number = 812\n\
         state = \"open\"\n\
         progress = \"none\"\n\
         +++\n{body}"
    )
}

/// A chrono-storage mock serving `objects` by key; any key not listed 404s. The
/// returned server is kept alive by the caller.
pub(super) async fn storage_with(
    objects: &[(String, Vec<u8>)],
) -> (Arc<ChronoStorageClient>, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "sa-token",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    for (key, bytes) in objects {
        Mock::given(method("GET"))
            .and(path("/api/buckets/logs/objects/download"))
            .and(query_param("key", key.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
            .mount(&server)
            .await;
    }
    // Catch-all: anything not explicitly served is absent.
    Mock::given(method("GET"))
        .and(path("/api/buckets/logs/objects/download"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: "logs".to_string(),
        nyxid_token_url: format!("{}/oauth/token", server.uri()),
        nyxid_client_id: "sa-client".to_string(),
        nyxid_client_secret: SecretString::from("sa-secret".to_string()),
    };
    (
        Arc::new(ChronoStorageClient::new(reqwest::Client::new(), config)),
        server,
    )
}

pub(super) fn index_key() -> String {
    format!("health/{SESSION_ID}/index.json")
}

/// State wired for the health endpoints: the log registry authorizes `author`, the
/// storage serves `objects`, and the runtime liveness is scripted.
pub(super) fn health_state(
    github_base: String,
    storage: Option<Arc<ChronoStorageClient>>,
    backend: Option<FakeSessionBackend>,
) -> AppState {
    let mut state = state(github_base, storage, log_config(&[], false), registry(&[]));
    state.session_backend = backend.map(|fake| Arc::new(fake) as Arc<dyn SessionBackend>);
    state
}

pub(super) async fn live_state(
    objects: &[(String, Vec<u8>)],
) -> (AppState, MockServer, MockServer) {
    let github = github_user_ok("author", AUTHOR_ID).await;
    let (storage, storage_server) = storage_with(objects).await;
    let state = health_state(
        github.uri(),
        Some(storage),
        Some(FakeSessionBackend::default().with_status_phase("Running")),
    );
    (state, github, storage_server)
}
