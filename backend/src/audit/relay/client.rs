//! The control plane's client for the internal relay protocol.
//!
//! ## Every call is bounded, and every retry is idempotent
//!
//! The start budget is paid before an audited handler runs and the completion
//! budget holds a finished response, so both are hard deadlines rather than
//! "until it works". Within a budget the client retries, which is safe precisely
//! because the protocol is keyed on `event_id`: a retry either creates the record
//! or is answered as an exact replay. It is never a second record.
//!
//! ## Secret hygiene
//!
//! The write and read tokens ride only the `Authorization` header, live in
//! [`SecretString`]s, and are rendered `<redacted>` by the hand-written `Debug`.
//! Nothing here logs a header, a request body (which is an audit record), a
//! response body, an upstream error string, or a URL. Failures carry a numeric
//! status and a `&'static str` category, both structurally incapable of carrying
//! a credential.
//!
//! ## A `409` is not a failure
//!
//! [`RelayClientError::Conflict`] means the id is already durable with different
//! content. In `required` mode the record is therefore already visible, so the
//! caller must not treat it as "unrecorded" — the distinction is why the error
//! type has a variant for it instead of a status code the caller has to parse.

use std::time::{Duration, Instant};

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::audit_relay::protocol::{
    DurableAck, LifecycleEventV1, RequestCompletionV1, RequestStartV1, EVENTS_PATH,
    REQUEST_STARTS_PATH,
};
use crate::audit_relay::query::{RecordsPageV1, RecordsQueryV1};
use crate::error::AppError;

use super::config::AuditDeliveryConfig;
use super::metrics::{RelayCallResult, RelayClientMetrics, RelayPhase};

/// Hard cap on a relay response body. An acknowledgement is a few hundred bytes;
/// a scoped page is bounded by the relay's own row ceiling.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Why a relay call did not produce an acknowledgement.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RelayClientError {
    /// The relay could not be reached, or answered `5xx`/`503`.
    #[error("the audit relay is unavailable ({kind})")]
    Unavailable { kind: &'static str },
    /// The relay refused the body (`400`) or the credentials (`401`).
    #[error("the audit relay refused the request ({kind})")]
    Rejected { kind: &'static str },
    /// The event id is already durable with different immutable content.
    #[error("the audit relay reports an event id conflict")]
    Conflict,
}

impl RelayClientError {
    /// The bounded telemetry label.
    pub fn call_result(self) -> RelayCallResult {
        match self {
            RelayClientError::Unavailable { .. } => RelayCallResult::Unavailable,
            RelayClientError::Rejected { .. } => RelayCallResult::Rejected,
            RelayClientError::Conflict => RelayCallResult::Conflict,
        }
    }

    /// The bounded reason category.
    pub fn kind(self) -> &'static str {
        match self {
            RelayClientError::Unavailable { kind } | RelayClientError::Rejected { kind } => kind,
            RelayClientError::Conflict => "conflict",
        }
    }

    /// Whether another attempt inside the same budget could succeed.
    fn is_retryable(self) -> bool {
        matches!(self, RelayClientError::Unavailable { .. })
    }
}

/// A client bound to one relay's internal service.
#[derive(Clone)]
pub struct AuditRelayClient {
    http: reqwest::Client,
    base_url: String,
    write_token: SecretString,
    read_token: SecretString,
    start_timeout: Duration,
    completion_timeout: Duration,
    metrics: RelayClientMetrics,
}

// Hand-written so neither credential can reach a log through a `{:?}` on the
// client, the middleware, or the state that holds them.
impl std::fmt::Debug for AuditRelayClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditRelayClient")
            .field("base_url", &self.base_url)
            .field("write_token", &"<redacted>")
            .field("read_token", &"<redacted>")
            .field("start_timeout", &self.start_timeout)
            .field("completion_timeout", &self.completion_timeout)
            .finish()
    }
}

impl AuditRelayClient {
    /// Build a client from resolved configuration.
    pub fn from_config(
        config: &AuditDeliveryConfig,
        metrics: RelayClientMetrics,
    ) -> Result<Self, AppError> {
        let base_url = config.relay_url.clone().ok_or_else(|| {
            AppError::Config(
                "FKST_AUDIT_RELAY_URL must be set to build the relay client".to_string(),
            )
        })?;
        // Only the CONNECT timeout lives on the client; each call applies its own
        // budget, so the start's tight deadline cannot govern a scoped read.
        let http = reqwest::Client::builder()
            .user_agent("fkst-hosted-api")
            .connect_timeout(Duration::from_millis(config.start_timeout_ms))
            .build()
            .map_err(|e| {
                AppError::Config(format!("failed to build the audit relay client: {e}"))
            })?;
        Ok(Self {
            http,
            base_url,
            write_token: config.write_token.clone(),
            read_token: config.read_token.clone(),
            start_timeout: Duration::from_millis(config.start_timeout_ms),
            completion_timeout: Duration::from_millis(config.completion_timeout_ms),
            metrics,
        })
    }

    /// Register a request start and wait for the durable acknowledgement.
    pub async fn register_start(
        &self,
        start: &RequestStartV1,
    ) -> Result<DurableAck, RelayClientError> {
        let url = format!("{}{REQUEST_STARTS_PATH}", self.base_url);
        self.post_with_budget(RelayPhase::Start, &url, start, self.start_timeout)
            .await
    }

    /// Commit a terminal request event and wait for the durable acknowledgement.
    pub async fn complete(
        &self,
        completion: &RequestCompletionV1,
    ) -> Result<DurableAck, RelayClientError> {
        let url = format!(
            "{}/internal/v1/audit/requests/{}/completion",
            self.base_url, completion.event_id
        );
        self.send_with_budget(
            RelayPhase::Completion,
            reqwest::Method::PUT,
            &url,
            completion,
            self.completion_timeout,
        )
        .await
    }

    /// Commit one sandbox lifecycle transition.
    pub async fn submit_lifecycle(
        &self,
        event: &LifecycleEventV1,
    ) -> Result<DurableAck, RelayClientError> {
        let url = format!("{}{EVENTS_PATH}", self.base_url);
        self.post_with_budget(RelayPhase::Lifecycle, &url, event, self.completion_timeout)
            .await
    }

    /// Run one scoped read. Uses the READ token — never the write token.
    pub async fn read_records(
        &self,
        query: &RecordsQueryV1,
        timeout: Duration,
    ) -> Result<RecordsPageV1, RelayClientError> {
        let url = format!("{}/internal/v1/audit/records", self.base_url);
        let started = Instant::now();
        let outcome = async {
            let response = self
                .http
                .get(&url)
                .timeout(timeout)
                .bearer_auth(self.read_token.expose_secret())
                .query(query)
                .send()
                .await
                .map_err(|error| RelayClientError::Unavailable {
                    kind: transport_kind(&error),
                })?;
            decode(response).await
        }
        .await;
        self.observe(RelayPhase::Read, &outcome, started.elapsed());
        outcome
    }

    /// POST a body under one budget, retrying idempotently inside it.
    async fn post_with_budget<B: serde::Serialize>(
        &self,
        phase: RelayPhase,
        url: &str,
        body: &B,
        budget: Duration,
    ) -> Result<DurableAck, RelayClientError> {
        self.send_with_budget(phase, reqwest::Method::POST, url, body, budget)
            .await
    }

    /// Send a body under one budget, retrying idempotently inside it.
    ///
    /// The retry loop is bounded by wall time rather than by an attempt count:
    /// the caller's promise is "this response is held for at most `budget`", and
    /// a count-based loop cannot honour that when each attempt is slow.
    async fn send_with_budget<B: serde::Serialize>(
        &self,
        phase: RelayPhase,
        method: reqwest::Method,
        url: &str,
        body: &B,
        budget: Duration,
    ) -> Result<DurableAck, RelayClientError> {
        let started = Instant::now();
        let mut attempt = 0u32;
        let outcome = loop {
            let remaining = budget.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break Err(RelayClientError::Unavailable { kind: "budget" });
            }
            let result = self.send_once(method.clone(), url, body, remaining).await;
            match result {
                Ok(ack) => break Ok(ack),
                Err(error) if error.is_retryable() => {
                    attempt += 1;
                    // A short, fixed pause. Anything adaptive would spend the
                    // budget the response is waiting on.
                    let pause = Duration::from_millis(25u64.saturating_mul(u64::from(attempt)));
                    if started.elapsed() + pause >= budget {
                        break Err(error);
                    }
                    tokio::time::sleep(pause).await;
                }
                Err(error) => break Err(error),
            }
        };
        self.observe(phase, &outcome, started.elapsed());
        outcome
    }

    /// One HTTP attempt.
    async fn send_once<B: serde::Serialize>(
        &self,
        method: reqwest::Method,
        url: &str,
        body: &B,
        timeout: Duration,
    ) -> Result<DurableAck, RelayClientError> {
        let response = self
            .http
            .request(method, url)
            .timeout(timeout)
            .bearer_auth(self.write_token.expose_secret())
            .json(body)
            .send()
            .await
            .map_err(|error| RelayClientError::Unavailable {
                kind: transport_kind(&error),
            })?;
        decode(response).await
    }

    /// Count one call and its duration.
    fn observe<T>(
        &self,
        phase: RelayPhase,
        outcome: &Result<T, RelayClientError>,
        elapsed: Duration,
    ) {
        let result = match outcome {
            Ok(_) => RelayCallResult::Ack,
            Err(error) => error.call_result(),
        };
        self.metrics.record_call(phase, result, elapsed);
        if let Err(error) = outcome {
            tracing::warn!(
                phase = phase.as_str(),
                reason = error.kind(),
                "audit relay call did not acknowledge"
            );
        }
    }
}

/// Decode a relay response, mapping its status onto the bounded error set.
async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, RelayClientError> {
    let status = response.status();
    if status.is_success() {
        let bytes = read_bounded(response).await?;
        return serde_json::from_slice(&bytes).map_err(|_| RelayClientError::Rejected {
            kind: "malformed_response",
        });
    }
    // The BODY is read only to distinguish a conflict from any other refusal;
    // its `message` is never logged or propagated.
    let code = conflict_code(response).await;
    match status.as_u16() {
        409 if code => Err(RelayClientError::Conflict),
        409 => Err(RelayClientError::Rejected { kind: "no_start" }),
        400 | 413 | 422 => Err(RelayClientError::Rejected { kind: "invalid" }),
        401 | 403 => Err(RelayClientError::Rejected { kind: "auth" }),
        429 => Err(RelayClientError::Unavailable { kind: "busy" }),
        // Includes the relay's own `503 relay_unavailable` / `relay_at_capacity`:
        // both mean "could not commit", which is retryable inside the budget.
        code if (500..600).contains(&code) => {
            Err(RelayClientError::Unavailable { kind: "upstream" })
        }
        _ => Err(RelayClientError::Rejected { kind: "unexpected" }),
    }
}

/// Whether an error body carries the relay's `event_id_conflict` code.
async fn conflict_code(response: reqwest::Response) -> bool {
    let Ok(body) = response.json::<Value>().await else {
        return false;
    };
    body.get("error").and_then(Value::as_str)
        == Some(crate::audit_relay::protocol::EVENT_ID_CONFLICT)
}

/// Read a response body incrementally, refusing anything past the cap.
async fn read_bounded(response: reqwest::Response) -> Result<Vec<u8>, RelayClientError> {
    let mut response = response;
    let mut body = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| RelayClientError::Unavailable {
                kind: transport_kind(&error),
            })?;
        let Some(chunk) = chunk else { break };
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(RelayClientError::Rejected {
                kind: "oversized_response",
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// A URL-free, credential-free category for a transport failure.
fn transport_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_body() || error.is_decode() {
        "body"
    } else {
        "transport"
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
