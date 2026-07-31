//! The LOG half of the sandbox endpoint's canary sweep.
//!
//! Its siblings cover the response JSON (`operations_sandboxes_isolation`), the
//! audit arguments (`operations_sandboxes_audit`), and the metrics exposition.
//! Logs are the surface none of those can reach: a handler that redacts perfectly
//! on the wire and then writes a collaborator login, a hidden runtime id, or a
//! bearer token into a structured field has leaked it just as effectively.
//!
//! ## Why this lives in its own test binary
//!
//! `tracing` caches per-callsite interest process-wide, and the capturing
//! subscriber is installed with the THREAD-LOCAL
//! `tracing::subscriber::set_default`. A sibling test running in parallel can
//! therefore register a callsite while no subscriber is installed on ITS thread,
//! after which the cached "nobody is interested" answer suppresses that callsite
//! for everyone — and the sweep would quietly observe only some of the endpoint's
//! log lines. One test per binary makes the capture complete and deterministic
//! rather than a function of scheduling.

mod sandbox_harness;

use axum::http::StatusCode;
use sandbox_harness::fleet;
use sandbox_harness::{
    harness, harness_with, HarnessSpec, ALICE, GRACE, OTHER_SESSION, SESSION, UNKNOWN_SESSION,
};

/// Two runtimes Alice may see, in a fleet that also holds rows she may not.
fn visible_and_hidden() -> Vec<fleet::Item> {
    vec![
        fleet::item("hidden-a", Some(OTHER_SESSION)),
        fleet::item("mine-running", Some(SESSION)),
        fleet::orphan("hidden-orphan"),
        fleet::item("hidden-b", Some(UNKNOWN_SESSION)),
    ]
}

#[tokio::test]
async fn no_log_line_carries_a_secret_a_hidden_runtime_or_an_access_list_canary() {
    let (captured, guard) = sandbox_harness::logs::CapturedLogs::install();
    {
        let healthy = harness_with(visible_and_hidden()).await;
        let cold =
            harness(HarnessSpec::new(fleet::snapshot(visible_and_hidden())).cold_registry()).await;
        let failing = harness(HarnessSpec::new(sandbox_harness::InventoryScript::Failure)).await;

        // Every path that emits log lines of its own: an allowed read for a
        // regular caller and for an administrator (whose snapshot holds every
        // hidden row), the withheld-row diagnostics the authorizer emits along
        // the way, a refused scope, a cold-projection refusal, and a backend
        // failure.
        healthy.snapshot(ALICE, "").await;
        healthy.snapshot(GRACE, "").await;
        assert_eq!(
            healthy.get(ALICE, "?scope=all").await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            cold.get(ALICE, "").await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            failing.get(GRACE, "").await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    drop(guard);

    let text = captured.text();
    assert!(
        !text.is_empty(),
        "the capture harness recorded nothing, so it proves nothing"
    );
    for canary in [
        // bearer tokens, in the exact form the fixture presents them
        "token-alice",
        "token-grace",
        "Bearer",
        // access-list contents: the session's collaborator, its per-session log
        // grantee, and the deployment's legacy cross-session grant
        "bob",
        "carol",
        "dana",
        // rows a regular caller may not see, and the sessions behind them
        "hidden-a",
        "hidden-b",
        "hidden-orphan",
        OTHER_SESSION,
        UNKNOWN_SESSION,
        // raw backend objects
        "kubeconfig",
        "apiVersion",
    ] {
        assert!(
            !text.contains(canary),
            "{canary} leaked into trace output:\n{text}"
        );
    }

    // The bounded diagnostics must still BE there — a capture that recorded
    // nothing useful would pass every assertion above for entirely the wrong
    // reason. Both are closed vocabulary, not data.
    for expected in ["unknown_context", "not_authorized"] {
        assert!(
            text.contains(expected),
            "the withheld-row reasons are the operator's only signal for {expected}:\n{text}"
        );
    }
    assert!(
        text.contains("runtime inventory read failed"),
        "a backend failure must still be diagnosable from the logs:\n{text}"
    );
}
