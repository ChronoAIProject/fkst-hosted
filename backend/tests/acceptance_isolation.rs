//! Milestone acceptance: hidden records may not leave a shadow.
//!
//! The sibling suites already prove that a hidden row is not RETURNED. This one
//! asks the harder question the epic's `AUTH-06` actually poses: can a caller
//! *infer* the hidden population from the shape of what they do receive?
//!
//! The attack surface is the page's own metadata. If authorization ran after the
//! source limit, hidden rows would consume page slots and the visible page would
//! come back short; if the cursor were derived before filtering, the next page
//! would start in the wrong place; if warnings were projected from the whole
//! fleet, a flood of hidden problems would change a visible caller's codes. Each
//! of those is a side channel, and each is invisible to a test that only checks
//! which rows came back.
//!
//! The method is differential: run the same request against two datasets that
//! differ ONLY in rows the caller cannot see, and assert the two answers are
//! indistinguishable. Hidden rows are deliberately placed BEFORE, BETWEEN, and
//! AFTER the visible ones in the sort order, because a bug that drops the first
//! `n` rows and a bug that truncates at the end have different signatures.

mod operations_harness;
#[path = "audit_relay_harness/mod.rs"]
mod relay;
mod sandbox_harness;

use operations_harness::{harness, item_ids, minutes_ago, Row, Sources, ALICE, BOB};
use sandbox_harness::{fleet, harness_with, HarnessSpec};
use serde_json::Value;

/// One frozen clock for a whole test.
///
/// Every instant a test uses is derived from ONE reading. The datasets are
/// compared across two harness constructions, and `minutes_ago` reads the wall
/// clock on every call, so re-deriving a fixture's timestamps per run would make
/// the two runs differ by a few milliseconds — and the cursor, which binds the
/// resolved window, would legitimately differ with them.
struct Clock {
    at: Vec<String>,
}

impl Clock {
    fn new() -> Self {
        Self {
            at: (0..=180).map(minutes_ago).collect(),
        }
    }

    fn at(&self, minutes: usize) -> &str {
        &self.at[minutes]
    }

    /// An explicit, shared query window wide enough for every fixture instant.
    fn window(&self) -> String {
        format!(
            "&from={}&to={}",
            urlencode(self.at(180)),
            urlencode(self.at(0))
        )
    }
}

/// Percent-encode the two characters an RFC3339 instant carries that a query
/// string would otherwise re-interpret.
fn urlencode(instant: &str) -> String {
    instant.replace(':', "%3A").replace('+', "%2B")
}

/// Alice's four visible calls, minute-spaced so the keyset order is total.
fn visible_only(clock: &Clock) -> Vec<Row> {
    vec![
        Row::api("a1", ALICE.0, clock.at(10)),
        Row::api("a2", ALICE.0, clock.at(20)),
        Row::api("a3", ALICE.0, clock.at(30)),
        Row::api("a4", ALICE.0, clock.at(40)),
    ]
}

/// The same four calls, with Bob's and an anonymous caller's rows interleaved
/// before, between, and after them — plus one sharing an instant with one of
/// Alice's, which is where a naive keyset implementation loses a row.
fn visible_and_hidden(clock: &Clock) -> Vec<Row> {
    let mut rows = vec![
        // BEFORE the newest visible row.
        Row::api("b0", BOB.0, clock.at(5)),
        Row::anonymous("x0", clock.at(6)),
    ];
    rows.extend(visible_only(clock));
    rows.extend([
        // BETWEEN, including one sharing Alice's exact instant.
        Row::api("b1", BOB.0, clock.at(10)),
        Row::api("b2", BOB.0, clock.at(15)),
        Row::anonymous("x1", clock.at(25)),
        Row::api("b3", BOB.0, clock.at(35)),
        // AFTER the oldest visible row.
        Row::api("b4", BOB.0, clock.at(50)),
        Row::anonymous("x2", clock.at(60)),
    ]);
    rows
}

/// The fields a caller can observe that must not move when hidden rows change.
fn observable(page: &Value) -> Value {
    serde_json::json!({
        "items": item_ids(page),
        "next_cursor": page["next_cursor"],
        "partial": page["partial"],
        "sources": page["sources"],
        "warnings": page["warnings"],
        "row_errors": page["row_errors"],
    })
}

/// Two pages of two, over both datasets, compared field by field.
#[tokio::test]
async fn hidden_rows_before_between_and_after_change_no_page_boundary() {
    let clock = Clock::new();
    let window = clock.window();
    let mut observed = Vec::new();
    for rows in [visible_only(&clock), visible_and_hidden(&clock)] {
        let harness = harness(Sources::Posthog(rows), true).await;
        let first = harness.page(ALICE, &format!("?limit=2{window}")).await;
        let cursor = first["next_cursor"]
            .as_str()
            .expect("a full page hands back a cursor")
            .to_string();
        let second = harness
            .page(ALICE, &format!("?limit=2{window}&cursor={cursor}"))
            .await;
        observed.push((observable(&first), cursor, observable(&second)));
    }

    let (clean_first, clean_cursor, clean_second) = &observed[0];
    let (mixed_first, mixed_cursor, mixed_second) = &observed[1];
    assert_eq!(
        clean_first, mixed_first,
        "the first page's rows, cursor, and metadata moved when only HIDDEN rows changed"
    );
    assert_eq!(
        clean_cursor, mixed_cursor,
        "the cursor is derived from hidden rows"
    );
    assert_eq!(
        clean_second, mixed_second,
        "the second page's boundary moved when only HIDDEN rows changed"
    );
    // The positive half: the isolation is not achieved by returning nothing.
    assert_eq!(
        clean_first["items"],
        serde_json::json!(["a1", "a2"]),
        "the visible rows themselves must still be served, newest first"
    );
    assert_eq!(clean_second["items"], serde_json::json!(["a3", "a4"]));
}

/// The same differential one layer down, in the relay's indexed SQL.
///
/// PostHog and the relay apply the viewer constraint independently, so a
/// regression in either is its own bug. This runs against a REAL relay over a
/// real SQLite file: two databases seeded identically except for rows belonging
/// to Bob and to nobody, interleaved before, between, and after Alice's.
#[tokio::test]
async fn a_hidden_row_never_shifts_a_relay_page_or_its_cursor() {
    // (event id, actor, seconds after the anchor). The instants are explicit so
    // the two databases order identically; the hidden rows are deliberately
    // adjacent to visible ones in that order.
    const VISIBLE: [(&str, i64, i64); 4] = [
        ("a1111111-1111-4111-8111-111111111111", relay::ALICE, 10),
        ("a2222222-2222-4222-8222-222222222222", relay::ALICE, 20),
        ("a3333333-3333-4333-8333-333333333333", relay::ALICE, 30),
        ("a4444444-4444-4444-8444-444444444444", relay::ALICE, 40),
    ];
    const HIDDEN: [(&str, Option<i64>, i64); 5] = [
        ("b0000000-0000-4000-8000-000000000000", Some(relay::BOB), 5),
        ("b1111111-1111-4111-8111-111111111111", Some(relay::BOB), 20),
        ("b2222222-2222-4222-8222-222222222222", Some(relay::BOB), 25),
        ("c1111111-1111-4111-8111-111111111111", None, 35),
        ("c2222222-2222-4222-8222-222222222222", None, 50),
    ];

    let mut observed = Vec::new();
    for include_hidden in [false, true] {
        let node = relay::Relay::start().await;
        let mut plan: Vec<(&str, Option<i64>, i64)> = VISIBLE
            .iter()
            .map(|(id, actor, offset)| (*id, Some(*actor), *offset))
            .collect();
        if include_hidden {
            plan.extend(HIDDEN);
        }
        for (event_id, actor_id, offset) in plan {
            seed_relay_row(&node, event_id, actor_id, offset).await;
        }

        // Two pages of two, keyset-paged exactly as the activity source does.
        let first = node
            .read_personal(relay::ALICE, None, "api_request", 2, None)
            .await;
        let boundary = first.last().expect("a full page").clone();
        let second = node
            .read_personal(
                relay::ALICE,
                None,
                "api_request",
                2,
                Some((boundary.sort_timestamp.clone(), boundary.event_id.clone())),
            )
            .await;
        observed.push((
            relay_ids(&first),
            boundary.sort_timestamp,
            relay_ids(&second),
        ));
    }

    assert_eq!(
        observed[0], observed[1],
        "a row Alice cannot see changed her relay page, its boundary, or the next page"
    );
    // Newest first: the 40s row is the most recent.
    assert_eq!(
        observed[0].0,
        vec![
            "a4444444-4444-4444-8444-444444444444",
            "a3333333-3333-4333-8333-333333333333"
        ],
        "the relay must still serve Alice's own rows, newest first"
    );
    assert_eq!(
        observed[0].2.len(),
        2,
        "the second page must tile, not repeat"
    );
}

/// Commit one start plus one completion at `offset` seconds after the anchor.
async fn seed_relay_row(
    node: &relay::Relay,
    event_id: &str,
    actor_id: Option<i64>,
    offset_secs: i64,
) {
    let client = node.client();
    client
        .register_start(&relay::Relay::start_body(event_id))
        .await
        .expect("the start is acknowledged");
    let completed_at = relay::anchor() + k8s_openapi::chrono::Duration::seconds(offset_secs);
    let mut completion = relay::Relay::completion_body(event_id, actor_id);
    completion.completed_at =
        fkst_control_plane::audit_relay::protocol::format_instant(completed_at);
    completion.duration_ms = u64::try_from(offset_secs * 1_000).unwrap_or(0);
    client
        .complete(&completion)
        .await
        .expect("the completion is acknowledged");
}

fn relay_ids(rows: &[fkst_control_plane::audit_relay::query::RecordRowV1]) -> Vec<String> {
    rows.iter().map(|row| row.event_id.clone()).collect()
}

/// The sandbox half: mutate every hidden runtime and prove the authorized
/// response is byte-identical, including its `item_count` and warning codes.
#[tokio::test]
async fn a_mutated_hidden_fleet_leaves_the_regular_response_byte_equivalent() {
    let visible = || fleet::item("rt-alice", Some(sandbox_harness::SESSION));

    let quiet = vec![visible()];
    let noisy = vec![
        // A hidden row sorted BEFORE the visible one (newer).
        fleet::with_created("rt-strangerA", Some(sandbox_harness::OTHER_SESSION), 1),
        visible(),
        // ... and several sorted AFTER it, in every attribution state that only a
        // global administrator may see.
        fleet::with_created("rt-strangerB", Some(sandbox_harness::OTHER_SESSION), 90),
        fleet::orphan("rt-orphan"),
        fleet::malformed("rt-malformed"),
        fleet::conflicted("rt-conflict", Some(sandbox_harness::OTHER_SESSION)),
    ];

    let quiet_bytes = harness_with(quiet)
        .await
        .snapshot_bytes(sandbox_harness::ALICE, "")
        .await;
    let noisy_harness = harness_with(noisy).await;
    let noisy_bytes = noisy_harness
        .snapshot_bytes(sandbox_harness::ALICE, "")
        .await;
    assert_eq!(
        String::from_utf8_lossy(&quiet_bytes),
        String::from_utf8_lossy(&noisy_bytes),
        "a hidden runtime changed an authorized caller's response"
    );

    // Positive control: the very rows that changed nothing for Alice are exactly
    // what a global administrator receives, so the equality above is isolation
    // rather than a backend that silently dropped them.
    let admin = noisy_harness
        .snapshot(sandbox_harness::GRACE, "?scope=all")
        .await;
    assert_eq!(
        sandbox_harness::item_ids(&admin).len(),
        6,
        "the administrator must still see the whole fleet: {admin}"
    );
    assert_eq!(noisy_harness.inventory_calls(), 2, "one list per request");
    assert_eq!(noisy_harness.forbidden_calls(), 0);
}

/// A hidden population large enough to exhaust every ceiling must still not
/// change what an authorized caller receives — the ceilings are applied to the
/// authorized set, not the raw fleet.
#[tokio::test]
async fn a_hidden_population_cannot_exhaust_an_authorized_callers_ceiling() {
    let mut items = vec![fleet::item("rt-alice", Some(sandbox_harness::SESSION))];
    for index in 0..64 {
        items.push(fleet::with_created(
            &format!("rt-hidden-{index}"),
            Some(sandbox_harness::OTHER_SESSION),
            index + 1,
        ));
    }
    // A result ceiling of two: comfortably above Alice's one authorized row and
    // far below the 65-row fleet.
    let harness =
        sandbox_harness::harness(HarnessSpec::new(fleet::snapshot(items)).max_result_items(2))
            .await;
    let page = harness.snapshot(sandbox_harness::ALICE, "").await;
    assert_eq!(sandbox_harness::item_ids(&page), vec!["rt-alice"]);
    assert_eq!(page["item_count"], 1);
}
