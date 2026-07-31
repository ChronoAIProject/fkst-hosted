//! The `/api/v1/operations/activity` authorization matrix, driven end to end
//! through the REAL router.
//!
//! Every assertion here is about something a client can observe: the status, the
//! stable error code, the items, and whether the deployment paid for an upstream
//! call at all. A refusal that still queried PostHog would be a refusal the
//! source could have leaked around, so "zero source calls" is asserted alongside
//! the status wherever the spec demands it.

mod operations_harness;

use axum::http::StatusCode;
use operations_harness::{
    error_code, harness, item_ids, minutes_ago, Row, Sources, ALICE, BOB, CAROL, DANA, ERIN,
    OTHER_SESSION, REPO_ADMIN, ROOT, SESSION,
};

/// The shared dataset: two humans' calls, a system lifecycle row, an anonymous
/// row, and a row for a session nobody in these fixtures may see.
fn dataset() -> Vec<Row> {
    vec![
        Row::api("ev-alice-1", ALICE.0, &minutes_ago(5)),
        Row::api("ev-alice-2", ALICE.0, &minutes_ago(15)).in_session(SESSION),
        Row::api("ev-bob-1", BOB.0, &minutes_ago(10)).in_session(SESSION),
        Row::lifecycle("ev-life-1", SESSION, &minutes_ago(20)),
        Row::lifecycle("ev-life-other", OTHER_SESSION, &minutes_ago(21)),
        Row::anonymous("ev-anon-1", &minutes_ago(25)),
    ]
}

#[tokio::test]
async fn a_missing_or_invalid_identity_is_unauthorized() {
    let harness = harness(Sources::Posthog(dataset()), true).await;

    let anonymous = harness.request(None, "").await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let bogus = harness.request(Some((0, "nobody")), "").await;
    assert_eq!(bogus.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        harness.source_calls().await,
        0,
        "an unauthenticated request must never reach the activity source"
    );
}

/// Identity is the FIRST gate, ahead of parameter parsing. A caller with no
/// token and a type-level query error gets `401`, not a `400` that would let the
/// parameter grammar be probed without ever presenting a credential.
#[tokio::test]
async fn identity_is_checked_before_the_parameters_are_parsed() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    for query in [
        "?limit=abc",
        "?actor_id=not-a-number",
        "?trigger_issue=x",
        "?status_code=foo",
        "?scope=everyone",
    ] {
        let response = harness.request(None, query).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{query}");
    }
    // The SAME queries are a `400` once an admitted identity is proven, so the
    // ordering costs no validation fidelity.
    for query in ["?limit=abc", "?scope=everyone"] {
        let response = harness.get(ALICE, query).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
    }
    assert_eq!(harness.source_calls().await, 0);
}

#[tokio::test]
async fn an_admitted_regular_user_sees_only_their_own_api_rows() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let page = harness.page(ALICE, "").await;
    assert_eq!(page["effective_scope"], "mine");
    assert_eq!(page["can_view_all"], false);
    assert_eq!(item_ids(&page), vec!["ev-alice-1", "ev-alice-2"]);
    assert_eq!(page["source_status"]["posthog"], "healthy");
    assert_eq!(page["source_status"]["relay"], "not_configured");
    assert_eq!(page["source_status"]["partial"], false);
    assert!(page.get("total").is_none(), "no total count is returned");

    // The identity and correlation keys `AUD-05` names must actually reach the
    // body. A field the fixed SELECT never asks for, or that capture only ever
    // wrote inside a nested object, is documented in the schema and absent from
    // every row a user sees — which is worse than not offering it.
    let item = &page["items"][0];
    assert_eq!(item["principal"]["id"], "github_user_token");
    assert_eq!(item["correlation"]["webhook_delivery_id"], "d-9f3a");
}

/// Two collaborators on one session each see only their OWN calls.
#[tokio::test]
async fn a_collaborator_never_sees_the_other_collaborators_api_rows() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let alice = harness.page(ALICE, "").await;
    let bob = harness.page(BOB, "").await;
    assert_eq!(item_ids(&alice), vec!["ev-alice-1", "ev-alice-2"]);
    assert_eq!(item_ids(&bob), vec!["ev-bob-1"]);
}

/// Ownership is the verified actor id and nothing else: an unattributed row is
/// global-admin-only because nobody can prove it is theirs.
#[tokio::test]
async fn anonymous_and_other_actors_rows_are_absent_from_a_personal_page() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let page = harness.page(ALICE, "").await;
    let ids = item_ids(&page);
    assert!(!ids.iter().any(|id| id.starts_with("ev-anon")), "{ids:?}");
    assert!(!ids.iter().any(|id| id.starts_with("ev-bob")), "{ids:?}");
}

#[tokio::test]
async fn a_global_admin_sees_every_actor_plus_system_and_anonymous_rows() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let page = harness.page(ROOT, "?record_kind=all").await;
    assert_eq!(page["effective_scope"], "all");
    assert_eq!(page["can_view_all"], true);
    let ids = item_ids(&page);
    for expected in [
        "ev-alice-1",
        "ev-alice-2",
        "ev-bob-1",
        "ev-anon-1",
        "ev-life-1",
        "ev-life-other",
    ] {
        assert!(ids.contains(&expected.to_string()), "{ids:?}");
    }
}

/// An administrator may deliberately exercise the personal semantics.
#[tokio::test]
async fn a_global_admin_selecting_mine_gets_the_same_own_actor_isolation() {
    let mut rows = dataset();
    rows.push(Row::api("ev-root-1", ROOT.0, &minutes_ago(1)));
    let harness = harness(Sources::Posthog(rows), true).await;
    let page = harness.page(ROOT, "?scope=mine").await;
    assert_eq!(page["effective_scope"], "mine");
    assert_eq!(
        page["can_view_all"], true,
        "the role is a server fact the UI labels controls with; it never widens \
         the scope that actually applied"
    );
    assert_eq!(item_ids(&page), vec!["ev-root-1"]);
}

#[tokio::test]
async fn a_regular_user_cannot_select_the_global_scope() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let response = harness.get(ALICE, "?scope=all").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(error_code(response).await, "operations_scope_forbidden");
    assert_eq!(harness.source_calls().await, 0);
}

/// `actor_id`/`actor_login` are forbidden in personal scope EVEN WHEN they equal
/// the caller: the server owns the identity predicate, so a client-supplied one
/// is always ambiguous authority.
#[tokio::test]
async fn a_regular_user_cannot_supply_an_actor_filter_even_for_themselves() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    for query in [
        "?actor_id=101",
        "?actor_login=alice",
        "?actor_id=102",
        "?actor_login=bob",
        "?scope=mine&actor_id=101",
    ] {
        let response = harness.get(ALICE, query).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{query}");
        assert_eq!(error_code(response).await, "operations_scope_forbidden");
    }
    assert_eq!(harness.source_calls().await, 0);
}

#[tokio::test]
async fn a_global_admin_may_narrow_by_actor() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let page = harness.page(ROOT, "?actor_id=101").await;
    assert_eq!(page["effective_scope"], "all");
    // The filter is narrowing only; authorization is the admin's global scope.
    assert!(!item_ids(&page).is_empty());
}

/// A regular caller asking for lifecycle rows must name ONE exact authorized
/// session. All four allowing tiers are exercised through the ROUTE, not only
/// through the pure policy: creator, collaborator, per-session log grantee, and
/// the deployment-wide legacy log admin.
#[tokio::test]
async fn every_allowing_tier_can_read_the_sessions_lifecycle_rows() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    for who in [ALICE, BOB, CAROL, DANA] {
        let page = harness
            .page(
                who,
                &format!("?record_kind=sandbox_lifecycle&session_id={SESSION}"),
            )
            .await;
        assert_eq!(item_ids(&page), vec!["ev-life-1"], "{}", who.1);
    }
}

/// A log grant is a LIFECYCLE tier, not a licence to read the session's humans.
/// Carol's own timeline for the session is empty because she made no calls —
/// the grant never surfaces Alice's or Bob's API-request rows.
#[tokio::test]
async fn a_log_grantee_gains_lifecycle_rows_but_no_other_humans_api_rows() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let page = harness
        .page(CAROL, &format!("?record_kind=all&session_id={SESSION}"))
        .await;
    assert_eq!(
        item_ids(&page),
        vec!["ev-life-1"],
        "shared session access adds lifecycle rows only"
    );
}

/// The timeline of an authorized session: the caller's OWN calls plus the
/// system's lifecycle rows, never another human's calls.
#[tokio::test]
async fn an_authorized_session_timeline_adds_lifecycle_rows_but_no_foreign_calls() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let alice = harness
        .page(ALICE, &format!("?record_kind=all&session_id={SESSION}"))
        .await;
    assert_eq!(item_ids(&alice), vec!["ev-alice-2", "ev-life-1"]);

    let bob = harness
        .page(BOB, &format!("?record_kind=all&session_id={SESSION}"))
        .await;
    assert_eq!(
        item_ids(&bob),
        vec!["ev-bob-1", "ev-life-1"],
        "a shared session adds lifecycle rows only"
    );
}

/// Unauthorized, nonexistent, and absent session ids are indistinguishable.
#[tokio::test]
async fn an_unauthorized_missing_or_unknown_lifecycle_session_is_one_stable_404() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let mut bodies = Vec::new();
    for query in [
        format!("?record_kind=sandbox_lifecycle&session_id={SESSION}"),
        format!("?record_kind=sandbox_lifecycle&session_id={OTHER_SESSION}"),
        "?record_kind=sandbox_lifecycle&session_id=sess-does-not-exist".to_string(),
        "?record_kind=sandbox_lifecycle".to_string(),
        "?record_kind=all".to_string(),
    ] {
        let response = harness.get(ERIN, &query).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{query}");
        bodies.push(operations_harness::body_json(response).await);
    }
    for body in &bodies {
        assert_eq!(body["error"], "activity_session_not_found");
        assert_eq!(
            body, &bodies[0],
            "every unresolvable session must answer identically or the endpoint \
             becomes a session-existence oracle"
        );
    }
    assert_eq!(harness.source_calls().await, 0);
}

/// Repository role is not a session tier. An unrelated verified user and the
/// repository's own owner/admin get the SAME `404` — the policy is pure and
/// never looks a repository role up, so there is nothing for an admin of
/// `acme/site` to inherit here.
#[tokio::test]
async fn neither_an_unrelated_user_nor_a_repository_admin_reaches_the_lifecycle_rows() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    for who in [ERIN, REPO_ADMIN] {
        let response = harness
            .get(who, &format!("?record_kind=all&session_id={SESSION}"))
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{}", who.1);
        assert_eq!(
            error_code(response).await,
            "activity_session_not_found",
            "{}",
            who.1
        );
    }
}

/// A regular caller's non-lifecycle history must not depend on the session
/// registry at all.
#[tokio::test]
async fn a_cold_session_registry_only_affects_lifecycle_scoped_requests() {
    let harness = harness(Sources::Posthog(dataset()), false).await;

    let personal = harness.page(ALICE, "").await;
    assert_eq!(
        item_ids(&personal),
        vec!["ev-alice-1", "ev-alice-2"],
        "personal API-request history needs no session authorization"
    );

    let lifecycle = harness
        .get(ALICE, &format!("?record_kind=all&session_id={SESSION}"))
        .await;
    assert_eq!(lifecycle.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        error_code(lifecycle).await,
        "session_visibility_unavailable",
        "a cold projection fails closed rather than answering a misleading empty"
    );

    // A global admin's all-history does not consult the registry either.
    let admin = harness.page(ROOT, "?record_kind=all").await;
    assert!(!item_ids(&admin).is_empty());
}

#[tokio::test]
async fn a_deployment_without_query_credentials_answers_a_stable_503() {
    let harness = harness(Sources::None, true).await;
    let response = harness.get(ALICE, "").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(response).await, "audit_query_not_configured");
}

#[tokio::test]
async fn every_malformed_parameter_is_a_four_hundred_before_any_source_call() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    for query in [
        "?scope=everyone",
        "?record_kind=everything",
        "?from=yesterday",
        "?from=2026-01-01T00:00:00Z&to=2020-01-01T00:00:00Z",
        "?from=2020-01-01T00:00:00Z&to=2026-01-01T00:00:00Z",
        "?limit=0",
        "?limit=100000",
        "?operation_id=no_such_operation",
        "?method=TRACE",
        "?status_code=42",
        "?status_class=6xx",
        "?outcome=fine",
        "?session_id=not%20a%20session",
        "?repo_full_name=acme",
        "?trigger_issue=0",
        "?request_id=req%20id",
    ] {
        let response = harness.get(ALICE, query).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(error_code(response).await, "invalid_request", "{query}");
    }
    assert_eq!(harness.source_calls().await, 0);
}

#[tokio::test]
async fn the_default_and_custom_ranges_are_echoed_on_the_page() {
    let harness = harness(Sources::Posthog(dataset()), true).await;

    let default = harness.page(ALICE, "").await;
    let from = default["from"].as_str().expect("from");
    let to = default["to"].as_str().expect("to");
    assert!(from < to, "{from} .. {to}");
    assert!(default["queried_at"].as_str().is_some());

    let explicit = harness
        .page(ALICE, "?from=2026-07-30T00:00:00Z&to=2026-07-30T12:00:00Z")
        .await;
    assert_eq!(explicit["from"], "2026-07-30T00:00:00.000Z");
    assert_eq!(explicit["to"], "2026-07-30T12:00:00.000Z");
}

#[tokio::test]
async fn every_fixed_filter_is_accepted_and_reaches_the_source() {
    let harness = harness(Sources::Posthog(dataset()), true).await;
    let page = harness
        .page(
            ALICE,
            "?method=GET&status_code=200&status_class=2xx&outcome=success\
             &operation_id=canvas_overview&repo_full_name=acme/site&trigger_issue=7\
             &request_id=req-0001&session_id=sess-alice&limit=5",
        )
        .await;
    assert_eq!(page["effective_scope"], "mine");
    let text = harness.last_query_text().await;
    for placeholder in [
        "{filter_method}",
        "{filter_status_code}",
        "{filter_status_low}",
        "{filter_outcome}",
        "{filter_operation_id}",
        "{filter_repo_full_name}",
        "{filter_trigger_issue}",
        "{filter_request_id}",
        "{filter_session_id}",
    ] {
        assert!(
            text.contains(placeholder),
            "{placeholder} missing from {text}"
        );
    }
}
