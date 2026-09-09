//! Milestone acceptance: the two READ latencies a browser actually waits on.
//!
//! Split from `acceptance_performance.rs` — which owns the WRITE path — for two
//! reasons. The obvious one is size: both halves are dense, and one file would be
//! past this repository's five-hundred-line ceiling. The useful one is that they
//! fail for different reasons and are read by different people: a regression here
//! is a query or a projection getting slower, and its fix lives in
//! `src/operations`, while a regression next door is the audit pipeline getting
//! more expensive and its fix lives in `src/audit`.
//!
//! Both measurements go through the REAL router, so they include identity
//! verification, the scope gate, the visibility projection, the source round
//! trip, the row decode, the merge, and serialization. Timing only the source
//! read — as an earlier version did — measured the cheapest link in the chain and
//! would have stayed green through a quadratic merge.

mod operations_harness;
mod performance_support;
mod sandbox_harness;

use std::time::Instant;

use performance_support::{summarize, Measurement};
use serde_json::json;

/// Fleet sizes the one-pass inventory latency is measured at.
///
/// The upper size is deliberately above the 500-row response ceiling, so the
/// measurement covers the case where authorization and truncation do real work.
const FLEET_SIZES: [usize; 3] = [10, 200, 1_000];

/// The two ACTIVITY query latencies the issue names: a first page, and a
/// filtered session timeline.
///
/// Measured through `GET /api/v1/operations/activity` on the real router, so the
/// number includes identity verification, the scope gate, the visibility
/// projection, the HogQL build, the source round trip, the row decode, the merge,
/// and serialization — which is what a browser waits for. Timing only the relay
/// read, as an earlier version did, measured the cheapest link in that chain.
#[tokio::test]
async fn the_activity_query_latency_is_recorded_and_within_budget() {
    use operations_harness::{dataset::Row, harness, minutes_ago, Sources, ALICE, SESSION};

    // A dataset well past one page, mixing the viewer's own rows, another
    // user's hidden rows, and the shared session's lifecycle rows — the shape
    // the source predicate has to work through on every query.
    let mut rows = Vec::new();
    for index in 0..600 {
        let at = minutes_ago(index % 600);
        rows.push(Row::api(
            &format!("a{index:07}-0000-4000-8000-000000000000"),
            ALICE.0,
            &at,
        ));
        rows.push(Row::api(
            &format!("b{index:07}-0000-4000-8000-000000000000"),
            operations_harness::BOB.0,
            &at,
        ));
        rows.push(Row::lifecycle(
            &format!("c{index:07}-0000-4000-8000-000000000000"),
            SESSION,
            &at,
        ));
    }
    let harness = harness(Sources::Posthog(rows), true).await;

    let first_page = query_profile(&harness, "?scope=mine", 30).await;
    let timeline = query_profile(
        &harness,
        &format!("?scope=mine&session_id={SESSION}&record_kind=all"),
        30,
    )
    .await;

    performance_support::write_artifact(
        "performance-activity.json",
        &[
            Measurement::new("activity_first_page_p95_ms", "milliseconds", first_page.p95),
            Measurement::new(
                "activity_session_timeline_p95_ms",
                "milliseconds",
                timeline.p95,
            ),
        ],
        &json!({ "artifact": "activity query latency" }),
    );

    // Measured ≈ 6 ms p95 for both shapes, debug profile, against a loopback
    // source. The 1 s ceiling is ~150x that: it catches an accidental
    // per-row upstream call or an O(n²) merge, and nothing else.
    assert!(
        first_page.p95 < 1_000.0,
        "the activity first page is {:.1} ms at p95; the budget is 1000 ms",
        first_page.p95
    );
    assert!(
        timeline.p95 < 1_000.0,
        "a filtered session timeline is {:.1} ms at p95; the budget is 1000 ms",
        timeline.p95
    );
}

/// One-pass inventory latency at representative fleet sizes.
///
/// The claim `SBOX-04` makes is that a response costs ONE backend list whatever
/// the fleet size is. Latency that grew super-linearly with the fleet would mean
/// the cost moved somewhere else — into authorization, sorting, or serialization
/// — so the measurement is taken at three sizes and the growth is bounded, rather
/// than a single number being asserted against a guess.
#[tokio::test]
async fn the_one_pass_inventory_latency_is_recorded_for_representative_fleets() {
    use sandbox_harness::{fleet, harness_with, ALICE, SESSION};

    let mut measurements = Vec::new();
    let mut per_row = Vec::new();
    for size in FLEET_SIZES {
        let items: Vec<fleet::Item> = (0..size)
            .map(|index| fleet::item(&format!("rt-{index:05}"), Some(SESSION)))
            .collect();
        let harness = harness_with(items).await;
        // A warm pass first, so the sample is not dominated by first-call setup.
        let _ = harness.snapshot(ALICE, "").await;
        let mut observations = Vec::with_capacity(20);
        for _ in 0..20 {
            let at = Instant::now();
            let _ = harness.snapshot(ALICE, "").await;
            observations.push(at.elapsed().as_secs_f64() * 1_000.0);
        }
        let profile = summarize(observations);
        per_row.push(profile.p95 / size as f64);
        measurements.push(Measurement::new(
            match size {
                10 => "inventory_p95_ms_fleet_10",
                200 => "inventory_p95_ms_fleet_200",
                _ => "inventory_p95_ms_fleet_1000",
            },
            "milliseconds",
            profile.p95,
        ));
    }
    performance_support::write_artifact(
        "performance-inventory.json",
        &measurements,
        &json!({ "artifact": "one-pass inventory latency" }),
    );

    // Per-row cost must not grow with the fleet: that is what "one pass" means
    // in a latency measurement. A 4x tolerance absorbs the fixed per-request
    // cost dominating the smallest fleet.
    let smallest = per_row.first().copied().unwrap_or(f64::MAX);
    let largest = per_row.last().copied().unwrap_or(0.0);
    assert!(
        largest <= smallest * 4.0,
        "per-row inventory cost grew from {smallest:.4} ms to {largest:.4} ms across \
         the fleet sizes; a one-pass read should be flat or cheaper per row"
    );
}
/// Percentiles for one activity query shape.
async fn query_profile(
    harness: &operations_harness::Harness,
    query: &str,
    samples: usize,
) -> performance_support::Profile {
    let _ = harness.page(operations_harness::ALICE, query).await;
    let mut observations = Vec::with_capacity(samples);
    for _ in 0..samples {
        let at = Instant::now();
        let _ = harness.page(operations_harness::ALICE, query).await;
        observations.push(at.elapsed().as_secs_f64() * 1_000.0);
    }
    summarize(observations)
}
