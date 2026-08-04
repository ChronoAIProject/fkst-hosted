//! The two responses `required` delivery mode can produce, and the honest
//! difference between them.
//!
//! ```text
//! start not durable      -> 503 audit_ingress_unavailable      NOTHING happened
//! completion not durable -> 503 audit_completion_unconfirmed   something MAY have
//! ```
//!
//! ## `audit_ingress_unavailable` is a clean refusal
//!
//! The middleware returns it BEFORE invoking the inner service, so no extractor
//! ran, no handler ran, and no side effect occurred. A client may retry it
//! exactly like any other `503`.
//!
//! ## `audit_completion_unconfirmed` is the operational trade-off, stated plainly
//!
//! The handler already ran and produced a response; only the durable record of
//! its OUTCOME could not be confirmed. The deployment therefore refuses to hand
//! back a status it cannot prove it recorded — and the client is left genuinely
//! uncertain whether the side effect happened.
//!
//! That ambiguity is deliberate and is the price of the "all requests" claim. The
//! alternatives are worse: returning the handler's status would assert a durable
//! record that may not exist, and suppressing the whole thing would lose the
//! invocation. What remains true is that the START is already durable, so the
//! relay closes the record as `incomplete` after its deadline and the invocation
//! stays visible in the global scope — with `status_code = null`, because no
//! system may invent a status it did not prove.
//!
//! **Operator guidance:** a client that receives this must treat the operation as
//! *unknown*, not as failed. Idempotent operations may be retried; non-idempotent
//! ones should be reconciled against their resource before retrying. The
//! condition is alertable through `fkst_audit_required_rejections_total{reason}`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use super::response::{codes, with_error_code, with_rejection};

/// The refusal returned when a request start could not be made durable.
///
/// Tagged as a rejection because it IS one: it short-circuits before the inner
/// service, exactly like the leader gate.
pub fn ingress_unavailable() -> Response {
    with_rejection(
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": codes::AUDIT_INGRESS_UNAVAILABLE,
                "message": "this request was not started because it could not be durably recorded",
            })),
        )
            .into_response(),
        codes::AUDIT_INGRESS_UNAVAILABLE,
    )
}

/// The answer returned when the handler ran but its outcome could not be
/// confirmed durable.
///
/// NOT tagged as a rejection: the handler did run, and calling it a pre-handler
/// rejection would misreport what happened.
pub fn completion_unconfirmed() -> Response {
    with_error_code(
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": codes::AUDIT_COMPLETION_UNCONFIRMED,
                "message": "the operation was performed but its outcome could not be durably \
                            recorded; treat the result as unknown and reconcile before retrying",
            })),
        )
            .into_response(),
        codes::AUDIT_COMPLETION_UNCONFIRMED,
    )
}

#[cfg(test)]
#[path = "required_tests.rs"]
mod tests;
