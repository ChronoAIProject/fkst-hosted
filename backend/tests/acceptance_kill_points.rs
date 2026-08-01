//! Milestone acceptance: the control plane and the relay dying at each of the
//! points the epic's reliability section names.
//!
//! The reliability suite next door covers storage and replicas. What it does not
//! cover is a process disappearing MID-REQUEST, which is the failure the required
//! -delivery contract exists for and the one nothing else stages:
//!
//! ```text
//!   register_start  ──┬── kill here: durable start, handler never ran
//!   handler         ──┼── kill here: durable start, handler half-ran
//!   complete        ──┴── kill here: durable terminal already committed
//! ```
//!
//! In every case the durable history must be honest afterwards: a start with no
//! completion closes as `incomplete` with a NULL status, never a status the relay
//! invented, and a completion that was committed stays exactly once.
//!
//! ## How a "kill" is staged
//!
//! A control-plane kill is an aborted task. `tokio::task::JoinHandle::abort()`
//! drops the request future at its current suspension point, which is precisely
//! what a `SIGKILL` does to an in-flight request as far as the RELAY can tell:
//! whatever was already sent is durable, whatever was not never arrives. Nothing
//! about the assertion depends on the process actually exiting — it depends on
//! the relay never hearing the rest of the conversation, and an aborted future
//! guarantees that.
//!
//! A relay kill is the harness's real restart: graceful shutdown, the serving
//! task joined so its `Database` — writer thread and pooled connections — is
//! genuinely gone, then a fresh process over the same file.

#[path = "audit_relay_harness/mod.rs"]
mod relay;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use fkst_control_plane::audit::relay::{AuditDelivery, AuditDeliveryMode, RelayClientMetrics};
use fkst_control_plane::audit::{
    audit_requests, AuditHandle, AuditMiddleware, OperationCatalog, ServiceIdentity,
};
use fkst_control_plane::audit_relay::query::RecordRowV1;
use k8s_openapi::chrono::{Duration as ChronoDuration, Utc};
use tower::ServiceExt;

/// The route every driven request hits.
const ROUTE: &str = "/kill-point";

/// Kill point 1: the control plane dies AFTER the durable start and BEFORE the
/// handler.
///
/// The handler-invocation counter is the assertion that makes this distinct from
/// kill point 2: required mode promises the start is durable before any handler
/// work happens, so a kill in this window must leave a durable start and a
/// handler that provably never ran.
#[tokio::test]
async fn a_control_plane_killed_between_the_start_and_the_handler_leaves_an_incomplete() {
    let node = relay::Relay::start().await;
    let invocations = Arc::new(AtomicUsize::new(0));
    let reached = Arc::new(tokio::sync::Notify::new());
    // `Blocking::BeforeHandler` parks an inner layer that sits between the audit
    // middleware and the route, so the abort is guaranteed to land in the window
    // this test is about rather than racing it with a sleep.
    let router = required_router(
        &node,
        invocations.clone(),
        reached.clone(),
        Blocking::BeforeHandler,
    );

    let waiting = reached.notified();
    let task = tokio::spawn(async move { router.oneshot(probe()).await });
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the request must reach the pre-handler window before the kill");
    task.abort();
    let _ = task.await;

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        0,
        "the handler ran even though the request was killed before it"
    );
    let row = incomplete_row(&node)
        .await
        .expect("required mode must have made the start durable before the handler");
    assert_eq!(row.terminal["outcome"], "incomplete");
    assert_eq!(
        row.terminal["status_code"],
        serde_json::Value::Null,
        "the relay invented a status for a handler that never ran"
    );
}

/// Kill point 2: the control plane dies DURING the handler.
///
/// The handler signals that it started and then never returns, so the abort is
/// guaranteed to land inside it. The durable start exists; the completion never
/// will.
#[tokio::test]
async fn a_control_plane_killed_during_the_handler_leaves_an_incomplete_not_a_status() {
    let node = relay::Relay::start().await;
    let invocations = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let router = required_router(
        &node,
        invocations.clone(),
        entered.clone(),
        Blocking::InHandler,
    );

    let waiting = entered.notified();
    let task = tokio::spawn(async move { router.oneshot(probe()).await });
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the handler must be entered before the kill");
    task.abort();
    let _ = task.await;

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "the handler was supposed to have started"
    );
    let row = incomplete_row(&node)
        .await
        .expect("a request killed inside its handler must leave its durable start");
    assert_eq!(row.terminal["outcome"], "incomplete");
    assert_eq!(
        row.terminal["status_code"],
        serde_json::Value::Null,
        "the relay invented a status for a request that never finished"
    );
}

/// Kill point 3: the control plane dies AFTER submitting the completion but
/// before the response reaches the caller.
///
/// The caller sees nothing, so its client retries — and a resurrected replica
/// re-submits the same terminal, because at-least-once delivery is the contract.
/// The history must still contain exactly one record with the handler's REAL
/// status: not two rows, and not the `incomplete` the sweep would otherwise
/// synthesize for a request whose caller vanished.
///
/// The retry is re-submitted verbatim from what the relay stored, which is the
/// only way to reproduce the same deterministic `event_id`: it is derived from
/// the request identity plus the start instant, so a freshly issued HTTP request
/// is a DIFFERENT record rather than a replay of this one.
#[tokio::test]
async fn a_control_plane_killed_after_the_completion_stores_exactly_one_terminal() {
    let node = relay::Relay::start().await;
    let invocations = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let router = required_router(&node, invocations.clone(), entered.clone(), Blocking::None);

    // The request runs to completion, so the terminal is durably committed; the
    // response is then dropped unread, which is what the caller of a killed
    // process observes.
    let response = router.oneshot(probe()).await.expect("the router answers");
    assert!(response.status().is_success());
    drop(response);

    let committed = terminal_row(&node)
        .await
        .expect("the completion must be durable before the response is released");
    let replay: fkst_control_plane::audit_relay::protocol::RequestCompletionV1 =
        serde_json::from_value(committed.terminal.clone())
            .expect("a stored terminal round-trips through its own protocol type");
    node.client()
        .complete(&replay)
        .await
        .expect("an at-least-once replay is acknowledged, never refused");

    // The sweep runs afterwards on purpose: a completed record must not be
    // rewritten into an `incomplete` just because its caller disappeared.
    node.sweep(Utc::now() + ChronoDuration::seconds(600)).await;

    let rows = node.read_all_recent().await;
    let matching: Vec<&RecordRowV1> = rows
        .iter()
        .filter(|row| row.terminal["route_template"] == ROUTE)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "the replay produced {} terminal records instead of one",
        matching.len()
    );
    assert_eq!(matching[0].terminal["outcome"], "success");
    assert_eq!(matching[0].terminal["status_code"], 200);
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "the replay re-ran the handler instead of only re-delivering its record"
    );
}

/// The relay dies BEFORE it can commit: the write is refused, and nothing is
/// stored.
///
/// The relay harness's restart is a real process death, so a client that submits
/// while the listener is gone gets a transport failure — which required mode
/// turns into a caller-visible refusal rather than a silent success. The
/// assertion is the pair: the write failed AND the record does not exist. A
/// relay that had acknowledged and lost it would fail only the second half.
#[tokio::test]
async fn a_relay_killed_before_the_commit_refuses_the_write_and_stores_nothing() {
    let node = relay::Relay::start().await;
    let client = node.client();
    let event_id = "f1f1f1f1-1111-4111-8111-f1f1f1f1f1f1";

    // Take the relay down and keep its file: `stop` returns the pieces a restart
    // needs, so the socket is closed while the database is intact.
    let stopped = node.stop().await;
    assert!(
        client
            .register_start(&relay::Relay::start_body(event_id))
            .await
            .is_err(),
        "a client acknowledged a start against a relay that was not running"
    );

    // Bring it back over the same file: the refused write must not be there.
    let node = stopped.resume().await;
    assert!(
        !node
            .read_all()
            .await
            .iter()
            .any(|row| row.event_id == event_id),
        "a write the relay refused turned up in its history after a restart"
    );
}

/// A relay restarted under sustained write pressure keeps every ACKNOWLEDGED
/// record and invents none.
///
/// This is the WAL-checkpoint window in the only form a test can honestly stage:
/// the relay runs in WAL mode, a hundred committed records guarantee the WAL has
/// been written and at least partly checkpointed, and the restart forces the
/// recovery path over whatever state the WAL was in. What is asserted is the
/// invariant a checkpoint could break — every acknowledged id survives, and no id
/// appears that was never acknowledged.
#[tokio::test]
async fn a_relay_restarted_under_write_pressure_keeps_exactly_the_acknowledged_records() {
    let node = relay::Relay::start().await;
    let client = node.client();
    let mut acknowledged = Vec::new();
    for index in 0..100u32 {
        let event_id = format!("{index:08x}-2222-4222-8222-222222222222");
        client
            .register_start(&relay::Relay::start_body(&event_id))
            .await
            .expect("the start is acknowledged");
        client
            .complete(&relay::Relay::completion_body(
                &event_id,
                Some(relay::ALICE),
            ))
            .await
            .expect("the completion is acknowledged");
        acknowledged.push(event_id);
    }

    let node = node.restart().await;
    let mut survived: Vec<String> = node
        .read_all()
        .await
        .into_iter()
        .map(|row| row.event_id)
        .collect();
    survived.sort();
    acknowledged.sort();
    assert_eq!(
        survived, acknowledged,
        "the set of records after the restart is not the set that was acknowledged"
    );
}

/// The request every kill point drives.
fn probe() -> Request<Body> {
    Request::get(ROUTE)
        .body(Body::empty())
        .expect("request builds")
}

/// Where the driven request is made to park, so a kill lands in a chosen window
/// rather than racing a sleep against a loopback round trip.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Blocking {
    /// Runs straight through.
    None,
    /// Parks BETWEEN the audit middleware and the route.
    BeforeHandler,
    /// Parks INSIDE the route handler.
    InHandler,
}

/// A router in REQUIRED delivery mode over `node`, whose handler counts its
/// invocations, and which parks at the chosen window.
fn required_router(
    node: &relay::Relay,
    invocations: Arc<AtomicUsize>,
    reached: Arc<tokio::sync::Notify>,
    blocking: Blocking,
) -> axum::Router {
    let (handle, _sink) = AuditHandle::recording();
    let middleware = AuditMiddleware::new(
        Arc::new(OperationCatalog::default()),
        handle,
        ServiceIdentity {
            version: "9.9.9".to_string(),
            environment: "acceptance-kill-points".to_string(),
        },
    )
    .with_delivery(AuditDelivery::with_client(
        AuditDeliveryMode::Required,
        node.client(),
        60,
        RelayClientMetrics::new(),
    ));

    let handler_reached = reached.clone();
    let mut router = axum::Router::new().route(
        ROUTE,
        get(move || {
            let invocations = invocations.clone();
            let reached = handler_reached.clone();
            async move {
                invocations.fetch_add(1, Ordering::SeqCst);
                if blocking == Blocking::InHandler {
                    reached.notify_waiters();
                    // Parked until the task is aborted, which is the kill.
                    std::future::pending::<()>().await;
                }
                "ok"
            }
        }),
    );

    if blocking == Blocking::BeforeHandler {
        // An inner layer standing in the window the epic names: the audit
        // middleware has already committed the durable start, and the request
        // has not yet reached anything that could produce a status.
        let gate = reached.clone();
        router = router.layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let gate = gate.clone();
                async move {
                    gate.notify_waiters();
                    std::future::pending::<()>().await;
                    next.run(request).await
                }
            },
        ));
    }

    router.layer(axum::middleware::from_fn_with_state(
        middleware,
        audit_requests,
    ))
}

/// The single row for [`ROUTE`], after running the relay's own closing sweep.
///
/// The sweep is what turns an abandoned durable start into an `incomplete`
/// terminal; it is a timer inside the relay process and has no HTTP surface, so
/// the harness drives it directly.
async fn incomplete_row(node: &relay::Relay) -> Option<RecordRowV1> {
    node.sweep(Utc::now() + ChronoDuration::seconds(600)).await;
    terminal_row(node).await
}

/// The single row for [`ROUTE`], without sweeping first.
async fn terminal_row(node: &relay::Relay) -> Option<RecordRowV1> {
    node.read_all_recent()
        .await
        .into_iter()
        .find(|row| row.terminal["route_template"] == ROUTE)
}
