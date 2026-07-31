//! Scoped-read tests: cross-user isolation, the load-bearing parentheses, keyset
//! paging, and the query-plan proof that the scope predicate is served by an
//! index rather than by post-fetch filtering.

use k8s_openapi::chrono::Duration;

use super::*;
use crate::audit_relay::db::Database;
use crate::audit_relay::protocol::format_instant;
use crate::audit_relay::query::{RecordsQueryV1, ResolvedScope};
use crate::audit_relay::test_support::{
    durable_lifecycle, durable_request, durable_request_in_session, now, open_database,
};

const ALICE: i64 = 101;
const BOB: i64 = 202;

fn query(kind: &str) -> RecordsQueryV1 {
    RecordsQueryV1 {
        scope: "mine".to_string(),
        actor_id: Some(ALICE),
        lifecycle_session_id: None,
        record_kind: kind.to_string(),
        from: format_instant(now() - Duration::hours(24)),
        to: format_instant(now() + Duration::hours(24)),
        limit: 50,
        ..RecordsQueryV1::default()
    }
}

/// The normalized window the HTTP handler would derive from `query`.
fn window_of(query: &RecordsQueryV1) -> ReadWindow {
    let parse = |raw: &str| {
        k8s_openapi::chrono::DateTime::parse_from_rfc3339(raw)
            .expect("a fixture instant parses")
            .with_timezone(&k8s_openapi::chrono::Utc)
    };
    let cursor = match (&query.cursor_timestamp, &query.cursor_event_id) {
        (Some(timestamp), Some(event_id)) => Some((parse(timestamp), event_id.clone())),
        _ => None,
    };
    ReadWindow::new(parse(&query.from), parse(&query.to), cursor)
}

fn mine(session: Option<&str>) -> ResolvedScope {
    ResolvedScope::Mine {
        actor_id: ALICE,
        lifecycle_session_id: session.map(str::to_string),
    }
}

/// Alice's two calls, Bob's one call, one unattributed call, and one lifecycle
/// row in a session they share.
async fn seeded() -> (tempfile::TempDir, Database) {
    let (dir, database) = open_database();
    durable_request(
        &database,
        "a1111111-1111-4111-8111-111111111111",
        Some(ALICE),
    )
    .await;
    durable_request_in_session(
        &database,
        "a2222222-2222-4222-8222-222222222222",
        Some(ALICE),
        "sess-1",
    )
    .await;
    durable_request_in_session(
        &database,
        "b1111111-1111-4111-8111-111111111111",
        Some(BOB),
        "sess-1",
    )
    .await;
    durable_request(&database, "c1111111-1111-4111-8111-111111111111", None).await;
    durable_lifecycle(&database, "d1111111-1111-4111-8111-111111111111", "sess-1").await;
    (dir, database)
}

async fn fetch_ids(
    database: &Database,
    query: RecordsQueryV1,
    scope: ResolvedScope,
) -> Vec<String> {
    let limit = query.limit;
    let built = build(&query, &scope, limit, &window_of(&query));
    database
        .read(move |connection| fetch(connection, &built))
        .await
        .expect("the read runs")
        .into_iter()
        .map(|record| record.event_id)
        .collect()
}

#[tokio::test]
async fn a_regular_viewer_sees_only_their_own_api_rows() {
    let (_dir, database) = seeded().await;
    let ids = fetch_ids(&database, query("api_request"), mine(None)).await;
    assert_eq!(ids.len(), 2, "Alice's two calls and nothing else: {ids:?}");
    assert!(ids.iter().all(|id| id.starts_with('a')));
}

#[tokio::test]
async fn a_shared_session_never_surfaces_another_humans_calls() {
    // The `all` kind adds the SESSION's system rows, and must not widen the
    // actor predicate on the API branch — Bob's call is in the same session.
    let (_dir, database) = seeded().await;
    let mut query = query("all");
    query.lifecycle_session_id = Some("sess-1".to_string());
    let ids = fetch_ids(&database, query, mine(Some("sess-1"))).await;
    assert!(
        ids.contains(&"a2222222-2222-4222-8222-222222222222".to_string()),
        "Alice's own call in the session is visible: {ids:?}"
    );
    assert!(
        ids.contains(&"d1111111-1111-4111-8111-111111111111".to_string()),
        "the session's system lifecycle row is visible: {ids:?}"
    );
    assert!(
        !ids.contains(&"b1111111-1111-4111-8111-111111111111".to_string()),
        "a collaborator's own API call must stay hidden: {ids:?}"
    );
}

#[tokio::test]
async fn an_unattributed_row_is_global_admin_only() {
    let (_dir, database) = seeded().await;
    let personal = fetch_ids(&database, query("api_request"), mine(None)).await;
    assert!(
        !personal.iter().any(|id| id.starts_with('c')),
        "no verified actor, no personal visibility"
    );

    let mut global = query("api_request");
    global.scope = "all".to_string();
    global.actor_id = None;
    let admin = fetch_ids(&database, global, ResolvedScope::All).await;
    assert!(
        admin.iter().any(|id| id.starts_with('c')),
        "an administrator sees unattributed rows: {admin:?}"
    );
    assert_eq!(admin.len(), 4, "all four API rows: {admin:?}");
}

#[tokio::test]
async fn a_personal_lifecycle_query_without_an_authorized_session_matches_nothing() {
    let (_dir, database) = seeded().await;
    let ids = fetch_ids(&database, query("sandbox_lifecycle"), mine(None)).await;
    assert!(
        ids.is_empty(),
        "a missing session predicate must fail closed, not widen: {ids:?}"
    );
}

#[tokio::test]
async fn a_started_row_is_never_returned() {
    let (_dir, database) = open_database();
    crate::audit_relay::test_support::register(&database, "e1111111-1111-4111-8111-111111111111")
        .await;
    let mut global = query("api_request");
    global.scope = "all".to_string();
    global.actor_id = None;
    let ids = fetch_ids(&database, global, ResolvedScope::All).await;
    assert!(
        ids.is_empty(),
        "an in-flight request has no terminal projection to show: {ids:?}"
    );
}

#[tokio::test]
async fn the_page_is_newest_first_and_the_cursor_tiles_without_overlap() {
    let (_dir, database) = open_database();
    for index in 0..5u8 {
        let event_id = format!("f{index}111111-1111-4111-8111-111111111111");
        crate::audit_relay::test_support::register(&database, &event_id).await;
        let mut terminal = crate::audit_relay::test_support::completion(&event_id, Some(ALICE));
        terminal.completed_at = format_instant(now() + Duration::seconds(i64::from(index)));
        terminal.duration_ms =
            u64::try_from(Duration::seconds(i64::from(index)).num_milliseconds()).unwrap_or(0);
        crate::audit_relay::test_support::commit(&database, terminal).await;
    }

    let mut first = query("api_request");
    first.limit = 2;
    let page_one = fetch_ids(&database, first.clone(), mine(None)).await;
    assert_eq!(page_one.len(), 2);
    assert_eq!(page_one[0], "f4111111-1111-4111-8111-111111111111");
    assert_eq!(page_one[1], "f3111111-1111-4111-8111-111111111111");

    let mut second = first;
    second.cursor_timestamp = Some(format_instant(now() + Duration::seconds(3)));
    second.cursor_event_id = Some(page_one[1].clone());
    let page_two = fetch_ids(&database, second, mine(None)).await;
    assert_eq!(page_two.len(), 2);
    assert_eq!(page_two[0], "f2111111-1111-4111-8111-111111111111");
    assert!(
        page_two.iter().all(|id| !page_one.contains(id)),
        "pages must tile without overlap"
    );
}

#[tokio::test]
async fn a_cursor_copied_from_another_viewer_still_only_pages_the_callers_own_rows() {
    // The relay's scope predicate is applied regardless of the cursor, so even a
    // perfectly valid resume point stolen from Bob cannot return Bob's rows.
    let (_dir, database) = seeded().await;
    let mut resumed = query("api_request");
    resumed.cursor_timestamp = Some(format_instant(now() + Duration::hours(1)));
    resumed.cursor_event_id = Some("zzzzzzzz-zzzz-4zzz-8zzz-zzzzzzzzzzzz".to_string());
    let ids = fetch_ids(&database, resumed, mine(None)).await;
    assert!(ids.iter().all(|id| id.starts_with('a')), "{ids:?}");
}

#[tokio::test]
async fn the_time_window_bounds_the_page() {
    let (_dir, database) = seeded().await;
    let mut narrow = query("api_request");
    narrow.from = format_instant(now() + Duration::hours(1));
    narrow.to = format_instant(now() + Duration::hours(2));
    assert!(fetch_ids(&database, narrow, mine(None)).await.is_empty());
}

#[tokio::test]
async fn filters_narrow_inside_the_scope_never_outside_it() {
    let (_dir, database) = seeded().await;
    let mut filtered = query("api_request");
    // Bob's id as an explicit filter cannot widen a personal scope: the scope
    // predicate is ANDed, so the result is empty rather than Bob's row.
    filtered.filter_actor_id = Some(BOB);
    assert!(fetch_ids(&database, filtered, mine(None)).await.is_empty());

    let mut by_method = query("api_request");
    by_method.filter_method = Some("GET".to_string());
    assert_eq!(fetch_ids(&database, by_method, mine(None)).await.len(), 2);

    let mut by_missing_method = query("api_request");
    by_missing_method.filter_method = Some("DELETE".to_string());
    assert!(fetch_ids(&database, by_missing_method, mine(None))
        .await
        .is_empty());
}

#[tokio::test]
async fn the_actor_predicate_is_served_by_its_scoped_index_before_the_limit() {
    // The query-plan proof. If the engine ever stopped using the scoped index,
    // it would be scanning rows the caller may not see and cutting the page from
    // them — which is the exact failure `AUTH-06` forbids.
    let (_dir, database) = seeded().await;
    let built = build(
        &query("api_request"),
        &mine(None),
        50,
        &window_of(&query("api_request")),
    );
    let plan = database
        .read(move |connection| explain(connection, &built))
        .await
        .expect("the plan is available");
    let rendered = plan.join(" | ");
    assert!(
        rendered.contains("audit_records_actor_terminal"),
        "the personal read must seek on the scoped actor index; plan was: {rendered}"
    );
    assert!(
        !rendered.to_ascii_uppercase().contains("SCAN AUDIT_RECORDS"),
        "the personal read must not scan the table; plan was: {rendered}"
    );
}

#[tokio::test]
async fn the_lifecycle_session_predicate_is_served_by_its_scoped_index() {
    let (_dir, database) = seeded().await;
    let mut lifecycle_query = query("sandbox_lifecycle");
    lifecycle_query.lifecycle_session_id = Some("sess-1".to_string());
    let built = build(
        &lifecycle_query,
        &mine(Some("sess-1")),
        50,
        &window_of(&lifecycle_query),
    );
    let plan = database
        .read(move |connection| explain(connection, &built))
        .await
        .expect("the plan is available");
    let rendered = plan.join(" | ");
    assert!(
        rendered.contains("audit_records_session_terminal"),
        "the session read must seek on the scoped session index; plan was: {rendered}"
    );
}

#[test]
fn the_all_branch_keeps_its_load_bearing_parentheses() {
    // Without them `AND … OR …` would bind so the lifecycle branch escaped the
    // time window and the actor predicate.
    let mut all_kinds = query("all");
    all_kinds.lifecycle_session_id = Some("sess-1".to_string());
    let built = build(
        &all_kinds,
        &mine(Some("sess-1")),
        50,
        &window_of(&all_kinds),
    );
    assert!(
        built.sql.contains("((record_kind = ") && built.sql.contains(") OR ("),
        "the union branch must be fully parenthesized: {}",
        built.sql
    );
}

#[test]
fn no_caller_value_is_interpolated_into_the_query_text() {
    let mut hostile = query("api_request");
    hostile.filter_session_id = Some("'; DROP TABLE audit_records; --".to_string());
    hostile.filter_actor_login = Some("' OR 1=1 --".to_string());
    let built = build(&hostile, &mine(None), 50, &window_of(&hostile));
    assert!(
        !built.sql.contains("DROP TABLE") && !built.sql.contains("OR 1=1"),
        "caller content must never reach the query text: {}",
        built.sql
    );
    assert!(built.values.iter().any(|value| matches!(
        value,
        rusqlite::types::Value::Text(text) if text.contains("DROP TABLE")
    )));
}
