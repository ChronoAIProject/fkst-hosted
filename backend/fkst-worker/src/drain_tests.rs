//! Suite for the graceful worker drain (issue #140a). Included via `#[path]`
//! from `drain.rs` so the drain source stays under the 500-line budget;
//! `super::*` resolves to the `drain` module.
//!
//! A fake controller (wiremock) records the worker's `Draining` / `Released` /
//! status posts, and real `sh` engine stubs (the same fixtures the supervise
//! suite uses) stand in for live sessions. No secret value is ever asserted or
//! printed.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde_json::json;
use wiremock::MockServer;

use fkst_shared::protocol::LifecycleState;

use super::{run_drain, DrainState};
use crate::engine::refresh::{AlreadyConfirmed, FenceConfirmer};
use crate::engine::supervise_test_support::*;

/// Mount every controller endpoint the drain (and the supervise loops it drives)
/// posts to, each answering 200.
async fn mount_all(server: &MockServer) {
    mount_draining(server).await;
    mount_released(server).await;
    mount_status(server).await;
    mount_refresh(server, json!({ "credentials": null, "gone": false })).await;
}

/// Spawn a real stub engine for a synthetic session id and register it on the
/// agent as a live supervised session (mirrors `spawn_supervise`'s prod wiring,
/// fence already confirmed so it behaves like a fresh dispatch). Returns the
/// session id so the test can assert on it.
async fn register_live_session(
    agent: &Arc<crate::WorkerAgent>,
    cfg: &fkst_engine::EngineConfig,
    session_id: &str,
) {
    let mut d = dispatch();
    d.session_id = session_id.to_string();
    let (running, guards, _runtime_dir) = spawn_engine(cfg, &d).await;
    let confirmer: Arc<dyn FenceConfirmer> = Arc::new(AlreadyConfirmed(d.fencing_id));
    let spawned = agent.spawn_supervise(
        session_id.to_string(),
        running,
        guards,
        None,
        format!("{}/{}", d.goal.repo.owner, d.goal.repo.name),
        SystemTime::now() + Duration::from_secs(3 * 3600),
        Some(d.fencing_id),
        confirmer,
    );
    assert!(spawned, "supervise must register the session");
}

/// A stub `fkst-framework` whose `supervise` branch TRAPS (ignores) SIGTERM, so
/// stopping it must escalate to SIGKILL after the engine's `stop_grace_secs` —
/// making a single stop genuinely outlast a tiny drain grace. Conformance still
/// passes (so `spawn_engine` succeeds).
fn term_ignoring_engine_stub(dir: &Path) -> PathBuf {
    let path = dir.join("term-ignoring-framework.sh");
    let script = r#"#!/bin/sh
case "$1" in
  conformance)
    echo "PASS graph-scan loaded 1 departments, 1 raisers, 1 queues"
    exit 0
    ;;
  supervise)
    trap '' TERM
    echo "event runtime running handles=3"
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]"
    while true; do sleep 1; done
    ;;
esac
"#;
    std::fs::write(&path, script).expect("write stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

// --- named tests -------------------------------------------------------------

/// The happy path: with N live sessions, `run_drain` flips the lifecycle to
/// Draining, sends EXACTLY ONE `Draining`, stops + `Released`-es every session,
/// empties the registry, and reports `released == total`.
#[tokio::test]
async fn run_drain_stops_pulling_sends_draining_and_releases() {
    let stub_dir = tempfile::tempdir().unwrap();
    let temp_root = tempfile::tempdir().unwrap();
    let cfg = stub_config(&engine_stub(stub_dir.path(), 0), temp_root.path());

    let server = MockServer::start().await;
    mount_all(&server).await;
    let agent = agent_for(&server);

    // Three live sessions, distinct ids.
    let ids = [
        "aaaaaaaa-0000-0000-0000-000000000001",
        "aaaaaaaa-0000-0000-0000-000000000002",
        "aaaaaaaa-0000-0000-0000-000000000003",
    ];
    for id in ids {
        register_live_session(&agent, &cfg, id).await;
    }
    assert_eq!(agent.running_session_ids().len(), 3);
    assert_eq!(agent.lifecycle(), LifecycleState::Active);

    // Let the supervise loops report their initial Running before draining.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        run_drain(&agent, Duration::from_secs(15)),
    )
    .await
    .expect("drain must finish well within the grace");

    // begin_drain flipped the gate.
    assert_eq!(
        agent.lifecycle(),
        LifecycleState::Draining,
        "drain must flip the lifecycle gate"
    );
    // Exactly one Draining, a Released per session, registry emptied.
    assert_eq!(
        request_count(&server, "/internal/v1/draining").await,
        1,
        "exactly one Draining message"
    );
    assert_eq!(
        request_count(&server, "/internal/v1/released").await,
        ids.len(),
        "one Released per session"
    );
    assert!(
        agent.running_session_ids().is_empty(),
        "registry must be empty after drain"
    );
    assert_eq!(outcome.released, ids.len());
    assert_eq!(outcome.total, ids.len());
    assert!(!outcome.timed_out, "a generous grace must not time out");
}

/// Zero sessions → a clean, fast drain: no panic, `released == 0`, not timed out,
/// and still one best-effort `Draining` so the controller marks the worker.
#[tokio::test]
async fn run_drain_with_no_sessions_is_clean() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let agent = agent_for(&server);

    assert!(agent.running_session_ids().is_empty());

    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        run_drain(&agent, Duration::from_secs(15)),
    )
    .await
    .expect("an empty drain must return immediately");

    assert_eq!(outcome.released, 0);
    assert_eq!(outcome.total, 0);
    assert!(!outcome.timed_out);
    assert_eq!(agent.lifecycle(), LifecycleState::Draining);
    assert_eq!(
        request_count(&server, "/internal/v1/draining").await,
        1,
        "an empty drain still announces Draining once"
    );
}

/// A session whose stop genuinely hangs (a TERM-ignoring engine, reaped only
/// after `stop_grace_secs`) must NOT wedge the drain: with a tiny grace,
/// `run_drain` returns promptly with `timed_out == true`. The leaked engine is
/// reaped by its detached supervise loop within `stop_grace_secs` (SIGKILL).
#[tokio::test]
async fn run_drain_respects_grace_deadline() {
    let stub_dir = tempfile::tempdir().unwrap();
    let temp_root = tempfile::tempdir().unwrap();
    // stop_grace_secs = 2: a TERM-ignoring stop takes ~2s (then SIGKILL), well
    // past the 50ms drain grace below — a genuine hang, self-cleaning in ~2s.
    let mut cfg = stub_config(
        &term_ignoring_engine_stub(stub_dir.path()),
        temp_root.path(),
    );
    cfg.stop_grace_secs = 2;

    let server = MockServer::start().await;
    mount_all(&server).await;
    let agent = agent_for(&server);

    register_live_session(&agent, &cfg, "bbbbbbbb-0000-0000-0000-000000000001").await;
    // Let the supervise loop confirm Running so the engine's TERM trap is live.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        run_drain(&agent, Duration::from_millis(50)),
    )
    .await
    .expect("run_drain MUST return on its grace deadline, never hang");

    assert!(
        outcome.timed_out,
        "a hung stop must trip the grace deadline; outcome = {outcome:?}"
    );
    assert_eq!(outcome.total, 1);
    assert_eq!(
        outcome.released, 0,
        "the hung session does not complete its Released before the deadline"
    );

    // Give the detached supervise loop time to SIGKILL-reap the leaked engine so
    // the test leaves no stray process behind.
    tokio::time::sleep(Duration::from_secs(3)).await;
}

/// The `DrainState` phase enum is `Debug` + `PartialEq` (used in the drain's
/// structured logs); a thin guard so a future refactor that drops a variant or
/// its derives fails loudly here rather than silently in a log line.
#[test]
fn drain_state_variants_are_distinct() {
    assert_ne!(DrainState::Active, DrainState::Draining);
    assert_ne!(DrainState::FlushingCheckpoints, DrainState::AwaitingAcks);
    assert_ne!(DrainState::Draining, DrainState::Terminated);
}
