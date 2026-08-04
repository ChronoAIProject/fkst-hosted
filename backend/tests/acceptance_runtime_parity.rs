//! Milestone acceptance: cross-adapter parity and the one-list cost, asserted at
//! the boundaries the sibling suites cannot reach.
//!
//! The adapter suites each prove their own projection. Neither can prove the
//! property the epic's `SBOX-03` actually states — that two adapters given
//! EQUIVALENT facts produce the same normalized row, and differ only where the
//! backends genuinely differ. That comparison has to happen after both rows have
//! been serialized by the public API, because the public shape is where a
//! divergence would reach a user.
//!
//! The second test covers `SBOX-04` for OpenSandbox at the transport level: the
//! inventory walk is a paginated LIST and nothing else. The Kubernetes twin is
//! `inventory_live_tests::the_inventory_read_costs_exactly_one_list_and_no_per_pod_get`;
//! this is deliberately the same claim made against a real HTTP server, because
//! OpenSandbox's per-item read is one URL away and a refactor could reach for it
//! without any type-level signal.

mod sandbox_harness;

use sandbox_harness::{fleet, harness_with};
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The fields the two backends legitimately disagree on.
///
/// Each entry is a deliberate difference recorded in the epic, not an accident:
/// `backend` and `backend_location` name the runtime substrate; `runtime_name`
/// and `runtime_uid` are Kubernetes object identity that OpenSandbox has no
/// analogue for; `restart_count` is unsupported (and must be `null`, NOT `0`,
/// which would assert "never restarted"); `deletion_timestamp` requires the
/// Kubernetes graceful-deletion window.
const INTENTIONAL_DIFFERENCES: [&str; 6] = [
    "backend",
    "backend_location",
    "runtime_name",
    "runtime_uid",
    "restart_count",
    "deletion_timestamp",
];

/// Equivalent facts in, equivalent normalized row out.
#[tokio::test]
async fn both_adapters_normalize_one_equivalent_fact_set_identically() {
    let session = Some(sandbox_harness::SESSION);

    // The Kubernetes row carries a deletion timestamp so the graceful-deletion
    // window is genuinely exercised as a difference rather than being absent on
    // both sides, which would make the comparison vacuous for that field.
    let k8s_item = fleet::Item {
        deletion_timestamp: Some(fleet::observed_at()),
        ..fleet::item("rt-1", session)
    };
    let k8s = harness_with(vec![k8s_item]).await;
    let k8s_row = one_item(&k8s.snapshot(sandbox_harness::ALICE, "").await);

    let osb = sandbox_harness::harness(
        sandbox_harness::HarnessSpec::new(fleet::snapshot(vec![fleet::opensandbox(
            "rt-1", session,
        )]))
        .opensandbox(),
    )
    .await;
    let osb_row = one_item(&osb.snapshot(sandbox_harness::ALICE, "").await);

    // Same key set: a field present on one adapter and absent on the other would
    // make the UI branch on backend, which is exactly what normalization is for.
    let k8s_keys: Vec<&String> = k8s_row.as_object().expect("object").keys().collect();
    let osb_keys: Vec<&String> = osb_row.as_object().expect("object").keys().collect();
    assert_eq!(
        k8s_keys, osb_keys,
        "the two adapters expose different fields"
    );

    let mut differing = Vec::new();
    for key in k8s_keys {
        if k8s_row[key] != osb_row[key] {
            differing.push(key.as_str());
        }
    }
    differing.sort_unstable();
    let mut expected = INTENTIONAL_DIFFERENCES.to_vec();
    expected.sort_unstable();
    assert_eq!(
        differing, expected,
        "the adapters diverge somewhere other than their documented differences"
    );

    // And the documented differences carry the documented VALUES, so this cannot
    // pass by both adapters emitting nulls everywhere.
    assert_eq!(k8s_row["backend"], "kubernetes");
    assert_eq!(osb_row["backend"], "opensandbox");
    assert_eq!(k8s_row["restart_count"], 0);
    assert_eq!(
        osb_row["restart_count"],
        Value::Null,
        "an unsupported restart count must be null, never a zero that reads as \
         'never restarted'"
    );
    assert_eq!(osb_row["runtime_name"], Value::Null);
    assert_eq!(osb_row["runtime_uid"], Value::Null);
    // The facts that are the SAME must actually be present, not jointly absent.
    assert_eq!(k8s_row["session_id"], sandbox_harness::SESSION);
    assert_eq!(osb_row["session_id"], sandbox_harness::SESSION);
    assert_eq!(k8s_row["status"], osb_row["status"]);
    assert_eq!(k8s_row["creator_id"], osb_row["creator_id"]);
}

fn one_item(snapshot: &Value) -> Value {
    let items = snapshot["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "expected exactly one row: {snapshot}");
    items[0].clone()
}

/// The OpenSandbox inventory read is a paginated LIST and nothing else.
///
/// The mock deliberately mounts a per-sandbox `GET` that FAILS the test if it is
/// ever called, rather than merely counting: an accidental per-item read would
/// otherwise still produce a plausible fleet and pass every content assertion.
#[tokio::test]
async fn the_opensandbox_inventory_walks_pages_and_reads_no_single_sandbox() {
    let server = MockServer::start().await;
    for page in 1..=3u32 {
        let has_next = page < 3;
        Mock::given(method("GET"))
            .and(path("/v1/sandboxes"))
            .and(wiremock::matchers::query_param("page", page.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{
                    "id": format!("sbx-{page}"),
                    "status": { "state": "Running" },
                    "createdAt": "2026-07-01T09:00:00Z",
                }],
                "pagination": {
                    "page": page,
                    "pageSize": 1,
                    "totalItems": 3,
                    "totalPages": 3,
                    "hasNextPage": has_next,
                },
            })))
            .expect(1)
            .mount(&server)
            .await;
    }
    // Any per-item read is a hard failure, not a silent extra call.
    Mock::given(method("GET"))
        .and(wiremock::matchers::path_regex(r"^/v1/sandboxes/.+$"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let client = fkst_control_plane::session_backend::opensandbox::OsbLifecycleClient::new(
        server.uri().parse().expect("a valid base url"),
        secrecy::SecretString::from("osb_acceptance_key".to_string()),
        reqwest::Client::new(),
    );
    let (views, truncated) = client
        .list_sandboxes_paged(&[("fkst-managed".to_string(), "true".to_string())])
        .await
        .expect("the paginated walk succeeds");

    assert_eq!(views.len(), 3, "the walk did not visit every page");
    assert!(!truncated, "a three-page walk is not a clipped one");

    // Exactly three requests reached the server, all of them the LIST route.
    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(requests.len(), 3, "the walk cost more than its three pages");
    for request in &requests {
        assert_eq!(request.url.path(), "/v1/sandboxes");
    }
}
