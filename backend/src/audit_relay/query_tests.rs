//! The read protocol: the scope is server-constructed, and a half-specified
//! `mine` scope can never reach SQL.

use super::*;
use crate::operations::test_support::{all, authorized_session, mine};

#[test]
fn a_mine_scope_without_an_actor_id_is_refused() {
    let wire = RelayScopeV1 {
        scope: SCOPE_MINE.to_string(),
        actor_id: None,
        lifecycle_session_id: Some("sess-1".to_string()),
    };
    assert_eq!(ResolvedScope::resolve(&wire), None);
}

#[test]
fn an_unknown_scope_spelling_is_refused_not_widened() {
    for spelling in ["", "everything", "ALL", "Mine"] {
        let wire = RelayScopeV1 {
            scope: spelling.to_string(),
            actor_id: Some(101),
            lifecycle_session_id: None,
        };
        assert_eq!(
            ResolvedScope::resolve(&wire),
            None,
            "`{spelling}` must not resolve"
        );
    }
}

#[test]
fn the_global_scope_resolves_without_an_actor() {
    let wire = RelayScopeV1 {
        scope: SCOPE_ALL.to_string(),
        actor_id: None,
        lifecycle_session_id: None,
    };
    assert_eq!(ResolvedScope::resolve(&wire), Some(ResolvedScope::All));
}

#[test]
fn a_personal_constraint_projects_its_verified_actor_and_authorized_session() {
    let session = authorized_session("sess-1", 101, "alice");
    let wire = RelayScopeV1::from_constraint(&mine(101, "alice", Some(session)));
    assert_eq!(wire.scope, SCOPE_MINE);
    assert_eq!(wire.actor_id, Some(101));
    assert_eq!(wire.lifecycle_session_id.as_deref(), Some("sess-1"));
    assert_eq!(
        ResolvedScope::resolve(&wire),
        Some(ResolvedScope::Mine {
            actor_id: 101,
            lifecycle_session_id: Some("sess-1".to_string()),
        })
    );
}

#[test]
fn a_global_constraint_projects_no_narrowing_at_all() {
    let wire = RelayScopeV1::from_constraint(&all(7, "root"));
    assert_eq!(wire.scope, SCOPE_ALL);
    assert_eq!(wire.actor_id, None);
    assert_eq!(wire.lifecycle_session_id, None);
}

#[test]
fn absent_filters_are_omitted_from_the_wire_entirely() {
    // An omitted filter and an explicitly-null one are different instructions to
    // the relay; skipping them keeps a caller from ever sending the second by
    // accident.
    let query = RecordsQueryV1 {
        scope: SCOPE_MINE.to_string(),
        actor_id: Some(101),
        record_kind: "api_request".to_string(),
        from: "2026-07-30T12:00:00.000Z".to_string(),
        to: "2026-07-31T12:00:00.000Z".to_string(),
        limit: 51,
        filter_method: Some("GET".to_string()),
        ..RecordsQueryV1::default()
    };
    let encoded = serde_json::to_value(&query).expect("query encodes");
    let object = encoded.as_object().expect("object");
    assert_eq!(object.get("filter_method"), Some(&serde_json::json!("GET")));
    for absent in [
        "filter_session_id",
        "cursor_event_id",
        "lifecycle_session_id",
    ] {
        assert!(!object.contains_key(absent), "`{absent}` must be omitted");
    }
    let decoded: RecordsQueryV1 = serde_json::from_value(encoded).expect("query decodes");
    assert_eq!(decoded, query);
}
