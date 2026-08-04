//! Terminal outcome derivation from the response that was actually returned.
//!
//! The rule is deliberately mechanical: the outcome is a function of the real
//! status plus the two explicit markers in [`super::response`]. Nothing is
//! inferred from a body, a message, or which handler ran — a record that
//! disagreed with the status the client received would make every dashboard and
//! every scoped query lie.
//!
//! ```text
//! rejection marker            -> rejected     (with the real status)
//! 408, or a timeout marker    -> timeout
//! 2xx                         -> success
//! 3xx                         -> redirect
//! 4xx                         -> client_error
//! 5xx                         -> server_error
//! ```
//!
//! [`crate::audit::AuditOutcome::Incomplete`] is deliberately not derivable
//! here: it means "no response ever existed", which by definition cannot be
//! observed from a response. The durable relay closes those records.

use axum::http::StatusCode;

use super::response::codes;
use crate::audit::event::AuditOutcome;

/// Derive the terminal outcome.
///
/// `rejected` is the [`super::response::AuditRejection`] marker; `timed_out` is
/// set when the route's own timeout produced the answer.
pub fn derive_outcome(status: StatusCode, rejected: bool, timed_out: bool) -> AuditOutcome {
    let code = status.as_u16();
    // A timeout is checked first: the timeout layer answers with 408, which
    // would otherwise read as an ordinary client error.
    if timed_out || code == 408 {
        return AuditOutcome::Timeout;
    }
    // A policy short-circuit keeps its real status but is classified by WHY it
    // happened, so "denied before the handler ran" stays distinguishable from a
    // handler that validated its input and said no.
    //
    // The marker is honoured only on the statuses a rejection can actually carry
    // (a 4xx answer, or the leader gate's 503). A marker on any other status is a
    // call-site mistake, and trusting it would build a record the event contract
    // rejects — turning one bug into a silently missing audit row.
    if rejected && ((400..500).contains(&code) || code == 503) {
        return AuditOutcome::Rejected;
    }
    match code {
        200..=299 => AuditOutcome::Success,
        300..=399 => AuditOutcome::Redirect,
        400..=499 => AuditOutcome::ClientError,
        // 5xx, plus the informational range. A terminal 1xx is not representable
        // in axum's response model; folding it here keeps the mapping total, and
        // the event contract will reject such a record loudly (with a drop metric
        // and a log) rather than let a nonsense status through silently.
        _ => AuditOutcome::ServerError,
    }
}

/// A stable code for a framework-produced error response that could not attach
/// one itself.
///
/// axum's `404`/`405` and tower-http's timeout response are built inside library
/// code, so there is no call site to tag them. Everything else must carry its own
/// code — this function never invents one for a status a handler produced.
pub fn framework_error_code(status: StatusCode, matched_route: bool) -> Option<&'static str> {
    match status {
        StatusCode::REQUEST_TIMEOUT => Some(codes::REQUEST_TIMEOUT),
        StatusCode::NOT_FOUND if !matched_route => Some(codes::ROUTE_NOT_FOUND),
        StatusCode::METHOD_NOT_ALLOWED => Some(codes::METHOD_NOT_ALLOWED),
        _ => None,
    }
}

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod tests;
