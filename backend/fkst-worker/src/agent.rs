//! The worker's registry client: registers up to the controller, heartbeats,
//! pulls work, and sends the drain acknowledgements — all over the internal
//! protocol with the shared-secret header.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde::Serialize;

use fkst_shared::protocol::{
    check_protocol_version, ControlMessage, Draining, Heartbeat, HeartbeatResponse, LifecycleState,
    PullRequest, PullResponse, RegisterRequest, RegisterResponse, Released, INTERNAL_AUTH_HEADER,
    PROTOCOL_VERSION,
};

use crate::config::WorkerConfig;

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
}

impl WorkerAgent {
    pub fn new(
        controller_url: String,
        auth: SecretString,
        worker_id: String,
        capacity: u32,
        engine_temp_root: String,
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
        }
    }

    /// Build from the validated worker config.
    pub fn from_config(config: &WorkerConfig) -> Self {
        Self::new(
            config.controller_url.clone(),
            config.internal_auth_token.clone(),
            config.worker_id.clone(),
            config.capacity,
            config.engine_temp_root.clone(),
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

    /// Send one heartbeat and act on any piggybacked control messages.
    pub async fn heartbeat(
        &self,
        state: LifecycleState,
        running: &[String],
    ) -> Result<HeartbeatResponse, AgentError> {
        let hb = Heartbeat {
            worker_id: self.worker_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            lifecycle_state: state,
            running_sessions: running.to_vec(),
            timestamp_unix_ms: now_unix_ms(),
        };
        let resp: HeartbeatResponse = self.post_json("/internal/v1/heartbeat", &hb).await?;
        for ctrl in &resp.control {
            match ctrl {
                ControlMessage::StopSession { session_id, reason } => {
                    // No engine to stop yet (#136); the no-op handoff still
                    // completes the protocol round-trip with a Released.
                    tracing::info!(session_id = %session_id, reason = %reason, "StopSession received; releasing (no engine yet)");
                    if let Err(e) = self.release(session_id).await {
                        tracing::warn!(error = %e, session_id = %session_id, "failed to send Released");
                    }
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
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn agent(uri: String) -> WorkerAgent {
        WorkerAgent::new(
            uri,
            SecretString::from("tok".to_string()),
            "w1".into(),
            4,
            "/tmp/e".into(),
        )
    }

    #[tokio::test]
    async fn register_sends_auth_header_and_parses_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/v1/register"))
            .and(header(INTERNAL_AUTH_HEADER, "tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(RegisterResponse {
                accepted: true,
                heartbeat_interval_secs: 10,
                controller_protocol_version: PROTOCOL_VERSION,
            }))
            .mount(&server)
            .await;

        let resp = agent(server.uri()).register().await.expect("register");
        assert!(resp.accepted);
        assert_eq!(resp.heartbeat_interval_secs, 10);
    }

    #[tokio::test]
    async fn register_fails_closed_on_401() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/v1/register"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = agent(server.uri())
            .register()
            .await
            .expect_err("must fail closed");
        assert!(matches!(err, AgentError::Unauthorized));
    }

    #[tokio::test]
    async fn heartbeat_releases_on_stop_session_control() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/v1/heartbeat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(HeartbeatResponse {
                acknowledged: true,
                control: vec![ControlMessage::StopSession {
                    session_id: "s1".into(),
                    reason: "drain".into(),
                }],
            }))
            .mount(&server)
            .await;
        // The released call is best-effort; mount it so it succeeds.
        Mock::given(method("POST"))
            .and(path("/internal/v1/released"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let resp = agent(server.uri())
            .heartbeat(LifecycleState::Active, &[])
            .await
            .expect("heartbeat");
        assert!(resp.acknowledged);
    }
}
