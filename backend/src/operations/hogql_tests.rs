//! The security tests for the fixed query text.
//!
//! These are the assertions that make "a request cannot contribute one character
//! to the query" checkable rather than aspirational: the mandatory predicates
//! appear in the OUTBOUND query source before `LIMIT`, hostile values stay in the
//! parameter map, and the `record_kind=all` union is parenthesized so its
//! lifecycle branch cannot escape the actor predicate.

use super::*;
use crate::operations::cursor::CursorKey;
use crate::operations::filters::{ActivityFilters, RecordKind, StatusClass};
use crate::operations::source::SourceQuery;
use crate::operations::test_support::{all, authorized_session, mine, range};

const VIEWER_ID: i64 = 101;
const VIEWER: &str = "alice";
const SESSION: &str = "sess-alice";

fn query(
    record_kind: RecordKind,
    constraint: crate::session_access::ActivityVisibilityConstraint,
) -> SourceQuery {
    SourceQuery {
        constraint,
        record_kind,
        range: range(),
        filters: ActivityFilters::default(),
        cursor: None,
        fetch_limit: 101,
    }
}

/// The index of `LIMIT` in the rendered query, so every "before LIMIT" assertion
/// is about position rather than mere presence.
fn limit_at(text: &str) -> usize {
    text.rfind("LIMIT").expect("the query always has a LIMIT")
}

fn assert_before_limit(text: &str, needle: &str) {
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("query does not contain {needle}:\n{text}"));
    assert!(
        at < limit_at(text),
        "{needle} must appear before LIMIT:\n{text}"
    );
}

/// The projection is an allowlist, so a field the response contract documents
/// but the SELECT never asks for is a field that is structurally always absent.
#[test]
fn every_correlation_and_identity_column_the_dtos_document_is_projected() {
    let built = build(&query(RecordKind::All, all(900, "root")));
    for column in [
        "properties.principal_id AS principal_id",
        "properties.session_id AS session_id",
        "properties.repo_full_name AS repo_full_name",
        "properties.installation_id AS installation_id",
        "properties.trigger_issue AS trigger_issue",
        "properties.webhook_delivery_id AS webhook_delivery_id",
        "properties.request_id AS request_id",
    ] {
        assert!(
            built.query.contains(column),
            "{column} missing from:\n{}",
            built.query
        );
    }
}

#[test]
fn the_personal_actor_predicate_is_in_the_outbound_query_before_limit() {
    let built = build(&query(
        RecordKind::ApiRequest,
        mine(VIEWER_ID, VIEWER, None),
    ));
    assert_before_limit(&built.query, "properties.actor_id = {viewer_actor_id}");
    assert_eq!(
        built.values.get("viewer_actor_id"),
        Some(&serde_json::json!(VIEWER_ID)),
        "the verified viewer id travels as a PARAMETER, never as query text"
    );
    // The predicate is in the SOURCE query — not applied to a fetched page.
    assert!(built.query.contains("WHERE"));
    assert!(!built.query.contains(&VIEWER_ID.to_string()));
}

#[test]
fn the_authorized_session_predicate_is_in_the_outbound_query_before_limit() {
    let session = authorized_session(SESSION, VIEWER_ID, VIEWER);
    let built = build(&query(
        RecordKind::SandboxLifecycle,
        mine(VIEWER_ID, VIEWER, Some(session)),
    ));
    assert_before_limit(
        &built.query,
        "properties.session_id = {authorized_session_id}",
    );
    assert_eq!(
        built.values.get("authorized_session_id"),
        Some(&serde_json::json!(SESSION))
    );
    assert!(
        !built.query.contains(SESSION),
        "the session id must not appear in query text:\n{}",
        built.query
    );
}

/// The parenthesization test the spec demands: the `record_kind=all` union must
/// bind so the lifecycle branch cannot escape the actor predicate.
#[test]
fn the_all_record_union_is_parenthesized_around_both_branches() {
    let session = authorized_session(SESSION, VIEWER_ID, VIEWER);
    let built = build(&query(
        RecordKind::All,
        mine(VIEWER_ID, VIEWER, Some(session)),
    ));
    let text = &built.query;

    // The whole union is one parenthesized group, and each branch is its own.
    let api_branch = "(event IN ({event_request_completed}, {event_request_incomplete}) \
                      AND properties.actor_id = {viewer_actor_id} \
                      AND properties.session_id = {authorized_session_id})";
    let lifecycle_branch =
        "(event = {event_sandbox_lifecycle} AND properties.session_id = {authorized_session_id})";
    let union = format!("({api_branch} OR {lifecycle_branch})");
    assert!(
        text.contains(&union),
        "the union must be fully parenthesized with the actor predicate INSIDE the \
         api branch; got:\n{text}"
    );
    assert_before_limit(text, &union);

    // Every parenthesis balances: an unbalanced group is exactly how an OR
    // escapes its intended scope.
    let mut depth = 0i32;
    for ch in text.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        assert!(depth >= 0, "unbalanced parentheses in:\n{text}");
    }
    assert_eq!(depth, 0, "unbalanced parentheses in:\n{text}");

    // The actor predicate occurs exactly once and inside the api branch, so the
    // lifecycle branch is reachable ONLY through the authorized session.
    assert_eq!(
        text.matches("properties.actor_id = {viewer_actor_id}")
            .count(),
        1
    );
    let actor_at = text
        .find("properties.actor_id = {viewer_actor_id}")
        .expect("actor predicate");
    let branch_at = text.find(api_branch).expect("api branch");
    assert!(actor_at > branch_at && actor_at < branch_at + api_branch.len());
}

/// Without a parenthesized union, `A AND B OR C` would admit every lifecycle row
/// in the store. Prove the rendered text cannot be read that way.
#[test]
fn the_lifecycle_branch_cannot_escape_the_actor_predicate() {
    let session = authorized_session(SESSION, VIEWER_ID, VIEWER);
    let built = build(&query(
        RecordKind::All,
        mine(VIEWER_ID, VIEWER, Some(session)),
    ));
    let text = &built.query;
    let or_at = text.find(" OR ").expect("the union has an OR");
    // Walk back from the OR: it must sit inside a group opened after the WHERE.
    let before = &text[..or_at];
    let depth: i32 = before.chars().fold(0, |depth, ch| match ch {
        '(' => depth + 1,
        ')' => depth - 1,
        _ => depth,
    });
    assert!(
        depth >= 1,
        "the OR must be nested inside at least one group, else it binds looser \
         than the surrounding ANDs:\n{text}"
    );
}

#[test]
fn the_global_scope_carries_no_actor_predicate() {
    let built = build(&query(RecordKind::All, all(900, "root")));
    assert!(!built.query.contains("viewer_actor_id"));
    assert!(!built.values.contains_key("viewer_actor_id"));
    assert!(!built.values.contains_key("authorized_session_id"));
}

/// Only the three fixed audit event names may ever reach the source.
#[test]
fn only_the_three_fixed_event_names_reach_the_source() {
    let built = build(&query(RecordKind::All, all(900, "root")));
    let events: Vec<_> = built
        .values
        .iter()
        .filter(|(key, _)| key.starts_with("event_"))
        .map(|(_, value)| value.clone())
        .collect();
    assert_eq!(events.len(), 3, "{events:?}");
    for expected in [
        crate::audit::event::EVENT_NAME,
        crate::audit::event::INCOMPLETE_EVENT_NAME,
        crate::audit::lifecycle::LIFECYCLE_EVENT_NAME,
    ] {
        assert!(
            events.contains(&serde_json::json!(expected)),
            "missing {expected} in {events:?}"
        );
    }
}

/// A hostile filter value stays a parameter. Nothing a caller writes becomes
/// query source, so quoting, comments, and HogQL fragments are inert.
#[test]
fn malicious_filter_values_remain_parameters() {
    let hostile = "' OR 1=1 --";
    let built = build(&SourceQuery {
        constraint: mine(VIEWER_ID, VIEWER, None),
        record_kind: RecordKind::ApiRequest,
        range: range(),
        filters: ActivityFilters {
            actor_login: Some(hostile.to_string()),
            request_id: Some(hostile.to_string()),
            repo_full_name: Some(hostile.to_string()),
            ..ActivityFilters::default()
        },
        cursor: None,
        fetch_limit: 101,
    });
    assert!(
        !built.query.contains("OR 1=1"),
        "a filter value must never enter query text:\n{}",
        built.query
    );
    assert!(!built.query.contains("--"));
    assert_eq!(
        built.values.get("filter_actor_login"),
        Some(&serde_json::json!(hostile))
    );
    // The query text contains ONLY placeholder names in braces.
    for fragment in built.query.split('{').skip(1) {
        let name = fragment.split('}').next().expect("a closed placeholder");
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "placeholder {name:?} is not a compile-time name:\n{}",
            built.query
        );
    }
}

#[test]
fn the_keyset_predicate_is_a_strict_after_comparison_and_never_an_offset() {
    let built = build(&SourceQuery {
        constraint: mine(VIEWER_ID, VIEWER, None),
        record_kind: RecordKind::ApiRequest,
        range: range(),
        filters: ActivityFilters::default(),
        cursor: Some(CursorKey {
            timestamp: range().to,
            event_id: "ev-9".to_string(),
        }),
        fetch_limit: 51,
    });
    assert_before_limit(
        &built.query,
        "(timestamp < {cursor_timestamp} OR (timestamp = {cursor_timestamp} \
         AND properties.event_id < {cursor_event_id}))",
    );
    assert!(
        !built.query.to_ascii_uppercase().contains("OFFSET"),
        "pagination must never use OFFSET:\n{}",
        built.query
    );
    assert_eq!(built.values.get("page_limit"), Some(&serde_json::json!(51)));
}

#[test]
fn the_order_is_timestamp_then_event_id_descending() {
    let built = build(&query(RecordKind::ApiRequest, all(900, "root")));
    assert!(
        built
            .query
            .contains("ORDER BY timestamp DESC, properties.event_id DESC"),
        "{}",
        built.query
    );
}

#[test]
fn every_fixed_filter_renders_its_own_parameterized_predicate() {
    let built = build(&SourceQuery {
        constraint: all(900, "root"),
        record_kind: RecordKind::ApiRequest,
        range: range(),
        filters: ActivityFilters {
            actor_id: Some(7),
            actor_login: Some("bob".to_string()),
            operation_id: Some("canvas_overview".to_string()),
            method: Some("GET".to_string()),
            status_code: Some(404),
            status_class: Some(StatusClass::ClientError),
            outcome: Some(crate::audit::event::AuditOutcome::Rejected),
            session_id: Some("sess-1".to_string()),
            repo_full_name: Some("acme/site".to_string()),
            trigger_issue: Some(7),
            request_id: Some("req-1".to_string()),
        },
        cursor: None,
        fetch_limit: 11,
    });
    for placeholder in [
        "filter_actor_id",
        "filter_actor_login",
        "filter_operation_id",
        "filter_method",
        "filter_status_code",
        "filter_status_low",
        "filter_status_high",
        "filter_outcome",
        "filter_session_id",
        "filter_repo_full_name",
        "filter_trigger_issue",
        "filter_request_id",
    ] {
        assert!(
            built.values.contains_key(placeholder),
            "missing binding {placeholder}"
        );
        assert_before_limit(&built.query, &format!("{{{placeholder}}}"));
    }
}

/// The request body is the exact `HogQLQuery` envelope PostHog documents.
#[test]
fn the_request_body_is_a_hogql_query_node() {
    let built = build(&query(RecordKind::ApiRequest, all(900, "root")));
    let body = built.request_body();
    assert_eq!(body["query"]["kind"], "HogQLQuery");
    assert_eq!(body["query"]["query"], built.query);
    assert!(body["query"]["values"].is_object());
}
