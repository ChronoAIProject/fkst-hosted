//! The PostHog query client and the activity source built on it.
//!
//! `POST {host}/api/projects/{project_id}/query/` with `kind: HogQLQuery` — the
//! documented public query API, for the same reason the capture side uses the
//! public capture API: it is the only contract a self-hosted deployment can rely
//! on across upgrades.
//!
//! ## Secret hygiene
//!
//! The read key rides only the `Authorization` header, is held in a
//! [`SecretString`], and is rendered `<redacted>` by the hand-written `Debug`.
//! Nothing here logs a header, a request body (which contains the caller's
//! filters), a response body (which contains audit rows), an upstream error
//! string, or a cursor. Failures carry a numeric status and a `&'static str`
//! category, both structurally incapable of carrying a URL or a credential.
//!
//! ## Bounded everywhere
//!
//! Connect and per-request timeouts come from configuration; the response body is
//! read incrementally against a hard cap, so a source that answers with an
//! unbounded stream costs one bounded allocation and a `502` rather than the
//! process's memory.

use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;

use crate::error::AppError;

use super::hogql;
use super::record::{ActivitySourceKind, DeliveryState};
use super::rows::{self, RowView};
use super::source::{ActivitySource, SourceError, SourcePage, SourceQuery};

/// Hard cap on a query response body. A page is at most a few hundred bounded
/// records; anything past this is a misrouted response or a hostile source.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// The decoded shape of a PostHog query response. Every other field the API
/// returns (`hogql`, `types`, `timings`, `hasMore`, …) is deliberately ignored.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct QueryResponse {
    pub columns: Vec<String>,
    pub results: Vec<Vec<Value>>,
}

/// A client bound to one self-hosted PostHog project's query endpoint.
#[derive(Clone)]
pub struct PosthogQueryClient {
    http: reqwest::Client,
    query_url: String,
    api_key: SecretString,
    timeout: Duration,
}

// Hand-written so the read key can never reach a log through a `{:?}` on the
// client, the source, or the state that holds them.
impl std::fmt::Debug for PosthogQueryClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PosthogQueryClient")
            .field("query_url", &self.query_url)
            .field("api_key", &"<redacted>")
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl PosthogQueryClient {
    /// Build a client for an already-resolved query URL.
    pub fn new(
        query_url: String,
        api_key: SecretString,
        timeout: Duration,
    ) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .user_agent("fkst-hosted-api")
            .connect_timeout(timeout)
            .build()
            .map_err(|e| {
                AppError::Config(format!("failed to build the activity query client: {e}"))
            })?;
        Ok(Self {
            http,
            query_url,
            api_key,
            timeout,
        })
    }

    /// Execute one fixed query and decode its column/result envelope.
    pub async fn query(&self, body: &Value) -> Result<QueryResponse, SourceError> {
        let response = self
            .http
            .post(&self.query_url)
            .timeout(self.timeout)
            .bearer_auth(self.api_key.expose_secret())
            .json(body)
            .send()
            .await
            .map_err(|error| {
                let kind = transport_kind(&error);
                tracing::warn!(kind, "operations: activity query transport failure");
                SourceError::Transient { kind }
            })?;

        let status = response.status();
        if !status.is_success() {
            let error = classify_status(status.as_u16());
            tracing::warn!(
                status = status.as_u16(),
                kind = error.kind(),
                "operations: activity query rejected"
            );
            return Err(error);
        }

        let bytes = read_bounded(response).await?;
        let payload: Value = serde_json::from_slice(&bytes).map_err(|_| {
            tracing::warn!("operations: activity query response was not JSON");
            SourceError::Upstream {
                kind: "malformed_response",
            }
        })?;
        decode_envelope(&payload)
    }
}

/// Read a response body incrementally, refusing anything past the cap.
async fn read_bounded(response: reqwest::Response) -> Result<Vec<u8>, SourceError> {
    let mut response = response;
    let mut body = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|error| {
            let kind = transport_kind(&error);
            tracing::warn!(kind, "operations: activity query body failed");
            SourceError::Transient { kind }
        })?;
        let Some(chunk) = chunk else { break };
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            tracing::warn!(
                limit = MAX_RESPONSE_BYTES,
                "operations: activity query response exceeded the body limit"
            );
            return Err(SourceError::Upstream {
                kind: "oversized_response",
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Decode the `{columns, results}` envelope. A response missing either is a
/// schema failure, not an empty page: answering "no rows" for a shape this build
/// cannot read would be exactly the confident-empty-result the epic forbids.
fn decode_envelope(payload: &Value) -> Result<QueryResponse, SourceError> {
    let schema_error = || SourceError::Upstream { kind: "schema" };
    let columns = payload
        .get("columns")
        .and_then(Value::as_array)
        .ok_or_else(schema_error)?
        .iter()
        .map(|column| column.as_str().map(str::to_string).ok_or_else(schema_error))
        .collect::<Result<Vec<_>, _>>()?;
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(schema_error)?
        .iter()
        .map(|row| {
            row.as_array()
                .cloned()
                .ok_or(SourceError::Upstream { kind: "schema" })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(QueryResponse { columns, results })
}

/// Map an upstream status onto the `502` / `503` split the endpoint documents.
fn classify_status(status: u16) -> SourceError {
    match status {
        401 | 403 => SourceError::Upstream { kind: "auth" },
        400 | 404 | 405 | 422 => SourceError::Upstream { kind: "schema" },
        408 | 429 => SourceError::Transient {
            kind: "upstream_busy",
        },
        code if (500..600).contains(&code) => SourceError::Transient {
            kind: "upstream_error",
        },
        _ => SourceError::Upstream { kind: "unexpected" },
    }
}

/// A URL-free, credential-free category for a transport failure. Returning a
/// `&'static str` makes leaking the request URL structurally impossible.
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

/// The PostHog implementation of the activity-source boundary.
#[derive(Debug)]
pub struct PosthogActivitySource {
    client: PosthogQueryClient,
}

impl PosthogActivitySource {
    pub fn new(client: PosthogQueryClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ActivitySource for PosthogActivitySource {
    fn kind(&self) -> ActivitySourceKind {
        ActivitySourceKind::Posthog
    }

    async fn fetch(&self, query: &SourceQuery) -> Result<SourcePage, SourceError> {
        // The predicate is built INTO the query text here, before the source's own
        // LIMIT — never applied to a fetched page (see `super::hogql`).
        let built = hogql::build(query);
        let response = self.client.query(&built.request_body()).await?;
        let index = rows::column_index(&response.columns);
        let mut page = SourcePage {
            records: Vec::with_capacity(response.results.len()),
            raw_rows: response.results.len(),
            row_errors: 0,
        };
        for values in &response.results {
            let view = RowView::new(&index, values);
            // A row that cannot be decoded is dropped HERE with its bounded
            // reason; the caller counts it and marks the page partial. Failing the
            // whole page would let one malformed record hide every well-formed one.
            match rows::decode(
                &view,
                ActivitySourceKind::Posthog,
                DeliveryState::VerifiedInPosthog,
            ) {
                Ok(record) => page.records.push(record),
                Err(error) => {
                    page.row_errors += 1;
                    tracing::warn!(
                        source = ActivitySourceKind::Posthog.as_str(),
                        reason = %error,
                        "operations: dropping an undecodable activity row"
                    );
                }
            }
        }
        Ok(page)
    }
}

#[cfg(test)]
#[path = "posthog_tests.rs"]
mod tests;
