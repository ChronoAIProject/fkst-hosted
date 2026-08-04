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

/// The wire bodies and the seeded cross-user dataset.
mod fixtures;

use std::path::PathBuf;
use std::sync::Arc;

use fkst_control_plane::audit::relay::{
    AuditDeliveryConfig, AuditDeliveryMode, AuditRelayClient, RelayClientMetrics,
};
use fkst_control_plane::audit_relay::protocol::format_instant;
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

    /// Start a relay over an EXISTING database file, taking ownership of the
    /// temporary directory holding it.
    ///
    /// Used by the storage-failure suite, which has to put a file into a
    /// particular state (wrong mode, foreign schema) BEFORE a relay ever sees
    /// it, and then prove what the relay does with it.
    pub async fn start_at(dir: TempDir, db_path: PathBuf) -> Self {
        Self::serve(Arc::new(dir), db_path).await
    }

    /// Stop this relay COMPLETELY and hand back the pieces needed to start it
    /// again over the same file.
    ///
    /// Separate from [`Relay::restart`] because a kill-point test has to observe
    /// the world WHILE the relay is down — a client submitting into a closed
    /// socket is the whole scenario — and `restart` never exposes that window.
    pub async fn stop(self) -> StoppedRelay {
        let dir = self.dir.clone();
        let db_path = self.db_path.clone();
        let mut this = self;
        if let Some(shutdown) = this.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = (&mut this.server).await;
        // The base URL is retained so a client built before the stop keeps
        // pointing at a port nothing is listening on, which is exactly the
        // shape a killed relay presents to an in-flight caller.
        drop(this);
        StoppedRelay { dir, db_path }
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
}

/// A relay that has been stopped, holding only its durable state.
///
/// It deliberately exposes nothing but [`StoppedRelay::resume`]: while a relay is
/// down there is no socket to talk to, and a handle that pretended otherwise
/// would let a test assert against a process that does not exist.
pub struct StoppedRelay {
    dir: Arc<TempDir>,
    db_path: PathBuf,
}

impl StoppedRelay {
    /// Start a fresh relay process over the same database file.
    pub async fn resume(self) -> Relay {
        Relay::serve(self.dir, self.db_path).await
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
