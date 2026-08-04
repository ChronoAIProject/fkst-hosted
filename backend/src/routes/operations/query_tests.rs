//! Normalization tests for the activity query parameters.

use k8s_openapi::chrono::{Duration, TimeZone, Utc};

use super::*;
use crate::operations::filters::StatusClass;

const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 200;
const MAX_RANGE_DAYS: u64 = 30;

fn now() -> k8s_openapi::chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 31, 12, 0, 0)
        .single()
        .expect("a valid fixed instant")
}

fn run(params: ActivityQueryParams) -> Result<NormalizedActivityRequest, AppError> {
    normalize(&params, now(), DEFAULT_LIMIT, MAX_LIMIT, MAX_RANGE_DAYS)
}

#[test]
fn an_empty_query_resolves_to_the_documented_defaults() {
    let request = run(ActivityQueryParams::default()).expect("defaults normalize");
    assert!(request.requested_scope.is_none());
    assert!(!request.cross_actor_filter);
    assert_eq!(request.record_kind, RecordKind::ApiRequest);
    assert_eq!(request.limit, DEFAULT_LIMIT);
    assert_eq!(request.range.to, now());
    assert_eq!(request.range.from, now() - Duration::hours(24));
    assert_eq!(request.filters, ActivityFilters::default());
    assert!(!request.cursor_present());
}

#[test]
fn the_scope_vocabulary_is_closed() {
    for (raw, expected) in [
        ("mine", RequestedScope::Personal),
        ("all", RequestedScope::Global),
    ] {
        let request = run(ActivityQueryParams {
            scope: Some(raw.to_string()),
            ..ActivityQueryParams::default()
        })
        .expect(raw);
        assert_eq!(request.requested_scope, Some(expected));
    }
    for bad in ["", "everyone", "ALL", "accessible"] {
        assert!(
            run(ActivityQueryParams {
                scope: Some(bad.to_string()),
                ..ActivityQueryParams::default()
            })
            .is_err(),
            "{bad}"
        );
    }
}

/// Either actor filter marks the request as cross-user, which the scope gate then
/// refuses for a regular caller — even when the value happens to be their own.
#[test]
fn either_actor_filter_marks_the_request_as_cross_user() {
    let by_id = run(ActivityQueryParams {
        actor_id: Some(101),
        ..ActivityQueryParams::default()
    })
    .expect("normalizes");
    assert!(by_id.cross_actor_filter);

    let by_login = run(ActivityQueryParams {
        actor_login: Some("alice".to_string()),
        ..ActivityQueryParams::default()
    })
    .expect("normalizes");
    assert!(by_login.cross_actor_filter);
    assert_eq!(by_login.filters.actor_login.as_deref(), Some("alice"));

    // A leading `@` is accepted and normalized away, as everywhere else.
    let at_prefixed = run(ActivityQueryParams {
        actor_login: Some("@alice".to_string()),
        ..ActivityQueryParams::default()
    })
    .expect("normalizes");
    assert_eq!(at_prefixed.filters.actor_login.as_deref(), Some("alice"));
}

#[test]
fn an_unvalidated_actor_login_is_refused_rather_than_dropped() {
    for bad in ["", "@", "a b", "a/b", &"x".repeat(40)] {
        assert!(
            run(ActivityQueryParams {
                actor_login: Some(bad.to_string()),
                ..ActivityQueryParams::default()
            })
            .is_err(),
            "{bad:?}"
        );
    }
}

#[test]
fn the_limit_is_bounded_by_the_deployment_maximum() {
    assert_eq!(
        run(ActivityQueryParams {
            limit: Some(1),
            ..ActivityQueryParams::default()
        })
        .expect("valid")
        .limit,
        1
    );
    assert_eq!(
        run(ActivityQueryParams {
            limit: Some(MAX_LIMIT),
            ..ActivityQueryParams::default()
        })
        .expect("valid")
        .limit,
        MAX_LIMIT
    );
    // A limit outside the window is refused, never silently clamped: a clamped
    // page would look complete while omitting rows the caller asked for.
    for bad in [0, MAX_LIMIT + 1, u32::MAX] {
        assert!(
            run(ActivityQueryParams {
                limit: Some(bad),
                ..ActivityQueryParams::default()
            })
            .is_err(),
            "{bad}"
        );
    }
}

#[test]
fn every_fixed_filter_normalizes_into_the_typed_set() {
    let request = run(ActivityQueryParams {
        record_kind: Some("all".to_string()),
        operation_id: Some("canvas_overview".to_string()),
        method: Some("get".to_string()),
        status_code: Some(404),
        status_class: Some("4xx".to_string()),
        outcome: Some("rejected".to_string()),
        session_id: Some("sess-1".to_string()),
        repo_full_name: Some("acme/site".to_string()),
        trigger_issue: Some(7),
        request_id: Some("req-0001".to_string()),
        cursor: Some("opaque".to_string()),
        ..ActivityQueryParams::default()
    })
    .expect("normalizes");
    assert_eq!(request.record_kind, RecordKind::All);
    assert_eq!(request.filters.method.as_deref(), Some("GET"));
    assert_eq!(request.filters.status_code, Some(404));
    assert_eq!(request.filters.status_class, Some(StatusClass::ClientError));
    assert_eq!(
        request.filters.outcome,
        Some(crate::audit::event::AuditOutcome::Rejected)
    );
    assert_eq!(request.session_id(), Some("sess-1"));
    assert_eq!(request.filters.repo_full_name.as_deref(), Some("acme/site"));
    assert_eq!(request.filters.trigger_issue, Some(7));
    assert_eq!(request.filters.request_id.as_deref(), Some("req-0001"));
    assert!(request.cursor_present());
}

#[test]
fn an_unvalidated_filter_is_a_four_hundred_never_a_silently_dropped_predicate() {
    let cases: Vec<ActivityQueryParams> = vec![
        ActivityQueryParams {
            record_kind: Some("everything".to_string()),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            operation_id: Some("no_such_operation".to_string()),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            method: Some("TRACE".to_string()),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            status_code: Some(42),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            status_class: Some("6xx".to_string()),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            outcome: Some("fine".to_string()),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            session_id: Some("not a session".to_string()),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            repo_full_name: Some("acme".to_string()),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            trigger_issue: Some(0),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            request_id: Some("req 0001".to_string()),
            ..ActivityQueryParams::default()
        },
        ActivityQueryParams {
            from: Some("yesterday".to_string()),
            ..ActivityQueryParams::default()
        },
    ];
    for params in cases {
        let error = run(params.clone()).expect_err(&format!("{params:?}"));
        assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    }
}

/// The refusal message names the parameter, never the value that failed.
#[test]
fn a_refusal_never_echoes_the_offending_value() {
    let error = run(ActivityQueryParams {
        session_id: Some("' OR 1=1 --".to_string()),
        ..ActivityQueryParams::default()
    })
    .expect_err("refused");
    let rendered = format!("{error}");
    assert!(rendered.contains("session_id"), "{rendered}");
    assert!(!rendered.contains("OR 1=1"), "{rendered}");
}
