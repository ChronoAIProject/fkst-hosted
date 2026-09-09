//! `X-Request-Id` acceptance, generation, and propagation.
//!
//! A request id is client-supplied, and it is echoed into a response header, a
//! structured log line, and a durable audit record. Each of those is a place a
//! hostile value could do damage — header smuggling, log-line forgery, an
//! unbounded analytics string — so the incoming value is *accepted*, never
//! merely trusted: it must be short and drawn from a documented ASCII set, or it
//! is replaced outright.
//!
//! Replacement is silent by design. A caller that sends a malformed id gets a
//! working request with a server-generated id rather than a `400`: correlation
//! is an operational nicety, not part of any API contract, and failing a product
//! request over it would be a self-inflicted outage.
//!
//! The request id is deliberately NOT the audit `event_id`. Clients reuse,
//! forge, and collide on request ids; delivery deduplication needs an identifier
//! this service derives itself (see [`crate::audit::event::derive_event_id`]).

use uuid::Uuid;

/// The propagated header name, lowercase (HTTP/2 requires lowercase, and axum's
/// `HeaderName` comparison is case-insensitive either way).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Longest accepted client value. Matches
/// [`crate::audit::validate::limits::REQUEST_ID`] so an accepted id can never be
/// rejected later by the event contract.
pub const MAX_REQUEST_ID_LEN: usize = crate::audit::validate::limits::REQUEST_ID;

/// The documented safe set: ASCII alphanumerics plus `-`, `_`, `.`, and `:`.
///
/// Wide enough for every id format in practical use (UUIDs, W3C trace ids,
/// Envoy/GCP request ids), narrow enough that a value can never contain a
/// separator that would let it forge a field in a structured log, a Prometheus
/// exposition line, or an HTTP header.
fn is_safe_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

/// Whether a client-supplied value may be propagated as-is.
pub fn is_acceptable(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_REQUEST_ID_LEN && value.chars().all(is_safe_char)
}

/// The normalized id plus whether it came from the client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedRequestId {
    /// The value propagated to the response and recorded on the event.
    pub value: String,
    /// `true` when the inbound header was missing or unacceptable and this
    /// service generated the value instead.
    pub generated: bool,
}

/// Accept `raw` when it is well-formed, otherwise generate a fresh UUID.
pub fn normalize_request_id(raw: Option<&str>) -> NormalizedRequestId {
    match raw {
        Some(value) if is_acceptable(value) => NormalizedRequestId {
            value: value.to_string(),
            generated: false,
        },
        _ => NormalizedRequestId {
            value: generate_request_id(),
            generated: true,
        },
    }
}

/// A fresh random request id (UUIDv4, hyphenated lowercase).
pub fn generate_request_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
#[path = "id_tests.rs"]
mod tests;
