//! A real `fkst-audit-relay` listening on a loopback port, over a real SQLite
//! file, driven by the real control-plane client.
//!
//! Nothing here mocks the relay: the point of the end-to-end suite is that the
//! protocol, the durability contract, and the scoped read all hold across an
//! actual socket and an actual restart. The tokens are obvious canaries so a
//! leak into a response or the database file is trivially detectable.

// Shared by several acceptance suites, each of which drives a different subset
// of the surface below.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use fkst_control_plane::audit::relay::{
    AuditDeliveryConfig, AuditDeliveryMode, AuditRelayClient, RelayClientMetrics,
};
use fkst_control_plane::audit_relay::protocol::{
    format_instant, ActorV1, CorrelationV1, LifecycleEventV1, PrincipalV1, RequestCompletionV1,
    RequestStartV1, PROTOCOL_SCHEMA_VERSION,
};
use fkst_control_plane::audit_relay::query::{RecordRowV1, RecordsQueryV1};
use fkst_control_plane::audit_relay::{
    build_router, Database, DatabaseSettings, RelayConfig, RelayMetrics, RelayState,
};
use k8s_openapi::chrono::{DateTime, Duration, Utc};
use secrecy::SecretString;
use tempfile::TempDir;

/// The write credential. A canary: it must never reach a response or the file.
pub const WRITE_TOKEN: &str = "canary-e2e-write-token-4f2a";
/// The read credential. Same canary rule.
pub const READ_TOKEN: &str = "canary-e2e-read-token-9b71";

/// The two verified actors the cross-user fixture uses.
pub const ALICE: i64 = 101;
pub const BOB: i64 = 202;

/// A running relay plus everything needed to talk to it.
pub struct Relay {
    /// Kept so the temporary directory outlives the relay.
    dir: Arc<TempDir>,
    db_path: PathBuf,
    /// A second handle on the SAME database, so a test can drive the relay's own
    /// maintenance sweep without going through the HTTP surface (the sweep has
    /// no endpoint — it is a timer inside the process).
    db: Database,
    config: Arc<RelayConfig>,
    base_url: String,
    http: reqwest::Client,
    /// Fires the server's graceful shutdown. Taken by [`Relay::restart`].
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    /// The serving task, awaited on restart so the previous relay's `RelayState`
    /// — and with it its `Database`, writer thread, and pooled read connections —
    /// is genuinely gone before a new one opens the same file.
    server: tokio::task::JoinHandle<()>,
}

impl Relay {
    /// Start a relay on an ephemeral loopback port over a fresh database.
    pub async fn start() -> Self {
        let dir = Arc::new(TempDir::new().expect("temp dir"));
        let db_path = dir.path().join("audit.sqlite3");
        Self::serve(dir, db_path).await
    }

    /// Stop this relay COMPLETELY, then start a new one over the same file.
    ///
    /// Dropping the handle is not enough and never was: the serving task owns
    /// the state, so a dropped handle leaves the old `Database` — its writer
    /// thread and its open connections — alive on the same file. That would test
    /// "a second handle can re-open the file" while quietly running the
    /// multi-writer shape this deployable's non-goals forbid. The shutdown
    /// signal plus the join is what makes the restart a real process death.
    pub async fn restart(mut self) -> Self {
        let dir = self.dir.clone();
        let db_path = self.db_path.clone();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        // The task returns once the listener is closed and in-flight requests
        // finish; awaiting it is what guarantees the old state is dropped.
        let _ = (&mut self.server).await;
        drop(self);
        Self::serve(dir, db_path).await
    }

    async fn serve(dir: Arc<TempDir>, db_path: PathBuf) -> Self {
        let database = Database::open(
            &db_path,
            DatabaseSettings {
                busy_timeout_ms: 5_000,
                writer_queue_capacity: 128,
                read_concurrency: 4,
                max_records: 1_000_000,
            },
        )
        .expect("the relay database opens");
        let settings = Arc::new(config(db_path.clone()));
        let db = database.clone();
        let state = RelayState::new(database, settings.clone(), RelayMetrics::new());
        let router = build_router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the relay binds a loopback port");
        let addr = listener.local_addr().expect("the bound address");
        let (shutdown, halt) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router.into_make_service())
                .with_graceful_shutdown(async move {
                    let _ = halt.await;
                })
                .await;
        });
        Self {
            dir,
            db_path,
            db,
            config: settings,
            base_url: format!("http://{addr}"),
            http: reqwest::Client::new(),
            shutdown: Some(shutdown),
            server,
        }
    }

    /// Run the relay's own maintenance sweep once, as its timer would.
    ///
    /// With no PostHog configured the sweep reduces to exactly one behaviour:
    /// closing starts whose completion deadline plus grace has passed. That is
    /// the only way an abandoned request becomes an `incomplete` terminal, and
    /// it has no HTTP surface to drive it from.
    pub async fn sweep(&self, now: DateTime<Utc>) {
        let state = RelayState::new(self.db.clone(), self.config.clone(), RelayMetrics::new());
        fkst_control_plane::audit_relay::RelayWorker::new(&state)
            .expect("the worker builds")
            .sweep(now)
            .await;
    }

    /// Every record in a window wide enough to hold a synthesized terminal,
    /// whose instant is `now` rather than the fixture anchor.
    pub async fn read_all_recent(&self) -> Vec<RecordRowV1> {
        self.client()
            .read_records(
                &RecordsQueryV1 {
                    scope: "all".to_string(),
                    record_kind: "api_request".to_string(),
                    from: format_instant(anchor() - Duration::days(7)),
                    to: format_instant(Utc::now() + Duration::days(7)),
                    limit: 100,
                    ..RecordsQueryV1::default()
                },
                std::time::Duration::from_secs(5),
            )
            .await
            .expect("the relay answers")
            .rows
    }

    /// The loopback base URL this relay is listening on.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// A control-plane client for this relay. Each call models one replica.
    pub fn client(&self) -> Arc<AuditRelayClient> {
        let config = AuditDeliveryConfig {
            mode: AuditDeliveryMode::Required,
            relay_url: Some(self.base_url.clone()),
            write_token: SecretString::from(WRITE_TOKEN.to_string()),
            read_token: SecretString::from(READ_TOKEN.to_string()),
            start_timeout_ms: 2_000,
            completion_timeout_ms: 2_000,
            incomplete_grace_secs: 60,
        };
        Arc::new(
            AuditRelayClient::from_config(&config, RelayClientMetrics::new())
                .expect("the relay client builds"),
        )
    }

    /// The raw database bytes (main file plus WAL), for the canary scan.
    pub fn database_bytes(&self) -> Vec<u8> {
        let mut bytes = std::fs::read(&self.db_path).unwrap_or_default();
        bytes.extend(std::fs::read(self.db_path.with_extension("sqlite3-wal")).unwrap_or_default());
        bytes
    }

    /// The global-scope page.
    pub async fn read_all(&self) -> Vec<RecordRowV1> {
        self.client()
            .read_records(
                &RecordsQueryV1 {
                    scope: "all".to_string(),
                    record_kind: "api_request".to_string(),
                    from: format_instant(anchor() - Duration::hours(24)),
                    to: format_instant(anchor() + Duration::hours(24)),
                    limit: 100,
                    ..RecordsQueryV1::default()
                },
                std::time::Duration::from_secs(5),
            )
            .await
            .expect("the relay answers")
            .rows
    }

    /// A personal-scope page for `actor_id`.
    pub async fn read_personal(
        &self,
        actor_id: i64,
        session_id: Option<&str>,
        record_kind: &str,
        limit: u32,
        cursor: Option<(String, String)>,
    ) -> Vec<RecordRowV1> {
        let (cursor_timestamp, cursor_event_id) = match cursor {
            Some((timestamp, event_id)) => (Some(timestamp), Some(event_id)),
            None => (None, None),
        };
        self.client()
            .read_records(
                &RecordsQueryV1 {
                    scope: "mine".to_string(),
                    actor_id: Some(actor_id),
                    lifecycle_session_id: session_id.map(str::to_string),
                    record_kind: record_kind.to_string(),
                    from: format_instant(anchor() - Duration::hours(24)),
                    to: format_instant(anchor() + Duration::hours(24)),
                    limit,
                    cursor_timestamp,
                    cursor_event_id,
                    ..RecordsQueryV1::default()
                },
                std::time::Duration::from_secs(5),
            )
            .await
            .expect("the relay answers")
            .rows
    }

    /// Attempt a write with an arbitrary bearer token; returns the status.
    pub async fn raw_write_with(&self, token: &str) -> axum::http::StatusCode {
        let response = self
            .http
            .post(format!(
                "{}/internal/v1/audit/request-starts",
                self.base_url
            ))
            .bearer_auth(token)
            .json(&Self::start_body("e1111111-1111-4111-8111-111111111111"))
            .send()
            .await
            .expect("the relay answers");
        axum::http::StatusCode::from_u16(response.status().as_u16()).expect("a valid status")
    }

    /// Attempt a read with an arbitrary bearer token; returns the status.
    pub async fn raw_read_with(&self, token: &str) -> axum::http::StatusCode {
        let response = self
            .http
            .get(format!("{}/internal/v1/audit/records", self.base_url))
            .bearer_auth(token)
            .query(&RecordsQueryV1 {
                scope: "all".to_string(),
                record_kind: "api_request".to_string(),
                from: format_instant(anchor() - Duration::hours(24)),
                to: format_instant(anchor()),
                limit: 10,
                ..RecordsQueryV1::default()
            })
            .send()
            .await
            .expect("the relay answers");
        axum::http::StatusCode::from_u16(response.status().as_u16()).expect("a valid status")
    }

    /// Two of Alice's calls (one inside `sess-1`), one of Bob's inside the same
    /// session, one unattributed call, and one system lifecycle row.
    pub async fn seed_cross_user_fixture(&self) {
        let client = self.client();
        let fixture: [(&str, Option<i64>, Option<&str>); 4] = [
            ("a1111111-1111-4111-8111-111111111111", Some(ALICE), None),
            (
                "a2222222-2222-4222-8222-222222222222",
                Some(ALICE),
                Some("sess-1"),
            ),
            (
                "b1111111-1111-4111-8111-111111111111",
                Some(BOB),
                Some("sess-1"),
            ),
            ("c1111111-1111-4111-8111-111111111111", None, None),
        ];
        for (index, (event_id, actor_id, session_id)) in fixture.into_iter().enumerate() {
            client
                .register_start(&Self::start_body(event_id))
                .await
                .expect("the start is acknowledged");
            let mut completion = Self::completion_body(event_id, actor_id);
            // Distinct completion instants so pagination has a total order.
            let completed_at = anchor() + Duration::seconds(index as i64);
            completion.completed_at = format_instant(completed_at);
            completion.duration_ms =
                u64::try_from((completed_at - anchor()).num_milliseconds()).unwrap_or(0);
            if let Some(session_id) = session_id {
                completion.session_id = Some(session_id.to_string());
                completion.correlation.session_id = Some(session_id.to_string());
            }
            client
                .complete(&completion)
                .await
                .expect("the completion is acknowledged");
        }
        client
            .submit_lifecycle(&Self::lifecycle_body(
                "d1111111-1111-4111-8111-111111111111",
                "sess-1",
            ))
            .await
            .expect("the lifecycle event is acknowledged");
    }

    /// A start body for `event_id`.
    pub fn start_body(event_id: &str) -> RequestStartV1 {
        RequestStartV1 {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: event_id.to_string(),
            request_id: format!("req-{event_id}"),
            started_at: format_instant(anchor()),
            method: "GET".to_string(),
            route_template: "/api/v1/overview".to_string(),
            operation_id: "canvas_overview".to_string(),
            service_version: "0.2.3".to_string(),
            deployment_environment: "test".to_string(),
            completion_deadline_at: format_instant(anchor() + Duration::seconds(60)),
        }
    }

    /// The terminal body matching [`Relay::start_body`].
    pub fn completion_body(event_id: &str, actor_id: Option<i64>) -> RequestCompletionV1 {
        let start = Self::start_body(event_id);
        RequestCompletionV1 {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: start.event_id.clone(),
            request_id: start.request_id.clone(),
            started_at: start.started_at.clone(),
            completed_at: format_instant(anchor()),
            method: start.method.clone(),
            route_template: start.route_template.clone(),
            operation_id: start.operation_id.clone(),
            arguments: serde_json::Map::new(),
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
                id: None,
            },
            status_code: Some(200),
            outcome: "success".to_string(),
            error_code: None,
            duration_ms: 0,
            session_id: None,
            correlation: CorrelationV1::default(),
            service_version: "0.2.3".to_string(),
            deployment_environment: "test".to_string(),
        }
    }

    /// A system lifecycle transition for `session_id`.
    pub fn lifecycle_body(event_id: &str, session_id: &str) -> LifecycleEventV1 {
        LifecycleEventV1 {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: event_id.to_string(),
            occurred_at: format_instant(anchor()),
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
            runtime_created_at: Some(format_instant(anchor())),
            incarnation_hint: None,
            creator_id: Some(ALICE),
            creator_login: Some("alice".to_string()),
            trigger_author_id: Some(ALICE),
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
}

/// A fixed instant so every derived timestamp is reproducible.
pub fn anchor() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-31T12:00:00.000Z")
        .expect("the fixed instant parses")
        .with_timezone(&Utc)
}

/// The relay configuration for a test listener: no PostHog, so the process is a
/// pure durable outbox and every assertion is about storage rather than delivery.
fn config(db_path: PathBuf) -> RelayConfig {
    RelayConfig {
        bind_addr: "127.0.0.1:0".parse().expect("loopback parses"),
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
        writer_queue_capacity: 128,
        max_read_concurrency: 4,
        max_read_rows: 500,
        max_range_days: 400,
        capture_batch_size: 50,
        max_capture_attempts: 5,
        retry_initial_secs: 5,
        retry_max_secs: 300,
        worker_interval_secs: 5,
        verification_batch_size: 100,
        posthog_host: None,
        posthog_project_token: SecretString::from(String::new()),
        posthog_project_id: None,
        posthog_query_api_key: SecretString::from(String::new()),
        environment: "test".to_string(),
    }
}
