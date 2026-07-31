//! The typed, allowlisted safe-argument contract (epic `AUD-03`).
//!
//! ```text
//! handler / audited extractor
//!   -> ToSafeAuditArguments        [this module]  project validated inputs
//!        -> Safe DTO               [auth|canvas|…] enumerate the allowed fields
//!             -> allowlist filter  [record_safe]   drop anything undocumented
//!                  -> AuditRequestContext::record_arguments
//! ```
//!
//! ## Why "serialize the request minus obvious secrets" is not on the table
//!
//! A denylist is only as good as the last person who added a field. This module
//! inverts that: a request DTO is never serialized, and every property that can
//! ever appear in a record is named — twice — in [`catalog`] and on the safe
//! DTO's own `ALLOWED_FIELDS`. A field that appears in neither is dropped by
//! [`record_safe`] at runtime and fails the coverage tests at build time, so the
//! failure mode of forgetting is a missing property, never a leaked one.
//!
//! ## Sealed, and deliberately not generic
//!
//! [`ToSafeAuditArguments`] has a private supertrait, so it can only be
//! implemented inside this module tree. There is no blanket implementation for
//! `Serialize`, and none may be added: the whole point is that a new endpoint
//! CANNOT get audit arguments by accident — someone has to write the projection,
//! and in doing so decide the boundary.
//!
//! ## The four parse states
//!
//! - `parsed` — a safe DTO was produced from validated inputs;
//! - `invalid` — a parser rejected the input; only [`InvalidInput`]'s bounded
//!   transport metadata is kept, never the bytes, the query, or the message;
//! - `not_applicable` — the operation takes no arguments at all;
//! - `unavailable` — the request was rejected before safe parsing could run
//!   (authentication, the leader gate, a route-scoped timeout).
//!
//! `unavailable` is the DEFAULT for an audited operation that declares a DTO and
//! never recorded one, and `not_applicable` the default for one that declares no
//! arguments. That default is derived from the operation's own policy in
//! [`crate::audit::request::policy`], so a rejected request is classified
//! honestly without every rejection site having to remember to say so — and
//! nothing is ever reordered to parse a secret-bearing body purely to improve
//! audit detail.

pub mod bounds;
pub mod catalog;
pub mod extract;

pub mod auth;
pub mod canvas;
pub mod canvas_write;
pub mod chat;
pub mod environments;
pub mod logs;
pub mod operations;
pub mod repos;
pub mod webhook;

use axum::http::Extensions;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::audit::event::ArgumentsParseStatus;
use crate::audit::request::context::with_context;

pub use bounds::{BoundedList, RUN_LATEST};
pub use catalog::SafeArgumentSpec;
pub use extract::{AuditedJson, AuditedPath, AuditedQuery};

/// Private supertrait: only this module tree may name it, so only this module
/// tree may implement [`ToSafeAuditArguments`].
mod sealed {
    pub trait Sealed {}
}

/// A serializable DTO that is the complete, documented audit boundary of exactly
/// one operation.
///
/// Implementors are hand-written per operation. The two constants are what make
/// the boundary checkable rather than aspirational: `OPERATION_ID` binds the DTO
/// to one `operationId` (so a second DTO on the same operation is a test
/// failure), and `ALLOWED_FIELDS` names every property it may emit (so an extra
/// one is dropped and logged instead of shipped).
pub trait BoundedAuditArguments: Serialize {
    /// The `operationId` this DTO is the one and only safe-argument policy for.
    const OPERATION_ID: &'static str;
    /// Every property name the DTO may emit. Always the matching [`catalog`]
    /// constant, never a second copy of the list.
    const ALLOWED_FIELDS: &'static [&'static str];

    /// How the arguments were obtained.
    ///
    /// `Parsed` unless the DTO had to OMIT a field because the caller's value
    /// was not in its validated form — an `invalid` record with the fields that
    /// did validate is more useful than a bare rejection, and still never echoes
    /// the value that failed.
    fn parse_status(&self) -> ArgumentsParseStatus {
        ArgumentsParseStatus::Parsed
    }
}

/// Project validated request inputs into that operation's safe DTO.
///
/// Implemented for small borrowed "input view" structs rather than for the
/// business request types themselves, because most operations' safe arguments
/// span the path, the query, the body, AND values only the handler can compute
/// (a clamped limit, a resolved label). The view is what makes "constructed from
/// validated fields" enforceable at the call site.
pub trait ToSafeAuditArguments: sealed::Sealed {
    type Safe: BoundedAuditArguments;

    fn to_safe_audit_arguments(&self) -> Self::Safe;
}

/// Project `input` and record the result on the request's audit context.
pub fn record<T: ToSafeAuditArguments>(extensions: &Extensions, input: &T) {
    record_safe(extensions, &input.to_safe_audit_arguments());
}

/// Record an already-built safe DTO.
///
/// The allowlist filter runs HERE rather than in each DTO, so every operation
/// gets it whether or not its author remembered to think about it.
pub fn record_safe<A: BoundedAuditArguments>(extensions: &Extensions, safe: &A) {
    let status = safe.parse_status();
    let values = allowlisted(safe);
    with_context(extensions, |context| {
        context.record_arguments(values, status)
    });
}

/// Record that a parser rejected the input, keeping only bounded transport
/// metadata.
pub fn record_invalid(extensions: &Extensions, invalid: &InvalidInput) {
    let values = filter(
        "invalid_input",
        catalog::INVALID_INPUT_FIELDS,
        serialize(invalid),
    );
    with_context(extensions, |context| {
        context.record_arguments(values, ArgumentsParseStatus::Invalid)
    });
}

/// Record that this operation takes no arguments.
pub fn record_not_applicable(extensions: &Extensions) {
    with_context(extensions, |context| {
        context.record_arguments(Map::new(), ArgumentsParseStatus::NotApplicable)
    });
}

/// The bounded transport metadata a rejected body contributes.
///
/// Every field is optional because each is known only sometimes: a body with no
/// `Content-Length` header declares no length, and a rejection raised before the
/// bytes were buffered observed none. Nothing here is derived from the payload
/// itself.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct InvalidInput {
    /// The normalized media type (parameters stripped, lower-cased).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// The caller's declared `Content-Length`, when it parsed as a number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_length_declared: Option<u64>,
    /// The bytes the bounded extractor actually buffered, when it got that far.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_bytes_observed: Option<u64>,
}

impl InvalidInput {
    /// The metadata a request's headers alone can supply.
    pub fn from_headers(headers: &axum::http::HeaderMap) -> Self {
        Self {
            content_type: headers
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(bounds::safe_content_type),
            content_length_declared: headers
                .get(axum::http::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok()),
            body_bytes_observed: None,
        }
    }

    /// The same metadata plus the byte count a bounded extractor buffered.
    pub fn with_observed_bytes(mut self, observed: usize) -> Self {
        self.body_bytes_observed = Some(u64::try_from(observed).unwrap_or(u64::MAX));
        self
    }
}

/// Serialize a safe DTO and drop anything outside its documented allowlist.
fn allowlisted<A: BoundedAuditArguments>(safe: &A) -> Map<String, Value> {
    filter(A::OPERATION_ID, A::ALLOWED_FIELDS, serialize(safe))
}

/// Serialize into a property map, or an EMPTY map with a loud error.
///
/// A DTO that fails to serialize, or that is not a JSON object, is a programmer
/// error. Recording nothing is the fail-closed answer: a partial or stringified
/// value could carry anything.
fn serialize<T: Serialize + ?Sized>(value: &T) -> Map<String, Value> {
    match serde_json::to_value(value) {
        Ok(Value::Object(map)) => map,
        Ok(_) => {
            tracing::error!("a safe audit argument DTO did not serialize to an object");
            Map::new()
        }
        Err(error) => {
            // The serde message names the FIELD that failed, never its value.
            tracing::error!(error = %error, "a safe audit argument DTO failed to serialize");
            Map::new()
        }
    }
}

/// Keep only the properties `allowed` names.
///
/// A dropped key is logged with the operation and the FIELD NAME — both compile-
/// time constants — and never with the value, which is precisely the thing whose
/// safety is in doubt.
fn filter(
    operation_id: &'static str,
    allowed: &'static [&'static str],
    values: Map<String, Value>,
) -> Map<String, Value> {
    let mut kept = Map::new();
    for (key, value) in values {
        if allowed.contains(&key.as_str()) {
            kept.insert(key, value);
        } else {
            tracing::error!(
                operation_id,
                field = %key,
                "safe audit arguments emitted a field outside the documented allowlist; dropping it"
            );
        }
    }
    kept
}

/// Shared assertions for this module tree's DTO tests (never in the binary).
#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
