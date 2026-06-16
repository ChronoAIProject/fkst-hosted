//! The worker's registry client: registers up to the controller, heartbeats,
//! pulls work, and sends the drain acknowledgements — all over the internal
//! protocol with the shared-secret header.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde::Serialize;

use fkst_engine::EngineConfig;
use fkst_shared::protocol::{
    check_protocol_version, ControlMessage, Draining, Heartbeat, HeartbeatResponse, LifecycleState,
    PullRequest, PullResponse, RegisterRequest, RegisterResponse, Released, ResolvedDispatch,
    INTERNAL_AUTH_HEADER, PROTOCOL_VERSION,
};

use crate::config::WorkerConfig;
use crate::engine::{execute_dispatch, ExecutedSession};

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
    /// In-memory registry of the sessions this worker currently runs, keyed by
    /// session id. The `ResolvedDispatch` arm inserts a spawned
    /// [`ExecutedSession`]; the heartbeat reports the keys as
    /// `running_sessions`. The lock is sync and is NEVER held across an await.
    /// The supervise loop that drains it lands in the next increment; for now an
    /// entry lives until the worker exits.
    sessions: Mutex<HashMap<String, ExecutedSession>>,
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
    /// (the sessions this worker currently runs).
    pub async fn heartbeat(&self, state: LifecycleState) -> Result<HeartbeatResponse, AgentError> {
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
                    // No supervise/stop loop yet (#151 increment 4 only spawns);
                    // the no-op handoff still completes the protocol round-trip
                    // with a Released.
                    tracing::info!(session_id = %session_id, reason = %reason, "StopSession received; releasing (no stop loop yet)");
                    if let Err(e) = self.release(session_id).await {
                        tracing::warn!(error = %e, session_id = %session_id, "failed to send Released");
                    }
                }
                ControlMessage::ResolvedDispatch(dispatch) => {
                    // #151 increment 4: spawn + register the engine. DORMANT in
                    // prod — the controller never emits this until the activation
                    // increment, so this is reachable only in tests today.
                    self.handle_resolved_dispatch(dispatch).await;
                }
            }
        }
        Ok(resp)
    }

    /// Spawn the engine for a resolved dispatch and register the running session
    /// (#151). A spawn failure is logged LOUDLY (never a secret) and swallowed —
    /// a single bad dispatch must NOT crash the worker. Idempotent: a dispatch
    /// for a session already registered is ignored (the controller's claim is the
    /// authoritative dedupe).
    async fn handle_resolved_dispatch(&self, dispatch: &ResolvedDispatch) {
        let session_id = dispatch.session_id.clone();
        if self
            .sessions
            .lock()
            .expect("session registry poisoned")
            .contains_key(&session_id)
        {
            tracing::debug!(session_id = %session_id, "dispatch for an already-running session; ignoring");
            return;
        }
        // `execute_dispatch` is awaited to completion (the lock is NOT held across
        // it); the spawned session is registered afterwards.
        match execute_dispatch(&self.engine_config, dispatch, &self.http).await {
            Ok(session) => {
                let pid = session.running.pid;
                self.sessions
                    .lock()
                    .expect("session registry poisoned")
                    .insert(session_id.clone(), session);
                tracing::info!(session_id = %session_id, pid, "engine spawned and session registered");
            }
            Err(error) => {
                // The error never carries a secret (see ExecError); log it loudly
                // and keep serving — the worker stays up.
                tracing::error!(session_id = %session_id, error = %error, "failed to execute dispatch; session NOT registered");
            }
        }
    }

    /// The session ids this worker currently runs (the heartbeat report). The
    /// lock is taken briefly and never held across an await.
    fn running_session_ids(&self) -> Vec<String> {
        self.sessions
            .lock()
            .expect("session registry poisoned")
            .keys()
            .cloned()
            .collect()
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

    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
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
