//! Orchestration tests: what reaches the sources, and how failures are mapped.

use std::sync::Arc;

use super::*;
use crate::operations::filters::{ActivityFilters, RecordKind};
use crate::operations::record::ActivitySourceKind;
use crate::operations::source::{ActivitySource, SourceError};
use crate::operations::test_support::{api_record, mine, range, FakeSource};
use crate::operations::{ActivityConcurrency, OperationsState};

const VIEWER_ID: i64 = 101;
const VIEWER: &str = "alice";

fn request() -> ActivityQueryRequest {
    ActivityQueryRequest {
        constraint: mine(VIEWER_ID, VIEWER, None),
        record_kind: RecordKind::ApiRequest,
        range: range(),
        filters: ActivityFilters::default(),
        cursor: None,
        limit: 10,
    }
}

#[tokio::test]
async fn both_sources_receive_the_same_typed_constraint_and_a_limit_plus_one() {
    let posthog = FakeSource::ok(
        ActivitySourceKind::Posthog,
        vec![api_record(
            "ev-1",
            VIEWER_ID,
            10,
            ActivitySourceKind::Posthog,
        )],
    );
    let relay = FakeSource::ok(ActivitySourceKind::Relay, Vec::new());
    let state = OperationsState::with_sources(
        Some(Arc::clone(&posthog) as Arc<dyn ActivitySource>),
        Some(Arc::clone(&relay) as Arc<dyn ActivitySource>),
    );

    let page = run(&state, VIEWER_ID, request())
        .await
        .expect("both sources answered");
    assert_eq!(page.items.len(), 1);

    for source in [posthog.queries(), relay.queries()] {
        let query = source.first().expect("the source was called").clone();
        assert_eq!(query.constraint.required_actor_id(), Some(VIEWER_ID));
        assert_eq!(
            query.fetch_limit, 11,
            "sources are asked for limit + 1 so the has-more probe needs no count"
        );
    }
}

#[tokio::test]
async fn a_deployment_with_no_source_answers_the_stable_not_configured_error() {
    let state = OperationsState::default();
    let error = run(&state, VIEWER_ID, request())
        .await
        .expect_err("no source can answer");
    assert!(
        matches!(error, crate::error::AppError::AuditQueryNotConfigured(_)),
        "{error:?}"
    );
    let snapshot = state.metrics.snapshot();
    assert_eq!(
        snapshot
            .queries()
            .find(|(_, _, result, _)| *result == "not_configured")
            .map(|(_, _, _, count)| count),
        Some(1)
    );
}

#[tokio::test]
async fn an_upstream_auth_or_schema_failure_is_a_bad_gateway() {
    let state = OperationsState::with_sources(
        Some(FakeSource::failing(
            ActivitySourceKind::Posthog,
            SourceError::Upstream { kind: "auth" },
        )),
        None,
    );
    let error = run(&state, VIEWER_ID, request())
        .await
        .expect_err("the only source refused");
    assert!(
        matches!(error, crate::error::AppError::Upstream(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_transient_failure_is_service_unavailable() {
    let state = OperationsState::with_sources(
        Some(FakeSource::failing(
            ActivitySourceKind::Posthog,
            SourceError::Transient { kind: "timeout" },
        )),
        None,
    );
    let error = run(&state, VIEWER_ID, request())
        .await
        .expect_err("the only source timed out");
    assert!(
        matches!(error, crate::error::AppError::Unavailable(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn local_capacity_exhaustion_is_a_rate_limit_with_a_bounded_retry_after() {
    let state = OperationsState {
        posthog: Some(FakeSource::ok(ActivitySourceKind::Posthog, Vec::new())),
        concurrency: ActivityConcurrency::new(0, 4),
        ..OperationsState::default()
    };
    let error = run(&state, VIEWER_ID, request())
        .await
        .expect_err("no capacity");
    match error {
        crate::error::AppError::RateLimited {
            retry_after_secs, ..
        } => assert!(
            (1..=60).contains(&retry_after_secs),
            "Retry-After must be bounded, got {retry_after_secs}"
        ),
        other => panic!("expected a rate limit, got {other:?}"),
    }
    let snapshot = state.metrics.snapshot();
    assert_eq!(
        snapshot
            .rejections()
            .find(|(reason, _)| *reason == "capacity_exhausted")
            .map(|(_, count)| count),
        Some(1)
    );
}

/// A refused request must cost the deployment nothing upstream.
#[tokio::test]
async fn a_capacity_refusal_never_calls_a_source() {
    let posthog = FakeSource::ok(ActivitySourceKind::Posthog, Vec::new());
    let state = OperationsState {
        posthog: Some(Arc::clone(&posthog) as Arc<dyn ActivitySource>),
        concurrency: ActivityConcurrency::new(0, 4),
        ..OperationsState::default()
    };
    let _ = run(&state, VIEWER_ID, request()).await;
    assert!(posthog.queries().is_empty());
}

#[tokio::test]
async fn a_partial_page_records_its_source_and_row_telemetry() {
    let state = OperationsState::with_sources(
        Some(FakeSource::ok(
            ActivitySourceKind::Posthog,
            vec![api_record(
                "ev-1",
                VIEWER_ID,
                10,
                ActivitySourceKind::Posthog,
            )],
        )),
        Some(FakeSource::failing(
            ActivitySourceKind::Relay,
            SourceError::Transient { kind: "connect" },
        )),
    );
    let page = run(&state, VIEWER_ID, request())
        .await
        .expect("posthog answered");
    assert!(page.status.partial);
    let snapshot = state.metrics.snapshot();
    assert_eq!(
        snapshot
            .partial()
            .find(|(source, _)| *source == "relay")
            .map(|(_, count)| count),
        Some(1)
    );
    assert_eq!(
        snapshot
            .rows()
            .find(|(result, _)| *result == "returned")
            .map(|(_, count)| count),
        Some(1)
    );
}
