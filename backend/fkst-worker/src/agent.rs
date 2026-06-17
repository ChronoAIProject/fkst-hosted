//! The worker's registry client: registers up to the controller, heartbeats,
//! pulls work, and sends the drain acknowledgements — all over the internal
//! protocol with the shared-secret header.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use fkst_engine::EngineConfig;
use fkst_shared::protocol::{
    check_protocol_version, ControlMessage, CredentialRefreshRequest, CredentialRefreshResponse,
    Draining, Heartbeat, HeartbeatResponse, LifecycleState, PullRequest, PullResponse,
    RefreshReason, RegisterRequest, RegisterResponse, Released, SessionStatus, StatusReport,
    TerminalExit, INTERNAL_AUTH_HEADER, PROTOCOL_VERSION,
};

use crate::config::WorkerConfig;

#[path = "agent_sessions.rs"]
mod sessions;

#[path = "agent_lifecycle.rs"]
mod lifecycle;

/// Errors talking to the controller. Transport / 5xx are transient (retried);
/// 4xx / auth / version / decode are fatal config-or-protocol problems.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("unauthorized (401): the controller rejected the internal-auth secret")]
    Unauthorized,
    #[error("controller returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("protocol error: {0}")]
    Version(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("controller did not accept registration")]
    Rejected,
}

impl AgentError {
    /// Transient errors are retried; fatal ones stop the worker (fail-closed).
    fn is_transient(&self) -> bool {
        match self {
            AgentError::Transport(_) => true,
            AgentError::Status { status, .. } => *status >= 500,
            _ => false,
        }
    }
}

/// A session this worker is actively driving: the stop signal the supervise loop
/// observes, plus the loop's join handle. The supervise loop OWNS the
/// `RunningSession`, the on-disk guards, and the refresh servicer; the registry
/// holds only the control surface (flip `stop` to command a stop) so the lock is
/// never held across the loop's work.
pub(crate) struct LiveSession {
    /// Flipped to `true` to command the supervise loop to stop the engine, send
    /// a Released, and exit.
    stop: watch::Sender<bool>,
    /// The supervise task; joined on a commanded stop so the agent's
    /// `StopSession` arm awaits the loop's drain (stop + Released).
    handle: JoinHandle<()>,
}

impl LiveSession {
    /// Bundle the supervise loop's stop signal + join handle for the registry.
    pub(crate) fn new(stop: watch::Sender<bool>, handle: JoinHandle<()>) -> Self {
        Self { stop, handle }
    }

    /// Split into the stop signal + join handle for the commanded-stop drain.
    pub(crate) fn into_parts(self) -> (watch::Sender<bool>, JoinHandle<()>) {
        (self.stop, self.handle)
    }
}

/// Registry client over the internal protocol.
pub struct WorkerAgent {
    http: reqwest::Client,
    controller_url: String,
    auth: SecretString,
    worker_id: String,
    capacity: u32,
    engine_temp_root: String,
    /// Per-worker retry jitter (ms), derived from the worker id so a fleet
    /// retrying after a controller restart decorrelates without a PRNG.
    jitter_ms: u64,
    /// Engine wiring for a dispatched session (#151). Used ONLY when a
    /// `ResolvedDispatch` arrives — which the controller never sends until the
    /// activation increment, so this stays unused in prod (the arm is dormant).
    engine_config: EngineConfig,
    /// In-memory registry of the sessions this worker currently drives, keyed by
    /// session id. The `ResolvedDispatch` arm (and the startup re-adopt) inserts
    /// a [`LiveSession`] once the supervise loop is spawned; the heartbeat
    /// reports the keys as `running_sessions`; a `StopSession` flips the stop
    /// signal, awaits the loop's drain, and removes the entry. The lock is sync
    /// and is NEVER held across an await.
    sessions: Mutex<HashMap<String, LiveSession>>,
    /// The worker's own drain state, the gate the pull loop reads to stop
    /// requesting work and the drain routine flips on SIGTERM (#140a). Encoded
    /// as a `u8` ([`LIFECYCLE_ACTIVE`] / [`LIFECYCLE_DRAINING`]) so it is a
    /// lock-free `AtomicU8` shared across the heartbeat / pull / drain tasks; the
    /// heartbeat's `LifecycleState` argument is passed explicitly by the run loop
    /// and is unrelated to this gate. Begins `Active`; `begin_drain` flips it to
    /// `Draining` idempotently and it never flips back (drain is terminal).
    lifecycle: Arc<AtomicU8>,
}

impl WorkerAgent {
    pub fn new(
        controller_url: String,
        auth: SecretString,
        worker_id: String,
        capacity: u32,
        engine_temp_root: String,
    ) -> Self {
        // The engine config drives a dispatched session (#151). Its `temp_root`
        // is the worker's reported engine temp dir; the rest defaults (the arm
        // is dormant in prod). `from_config` overrides with the env-loaded one.
        let engine_config = EngineConfig {
            temp_root: engine_temp_root.clone().into(),
            ..EngineConfig::default()
        };
        Self::with_engine_config(
            controller_url,
            auth,
            worker_id,
            capacity,
            engine_temp_root,
            engine_config,
        )
    }

    /// Inner constructor taking the resolved [`EngineConfig`], so `from_config`
    /// can supply the env-loaded one and `new` a defaulted one.
    fn with_engine_config(
        controller_url: String,
        auth: SecretString,
        worker_id: String,
        capacity: u32,
        engine_temp_root: String,
        engine_config: EngineConfig,
    ) -> Self {
        let mut hasher = DefaultHasher::new();
        worker_id.hash(&mut hasher);
        let jitter_ms = hasher.finish() % 1000;
        Self {
            http: reqwest::Client::new(),
            controller_url: controller_url.trim_end_matches('/').to_string(),
            auth,
            worker_id,
            capacity,
            engine_temp_root,
            jitter_ms,
            engine_config,
            sessions: Mutex::new(HashMap::new()),
            lifecycle: Arc::new(AtomicU8::new(lifecycle::ACTIVE)),
        }
    }

    /// Build from the validated worker config. The engine config is loaded from
    /// the process environment (its defaults are all present, so this only fails
    /// on an explicitly malformed engine var); on a load error we fall back to a
    /// defaulted config rooted at the worker's engine temp dir and log it — a
    /// worker must still serve heartbeats even with a dormant dispatch arm.
    pub fn from_config(config: &WorkerConfig) -> Self {
        let engine_config = match EngineConfig::load_from_env() {
            Ok(mut cfg) => {
                // Pin the temp root to the worker's reported engine temp dir so a
                // dispatched session's clone/runtime/codex dirs land where the
                // worker advertises capacity.
                cfg.temp_root = config.engine_temp_root.clone().into();
                cfg
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "engine config failed to load from env; using defaults (dispatch is dormant)"
                );
                EngineConfig {
                    temp_root: config.engine_temp_root.clone().into(),
                    ..EngineConfig::default()
                }
            }
        };
        Self::with_engine_config(
            config.controller_url.clone(),
            config.internal_auth_token.clone(),
            config.worker_id.clone(),
            config.capacity,
            config.engine_temp_root.clone(),
            engine_config,
        )
    }

    /// POST `body` to `path` with the internal-auth header and decode the JSON
    /// answer.
    async fn post_json<Req, Resp>(&self, path: &str, body: &Req) -> Result<Resp, AgentError>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let url = format!("{}{}", self.controller_url, path);
        let resp = self
            .http
            .post(&url)
            .header(INTERNAL_AUTH_HEADER, self.auth.expose_secret())
            .json(body)
            .send()
            .await
            .map_err(|e| AgentError::Transport(e.to_string()))?;
        let status = resp.status();
        if status.as_u16() == 401 {
            return Err(AgentError::Unauthorized);
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AgentError::Status {
                status: status.as_u16(),
                body: text,
            });
        }
        resp.json::<Resp>()
            .await
            .map_err(|e| AgentError::Decode(e.to_string()))
    }

    /// Bounded exponential backoff + per-worker jitter.
    fn backoff(&self, attempt: u32) -> Duration {
        const BASE_MS: u64 = 500;
        const CAP_MS: u64 = 30_000;
        let exp = BASE_MS.saturating_mul(1u64 << attempt.min(6));
        Duration::from_millis(exp.min(CAP_MS) + self.jitter_ms)
    }

    /// Register up to the controller, retrying transient failures forever (a
    /// worker that cannot reach its controller is useless but must not crash-
    /// loop tightly). Fatal on a wrong secret or an incompatible protocol.
    pub async fn register(&self) -> Result<RegisterResponse, AgentError> {
        let req = RegisterRequest {
            worker_id: self.worker_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            capacity: self.capacity,
            engine_temp_root: self.engine_temp_root.clone(),
        };
        let mut attempt: u32 = 0;
        loop {
            match self
                .post_json::<_, RegisterResponse>("/internal/v1/register", &req)
                .await
            {
                Ok(resp) => {
                    check_protocol_version(resp.controller_protocol_version)
                        .map_err(|e| AgentError::Version(e.to_string()))?;
                    if !resp.accepted {
                        return Err(AgentError::Rejected);
                    }
                    tracing::info!(
                        worker_id = %self.worker_id,
                        heartbeat_interval_secs = resp.heartbeat_interval_secs,
                        "registered with controller"
                    );
                    return Ok(resp);
                }
                Err(e) if e.is_transient() => {
                    attempt = attempt.saturating_add(1);
                    let backoff = self.backoff(attempt);
                    tracing::warn!(
                        attempt,
                        error = %e,
                        backoff_ms = backoff.as_millis() as u64,
                        "registration failed; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                }
                Err(e) => {
                    tracing::error!(error = %e, "registration failed fatally (wrong secret or protocol)");
                    return Err(e);
                }
            }
        }
    }

    /// Send one heartbeat and act on any piggybacked control messages. The
    /// reported `running_sessions` are the keys of the in-memory session registry
    /// (the sessions this worker currently drives).
    ///
    /// Takes `self: &Arc<Self>` because a dispatch spawns a supervise loop that
    /// holds an `Arc<WorkerAgent>` for the duration of the session (it calls
    /// `refresh_credential` / `report_status` / `release` / `engine_runner`).
    pub async fn heartbeat(
        self: &Arc<Self>,
        state: LifecycleState,
    ) -> Result<HeartbeatResponse, AgentError> {
        let hb = Heartbeat {
            worker_id: self.worker_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            lifecycle_state: state,
            running_sessions: self.running_session_ids(),
            timestamp_unix_ms: now_unix_ms(),
        };
        let resp: HeartbeatResponse = self.post_json("/internal/v1/heartbeat", &hb).await?;
        for ctrl in &resp.control {
            match ctrl {
                ControlMessage::StopSession { session_id, reason } => {
                    tracing::info!(session_id = %session_id, reason = %reason, "StopSession received");
                    self.stop_session(session_id).await;
                }
                ControlMessage::ResolvedDispatch(dispatch) => {
                    // #151: spawn + supervise the engine. DORMANT in prod — the
                    // controller never emits this until the activation increment,
                    // so this is reachable only in tests today.
                    self.handle_resolved_dispatch(dispatch).await;
                }
            }
        }
        Ok(resp)
    }

    /// Pull work from the controller (empty until #135).
    pub async fn pull(&self, available_capacity: u32) -> Result<PullResponse, AgentError> {
        let req = PullRequest {
            worker_id: self.worker_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            available_capacity,
        };
        self.post_json("/internal/v1/pull", &req).await
    }

    /// Announce that the worker has begun draining (sendable now; no path
    /// triggers a real drain until #140).
    pub async fn send_draining(
        &self,
        sessions: &[String],
        checkpoint_done: bool,
    ) -> Result<(), AgentError> {
        let d = Draining {
            worker_id: self.worker_id.clone(),
            sessions: sessions.to_vec(),
            checkpoint_done,
        };
        let _: serde_json::Value = self.post_json("/internal/v1/draining", &d).await?;
        Ok(())
    }

    /// Acknowledge that a session's engine is stopped, so the controller can
    /// reassign without a double-run.
    pub async fn release(&self, session_id: &str) -> Result<(), AgentError> {
        let r = Released {
            worker_id: self.worker_id.clone(),
            session_id: session_id.to_string(),
        };
        let _: serde_json::Value = self.post_json("/internal/v1/released", &r).await?;
        Ok(())
    }

    /// Ask the controller to mint a fresh GitHub installation token for a running
    /// session (#151). The worker holds no App key, so EVERY fresh token comes
    /// from this fence-stamped RPC. `fencing_id` is the claim's current fence the
    /// controller checks: a stale fence is refused with `credentials: None` (the
    /// caller self-fences), an uninstalled App answers `gone: true` (the caller
    /// fails the session). The token in the answer is a `SecretString` (never
    /// logged); only the supervise loop's token writer exposes it.
    pub async fn refresh_credential(
        &self,
        session_id: &str,
        fencing_id: i64,
        repo_ref: &str,
        reason: RefreshReason,
    ) -> Result<CredentialRefreshResponse, AgentError> {
        let req = CredentialRefreshRequest {
            worker_id: self.worker_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            session_id: session_id.to_string(),
            fencing_id,
            repo_ref: repo_ref.to_string(),
            reason,
        };
        self.post_json("/internal/v1/credential-refresh", &req)
            .await
    }

    /// Report a session's lifecycle status to the controller (#151). Fence-
    /// stamped: a superseded worker's report is a controller-side no-op (it
    /// cannot overwrite the claim's status). `terminal` carries the exit only on
    /// a terminal status (Stopped/Failed). Best-effort at the call site: a failed
    /// report is logged and retried by the supervise loop on its next tick.
    pub async fn report_status(
        &self,
        session_id: &str,
        fencing_id: i64,
        status: SessionStatus,
        terminal: Option<TerminalExit>,
    ) -> Result<(), AgentError> {
        let report = StatusReport {
            worker_id: self.worker_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            session_id: session_id.to_string(),
            fencing_id,
            status,
            terminal,
            timestamp_unix_ms: now_unix_ms(),
        };
        let _: serde_json::Value = self
            .post_json("/internal/v1/status-report", &report)
            .await?;
        Ok(())
    }

    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    // --- session-registry accessors (used by the `sessions` child module) -----

    /// The session ids this worker currently drives (the heartbeat report). The
    /// lock is taken briefly and never held across an await.
    pub(crate) fn running_session_ids(&self) -> Vec<String> {
        self.sessions
            .lock()
            .expect("session registry poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// Whether a session id is already in the registry (the at-most-one-loop
    /// dedupe). The lock is taken briefly and never held across an await.
    pub(crate) fn is_driving(&self, session_id: &str) -> bool {
        self.sessions
            .lock()
            .expect("session registry poisoned")
            .contains_key(session_id)
    }

    /// Insert a freshly-spawned [`LiveSession`] into the registry.
    pub(crate) fn insert_live_session(&self, session_id: String, live: LiveSession) {
        self.sessions
            .lock()
            .expect("session registry poisoned")
            .insert(session_id, live);
    }

    /// Remove + return a session's [`LiveSession`] (the commanded-stop drain).
    pub(crate) fn take_live_session(&self, session_id: &str) -> Option<LiveSession> {
        self.sessions
            .lock()
            .expect("session registry poisoned")
            .remove(session_id)
    }

    /// The worker's engine config (the dispatch + re-adopt paths read it).
    pub(crate) fn engine_config(&self) -> &EngineConfig {
        &self.engine_config
    }

    /// The configured engine temp root — the re-adopt scan root.
    pub(crate) fn engine_temp_root_path(&self) -> &std::path::Path {
        self.engine_config.temp_root.as_path()
    }

    /// The shared HTTP client (the dispatch path reuses it for the presigned
    /// ornn-zip fetch, so a worker keeps one connection pool).
    pub(crate) fn http_client(&self) -> &reqwest::Client {
        &self.http
    }
}

/// Milliseconds since the Unix epoch (saturating; never panics on a clock skew).
fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
