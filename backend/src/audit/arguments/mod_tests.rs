//! Unit tests for the safe-argument contract itself: the allowlist filter, the
//! four parse states, and the malformed-input shape.

use super::*;
use crate::audit::request::AuditRequestContext;
use axum::http::{header, HeaderMap, HeaderValue};

/// A context installed on a fresh extension map, plus the map.
fn context() -> (AuditRequestContext, Extensions) {
    let context = AuditRequestContext::new();
    let mut extensions = Extensions::new();
    context.install(&mut extensions);
    (context, extensions)
}

/// A DTO that deliberately emits one property its allowlist does not name — the
/// exact mistake the filter exists to catch.
#[derive(Serialize)]
struct LeakyArguments {
    allowed: &'static str,
    undocumented: &'static str,
}

impl BoundedAuditArguments for LeakyArguments {
    const OPERATION_ID: &'static str = "test_leaky_operation";
    const ALLOWED_FIELDS: &'static [&'static str] = &["allowed"];
}

/// A DTO that reports itself invalid because it dropped a field.
#[derive(Serialize)]
struct PartialArguments {
    allowed: &'static str,
}

impl BoundedAuditArguments for PartialArguments {
    const OPERATION_ID: &'static str = "test_partial_operation";
    const ALLOWED_FIELDS: &'static [&'static str] = &["allowed"];

    fn parse_status(&self) -> ArgumentsParseStatus {
        ArgumentsParseStatus::Invalid
    }
}

/// The load-bearing property: a field outside the documented allowlist is
/// DROPPED, not shipped. Forgetting to update the catalog costs a property; it
/// can never cost a leak.
#[test]
fn a_field_outside_the_allowlist_is_dropped() {
    let (context, extensions) = context();
    record_safe(
        &extensions,
        &LeakyArguments {
            allowed: "kept",
            undocumented: "canary-undocumented-value",
        },
    );
    let frozen = context.freeze();
    assert_eq!(frozen.arguments.len(), 1);
    assert_eq!(
        frozen.arguments.get("allowed").and_then(|v| v.as_str()),
        Some("kept")
    );
    assert!(!frozen.arguments.contains_key("undocumented"));
    let rendered = serde_json::to_string(&frozen.arguments).expect("serializes");
    assert!(
        !rendered.contains("canary-undocumented-value"),
        "{rendered}"
    );
}

#[test]
fn a_recorded_dto_reports_its_own_parse_status() {
    let (context, extensions) = context();
    record_safe(&extensions, &PartialArguments { allowed: "kept" });
    assert_eq!(
        context.freeze().arguments_parse_status,
        ArgumentsParseStatus::Invalid
    );
}

#[test]
fn not_applicable_records_an_empty_map() {
    let (context, extensions) = context();
    record_not_applicable(&extensions);
    let frozen = context.freeze();
    assert!(frozen.arguments.is_empty());
    assert_eq!(
        frozen.arguments_parse_status,
        ArgumentsParseStatus::NotApplicable
    );
}

/// The default when NOTHING recorded is supplied by the operation's own policy,
/// which is how a request rejected before its safe parse is distinguished from
/// one that has no arguments at all.
#[test]
fn an_unrecorded_context_takes_the_supplied_default() {
    let (context, _extensions) = context();
    assert_eq!(
        context
            .freeze_with_default(ArgumentsParseStatus::Unavailable)
            .arguments_parse_status,
        ArgumentsParseStatus::Unavailable
    );
    assert_eq!(
        context.freeze().arguments_parse_status,
        ArgumentsParseStatus::NotApplicable
    );
}

/// The malformed-input shape: transport metadata only. No bytes, no lossy text,
/// no parser excerpt, no query string.
#[test]
fn invalid_input_records_only_bounded_transport_metadata() {
    let (context, extensions) = context();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=canary-charset"),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("4096"));
    record_invalid(
        &extensions,
        &InvalidInput::from_headers(&headers).with_observed_bytes(37),
    );

    let frozen = context.freeze();
    assert_eq!(frozen.arguments_parse_status, ArgumentsParseStatus::Invalid);
    assert_eq!(
        frozen
            .arguments
            .get("content_type")
            .and_then(|v| v.as_str()),
        Some("application/json")
    );
    assert_eq!(
        frozen
            .arguments
            .get("content_length_declared")
            .and_then(serde_json::Value::as_u64),
        Some(4096)
    );
    assert_eq!(
        frozen
            .arguments
            .get("body_bytes_observed")
            .and_then(serde_json::Value::as_u64),
        Some(37)
    );
    assert_eq!(frozen.arguments.len(), 3, "nothing else may be recorded");
    let rendered = serde_json::to_string(&frozen.arguments).expect("serializes");
    assert!(!rendered.contains("canary-charset"), "{rendered}");
}

#[test]
fn absent_headers_simply_omit_their_metadata() {
    let invalid = InvalidInput::from_headers(&HeaderMap::new());
    assert_eq!(invalid, InvalidInput::default());
    let (context, extensions) = context();
    record_invalid(&extensions, &invalid);
    let frozen = context.freeze();
    assert!(frozen.arguments.is_empty());
    assert_eq!(frozen.arguments_parse_status, ArgumentsParseStatus::Invalid);
}

/// A malformed `Content-Length` is not a number, so it is not recorded — the
/// alternative would be echoing a caller-chosen string as a "length".
#[test]
fn a_non_numeric_content_length_is_dropped() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_static("canary-not-a-number"),
    );
    assert_eq!(
        InvalidInput::from_headers(&headers).content_length_declared,
        None
    );
}

/// Recording is a no-op without an installed context, so any call site may use
/// it unconditionally.
#[test]
fn recording_without_a_context_is_harmless() {
    let extensions = Extensions::new();
    record_safe(&extensions, &PartialArguments { allowed: "kept" });
    record_not_applicable(&extensions);
    record_invalid(&extensions, &InvalidInput::default());
}

/// Two writers disagreeing about one request's arguments is a programmer error:
/// the FIRST value is kept and the conflict is counted, never merged.
#[test]
fn a_conflicting_second_write_keeps_the_first_and_is_counted() {
    let (context, extensions) = context();
    record_safe(&extensions, &PartialArguments { allowed: "first" });
    record_safe(&extensions, &PartialArguments { allowed: "second" });
    let frozen = context.freeze();
    assert_eq!(
        frozen.arguments.get("allowed").and_then(|v| v.as_str()),
        Some("first")
    );
    assert_eq!(frozen.conflicts, 1);
}

/// Arguments and top-level correlation must AGREE. A second, different write to
/// a correlation slot is the same programmer error as a conflicting argument
/// write: the first value is kept, the conflict is counted, and the middleware
/// turns that count into a bounded metric.
#[test]
fn a_conflicting_correlation_write_is_detected_and_counted() {
    let (context, _extensions) = context();
    context.record_repo_full_name("acme/site");
    context.record_trigger_issue(42);
    context.record_session_id("session-one");
    // Two layers AGREEING is normal and must not count.
    context.record_repo_full_name("acme/site");
    // Two layers DISAGREEING is not.
    context.record_repo_full_name("attacker/other");
    context.record_trigger_issue(99);
    context.record_session_id("session-two");

    let frozen = context.freeze();
    assert_eq!(
        frozen.correlation.repo_full_name.as_deref(),
        Some("acme/site"),
        "the first verified value wins; a later writer may never overwrite it"
    );
    assert_eq!(frozen.correlation.trigger_issue, Some(42));
    assert_eq!(
        frozen.correlation.session_id.as_deref(),
        Some("session-one")
    );
    assert_eq!(
        frozen.conflicts, 3,
        "one per disagreeing slot, never merged"
    );
}

/// The values a conflict carries may themselves be sensitive, so the resolution
/// keeps the first and never concatenates or joins the two.
#[test]
fn a_conflict_never_concatenates_the_two_values() {
    let (context, _extensions) = context();
    context.record_session_id("first");
    context.record_session_id("canary-second-session");
    let frozen = context.freeze();
    let rendered = format!("{:?}", frozen.correlation);
    assert!(!rendered.contains("canary-second-session"), "{rendered}");
}
