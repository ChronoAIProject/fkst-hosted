//! Typed response markers the audit middleware reads instead of the body.
//!
//! A terminal record needs two things the HTTP status alone cannot express: the
//! *stable* application error code, and whether the answer was a pre-handler
//! policy rejection. Both travel as response extensions.
//!
//! ## Why an extension and not the JSON body
//!
//! Reading the code out of the body would mean buffering it. Log downloads,
//! outcome blobs, and the chat SSE stream are deliberately streamed, so
//! buffering would either break them or hold whole bundles in memory — to
//! recover a string the producer already had in hand. Extensions cost nothing,
//! survive every Tower layer between the handler and the middleware, and cannot
//! be forged by a client.
//!
//! ## Why the code is `&'static str`
//!
//! Only a compile-time constant can be attached, so a code is bounded by
//! construction: no error message, no formatted value, and nothing derived from
//! request data can ever end up in this field. That is the whole redaction
//! argument for `error_code` (epic `AUD-03`), enforced by the type system rather
//! than by review.

use axum::response::Response;

/// The stable machine-readable error code for a response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditErrorCode(pub &'static str);

/// Marks a response as produced by a pre-handler policy short-circuit —
/// authentication, authorization, or the leader-readiness gate — so the record
/// says `rejected` with the real status rather than a generic client error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditRejection;

/// Bounded stable codes for the responses that are NOT built from
/// [`crate::error::AppError`].
///
/// `AppError` carries its own codes (`invalid_request`, `not_found`, …); this
/// module is only for the hand-built responses the spec names: the browser OAuth
/// HTML pages, the leader gate, the webhook's signature rejection, and axum's
/// own routing/timeout answers.
pub mod codes {
    /// The route-scoped `TimeoutLayer` answered before the handler returned.
    pub const REQUEST_TIMEOUT: &str = "request_timeout";
    /// This election-enabled replica is not the resync-complete leader.
    pub const LEADER_NOT_READY: &str = "leader_not_ready";
    /// No route matched the request path.
    pub const ROUTE_NOT_FOUND: &str = "route_not_found";
    /// The path matched but no handler serves that method.
    pub const METHOD_NOT_ALLOWED: &str = "method_not_allowed";
    /// The webhook's `X-Hub-Signature-256` was missing or did not verify.
    pub const WEBHOOK_SIGNATURE_INVALID: &str = "webhook_signature_invalid";
    /// The webhook route ran without a configured secret (defensive).
    pub const WEBHOOK_NOT_CONFIGURED: &str = "webhook_not_configured";
    /// A browser OAuth page rejected the request (missing/tampered state, …).
    pub const OAUTH_INVALID_REQUEST: &str = "oauth_invalid_request";
    /// A browser OAuth page could not verify the caller.
    pub const OAUTH_UNAUTHORIZED: &str = "oauth_unauthorized";
    /// A browser page denied an authenticated caller.
    pub const OAUTH_FORBIDDEN: &str = "oauth_forbidden";
    /// A browser page found no such resource.
    pub const OAUTH_NOT_FOUND: &str = "oauth_not_found";
    /// A browser page could not reach a dependency, or the feature is off.
    pub const OAUTH_UNAVAILABLE: &str = "oauth_unavailable";
    /// A browser page's upstream dependency failed.
    pub const OAUTH_UPSTREAM: &str = "oauth_upstream";

    /// The code for a hand-built browser (HTML) response of `status`.
    ///
    /// The browser paths render HTML rather than the JSON envelope, so they have
    /// no `error` field to carry a code; this maps their status onto the same
    /// bounded vocabulary so an operator can correlate the two surfaces.
    pub fn for_browser_status(status: axum::http::StatusCode) -> &'static str {
        match status {
            axum::http::StatusCode::UNAUTHORIZED => OAUTH_UNAUTHORIZED,
            axum::http::StatusCode::FORBIDDEN => OAUTH_FORBIDDEN,
            axum::http::StatusCode::NOT_FOUND => OAUTH_NOT_FOUND,
            axum::http::StatusCode::SERVICE_UNAVAILABLE => OAUTH_UNAVAILABLE,
            axum::http::StatusCode::BAD_GATEWAY => OAUTH_UPSTREAM,
            _ => OAUTH_INVALID_REQUEST,
        }
    }
}

/// Attach a stable error code to an already-built response.
pub fn tag_error_code(response: &mut Response, code: &'static str) {
    response.extensions_mut().insert(AuditErrorCode(code));
}

/// Mark an already-built response as a pre-handler policy rejection.
pub fn tag_rejected(response: &mut Response) {
    response.extensions_mut().insert(AuditRejection);
}

/// Attach a stable error code, taking and returning the response so it composes
/// with the `…into_response()` style used across the route modules.
pub fn with_error_code(mut response: Response, code: &'static str) -> Response {
    tag_error_code(&mut response, code);
    response
}

/// Attach a stable error code AND the rejection marker.
pub fn with_rejection(mut response: Response, code: &'static str) -> Response {
    tag_error_code(&mut response, code);
    tag_rejected(&mut response);
    response
}

/// The stable code attached to a response, if any.
pub fn error_code_of(response: &Response) -> Option<&'static str> {
    response
        .extensions()
        .get::<AuditErrorCode>()
        .map(|code| code.0)
}

/// Whether a response was marked as a pre-handler policy rejection.
pub fn is_rejected(response: &Response) -> bool {
    response.extensions().get::<AuditRejection>().is_some()
}

#[cfg(test)]
#[path = "response_tests.rs"]
mod tests;
