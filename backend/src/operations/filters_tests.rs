//! Filter and time-range validation: the closed vocabulary, and the refusals
//! that keep a caller from widening a query by supplying something odd.

use k8s_openapi::chrono::Duration;

use super::*;
use crate::audit::event::AuditOutcome;
use crate::operations::test_support::anchor;

#[test]
fn the_record_kind_vocabulary_is_closed() {
    assert_eq!(
        RecordKind::parse("api_request").expect("valid"),
        RecordKind::ApiRequest
    );
    assert_eq!(
        RecordKind::parse("sandbox_lifecycle").expect("valid"),
        RecordKind::SandboxLifecycle
    );
    assert_eq!(RecordKind::parse("all").expect("valid"), RecordKind::All);
    for bad in ["", "API_REQUEST", "events", "fkst api request completed"] {
        assert!(RecordKind::parse(bad).is_err(), "{bad}");
    }
    assert_eq!(RecordKind::default(), RecordKind::ApiRequest);
}

#[test]
fn only_the_lifecycle_bearing_kinds_need_an_authorized_session() {
    assert!(!RecordKind::ApiRequest.includes_lifecycle());
    assert!(RecordKind::SandboxLifecycle.includes_lifecycle());
    assert!(RecordKind::All.includes_lifecycle());
    assert!(RecordKind::ApiRequest.includes_api_requests());
    assert!(!RecordKind::SandboxLifecycle.includes_api_requests());
    assert!(RecordKind::All.includes_api_requests());
}

#[test]
fn the_status_class_vocabulary_is_closed_and_half_open() {
    assert_eq!(
        StatusClass::parse("4xx").expect("valid").bounds(),
        (400, 500)
    );
    for bad in ["6xx", "2XX", "200", ""] {
        assert!(StatusClass::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_outcome_vocabulary_matches_the_audit_contract() {
    assert_eq!(
        parse_outcome("rejected").expect("valid"),
        AuditOutcome::Rejected
    );
    assert_eq!(
        parse_outcome("incomplete").expect("valid"),
        AuditOutcome::Incomplete
    );
    for bad in ["ok", "SUCCESS", "", "succeeded"] {
        assert!(parse_outcome(bad).is_err(), "{bad:?}");
    }
}

/// An unknown operation id is a `400`, not a confidently empty page.
#[test]
fn an_operation_id_must_exist_in_the_server_catalog() {
    assert_eq!(
        parse_operation_id("canvas_overview").expect("declared"),
        "canvas_overview"
    );
    assert_eq!(
        parse_operation_id("operations_list_activity").expect("declared"),
        "operations_list_activity"
    );
    // The sentinel for a request that matched no documented route is a real
    // recorded value, so it is queryable.
    assert_eq!(
        parse_operation_id("<unmatched>").expect("sentinel"),
        "<unmatched>"
    );
    for bad in ["", "nope", "canvas_overview; DROP", "CANVAS_OVERVIEW"] {
        assert!(parse_operation_id(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_method_status_and_identifier_filters_reject_anything_unvalidated() {
    assert_eq!(parse_method("get").expect("valid"), "GET");
    for bad in ["HEAD", "OPTIONS", "TRACE", "", "GET; --"] {
        assert!(parse_method(bad).is_err(), "{bad}");
    }
    assert_eq!(parse_status_code(404).expect("valid"), 404);
    for bad in [0u16, 99, 600, 65535] {
        assert!(parse_status_code(bad).is_err(), "{bad}");
    }
    assert_eq!(
        parse_repo_full_name("acme/site").expect("valid"),
        "acme/site"
    );
    for bad in ["acme", "acme/", "/site", "acme/si te", "acme/site/extra"] {
        assert!(parse_repo_full_name(bad).is_err(), "{bad}");
    }
    assert!(parse_trigger_issue(0).is_err());
    assert!(parse_trigger_issue(-1).is_err());
    assert_eq!(parse_trigger_issue(7).expect("valid"), 7);
    assert!(parse_session_id("bad id").is_err());
    assert!(parse_request_id("req id").is_err());
    assert_eq!(parse_request_id("req-0001").expect("valid"), "req-0001");
}

#[test]
fn an_omitted_range_is_the_last_twenty_four_hours() {
    let now = anchor();
    let range = resolve_range(None, None, now, 30).expect("defaults");
    assert_eq!(range.to, now);
    assert_eq!(range.from, now - Duration::hours(24));
}

#[test]
fn an_explicit_range_is_parsed_as_utc_and_rendered_with_millisecond_precision() {
    let now = anchor();
    let range = resolve_range(
        Some("2026-07-31T00:00:00+02:00"),
        Some("2026-07-31T10:00:00Z"),
        now,
        30,
    )
    .expect("valid");
    assert_eq!(range.from_rfc3339(), "2026-07-30T22:00:00.000Z");
    assert_eq!(range.to_rfc3339(), "2026-07-31T10:00:00.000Z");
}

#[test]
fn an_inverted_equal_oversized_or_future_range_is_refused() {
    let now = anchor();
    let cases = [
        ("2026-07-31T10:00:00Z", "2026-07-31T09:00:00Z", 30),
        ("2026-07-31T10:00:00Z", "2026-07-31T10:00:00Z", 30),
        ("2026-01-01T00:00:00Z", "2026-07-31T00:00:00Z", 30),
        ("2027-01-01T00:00:00Z", "2027-01-02T00:00:00Z", 30),
    ];
    for (from, to, max_days) in cases {
        let error =
            resolve_range(Some(from), Some(to), now, max_days).expect_err(&format!("{from}..{to}"));
        assert!(matches!(error, AppError::Validation(_)), "{from}..{to}");
    }
}

#[test]
fn a_malformed_bound_names_its_own_parameter_without_echoing_the_value() {
    let now = anchor();
    let error = resolve_range(Some("yesterday"), None, now, 30).expect_err("malformed");
    let rendered = format!("{error}");
    assert!(rendered.contains("from"), "{rendered}");
    assert!(!rendered.contains("yesterday"), "{rendered}");
}

/// The binding fields are what a cursor digest is taken over, so they must be
/// stable and must change when the filters do.
#[test]
fn binding_fields_are_stable_ordered_and_filter_sensitive() {
    let filters = ActivityFilters {
        method: Some("GET".to_string()),
        status_code: Some(200),
        session_id: Some("sess-1".to_string()),
        ..ActivityFilters::default()
    };
    assert_eq!(
        filters.binding_fields(),
        vec![
            "method=GET".to_string(),
            "status_code=200".to_string(),
            "session_id=sess-1".to_string(),
        ]
    );
    assert!(ActivityFilters::default().binding_fields().is_empty());
}
