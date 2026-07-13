//! The `supervise` exec + graceful-signal loop, plus the optional log TEE.
//!
//! Split out of [`super::driver`] so the driver's launch sequence stays under the
//! module-size cap and this one concern — spawn `fkst-framework supervise`, forward
//! SIGTERM/SIGINT to its process group, and (when log streaming is on) tee its
//! stdout/stderr into the collector without altering the inherited stream — lives in
//! one place. The tee invariant is load-bearing: the child's bytes reach this
//! process's own stdout/stderr verbatim so `kubectl logs` + the health scrape never
//! change; forwarding to the collector is best-effort on top.

use std::process::{ExitCode, Stdio};

use nix::sys::signal::Signal;
use nix::unistd::Pid;
use tokio::process::Command;
use tokio::signal::unix::{signal, SignalKind};

use super::log_stream::classify::LogClass;
use super::log_stream::collector::CollectorRecord;
use super::log_stream::tee::tee_reader;
use super::plan::exit_status_to_code;

/// The bundled substrate binary the session execs (image-baked, §Dockerfile). Also
/// the `--framework-bin` arg the driver builds, so it is the shared source of truth.
pub(crate) const FRAMEWORK_BIN: &str = "/usr/local/bin/fkst-framework";

/// Spawn `fkst-framework supervise` with the built argv + env in its OWN process
/// group and supervise it, forwarding SIGTERM/SIGINT to the child's group so the
/// reconciler's pod-delete (SIGTERM) drains supervise + its descendants (codex,
/// git) gracefully. Returns the child's exit code as this process's [`ExitCode`].
///
/// When `log_sender` is `Some`, the child's stdout/stderr are PIPED and tee'd: every
/// byte is re-emitted verbatim to this process's own stdout/stderr (so `kubectl logs`
/// and the health scrape are UNCHANGED) AND each complete line is forwarded to the
/// log collector. When `None`, the streams INHERIT exactly as before (no collector).
pub(super) async fn exec_supervise(
    args: Vec<String>,
    env: Vec<(String, String)>,
    log_sender: Option<std::sync::mpsc::SyncSender<CollectorRecord>>,
) -> Result<ExitCode, String> {
    let streaming = log_sender.is_some();
    let mut command = Command::new(FRAMEWORK_BIN);
    command
        .args(&args)
        // The child env is the FULL environment (built from `std::env::vars()` +
        // overrides), so clear-then-set makes it deterministic and drops nothing
        // unexpected.
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        // A new process group (pgid == child pid) lets us signal the whole tree.
        .process_group(0)
        .kill_on_drop(false);
    if streaming {
        // Piped so the tee can copy → inherited stream AND → collector. Without a
        // collector we inherit (default), leaving the log path byte-for-byte as-is.
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    tracing::info!(bin = FRAMEWORK_BIN, args = ?args, streaming, "run-substrate: exec supervise");

    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn {FRAMEWORK_BIN}: {error}"))?;
    let pid = child
        .id()
        .ok_or_else(|| "supervise child exited before yielding a pid".to_string())?;

    // Tee the piped streams (if streaming). Each task reads to EOF (child exit),
    // re-emitting bytes to the inherited stream first, then forwarding framed lines.
    let tee_tasks = spawn_tees(&mut child, log_sender);

    let mut sigterm = signal(SignalKind::terminate())
        .map_err(|error| format!("install SIGTERM handler: {error}"))?;
    let mut sigint = signal(SignalKind::interrupt())
        .map_err(|error| format!("install SIGINT handler: {error}"))?;

    let status = loop {
        tokio::select! {
            // biased: always check child exit first so a race between exit and a
            // signal never re-signals a dead group.
            biased;
            result = child.wait() => break result.map_err(|error| format!("await supervise: {error}"))?,
            _ = sigterm.recv() => forward_signal(pid, Signal::SIGTERM),
            _ = sigint.recv() => forward_signal(pid, Signal::SIGTERM),
        }
    };

    // Drain the tees so the final lines are forwarded and the tee's sender clones
    // drop (which lets the collector see end-of-stream). The child is already reaped,
    // so the piped streams are at EOF and these complete promptly.
    for task in tee_tasks {
        let _ = task.await;
    }

    let code = exit_status_to_code(status.code());
    tracing::info!(code, "run-substrate: supervise exited");
    Ok(ExitCode::from(code))
}

/// Spawn the stdout/stderr tee tasks for a piped supervise child. Returns the task
/// handles (empty when not streaming or when a stream handle is unexpectedly absent).
/// Both streams route to the [`LogClass::Supervise`] tree file; the driver's own
/// records reach `driver.log` via the collector handle instead.
fn spawn_tees(
    child: &mut tokio::process::Child,
    log_sender: Option<std::sync::mpsc::SyncSender<CollectorRecord>>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let Some(sender) = log_sender else {
        return Vec::new();
    };
    let mut tasks = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let sender = sender.clone();
        tasks.push(tokio::spawn(async move {
            let _ = tee_reader(stdout, tokio::io::stdout(), sender, LogClass::Supervise).await;
        }));
    }
    if let Some(stderr) = child.stderr.take() {
        tasks.push(tokio::spawn(async move {
            let _ = tee_reader(stderr, tokio::io::stderr(), sender, LogClass::Supervise).await;
        }));
    }
    tasks
}

/// Send `signal` to the whole process group `pgid` (relocated from the deleted
/// `engine::process`).
fn signal_group(pgid: i32, signal: Signal) -> Result<(), nix::Error> {
    nix::sys::signal::killpg(Pid::from_raw(pgid), signal)
}

/// Forward `signal` to the supervise child's process GROUP (it is
/// `process_group(0)`, so pgid == pid). `ESRCH` (already gone) is a benign no-op.
fn forward_signal(pid: u32, signal: Signal) {
    match signal_group(pid as i32, signal) {
        Ok(()) => tracing::info!(
            pid,
            ?signal,
            "run-substrate: forwarded signal to supervise group"
        ),
        Err(nix::Error::ESRCH) => {}
        Err(error) => tracing::warn!(pid, %error, "run-substrate: could not forward signal"),
    }
}
