//! Cursor tests: the round trip, and every way a cursor from a different query
//! must be refused rather than silently reset.

use k8s_openapi::chrono::Duration;

use super::*;
use crate::operations::filters::{ActivityFilters, RecordKind, TimeRange};
use crate::operations::test_support::{anchor, range};

const SESSION: &str = "sess-alice";

fn binding() -> CursorBinding {
    CursorBinding {
        scope: "mine",
        viewer_id: Some(101),
        session_id: None,
        record_kind: RecordKind::ApiRequest,
        range: range(),
        filters: ActivityFilters::default(),
    }
}

fn key() -> CursorKey {
    CursorKey {
        timestamp: anchor() - Duration::minutes(5),
        event_id: "1b1e6b9c-0000-4000-8000-000000000001".to_string(),
    }
}

#[test]
fn a_cursor_round_trips_within_the_same_query() {
    let binding = binding();
    let encoded = encode(&key(), &binding).expect("encodes");
    let decoded = decode(&encoded, &binding).expect("decodes");
    assert_eq!(decoded, key());
    assert!(
        encoded.len() <= MAX_CURSOR_LEN,
        "a server-issued cursor must fit its own bound"
    );
}

#[test]
fn the_encoded_cursor_carries_no_identity_in_the_clear() {
    let encoded = encode(&key(), &binding()).expect("encodes");
    assert!(!encoded.contains("101"), "{encoded}");
    assert!(!encoded.contains("mine"), "{encoded}");
}

/// The whole point of the digest: a cursor minted for one viewer must not resume
/// another's page, and must not be quietly discarded either.
#[test]
fn a_cursor_from_another_viewer_is_refused() {
    let mine = binding();
    let encoded = encode(&key(), &mine).expect("encodes");
    let theirs = CursorBinding {
        viewer_id: Some(202),
        ..mine
    };
    let error = decode(&encoded, &theirs).expect_err("must not decode");
    assert!(
        matches!(error, AppError::InvalidActivityCursor(_)),
        "{error:?}"
    );
}

#[test]
fn a_cursor_from_another_scope_session_kind_range_or_filter_is_refused() {
    let base = binding();
    let encoded = encode(&key(), &base).expect("encodes");

    let mutations: Vec<(&str, CursorBinding)> = vec![
        (
            "scope",
            CursorBinding {
                scope: "all",
                viewer_id: None,
                ..base.clone()
            },
        ),
        (
            "session",
            CursorBinding {
                session_id: Some(SESSION.to_string()),
                ..base.clone()
            },
        ),
        (
            "record kind",
            CursorBinding {
                record_kind: RecordKind::All,
                ..base.clone()
            },
        ),
        (
            "range",
            CursorBinding {
                range: TimeRange {
                    from: base.range.from - Duration::hours(1),
                    to: base.range.to,
                },
                ..base.clone()
            },
        ),
        (
            "filters",
            CursorBinding {
                filters: ActivityFilters {
                    method: Some("GET".to_string()),
                    ..ActivityFilters::default()
                },
                ..base.clone()
            },
        ),
    ];
    for (what, mutated) in mutations {
        let error = decode(&encoded, &mutated).expect_err(&format!(
            "a cursor bound to a different {what} must be refused"
        ));
        assert!(
            matches!(error, AppError::InvalidActivityCursor(_)),
            "{what}: {error:?}"
        );
    }
}

#[test]
fn a_malformed_oversized_or_wrongly_versioned_cursor_is_refused() {
    let binding = binding();
    let oversized = "A".repeat(MAX_CURSOR_LEN + 1);
    for raw in ["", "not-base64!!", "YWJj", oversized.as_str()] {
        let error = decode(raw, &binding).expect_err(raw);
        assert!(matches!(error, AppError::InvalidActivityCursor(_)), "{raw}");
    }

    // A payload that is well-formed base64 JSON but the wrong version.
    let payload =
        serde_json::json!({"v": 99, "ts": "2026-07-31T11:55:00.000Z", "id": "x", "d": "y"});
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        serde_json::to_vec(&payload).expect("serializes"),
    );
    let error = decode(&encoded, &binding).expect_err("wrong version");
    assert!(
        matches!(error, AppError::InvalidActivityCursor(_)),
        "{error:?}"
    );
}

/// The event id inside a cursor becomes a query parameter, so its syntax is
/// bounded exactly like every other identifier on this surface.
#[test]
fn a_cursor_with_an_unbounded_or_unsafe_event_id_is_refused() {
    let binding = binding();
    for id in ["", &"x".repeat(200), "a'b", "a b"] {
        let payload = serde_json::json!({
            "v": 1,
            "ts": "2026-07-31T11:55:00.000Z",
            "id": id,
            "d": binding.digest(),
        });
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            serde_json::to_vec(&payload).expect("serializes"),
        );
        let error = decode(&encoded, &binding).expect_err(id);
        assert!(matches!(error, AppError::InvalidActivityCursor(_)), "{id}");
    }
}

/// The digest must actually distinguish orderings: two filter sets that differ
/// only in which field is set must not collide.
#[test]
fn distinct_filter_sets_produce_distinct_digests() {
    let base = binding();
    let with_method = CursorBinding {
        filters: ActivityFilters {
            method: Some("GET".to_string()),
            ..ActivityFilters::default()
        },
        ..base.clone()
    };
    let with_operation = CursorBinding {
        filters: ActivityFilters {
            operation_id: Some("GET".to_string()),
            ..ActivityFilters::default()
        },
        ..base.clone()
    };
    assert_ne!(base.digest(), with_method.digest());
    assert_ne!(with_method.digest(), with_operation.digest());
}
