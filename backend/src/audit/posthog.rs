//! PostHog capture transport.
//!
//! Deliberately the **public** capture API (`POST {host}/capture/` and
//! `POST {host}/batch/`), never an internal Kafka/ClickHouse path: the public
//! contract is the only one a self-hosted deployment can rely on across
//! upgrades, and it is the one that honours event `uuid` deduplication.
//!
//! A `2xx` here means **accepted by capture** — PostHog has taken the payload,
//! not proven it query-visible. The naming is enforced throughout
//! ([`DeliveryResult::Accepted`]); calling it "delivered" would paper over the
//! ingestion lag that the read side has to reason about (epic `AUD-07`).
//!
//! Secret hygiene: the project token rides only the JSON body's `api_key` field
//! and is held in a [`SecretString`]. Nothing in this module logs a request body,
//! a response body, or the token — a response body may echo the input, so even a
//! debug log of it could re-emit forbidden data. Errors carry a numeric status
//! and a `&'static str` transport category, both structurally incapable of
//! carrying a URL or a credential.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};

use super::config::AuditConfig;
use super::metrics::DeliveryResult;
use super::projection::CaptureEvent;
use crate::error::AppError;

/// PostHog's success marker in the capture/batch response body.
const OK_STATUS_NUMERIC: u64 = 1;
const OK_STATUS_TEXT: &str = "Ok";

/// Why a capture attempt failed.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CaptureError {
    /// The request never produced a response (connect/timeout/body error). The
    /// category is a fixed string, so no URL can leak through it.
    #[error("posthog capture transport failure ({kind})")]
    Transport { kind: &'static str },
    /// A status PostHog may recover from: `408`, `429`, or `5xx`.
    #[error("posthog capture returned retryable status {status}")]
    RetryableStatus {
        status: u16,
        /// A numeric `Retry-After`, when the server sent one.
        retry_after: Option<Duration>,
    },
    /// A payload/authentication/configuration failure. Retrying it would loop
    /// forever against the same rejection.
    #[error("posthog capture returned permanent status {status}")]
    PermanentStatus { status: u16 },
    /// A `2xx` whose body is not PostHog's success envelope — typically a
    /// misrouted proxy response or a rejected project token. Treated as
    /// permanent: it is a configuration error, not a transient blip.
    #[error("posthog capture returned an unusable response body")]
    InvalidResponse,
}

impl CaptureError {
    /// Whether another attempt could plausibly succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            CaptureError::Transport { .. } | CaptureError::RetryableStatus { .. }
        )
    }

    /// The server-requested delay, if any. The caller still caps it.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            CaptureError::RetryableStatus { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// The bounded metric label for this failure.
    pub fn delivery_result(&self) -> DeliveryResult {
        if self.is_retryable() {
            DeliveryResult::Retryable
        } else {
            DeliveryResult::Permanent
        }
    }
}

/// A client bound to one self-hosted PostHog project.
#[derive(Clone)]
pub struct PostHogClient {
    http: reqwest::Client,
    capture_url: String,
    batch_url: String,
    project_token: SecretString,
    timeout: Duration,
}

// Hand-written `Debug` so the project token can never reach a log through a
// `{:?}` on the client, the worker, or the state that holds them.
impl std::fmt::Debug for PostHogClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostHogClient")
            .field("capture_url", &self.capture_url)
            .field("batch_url", &self.batch_url)
            .field("project_token", &"<redacted>")
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl PostHogClient {
    /// Build a client from resolved configuration.
    ///
    /// Fails only on a genuinely unusable configuration (no host, or an HTTP
    /// client that cannot be constructed) — both startup errors, never runtime
    /// ones.
    pub fn from_config(config: &AuditConfig) -> Result<Self, AppError> {
        let (capture_url, batch_url) = match (config.capture_url(), config.batch_url()) {
            (Some(capture), Some(batch)) => (capture, batch),
            _ => {
                return Err(AppError::Config(
                    "FKST_POSTHOG_HOST must be set to build the audit capture client".to_string(),
                ))
            }
        };
        // Only the CONNECT timeout lives on the client; the per-request budget is
        // applied per call so a future long-running endpoint on the same client
        // is not silently governed by this one.
        let http = reqwest::Client::builder()
            .user_agent("fkst-hosted-api")
            .connect_timeout(Duration::from_millis(config.capture_timeout_ms))
            .build()
            .map_err(|e| AppError::Config(format!("failed to build the audit http client: {e}")))?;
        Ok(Self {
            http,
            capture_url,
            batch_url,
            project_token: config.project_token.clone(),
            timeout: Duration::from_millis(config.capture_timeout_ms),
        })
    }

    /// Send one batch. A single event uses `/capture/`, more use `/batch/`;
    /// both carry stable per-event `uuid`s so a retry deduplicates server-side.
    pub async fn capture(&self, events: &[CaptureEvent]) -> Result<(), CaptureError> {
        let Some((url, body)) = self.payload(events) else {
            return Ok(());
        };
        let response = self
            .http
            .post(url)
            .timeout(self.timeout)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                let kind = transport_kind(&e);
                tracing::warn!(kind, "audit capture transport failure");
                CaptureError::Transport { kind }
            })?;

        let status = response.status();
        if !status.is_success() {
            let error = classify_status(&response);
            tracing::warn!(
                status = status.as_u16(),
                retryable = error.is_retryable(),
                events = events.len(),
                "audit capture rejected"
            );
            return Err(error);
        }
        // A 2xx is not enough: a reverse proxy or a token rejection can answer
        // 200 with something that is not PostHog's success envelope.
        let body: Value = response.json().await.map_err(|_| {
            tracing::warn!("audit capture response was not JSON");
            CaptureError::InvalidResponse
        })?;
        if !is_success_envelope(&body) {
            tracing::warn!("audit capture response was not a posthog success envelope");
            return Err(CaptureError::InvalidResponse);
        }
        Ok(())
    }

    /// The target URL and JSON body for `events`, or `None` when there is
    /// nothing to send.
    fn payload(&self, events: &[CaptureEvent]) -> Option<(&str, Value)> {
        match events {
            [] => None,
            [single] => Some((
                self.capture_url.as_str(),
                json!({
                    "api_key": self.project_token.expose_secret(),
                    "event": single.event,
                    "distinct_id": single.distinct_id,
                    "uuid": single.uuid,
                    "timestamp": single.timestamp,
                    "properties": single.properties,
                }),
            )),
            many => Some((
                self.batch_url.as_str(),
                json!({
                    "api_key": self.project_token.expose_secret(),
                    "batch": many,
                }),
            )),
        }
    }
}

/// Map a non-2xx response to a retryable or permanent failure.
fn classify_status(response: &reqwest::Response) -> CaptureError {
    let status = response.status();
    let code = status.as_u16();
    if code == 408 || code == 429 || status.is_server_error() {
        return CaptureError::RetryableStatus {
            status: code,
            retry_after: numeric_retry_after(response),
        };
    }
    CaptureError::PermanentStatus { status: code }
}

/// Read a NUMERIC `Retry-After` (seconds). The HTTP-date form is deliberately
/// ignored: parsing it would need a date parser on a hostile input path, and the
/// caller's capped backoff is a safe fallback.
fn numeric_retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// PostHog answers `{"status": 1}` (older builds `{"status": "Ok"}`).
fn is_success_envelope(body: &Value) -> bool {
    match body.get("status") {
        Some(Value::Number(n)) => n.as_u64() == Some(OK_STATUS_NUMERIC),
        Some(Value::String(s)) => s.eq_ignore_ascii_case(OK_STATUS_TEXT),
        _ => false,
    }
}

/// A URL-free, credential-free category for a transport failure. Returning a
/// `&'static str` makes leaking the request URL structurally impossible.
fn transport_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_request() {
        "request"
    } else if error.is_body() || error.is_decode() {
        "body"
    } else {
        "other"
    }
}

#[cfg(test)]
#[path = "posthog_tests.rs"]
mod tests;
