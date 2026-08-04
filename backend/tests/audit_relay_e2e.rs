//! End-to-end tests for the durable audit relay (issue #5678), driven through a
//! REAL HTTP listener so the whole stack — client, protocol, SQLite commit,
//! scoped read — is exercised the way production wires it.
//!
//! What these cover that the unit tests cannot:
//!
//! - **Two control-plane replicas against one relay.** Both hold their own
//!   client, both write concurrently, and one deliberately duplicates the
//!   other's event id. Nothing may be lost, doubled, or corrupted.
//! - **Restart persistence over the wire.** A relay that goes away and comes
//!   back on the same volume must still answer for everything it acknowledged.
//! - **Cross-user isolation end to end.** Alice cannot reach Bob through a direct
//!   filter, through a shared session's `all` timeline, through a pagination
//!   boundary, or with a cursor copied from Bob's page; a global admin sees both
//!   plus the unattributed row.
//! - **Secret canaries.** The write and read tokens must not appear in any
//!   response body or in the database file.

mod audit_relay_harness;

use audit_relay_harness::{Relay, ALICE, BOB, READ_TOKEN, WRITE_TOKEN};
use fkst_control_plane::audit::relay::RelayClientError;

#[tokio::test]
async fn two_replicas_write_concurrently_without_loss_or_duplication() {
    let relay = Relay::start().await;
    let replica_a = relay.client();
    let replica_b = relay.client();
    let shared = "99999999-9999-4999-8999-999999999999";

    let mut tasks = Vec::new();
    for (replica, client) in [(0u8, replica_a.clone()), (1, replica_b.clone())] {
        for index in 0..8u8 {
            let event_id = format!("{replica}{index}111111-1111-4111-8111-111111111111");
            let client = client.clone();
            tasks.push(tokio::spawn(async move {
                client.register_start(&Relay::start_body(&event_id)).await
            }));
        }
        // Both replicas deliberately register the SAME id.
        let client = client.clone();
        let shared = shared.to_string();
        tasks.push(tokio::spawn(async move {
            client.register_start(&Relay::start_body(&shared)).await
        }));
    }
    for task in tasks {
        task.await
            .expect("the task joins")
            .expect("every write is acknowledged");
    }

    // Complete them all, from whichever replica happens to hold the response.
    for replica in 0..2u8 {
        for index in 0..8u8 {
            let event_id = format!("{replica}{index}111111-1111-4111-8111-111111111111");
            replica_a
                .complete(&Relay::completion_body(&event_id, Some(ALICE)))
                .await
                .expect("the completion is acknowledged");
        }
    }
    replica_b
        .complete(&Relay::completion_body(shared, Some(ALICE)))
        .await
        .expect("the shared completion is acknowledged");

    let rows = relay.read_all().await;
    assert_eq!(rows.len(), 17, "16 unique records plus the shared one");
    let mut ids: Vec<String> = rows.iter().map(|row| row.event_id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 17, "no event id may appear twice");
}

#[tokio::test]
async fn an_acknowledged_record_survives_a_relay_restart() {
    let relay = Relay::start().await;
    let event_id = "a1111111-1111-4111-8111-111111111111";
    relay
        .client()
        .register_start(&Relay::start_body(event_id))
        .await
        .expect("the start is acknowledged");
    relay
        .client()
        .complete(&Relay::completion_body(event_id, Some(ALICE)))
        .await
        .expect("the completion is acknowledged");

    let restarted = relay.restart().await;
    let rows = restarted.read_all().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_id, event_id);
    assert_eq!(rows[0].state, "complete");
}

#[tokio::test]
async fn a_replayed_completion_after_a_restart_is_acknowledged_not_duplicated() {
    let relay = Relay::start().await;
    let event_id = "a1111111-1111-4111-8111-111111111111";
    relay
        .client()
        .register_start(&Relay::start_body(event_id))
        .await
        .expect("acknowledged");
    relay
        .client()
        .complete(&Relay::completion_body(event_id, Some(ALICE)))
        .await
        .expect("acknowledged");

    let restarted = relay.restart().await;
    restarted
        .client()
        .complete(&Relay::completion_body(event_id, Some(ALICE)))
        .await
        .expect("an exact replay is acknowledged");
    assert_eq!(restarted.read_all().await.len(), 1);

    // A DIFFERENT terminal for the same id must be refused, never overwrite.
    let mut divergent = Relay::completion_body(event_id, Some(ALICE));
    divergent.status_code = Some(500);
    divergent.outcome = "server_error".to_string();
    let error = restarted
        .client()
        .complete(&divergent)
        .await
        .expect_err("history is append-only");
    assert_eq!(error, RelayClientError::Conflict);
}

#[tokio::test]
async fn alice_cannot_reach_bob_through_any_read_shape() {
    let relay = Relay::start().await;
    relay.seed_cross_user_fixture().await;

    // 1. A direct personal read.
    let alice_rows = relay
        .read_personal(ALICE, None, "api_request", 50, None)
        .await;
    let alice_ids: Vec<&str> = alice_rows.iter().map(|row| row.event_id.as_str()).collect();
    assert!(
        alice_ids.iter().all(|id| id.starts_with('a')),
        "Alice's personal read returned {alice_ids:?}"
    );

    // 2. A shared session's `all` timeline: system rows yes, Bob's calls no.
    let shared = relay
        .read_personal(ALICE, Some("sess-1"), "all", 50, None)
        .await;
    let shared_ids: Vec<&str> = shared.iter().map(|row| row.event_id.as_str()).collect();
    assert!(
        shared_ids.contains(&"d1111111-1111-4111-8111-111111111111"),
        "the session's system lifecycle row is visible: {shared_ids:?}"
    );
    assert!(
        !shared_ids.iter().any(|id| id.starts_with('b')),
        "a collaborator's own calls must stay hidden: {shared_ids:?}"
    );

    // 3. A pagination boundary: every page, to exhaustion, is still only Alice's.
    let mut cursor: Option<(String, String)> = None;
    let mut seen = 0usize;
    loop {
        let page = relay
            .read_personal(ALICE, None, "api_request", 1, cursor.clone())
            .await;
        if page.is_empty() {
            break;
        }
        assert!(page.iter().all(|row| row.event_id.starts_with('a')));
        seen += page.len();
        let last = page.last().expect("a non-empty page");
        cursor = Some((last.sort_timestamp.clone(), last.event_id.clone()));
        assert!(seen <= 4, "the fixture has fewer rows than this");
    }
    assert_eq!(seen, 2, "Alice has exactly two API rows");

    // 4. A cursor copied from Bob's page.
    let bob_rows = relay
        .read_personal(BOB, None, "api_request", 50, None)
        .await;
    let bobs_cursor = bob_rows
        .first()
        .map(|row| (row.sort_timestamp.clone(), row.event_id.clone()))
        .expect("Bob has a row");
    let stolen = relay
        .read_personal(ALICE, None, "api_request", 50, Some(bobs_cursor))
        .await;
    assert!(
        stolen.iter().all(|row| row.event_id.starts_with('a')),
        "a stolen cursor cannot widen a scope"
    );
}

#[tokio::test]
async fn a_global_admin_sees_both_users_plus_the_unattributed_row() {
    let relay = Relay::start().await;
    relay.seed_cross_user_fixture().await;
    let rows = relay.read_all().await;
    let ids: Vec<&str> = rows.iter().map(|row| row.event_id.as_str()).collect();
    assert!(ids.iter().any(|id| id.starts_with('a')), "{ids:?}");
    assert!(ids.iter().any(|id| id.starts_with('b')), "{ids:?}");
    assert!(
        ids.iter().any(|id| id.starts_with('c')),
        "the unattributed row is global-admin-only, and an admin must see it: {ids:?}"
    );
}

#[tokio::test]
async fn a_read_token_cannot_write_and_a_write_token_cannot_read() {
    let relay = Relay::start().await;
    // The client keeps them apart by construction; this proves the RELAY does
    // too, so a mis-wired caller is refused rather than trusted.
    assert_eq!(
        relay.raw_write_with(READ_TOKEN).await,
        axum::http::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        relay.raw_read_with(WRITE_TOKEN).await,
        axum::http::StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn no_credential_canary_reaches_a_response_or_the_database_file() {
    let relay = Relay::start().await;
    relay.seed_cross_user_fixture().await;
    let rendered = serde_json::to_string(&relay.read_all().await).expect("rows encode");
    for canary in [WRITE_TOKEN, READ_TOKEN] {
        assert!(
            !rendered.contains(canary),
            "`{canary}` must not appear in a relay response"
        );
    }
    let stored = relay.database_bytes();
    for canary in [WRITE_TOKEN, READ_TOKEN] {
        assert!(
            !stored
                .windows(canary.len())
                .any(|window| window == canary.as_bytes()),
            "`{canary}` must not appear in the relay's database file"
        );
    }
}
