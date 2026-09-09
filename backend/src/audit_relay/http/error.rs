//! The relay's error envelope: stable codes, and nothing else.
//!
//! Every variant renders `{"error": "<stable code>", "message": "<fixed text>"}`.
//! The message is a compile-time constant per variant, never an upstream string,
//! a SQLite message, a path, a field value, or a token — a relay that echoed what
//! it was sent would become a way to read back exactly the content the audit
//! contract forbids storing.
//!
//! The one deliberately structured code is [`RelayError::Conflict`]
//! (`event_id_conflict`): the control plane branches on it to distinguish "this
//! id already names a different fact" from "the relay is down", and those need
//! different reactions.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use super::super::db::DbError;
use super::super::metrics::IngressResult;

/// A relay handler's result.
pub type RelayResult<T> = Result<T, RelayError>;

/// Why a relay call failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RelayError {
    /// Missing or refused bearer credentials.
    #[error("unauthorized")]
    Unauthorized,
    /// The body or query failed the protocol/audit contract.
    #[error("invalid request")]
    Invalid(&'static str),
    /// The event id exists with different immutable content.
    #[error("event id conflict")]
    Conflict,
    /// A completion arrived with no registered start.
    #[error("no registered start")]
    NoStart,
    /// The configured record ceiling was reached.
    #[error("relay at capacity")]
    Capacity,
    /// Storage could not answer.
    #[error("relay storage unavailable")]
    Unavailable,
}

impl RelayError {
    /// The stable machine-readable code.
    pub fn code(self) -> &'static str {
        match self {
            RelayError::Unauthorized => "unauthorized",
            RelayError::Invalid(_) => "invalid_request",
            RelayError::Conflict => super::super::protocol::EVENT_ID_CONFLICT,
            RelayError::NoStart => "no_registered_start",
            RelayError::Capacity => "relay_at_capacity",
            RelayError::Unavailable => "relay_unavailable",
        }
    }

    /// The HTTP status.
    pub fn status(self) -> StatusCode {
        match self {
            RelayError::Unauthorized => StatusCode::UNAUTHORIZED,
            RelayError::Invalid(_) => StatusCode::BAD_REQUEST,
            RelayError::Conflict | RelayError::NoStart => StatusCode::CONFLICT,
            RelayError::Capacity | RelayError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// The bounded telemetry label.
    pub fn ingress_result(self) -> IngressResult {
        match self {
            RelayError::Unauthorized => IngressResult::Unauthorized,
            RelayError::Invalid(_) => IngressResult::Rejected,
            RelayError::Conflict | RelayError::NoStart => IngressResult::Conflict,
            RelayError::Capacity | RelayError::Unavailable => IngressResult::Unavailable,
        }
    }

    /// The client-safe message. A `&'static str` per variant: the field NAME a
    /// validation failure concerns is a compile-time constant, and no value ever
    /// reaches this text.
    fn message(self) -> &'static str {
        match self {
            RelayError::Unauthorized => "bearer credentials are missing or not accepted",
            RelayError::Invalid(field) => field,
            RelayError::Conflict => "this event id is already durable with different content",
            RelayError::NoStart => "no registered request start for this event id",
            RelayError::Capacity => "the relay is at its configured record capacity",
            RelayError::Unavailable => "the relay could not durably record this event",
        }
    }
}

impl From<DbError> for RelayError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::Conflict => RelayError::Conflict,
            DbError::NoStart => RelayError::NoStart,
            DbError::Capacity => RelayError::Capacity,
            // Busy and internal failures are both "we could not commit"; the
            // caller retries idempotently either way, and naming the difference
            // on the wire would tell an attacker about storage internals.
            DbError::Unavailable(_) | DbError::Busy | DbError::Internal(_) => {
                RelayError::Unavailable
            }
        }
    }
}

impl IntoResponse for RelayError {
    fn into_response(self) -> Response {
        (
            self.status(),
            Json(json!({ "error": self.code(), "message": self.message() })),
        )
            .into_response()
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
