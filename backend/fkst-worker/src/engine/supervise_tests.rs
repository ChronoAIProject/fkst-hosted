//! Suite for the worker supervise loop + credential-refresh servicer (issue
//! #151, increment 5). A fake controller (wiremock, recording the worker's
//! posts) plus a fake engine (the same `sh` `fkst-framework` stub the executor
//! tests use) drive synthetic sessions. No secret value is ever asserted or
//! printed. The scaffolding lives in `supervise_test_support` (so each test file
//! stays under the 500-line budget); this file holds the named tests.

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::watch;
use wiremock::MockServer;

use fkst_engine::SessionRunner;

use super::*;
use crate::engine::refresh::{
    request_file_path, token_file_path, AlreadyConfirmed, NeverConfirmed,
};
use crate::engine::supervise_test_support::*;

// --- the named tests ---------------------------------------------------------

/// A `.request` appears → the servicer verifies the nonce, POSTs credential-
/// refresh, writes the 0600 token file, and DELETES the `.request` file.
#[tokio::test]
async fn jit_request_mints_over_rpc_and_writes_token() {
    let stub_dir = tempfile::tempdir().unwrap();
    let temp_root = tempfile::tempdir().unwrap();
    let cfg = stub_config(&engine_stub(stub_dir.path(), 0), temp_root.path());
    let d = dispatch();
    let (mut running, _guards, runtime_dir) = spawn_engine(&cfg, &d).await;

    let server = MockServer::start().await;
    mount_refresh(
        &server,
        json!({
            "credentials": { "token": "ghs_fresh_jit", "expires_at_unix_ms": far_future_unix_ms() },
            "gone": false
        }),
    )
    .await;
    let agent = agent_for(&server);

    // The engine wrote a token file + the controller's nonce at startup; drop a
    // matching mint request (the helper's "please re-mint" signal).
    let request_path = request_file_path(&runtime_dir);
    std::fs::write(&request_path, "controller-nonce-abc").unwrap();
    let token_path = token_file_path(&runtime_dir);
    let before = std::fs::read_to_string(&token_path).unwrap();

    let mut refresh = refresh_state(
        &agent,
        &d,
        &runtime_dir,
        Arc::new(AlreadyConfirmed(d.fencing_id)),
    );
    refresh.clear_cooldown_for_test();
    let outcome = refresh.service_tick().await;
    assert!(
        matches!(outcome, crate::engine::refresh::RefreshOutcome::Refreshed),
        "got {outcome:?}"
    );

    // The request file was deleted (the "fresh token ready" signal).
    assert!(
        !request_path.exists(),
        ".request must be deleted after mint"
    );
    // The token file was rewritten (a fresh token landed) at 0600.
    let after = std::fs::read_to_string(&token_path).unwrap();
    assert_ne!(before, after, "token file must be rewritten");
    let mode = std::fs::metadata(&token_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "token file must be 0600");
    assert_eq!(refresh_request_count(&server).await, 1);

    SessionRunner::new(cfg.clone())
        .stop(&mut running)
        .await
        .ok();
}

/// A `.request` with the WRONG nonce is ignored (no mint), and the stray request
/// file is cleared — the nonce gate is load-bearing.
#[tokio::test]
async fn jit_request_with_bad_nonce_is_ignored() {
    let stub_dir = tempfile::tempdir().unwrap();
    let temp_root = tempfile::tempdir().unwrap();
    let cfg = stub_config(&engine_stub(stub_dir.path(), 0), temp_root.path());
    let d = dispatch();
    let (mut running, _guards, runtime_dir) = spawn_engine(&cfg, &d).await;

    let server = MockServer::start().await;
    mount_refresh(&server, json!({ "credentials": null, "gone": false })).await;
    let agent = agent_for(&server);

    let request_path = request_file_path(&runtime_dir);
    std::fs::write(&request_path, "WRONG-NONCE").unwrap();

    // Deliberately do NOT clear the cooldown: the freshly-minted token's cooldown
    // gate keeps the periodic path quiet, so this isolates the nonce gate — the
    // ONLY reason no mint fires is the nonce mismatch.
    let mut refresh = refresh_state(
        &agent,
        &d,
        &runtime_dir,
        Arc::new(AlreadyConfirmed(d.fencing_id)),
    );
    let outcome = refresh.service_tick().await;
    assert!(
        matches!(outcome, crate::engine::refresh::RefreshOutcome::Skipped),
        "a bad nonce must not mint; got {outcome:?}"
    );
    assert!(!request_path.exists(), "stray request file must be cleared");
    assert_eq!(
        refresh_request_count(&server).await,
        0,
        "no mint on a bad nonce"
    );

    SessionRunner::new(cfg.clone())
        .stop(&mut running)
        .await
        .ok();
}

/// credential-refresh returns `credentials: None` → the servicer self-fences
/// (StaleFence), does NOT write a token, and never mints again.
#[tokio::test]
async fn stale_fence_response_self_fences() {
    let stub_dir = tempfile::tempdir().unwrap();
    let temp_root = tempfile::tempdir().unwrap();
    let cfg = stub_config(&engine_stub(stub_dir.path(), 0), temp_root.path());
    let d = dispatch();
    let (mut running, _guards, runtime_dir) = spawn_engine(&cfg, &d).await;

    let server = MockServer::start().await;
    mount_refresh(&server, json!({ "credentials": null, "gone": false })).await;
    let agent = agent_for(&server);

    let request_path = request_file_path(&runtime_dir);
    std::fs::write(&request_path, "controller-nonce-abc").unwrap();
    let token_path = token_file_path(&runtime_dir);
    let before = std::fs::read_to_string(&token_path).unwrap();

    let mut refresh = refresh_state(
        &agent,
        &d,
        &runtime_dir,
        Arc::new(AlreadyConfirmed(d.fencing_id)),
    );
    refresh.clear_cooldown_for_test();
    let outcome = refresh.service_tick().await;
    assert!(
        matches!(outcome, crate::engine::refresh::RefreshOutcome::StaleFence),
        "got {outcome:?}"
    );
    // No token rewrite on a stale fence.
    assert_eq!(
        std::fs::read_to_string(&token_path).unwrap(),
        before,
        "a stale fence must NOT rewrite the token"
    );

    SessionRunner::new(cfg.clone())
        .stop(&mut running)
        .await
        .ok();
}

/// `gone: true` → the servicer reports Fatal so the supervise loop fails the
/// session (the worker never silently 401s).
#[tokio::test]
async fn gone_response_fails_the_session() {
    let stub_dir = tempfile::tempdir().unwrap();
    let temp_root = tempfile::tempdir().unwrap();
    let cfg = stub_config(&engine_stub(stub_dir.path(), 0), temp_root.path());
    let d = dispatch();
    let (mut running, _guards, runtime_dir) = spawn_engine(&cfg, &d).await;

    let server = MockServer::start().await;
    mount_refresh(&server, json!({ "credentials": null, "gone": true })).await;
    let agent = agent_for(&server);

    let request_path = request_file_path(&runtime_dir);
    std::fs::write(&request_path, "controller-nonce-abc").unwrap();

    let mut refresh = refresh_state(
        &agent,
        &d,
        &runtime_dir,
        Arc::new(AlreadyConfirmed(d.fencing_id)),
    );
    refresh.clear_cooldown_for_test();
    let outcome = refresh.service_tick().await;
    assert!(
        matches!(
            outcome,
            crate::engine::refresh::RefreshOutcome::Fatal { .. }
        ),
        "gone must be fatal; got {outcome:?}"
    );

    SessionRunner::new(cfg.clone())
        .stop(&mut running)
        .await
        .ok();
}

/// An adopted session's refresh servicer is PARKED until the fence-confirmer says
/// yes (the seam): a never-confirmed session never mints, never writes a token —
/// even with a pending, correctly-nonced `.request`.
#[tokio::test]
async fn adopted_engine_does_not_mint_until_fence_confirmed() {
    let stub_dir = tempfile::tempdir().unwrap();
    let temp_root = tempfile::tempdir().unwrap();
    let cfg = stub_config(&engine_stub(stub_dir.path(), 0), temp_root.path());
    let d = dispatch();
    let (mut running, _guards, runtime_dir) = spawn_engine(&cfg, &d).await;

    let server = MockServer::start().await;
    mount_refresh(
        &server,
        json!({
            "credentials": { "token": "ghs_should_not_happen", "expires_at_unix_ms": far_future_unix_ms() },
            "gone": false
        }),
    )
    .await;
    let agent = agent_for(&server);

    let request_path = request_file_path(&runtime_dir);
    std::fs::write(&request_path, "controller-nonce-abc").unwrap();
    let token_path = token_file_path(&runtime_dir);
    let before = std::fs::read_to_string(&token_path).unwrap();

    // NeverConfirmed: the parked seam. Several ticks must never mint.
    let mut refresh = refresh_state(&agent, &d, &runtime_dir, Arc::new(NeverConfirmed));
    for _ in 0..5 {
        let outcome = refresh.service_tick().await;
        assert!(
            matches!(outcome, crate::engine::refresh::RefreshOutcome::Skipped),
            "a parked (unconfirmed) session must never mint; got {outcome:?}"
        );
    }
    assert!(refresh.armed_fencing_id().is_none(), "never armed");
    assert_eq!(
        std::fs::read_to_string(&token_path).unwrap(),
        before,
        "a parked session must NOT rewrite the token"
    );
    assert_eq!(
        refresh_request_count(&server).await,
        0,
        "no mint while parked"
    );
    assert!(
        request_path.exists(),
        "parked: the request is left for after arming"
    );

    SessionRunner::new(cfg.clone())
        .stop(&mut running)
        .await
        .ok();
}

/// Running → (engine exits) → Stopped produces the right StatusReports through
/// the full supervise loop. The stub exits cleanly after ~1s.
#[tokio::test]
async fn status_transitions_are_reported() {
    let stub_dir = tempfile::tempdir().unwrap();
    let temp_root = tempfile::tempdir().unwrap();
    let cfg = stub_config(&engine_stub(stub_dir.path(), 1), temp_root.path());
    let d = dispatch();
    let (running, guards, _runtime_dir) = spawn_engine(&cfg, &d).await;

    let server = MockServer::start().await;
    mount_status(&server).await;
    mount_refresh(&server, json!({ "credentials": null, "gone": false })).await;
    mount_released(&server).await;
    let agent = agent_for(&server);

    let refresh = refresh_state(
        &agent,
        &d,
        &running.runtime_dir.clone(),
        Arc::new(AlreadyConfirmed(d.fencing_id)),
    );
    let ctx = SuperviseContext {
        agent: agent.clone(),
        session_id: d.session_id.clone(),
        refresh,
        initial_fencing_id: Some(d.fencing_id),
    };
    let (_stop_tx, stop_rx) = watch::channel(false);
    // The stub exits after ~1s; the loop should observe the terminal status and
    // report Stopped. Bound it so a hang fails loudly.
    tokio::time::timeout(
        Duration::from_secs(10),
        supervise_session(ctx, running, guards, None, stop_rx),
    )
    .await
    .expect("supervise loop must finish when the engine exits");

    let statuses = received_statuses(&server).await;
    assert!(
        statuses.iter().any(|s| s.status == SessionStatus::Running),
        "the initial Running status must be reported: {statuses:?}"
    );
    let terminal = statuses
        .iter()
        .find(|s| s.status == SessionStatus::Stopped)
        .expect("a terminal Stopped status must be reported");
    assert_eq!(terminal.fencing_id, d.fencing_id, "the fence is echoed");
    assert!(
        terminal.terminal.is_some(),
        "terminal report carries the exit"
    );
}

/// A StopSession flips the stop signal → the loop stops the engine, sends a
/// Released, and reports a terminal Stopped status.
#[tokio::test]
async fn stop_session_stops_engine_and_releases() {
    let stub_dir = tempfile::tempdir().unwrap();
    let temp_root = tempfile::tempdir().unwrap();
    let cfg = stub_config(&engine_stub(stub_dir.path(), 0), temp_root.path());
    let d = dispatch();
    let (running, guards, _runtime_dir) = spawn_engine(&cfg, &d).await;

    let server = MockServer::start().await;
    mount_status(&server).await;
    mount_refresh(&server, json!({ "credentials": null, "gone": false })).await;
    mount_released(&server).await;
    let agent = agent_for(&server);

    let refresh = refresh_state(
        &agent,
        &d,
        &running.runtime_dir.clone(),
        Arc::new(AlreadyConfirmed(d.fencing_id)),
    );
    let ctx = SuperviseContext {
        agent: agent.clone(),
        session_id: d.session_id.clone(),
        refresh,
        initial_fencing_id: Some(d.fencing_id),
    };
    let (stop_tx, stop_rx) = watch::channel(false);
    let loop_handle = tokio::spawn(supervise_session(ctx, running, guards, None, stop_rx));

    // Let the loop report Running, then command a stop.
    tokio::time::sleep(Duration::from_millis(700)).await;
    stop_tx.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(10), loop_handle)
        .await
        .expect("loop joins after stop")
        .expect("loop task did not panic");

    // The controller saw a Released and a terminal Stopped status.
    assert!(
        saw_released(&server).await,
        "a Released must be sent on a commanded stop"
    );
    let statuses = received_statuses(&server).await;
    assert!(
        statuses.iter().any(|s| s.status == SessionStatus::Stopped),
        "a terminal Stopped status must be reported on stop: {statuses:?}"
    );
}
