//! In-process, GET-only dispatch of the concierge's data reads through this
//! deployment's own router.
//!
//! This is the whole security model of the chat feature, expressed as one type.
//! Every question the concierge answers about live data becomes a real
//! `GET /api/v1/...` request carrying the **calling user's own bearer token**,
//! executed by the **real router**. So chat inherits, with zero duplicated logic:
//!
//! * the `GithubUser` extractor — token verification plus the deployment access
//!   policy ([`crate::github_identity`]);
//! * the log/observe three-tier authorization ([`crate::routes::logs`]);
//! * canvas visibility scoping (`resolve_visible_repo`);
//! * the leader-readiness gate on the `/api/v1` nest ([`crate::router`]).
//!
//! A user therefore can never see through chat anything the dashboard would refuse
//! them, and no authorization rule has to be restated here to stay in sync.
//!
//! Only `GET` is exposed — there is no method parameter and no other request verb
//! on the type, so "chat performs a write" is not a bug that can be introduced by
//! passing the wrong argument. Mutations are confirm-gated and executed by the SPA.
//!
//! Accepted cost: each dispatch re-runs the `GithubUser` extractor, i.e. one
//! `GET {github_api_base_url}/user` per tool call. That is correct-by-construction
//! (the identity is verified exactly as it would be for a browser request);
//! caching identity verification is deliberately out of scope.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use secrecy::{ExposeSecret, SecretString};
use tower::ServiceExt;

// The header name is taken from the endpoint that READS it, never restated here:
// a second copy could drift and silently narrow what the concierge sees.
use crate::routes::canvas::BROADER_TOKEN_HEADER;
use crate::state::SelfRouter;

/// Hard cap on one dispatched response body. Tool results are re-sent to the model
/// on every subsequent iteration of a turn, so an unbounded log tail would blow the
/// context window (and the bill) rather than merely being large.
pub const MAX_TOOL_RESULT_BYTES: usize = 48 * 1024;

/// Wall-clock budget for ONE dispatched read.
///
/// Comfortably under the default 120s whole-turn deadline, and deliberately so: some
/// read endpoints scale with how much history a repository has accumulated (the canvas
/// session list assembles every trigger issue the repository ever had), and a single one
/// of those can otherwise eat a turn's entire budget. When that happens the user gets
/// `deadline_exceeded` — a dead end that says nothing about what went wrong.
///
/// With this bound the slow call comes back as a 504 RESULT instead, leaving the model
/// most of the turn to say plainly that the lookup timed out and suggest the dashboard.
/// A truthful partial answer beats an opaque failure.
pub const DISPATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// A dispatch failure. Both variants are process-level faults, NOT the "the API
/// said 403" case — an HTTP error status is a successful dispatch whose status is
/// data the model must see and explain.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    /// The router handle was never populated: something dispatched before
    /// `build_router` ran. A startup-order bug, not a user error.
    #[error("the self-router handle is not populated")]
    RouterUnset,
    /// The request could not be constructed (an unparseable URI from a tool).
    #[error("could not build the dispatched request: {0}")]
    BadRequest(String),
    /// The router itself failed, or its body could not be collected.
    #[error("dispatched request failed: {0}")]
    Transport(String),
}

/// One dispatched response, reduced to what a tool result needs.
#[derive(Debug, Clone)]
pub struct DispatchResponse {
    /// The HTTP status. Carried through verbatim — a 403 is an ANSWER ("you do not
    /// have access to that session"), not a failure to report as an outage.
    pub status: u16,
    pub body: serde_json::Value,
    /// Whether the body was cut at [`MAX_TOOL_RESULT_BYTES`].
    pub truncated: bool,
}

/// A GET-only client over this process's own router.
#[derive(Clone)]
pub struct SelfDispatch {
    router: SelfRouter,
}

impl SelfDispatch {
    pub fn new(router: SelfRouter) -> Self {
        Self { router }
    }

    /// Issue `GET {path_and_query}` as the user identified by `bearer`.
    ///
    /// `path_and_query` must already be percent-encoded by the caller (the tools
    /// layer does this per dynamic segment). `broader`, when present, is forwarded
    /// as [`BROADER_TOKEN_HEADER`] so overview enumeration matches the dashboard's.
    pub async fn get(
        &self,
        path_and_query: &str,
        bearer: &SecretString,
        broader: Option<&SecretString>,
    ) -> Result<DispatchResponse, DispatchError> {
        let router = self.router.get().cloned().ok_or_else(|| {
            tracing::error!(
                "chat dispatch attempted before build_router populated the self-router handle"
            );
            DispatchError::RouterUnset
        })?;

        let mut builder = Request::get(path_and_query).header(
            header::AUTHORIZATION,
            format!("Bearer {}", bearer.expose_secret()),
        );
        if let Some(token) = broader {
            builder = builder.header(BROADER_TOKEN_HEADER, token.expose_secret());
        }
        let request = builder
            .body(Body::empty())
            .map_err(|e| DispatchError::BadRequest(e.to_string()))?;

        // Bounded, and a timeout is a RESULT rather than an error: the model can explain
        // "that lookup timed out" and move on, where a `DispatchError` would surface as a
        // bare tool failure with no actionable content. See `DISPATCH_TIMEOUT`.
        let response = match tokio::time::timeout(DISPATCH_TIMEOUT, router.oneshot(request)).await {
            Ok(result) => result.map_err(|e| DispatchError::Transport(e.to_string()))?,
            Err(_) => {
                tracing::warn!(
                    path = %redact_query(path_and_query),
                    timeout_secs = DISPATCH_TIMEOUT.as_secs(),
                    "chat dispatch timed out"
                );
                return Ok(DispatchResponse {
                    status: StatusCode::GATEWAY_TIMEOUT.as_u16(),
                    body: serde_json::json!({
                        "error": "dispatch_timeout",
                        "message": format!(
                            "this lookup did not finish within {}s; it may be unusually large",
                            DISPATCH_TIMEOUT.as_secs()
                        ),
                    }),
                    truncated: false,
                });
            }
        };
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|e| DispatchError::Transport(e.to_string()))?
            .to_bytes();

        let (body, truncated) = decode_body(&bytes);
        // Never the bearer, never the body — only shape.
        tracing::debug!(
            path = %redact_query(path_and_query),
            status = status.as_u16(),
            bytes = bytes.len(),
            truncated,
            "chat dispatch completed"
        );
        Ok(DispatchResponse {
            status: status.as_u16(),
            body,
            truncated,
        })
    }
}

/// Interpret a dispatched body as JSON, truncating oversized payloads.
///
/// A truncated payload is almost never valid JSON any more, so it is wrapped as
/// `{"truncated_text": ...}` — the model then sees plainly that it holds a
/// fragment, instead of being handed something that looks complete. Complete
/// non-JSON bodies (which this read surface should not produce) become
/// `{"text": ...}` rather than an error, because the status is still useful.
fn decode_body(bytes: &[u8]) -> (serde_json::Value, bool) {
    if bytes.len() > MAX_TOOL_RESULT_BYTES {
        let text = truncate_utf8(bytes, MAX_TOOL_RESULT_BYTES);
        let value = match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => value,
            Err(_) => serde_json::json!({ "truncated_text": text }),
        };
        return (value, true);
    }
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(value) => (value, false),
        Err(_) => (
            serde_json::json!({ "text": String::from_utf8_lossy(bytes) }),
            false,
        ),
    }
}

/// Decode at most `max_bytes`, cut back to the last complete UTF-8 character.
fn truncate_utf8(bytes: &[u8], max_bytes: usize) -> String {
    let mut end = max_bytes.min(bytes.len());
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Drop the query string before logging a dispatched path.
///
/// Query values are model-chosen arguments (a log file path, a search term); the
/// path alone identifies the operation, which is all a debug line needs.
fn redact_query(path_and_query: &str) -> &str {
    match path_and_query.split_once('?') {
        Some((path, _)) => path,
        None => path_and_query,
    }
}

/// Whether a dispatched status means the request succeeded.
///
/// Shared with the tools layer so "did this produce usable data?" is decided in one
/// place: a 200 is data, everything else is an explanation for the model.
pub fn is_success(status: u16) -> bool {
    StatusCode::from_u16(status)
        .map(|s| s.is_success())
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
