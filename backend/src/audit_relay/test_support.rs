//! Shared, credential-free fixtures for the relay's unit tests.
//!
//! Every token here is an obvious canary string, so a test that accidentally
//! logs, stores, or returns one is trivially detectable — several tests below
//! assert exactly that.

use std::sync::Arc;

use k8s_openapi::chrono::{DateTime, Duration, Utc};
use secrecy::SecretString;
use serde_json::Map;
use tempfile::TempDir;

use super::config::RelayConfig;
use super::db::{Database, DatabaseSettings};
use super::http::{build_router, RelayState};
use super::metrics::RelayMetrics;
use super::protocol::{
    ActorV1, CorrelationV1, LifecycleEventV1, PrincipalV1, RequestCompletionV1, RequestStartV1,
    PROTOCOL_SCHEMA_VERSION,
};

/// The write credential every relay test presents. A canary: it must never
/// appear in the database file, a log, a metric, or a response.
pub const WRITE_TOKEN: &str = "canary-relay-write-token-4f2a";
/// The read credential. Same canary rule.
pub const READ_TOKEN: &str = "canary-relay-read-token-9b71";

/// A fixed instant so every derived timestamp in a test is reproducible.
pub fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-31T12:00:00.000Z")
        .expect("fixed instant parses")
        .with_timezone(&Utc)
}

/// A relay configuration pointing at `db_path`, with capture/verification off.
pub fn config(db_path: std::path::PathBuf) -> RelayConfig {
    RelayConfig {
        bind_addr: "127.0.0.1:0".parse().expect("loopback address parses"),
        db_path,
        write_token: SecretString::from(WRITE_TOKEN.to_string()),
        read_token: SecretString::from(READ_TOKEN.to_string()),
        max_body_bytes: 65_536,
        max_records: 1_000_000,
        verification_delay_secs: 30,
        verification_max_age_secs: 300,
        verified_retention_days: 7,
        audit_retention_days: 90,
        incomplete_grace_secs: 60,
        busy_timeout_ms: 5_000,
        writer_queue_capacity: 64,
        max_read_concurrency: 2,
        max_read_rows: 500,
        max_range_days: 400,
        capture_batch_size: 10,
        max_capture_attempts: 3,
        retry_initial_secs: 5,
        retry_max_secs: 60,
        worker_interval_secs: 1,
        verification_batch_size: 50,
        posthog_host: None,
        posthog_project_token: SecretString::from(String::new()),
        posthog_project_id: None,
        posthog_query_api_key: SecretString::from(String::new()),
        environment: "test".to_string(),
    }
}

/// The database settings a test relay opens with.
pub fn settings() -> DatabaseSettings {
    DatabaseSettings {
        busy_timeout_ms: 5_000,
        writer_queue_capacity: 64,
        read_concurrency: 2,
        max_records: 1_000_000,
    }
}

/// A temporary directory plus an opened database inside it.
pub fn open_database() -> (TempDir, Database) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("audit.sqlite3");
    let database = Database::open(&path, settings()).expect("database opens");
    (dir, database)
}

/// A relay state + router over a fresh temporary database.
pub fn relay() -> (TempDir, RelayState, axum::Router) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("audit.sqlite3");
    let database = Database::open(&path, settings()).expect("database opens");
    let state = RelayState::new(database, Arc::new(config(path)), RelayMetrics::new());
    let router = build_router(state.clone());
    (dir, state, router)
}

/// A request start for `event_id`, started at [`now`] with a 60s deadline.
pub fn start(event_id: &str) -> RequestStartV1 {
    RequestStartV1 {
        schema_version: PROTOCOL_SCHEMA_VERSION,
        event_id: event_id.to_string(),
        request_id: format!("req-{event_id}"),
        started_at: super::protocol::format_instant(now()),
        method: "GET".to_string(),
        route_template: "/api/v1/overview".to_string(),
        operation_id: "canvas_overview".to_string(),
        service_version: "0.2.3".to_string(),
        deployment_environment: "test".to_string(),
        completion_deadline_at: super::protocol::format_instant(now() + Duration::seconds(60)),
    }
}

/// The terminal completion matching [`start`], attributed to `actor_id`.
pub fn completion(event_id: &str, actor_id: Option<i64>) -> RequestCompletionV1 {
    let started = start(event_id);
    RequestCompletionV1 {
        schema_version: PROTOCOL_SCHEMA_VERSION,
        event_id: started.event_id.clone(),
        request_id: started.request_id.clone(),
        started_at: started.started_at.clone(),
        completed_at: super::protocol::format_instant(now() + Duration::milliseconds(120)),
        method: started.method.clone(),
        route_template: started.route_template.clone(),
        operation_id: started.operation_id.clone(),
        arguments: Map::new(),
        arguments_parse_status: "parsed".to_string(),
        actor_id,
        actor: match actor_id {
            Some(id) => ActorV1 {
                kind: "github_user".to_string(),
                id: Some(id),
                login: Some(format!("user-{id}")),
                authentication: "bearer".to_string(),
            },
            None => ActorV1 {
                kind: "anonymous".to_string(),
                id: None,
                login: None,
                authentication: "none".to_string(),
            },
        },
        principal: PrincipalV1 {
            kind: "github_user_token".to_string(),
            id: Some("github_user_token".to_string()),
        },
        status_code: Some(200),
        outcome: "success".to_string(),
        error_code: None,
        duration_ms: 120,
        session_id: None,
        correlation: CorrelationV1::default(),
        service_version: "0.2.3".to_string(),
        deployment_environment: "test".to_string(),
    }
}

/// A completion correlated to `session_id`.
pub fn completion_in_session(
    event_id: &str,
    actor_id: Option<i64>,
    session_id: &str,
) -> RequestCompletionV1 {
    let mut completion = completion(event_id, actor_id);
    completion.session_id = Some(session_id.to_string());
    completion.correlation.session_id = Some(session_id.to_string());
    completion
}

/// A sandbox lifecycle event for `session_id`.
pub fn lifecycle(event_id: &str, session_id: &str) -> LifecycleEventV1 {
    LifecycleEventV1 {
        schema_version: PROTOCOL_SCHEMA_VERSION,
        event_id: event_id.to_string(),
        occurred_at: super::protocol::format_instant(now()),
        lifecycle_action: "created".to_string(),
        actor: ActorV1 {
            kind: "system".to_string(),
            id: None,
            login: None,
            authentication: "internal".to_string(),
        },
        principal: PrincipalV1 {
            kind: "reconciler".to_string(),
            id: Some("reconciler".to_string()),
        },
        session_id: session_id.to_string(),
        backend: "opensandbox".to_string(),
        runtime_id: Some("sbx-1".to_string()),
        runtime_created_at: Some(super::protocol::format_instant(now())),
        incarnation_hint: None,
        creator_id: Some(101),
        creator_login: Some("alice".to_string()),
        trigger_author_id: Some(101),
        trigger_author_login: Some("alice".to_string()),
        correlation: CorrelationV1 {
            session_id: Some(session_id.to_string()),
            repo_full_name: Some("acme/site".to_string()),
            installation_id: Some(4242),
            trigger_issue: Some(7),
            webhook_delivery_id: None,
            request_id: None,
        },
        reason_code: None,
        service_version: "0.2.3".to_string(),
        deployment_environment: "test".to_string(),
    }
}

/// Register a start and assert it committed.
pub async fn register(database: &Database, event_id: &str) -> super::db::ingest::Ingested {
    let body = start(event_id);
    let identity = body.to_identity().expect("valid start");
    database
        .write(move |transaction| {
            super::db::ingest::register_start(transaction, &body, &identity, now())
        })
        .await
        .expect("the start commits")
}

/// Parse a wire instant exactly as the HTTP handler does, for the tests that
/// drive the storage layer directly.
pub fn wire_instant(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .expect("a wire instant parses")
        .with_timezone(&Utc)
}

/// Commit a terminal completion and assert it committed.
pub async fn commit(
    database: &Database,
    completion: RequestCompletionV1,
) -> super::db::ingest::Ingested {
    let terminal_at = wire_instant(&completion.completed_at);
    database
        .write(move |transaction| {
            super::db::ingest::commit_completion(transaction, &completion, terminal_at, now())
        })
        .await
        .expect("the completion commits")
}

/// Register a start and immediately complete it, attributed to `actor_id`.
pub async fn durable_request(database: &Database, event_id: &str, actor_id: Option<i64>) {
    register(database, event_id).await;
    commit(database, completion(event_id, actor_id)).await;
}

/// Register a start and complete it inside `session_id`.
pub async fn durable_request_in_session(
    database: &Database,
    event_id: &str,
    actor_id: Option<i64>,
    session_id: &str,
) {
    register(database, event_id).await;
    commit(
        database,
        completion_in_session(event_id, actor_id, session_id),
    )
    .await;
}

/// Commit one lifecycle transition.
pub async fn durable_lifecycle(database: &Database, event_id: &str, session_id: &str) {
    let event = lifecycle(event_id, session_id);
    let occurred_at = wire_instant(&event.occurred_at);
    database
        .write(move |transaction| {
            super::db::ingest::commit_lifecycle(transaction, &event, occurred_at, now())
        })
        .await
        .expect("the lifecycle event commits");
}

/// A relay state + worker whose PostHog host is `server`.
///
/// `tune` gets the resolved configuration before the worker is built, so a test
/// can narrow a batch size or a retry budget without a second constructor.
pub async fn worker_against(
    server: &wiremock::MockServer,
    tune: impl FnOnce(&mut RelayConfig),
) -> (TempDir, RelayState, super::worker::RelayWorker) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("audit.sqlite3");
    let database = Database::open(&path, settings()).expect("database opens");
    let mut relay_config = config(path);
    relay_config.posthog_host = Some(server.uri().trim_end_matches('/').to_string());
    relay_config.posthog_project_token = SecretString::from("phc_test_token".to_string());
    relay_config.posthog_project_id = Some("42".to_string());
    relay_config.posthog_query_api_key = SecretString::from("phx_test_key".to_string());
    tune(&mut relay_config);
    let state = RelayState::new(database, Arc::new(relay_config), RelayMetrics::new());
    let worker = super::worker::RelayWorker::new(&state).expect("worker builds");
    (dir, state, worker)
}

/// Mount a capture endpoint that always accepts.
pub async fn accepting_capture(server: &wiremock::MockServer) {
    for endpoint in ["/capture/", "/batch/"] {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path(endpoint))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"status": 1})),
            )
            .mount(server)
            .await;
    }
}

/// Mount a verification endpoint returning exactly `visible`.
pub async fn verification_returning(server: &wiremock::MockServer, visible: &[&str]) {
    let results: Vec<Vec<String>> = visible
        .iter()
        .map(|event_id| vec![(*event_id).to_string()])
        .collect();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/projects/42/query/"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "columns": ["event_id"],
                "results": results,
            })),
        )
        .mount(server)
        .await;
}

/// The current state of one stored record.
pub async fn state_of(database: &Database, event_id: &str) -> super::record::RecordState {
    let event_id = event_id.to_string();
    let raw: String = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT state FROM audit_records WHERE event_id = ?1",
                    rusqlite::params![event_id],
                    |row| row.get(0),
                )
                .map_err(|error| super::db::classify(&error))
        })
        .await
        .expect("reads the state");
    super::record::RecordState::parse(&raw).expect("a known state")
}

/// A relay state + worker over a real on-disk database, with no PostHog
/// configured (a pure durable outbox).
pub fn local_worker() -> (TempDir, RelayState, super::worker::RelayWorker) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("audit.sqlite3");
    let database = Database::open(&path, settings()).expect("database opens");
    let state = RelayState::new(database, Arc::new(config(path)), RelayMetrics::new());
    let worker = super::worker::RelayWorker::new(&state).expect("worker builds");
    (dir, state, worker)
}
