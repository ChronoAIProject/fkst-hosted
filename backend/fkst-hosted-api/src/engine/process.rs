//! Low-level engine process management: spawn `supervise` in its own process
//! group, drain BOTH its stdout and stderr into a single bounded ring buffer,
//! scan for the empirical ready/panic markers, run the `conformance`
//! pre-flight under a wall-clock cap, and reap whole process groups with a
//! SIGTERM -> SIGKILL grace ladder.
//!
//! Contract notes (issue #17 spike):
//! - `supervise` is ALWAYS spawned directly (never the engine's wrapper
//!   binary, which orphans its child by design — spike Q7) in its OWN process
//!   group (`process_group(0)`, PGID == child PID) so `killpg` reaps the
//!   supervisor AND its framework grandchildren.
//! - Readiness requires BOTH the `event runtime running` and at least one
//!   `consumer started` message: the runtime-running line alone is emitted
//!   even in the half-alive unset-`FKST_RUNTIME_ROOT` mode (spike Q9). The
//!   markers are message-text-only because the engine's ANSI styling breaks
//!   any substring spanning a field boundary (see [`is_ready`]).
//! - STREAM WIRING (issue #50): `supervise` writes its readiness markers
//!   (`event runtime running` AND `consumer started`) to STDOUT, while Rust
//!   panics surface on STDERR. Both streams are therefore piped and drained
//!   into the SAME shared [`OutputBuffer`] so the marker scan and the panic
//!   scan both see their evidence. (Piping only stderr — the original wiring —
//!   discarded every readiness marker, so the ready-wait always timed out.)
//! - Package-root wiring is FLAG-ONLY: `FKST_PACKAGE_ROOT`/`FKST_PACKAGE_ROOTS`
//!   env substitution exists upstream but its precedence against the flag is
//!   untested (spike Q8), so both variables are removed from the child env.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::engine::error::{truncate_output_lossy, RunnerError};

/// Byte cap of the shared output ring buffer (64 KiB) that merges the
/// child's stdout and stderr. The drain tasks run for the child's lifetime so
/// neither pipe ever backpressures the engine.
pub const OUTPUT_RING_CAP_BYTES: usize = 64 * 1024;

/// Byte cap of a SINGLE output line fed into the ring (8 KiB): a newline-free
/// blast is truncated at this length and the remainder discarded, so each
/// drain task's memory stays bounded regardless of engine output shape.
pub const OUTPUT_LINE_CAP_BYTES: usize = 8 * 1024;

/// Byte cap of a SINGLE stdout line forwarded to the journaling layer
/// (2 MiB — comfortably above the 1 MiB `FKST_RAISED_MAX_LINE_BYTES`
/// default, so the journaler itself decides the oversize/malformed verdict).
/// The remainder of a longer physical line is discarded, never buffered.
pub const STDOUT_LINE_CAP_BYTES: usize = 2 * 1024 * 1024;

/// Capacity of the stdout line channel. When the consumer falls behind, the
/// drain task DROPS lines (counted + warned) instead of blocking: stdout
/// journaling is observability and must never backpressure the engine.
pub const STDOUT_CHANNEL_CAPACITY: usize = 1024;

/// Per-stream byte cap when collecting conformance output (pre-truncation).
const CONFORMANCE_CAPTURE_LIMIT: u64 = 256 * 1024;

/// Poll interval for reap loops.
const REAP_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Bounded number of polls after SIGKILL before declaring the group
/// unreapable (20 x 100 ms = 2 s).
const SIGKILL_REAP_POLLS: u32 = 20;

/// Bounded, shared, append-only merge of a child's stdout AND stderr. Cheap
/// to clone (an `Arc` handle); each drain task appends, readers snapshot. Both
/// stream drains share one instance so the ready markers (stdout) and panic
/// markers (stderr) are visible to a single scan (issue #50).
#[derive(Debug, Clone)]
pub struct OutputBuffer {
    inner: Arc<Mutex<String>>,
    cap: usize,
}

impl OutputBuffer {
    /// New empty buffer keeping at most `cap` bytes (the newest tail).
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(String::new())),
            cap,
        }
    }

    /// Append one line (a `\n` is added) and trim the FRONT down to the cap,
    /// never splitting a UTF-8 character: the newest output always survives.
    /// Both stream drains call this against the same `Arc`, so the `Mutex`
    /// serialises interleaved stdout/stderr writes at line granularity.
    pub fn append_line(&self, line: &str) {
        let mut text = self.inner.lock().expect("output buffer lock poisoned");
        text.push_str(line);
        text.push('\n');
        if text.len() > self.cap {
            let mut cut = text.len() - self.cap;
            while cut < text.len() && !text.is_char_boundary(cut) {
                cut += 1;
            }
            text.drain(..cut);
        }
    }

    /// Copy of the current buffer contents.
    pub fn snapshot(&self) -> String {
        self.inner
            .lock()
            .expect("output buffer lock poisoned")
            .clone()
    }
}

/// True when the merged supervise output shows the runtime wired AND at least
/// one department consumer started. Both markers are emitted on the child's
/// STDOUT (issue #50), which is why [`spawn_supervise`] pipes and drains stdout
/// into the same buffer this scans. Requiring the consumer line guards against
/// the half-alive mode where `event runtime running` is emitted but every
/// consumer thread has panicked (spike Q9 / E6).
///
/// ANSI-SAFETY: the engine emits `tracing` output WITH ANSI styling, and field
/// names are styled separately from their values
/// (`event runtime running ESC[3mhandles ESC[0m ESC[2m= ...`), so a marker
/// spanning a field boundary (`handles=`, `dept=`) never matches the raw
/// stream. The markers therefore match only the un-styled MESSAGE text —
/// exactly the substrings the issue #17 spike's verdict greps used
/// (empirically re-verified against the pinned engine image in this issue's
/// wiring check). Selectivity for the half-alive guard is unchanged: that
/// mode emits NO `consumer started` line at all.
pub fn is_ready(output: &str) -> bool {
    output.contains("event runtime running") && output.contains("consumer started")
}

/// True when the merged supervise output contains a Rust panic marker. Panics
/// surface on STDERR (issue #50); any panic during startup is treated as a
/// startup failure, never as ready.
pub fn is_panicked(output: &str) -> bool {
    output.contains("panicked at")
}

/// Best-effort PID liveness: `kill(pid, 0)`.
///
/// `Ok` => alive; `EPERM` => alive but not ours (still alive); `ESRCH` (or
/// any other error) => gone.
///
/// PID-REUSE CAVEAT: a bare PID check cannot distinguish a reused PID from
/// the original process. Cross-pod takeover safety relies on the
/// pool-manager's fencing token + the source-of-truth store, never on this
/// primitive alone.
pub fn is_pid_alive(pid: i32) -> bool {
    match kill(Pid::from_raw(pid), None) {
        Ok(()) => true,
        Err(nix::Error::EPERM) => true,
        Err(_) => false,
    }
}

/// Send `signal` to the whole process group `pgid`.
pub fn signal_group(pgid: i32, signal: Signal) -> Result<(), nix::Error> {
    killpg(Pid::from_raw(pgid), signal)
}

/// SIGKILL a whole process group, tolerating `ESRCH` (already gone) and
/// `EPERM` (the Darwin zombie-group quirk); any other errno is logged.
pub fn kill_group_quiet(pgid: i32) {
    if let Err(err) = signal_group(pgid, Signal::SIGKILL) {
        if err != nix::Error::ESRCH && err != nix::Error::EPERM {
            tracing::error!(pgid, errno = %err, "process group kill failed");
        }
    }
}

/// Cancellation guard for an in-flight child process group.
///
/// `start()` / `run_conformance()` futures can be dropped mid-flight (an
/// axum client disconnect, an outer `select!`/timeout). With
/// `kill_on_drop(false)` a plainly-dropped [`Child`] would orphan the whole
/// spawned group, so every pre-return child is held armed by this guard:
/// dropping it WITHOUT [`Self::defuse`] SIGKILLs the process group and
/// best-effort reaps the child (a spawned task when a runtime is available,
/// else a detached thread blocking on `waitpid`). Every normal completion
/// path defuses the guard and takes ownership of the child back.
#[derive(Debug)]
pub struct ChildGroupGuard {
    pgid: i32,
    child: Option<Child>,
}

impl ChildGroupGuard {
    /// Arm a guard over `child`, whose process group is `pgid`.
    pub fn new(child: Child, pgid: i32) -> Self {
        Self {
            pgid,
            child: Some(child),
        }
    }

    /// Mutable child access (`wait`/`try_wait`) while the guard stays armed.
    pub fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("guard already defused")
    }

    /// Disarm the guard and take the child back (normal completion).
    pub fn defuse(mut self) -> Child {
        self.child.take().expect("guard already defused")
    }
}

impl Drop for ChildGroupGuard {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return; // defused: ownership was handed back on a normal path
        };
        // An already-reaped child needs no signal — and its PGID may have
        // been recycled, so killpg would be dangerous.
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        let pgid = self.pgid;
        tracing::warn!(
            pgid,
            "in-flight engine future dropped; killing its process group"
        );
        kill_group_quiet(pgid);
        // Best-effort reap (no zombies): Drop cannot await, so hand the
        // SIGKILLed child to a spawned task, or — outside a runtime, where
        // tokio's orphan reaper is not running — to a detached thread.
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let _ = child.wait().await;
                });
            }
            Err(_) => {
                std::thread::spawn(move || {
                    let _ = nix::sys::wait::waitpid(Pid::from_raw(pgid), None);
                    drop(child);
                });
            }
        }
    }
}

/// A spawned `supervise` child: the handle, its PID (== PGID, own process
/// group), the shared output ring buffer fed by the stdout AND stderr drain
/// tasks, the join handles of those two tasks, and the line-framed stdout
/// stream the journaling layer consumes.
///
/// STREAM OWNERSHIP (issues #50 + #25): a child's stdout can be drained
/// exactly once, so the single stdout drain FANS OUT — each line is both
/// appended to the shared [`OutputBuffer`] (#50's readiness/panic scan) AND
/// forwarded to [`Self::stdout_lines`] (#25's `RAISED:` journaling). There is
/// exactly one consumer of `child.stdout`.
#[derive(Debug)]
pub struct SpawnedChild {
    pub child: Child,
    pub pid: i32,
    pub output: OutputBuffer,
    /// Drain task handles: `[stdout, stderr]`. Held so the caller can abort
    /// both on teardown; a child's stdout/stderr can each be drained only
    /// once, so the stdout tap (issue #25 RAISED-line journaling) fans out
    /// from the stdout drain rather than re-piping.
    pub drains: [tokio::task::JoinHandle<()>; 2],
    /// Complete stdout lines (without trailing newlines), capped per line at
    /// [`STDOUT_LINE_CAP_BYTES`], fanned out from the stdout drain. Raw bytes:
    /// `RAISED:` payload parsing (including lossy-UTF-8 handling) is owned by
    /// the journal layer.
    pub stdout_lines: tokio::sync::mpsc::Receiver<Vec<u8>>,
}

/// Spawn `fkst-framework supervise` for the materialized package at
/// `pkg_root` with runtime root `rt_root`.
///
/// - Own process group (`process_group(0)`): PGID == child PID, so a single
///   `killpg` reaps the supervisor and its framework grandchildren.
/// - `kill_on_drop(false)`: dropping the handle never kills a live engine;
///   lifecycle is managed explicitly by the runner.
/// - Env: `FKST_RUNTIME_ROOT=<rt_root>`, `FKST_DURABLE_ROOT=<rt_root>/durable`
///   (fresh per attempt — a stale `delivery.redb` would replay lease state,
///   spike Q6). `FKST_PACKAGE_ROOT`/`FKST_PACKAGE_ROOTS` are removed so the
///   `--package-root` flag is the single wiring (untested precedence,
///   spike Q8).
/// - BOTH stdout and stderr are piped and drained into one shared
///   [`OutputBuffer`] by two background tasks for the child's lifetime,
///   preventing pipe backpressure. The readiness markers (`event runtime
///   running`, `consumer started`) arrive on STDOUT and the panic marker on
///   STDERR (issue #50), so the merged buffer is what [`is_ready`] /
///   [`is_panicked`] must scan.
/// - The stdout drain ADDITIONALLY fans each line out to
///   [`SpawnedChild::stdout_lines`] (the journaling layer's `RAISED:` stream,
///   issue #25). This is OBSERVATION ONLY: args, env, and `FKST_RUNTIME_ROOT`
///   wiring are byte-identical to the pre-tap contract (CANON: the engine
///   invocation is never altered), and a slow/absent journal consumer drops
///   lines instead of backpressuring the engine.
pub fn spawn_supervise(
    framework_bin: &Path,
    pkg_root: &Path,
    rt_root: &Path,
) -> Result<SpawnedChild, RunnerError> {
    let mut command = Command::new(framework_bin);
    command
        .arg("supervise")
        .arg("--project-root")
        .arg(pkg_root)
        .arg("--package-root")
        .arg(pkg_root)
        .arg("--framework-bin")
        .arg(framework_bin)
        .env("FKST_RUNTIME_ROOT", rt_root)
        .env("FKST_DURABLE_ROOT", rt_root.join("durable"))
        .env_remove("FKST_PACKAGE_ROOT")
        .env_remove("FKST_PACKAGE_ROOTS")
        .process_group(0)
        .kill_on_drop(false)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(RunnerError::Spawn)?;
    let pid = child.id().map(|id| id as i32).ok_or_else(|| {
        RunnerError::Spawn(std::io::Error::other(
            "supervise child exited before its pid could be read",
        ))
    })?;
    let stdout_pipe = child.stdout.take().ok_or_else(|| {
        RunnerError::Spawn(std::io::Error::other("supervise stdout pipe missing"))
    })?;
    let stderr_pipe = child.stderr.take().ok_or_else(|| {
        RunnerError::Spawn(std::io::Error::other("supervise stderr pipe missing"))
    })?;

    let buffer = OutputBuffer::new(OUTPUT_RING_CAP_BYTES);
    // The readiness markers (`event runtime running`, `consumer started`)
    // arrive on STDOUT — this is the SHARED SOURCE for them. A child's stdout
    // can be drained exactly once, so the stdout drain FANS OUT: each line is
    // appended to the shared buffer (readiness/panic scan, #50) AND forwarded
    // to the journal channel (#25). Re-piping stdout for a second consumer is
    // impossible — there is exactly one reader of `child.stdout`.
    let (stdout_tx, stdout_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(STDOUT_CHANNEL_CAPACITY);
    let stdout_drain = spawn_stdout_fanout_drain(stdout_pipe, buffer.clone(), stdout_tx);
    // Panics surface on STDERR; both drains append to the same buffer.
    let stderr_drain = spawn_stream_drain(stderr_pipe, buffer.clone());

    tracing::info!(pid, pkg_root = %pkg_root.display(), rt_root = %rt_root.display(), "session.spawn");

    Ok(SpawnedChild {
        child,
        pid,
        output: buffer,
        drains: [stdout_drain, stderr_drain],
        stdout_lines: stdout_rx,
    })
}

/// Spawn a background task that drains one child stream (`pipe`) line-by-line
/// into the shared `buffer` for the child's lifetime, preventing pipe
/// backpressure. Each line is length-capped at [`OUTPUT_LINE_CAP_BYTES`] (a
/// newline-free blast is truncated and the remainder discarded — counted,
/// never stored), and the buffer itself is ring-capped at
/// [`OUTPUT_RING_CAP_BYTES`]. Both the stdout and stderr drains share one
/// `buffer`, so the merged view is what the marker scans read.
fn spawn_stream_drain<R>(pipe: R, buffer: OutputBuffer) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(pipe);
        let mut line_buf: Vec<u8> = Vec::with_capacity(256);
        loop {
            line_buf.clear();
            // Length-capped read: a newline-free blast can never grow memory
            // past the per-line cap (next_line() would buffer the whole
            // physical line). The remainder of an oversized line is discarded
            // — counted, never stored.
            let mut limited = (&mut reader).take(OUTPUT_LINE_CAP_BYTES as u64);
            let read = match limited.read_until(b'\n', &mut line_buf).await {
                Ok(0) => break, // EOF
                Ok(n) => n,
                Err(_) => break,
            };
            let newline_terminated = line_buf.last() == Some(&b'\n');
            if newline_terminated {
                line_buf.pop();
                if line_buf.last() == Some(&b'\r') {
                    line_buf.pop();
                }
            }
            let mut line = String::from_utf8_lossy(&line_buf).into_owned();
            if !newline_terminated && read == OUTPUT_LINE_CAP_BYTES {
                let dropped = discard_until_newline(&mut reader).await;
                line.push_str(&format!(" [line truncated: {dropped} bytes dropped]"));
            }
            buffer.append_line(&line);
        }
    })
}

/// Spawn the STDOUT drain that FANS OUT: like [`spawn_stream_drain`] it reads
/// the child's stdout line-by-line and appends each line to the shared
/// `buffer` (so #50's readiness/panic scan sees the markers), AND it forwards
/// the raw line bytes to `journal_tx` (issue #25's `RAISED:` journaling). One
/// read loop, two sinks — `child.stdout` is consumed exactly once.
///
/// The two sinks have INDEPENDENT line caps. The shared buffer keeps #50's
/// 8 KiB [`OUTPUT_LINE_CAP_BYTES`] discipline (readiness markers are short);
/// the journal channel keeps issue #25's larger [`STDOUT_LINE_CAP_BYTES`]
/// (the journaler's own `FKST_RAISED_MAX_LINE_BYTES` makes the
/// oversize/malformed verdict). Forwarding to the journal is LOSSY by design:
/// a full or closed channel drops the line (counted + warned) so journaling
/// can never backpressure the engine's pipe.
fn spawn_stdout_fanout_drain<R>(
    pipe: R,
    buffer: OutputBuffer,
    journal_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        // Read up to the LARGER journal cap so a long RAISED payload reaches
        // the journaler intact; the shared-buffer sink truncates its own copy
        // to OUTPUT_LINE_CAP_BYTES (the readiness markers are short, so the
        // scan is unaffected by an oversized RAISED line).
        let mut reader = BufReader::new(pipe);
        let mut line_buf: Vec<u8> = Vec::with_capacity(256);
        let mut dropped_lines: u64 = 0;
        loop {
            line_buf.clear();
            let mut limited = (&mut reader).take(STDOUT_LINE_CAP_BYTES as u64);
            let read = match limited.read_until(b'\n', &mut line_buf).await {
                Ok(0) => break, // EOF
                Ok(n) => n,
                Err(_) => break,
            };
            let newline_terminated = line_buf.last() == Some(&b'\n');
            if newline_terminated {
                line_buf.pop();
                if line_buf.last() == Some(&b'\r') {
                    line_buf.pop();
                }
            } else if read == STDOUT_LINE_CAP_BYTES {
                // Oversized physical line: discard its remainder (counted,
                // never stored). The journaler's own byte cap will declare
                // the truncated payload malformed.
                let _ = discard_until_newline(&mut reader).await;
            }

            // Sink 1: the shared buffer (readiness/panic scan, #50). Keep the
            // buffer's own 8 KiB discipline so a long RAISED line cannot bloat
            // the ring; the markers it scans for are short.
            let decoded = String::from_utf8_lossy(&line_buf);
            let buffer_line = if decoded.len() > OUTPUT_LINE_CAP_BYTES {
                let mut cut = OUTPUT_LINE_CAP_BYTES;
                while cut > 0 && !decoded.is_char_boundary(cut) {
                    cut -= 1;
                }
                format!("{} [line truncated]", &decoded[..cut])
            } else {
                decoded.into_owned()
            };
            buffer.append_line(&buffer_line);

            // Sink 2: the journal channel (#25). Lossy: a full/closed channel
            // drops the line rather than stalling the engine's pipe.
            match journal_tx.try_send(std::mem::take(&mut line_buf)) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    dropped_lines += 1;
                    if dropped_lines == 1 || dropped_lines.is_multiple_of(1000) {
                        tracing::warn!(
                            dropped_lines,
                            "journal stdout consumer behind; dropping lines"
                        );
                    }
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    // No journal consumer: keep draining to EOF so the buffer
                    // sink and the pipe stay live; just stop forwarding.
                }
            }
        }
        if dropped_lines > 0 {
            tracing::warn!(
                dropped_lines,
                "journal stdout drain finished with dropped lines"
            );
        }
    })
}

/// Run the `conformance` pre-flight in its own process group under a
/// wall-clock cap.
///
/// Exit 0 => `Ok(())`. Non-zero => `ConformanceFailed { code, stderr }` with
/// the captured stderr+stdout (lossy, truncated to `error_capture_bytes`).
/// Timeout => the conformance process GROUP is SIGKILLed, the child reaped,
/// and `ConformanceFailed { code: -1 }` returned with a timeout message.
pub async fn run_conformance(
    framework_bin: &Path,
    pkg_root: &Path,
    rt_root: &Path,
    timeout: Duration,
    error_capture_bytes: usize,
) -> Result<(), RunnerError> {
    let started = Instant::now();
    let mut command = Command::new(framework_bin);
    command
        .arg("conformance")
        .arg("--project-root")
        .arg(pkg_root)
        .arg("--package-root")
        .arg(pkg_root)
        .env("FKST_RUNTIME_ROOT", rt_root)
        .env_remove("FKST_PACKAGE_ROOT")
        .env_remove("FKST_PACKAGE_ROOTS")
        .process_group(0)
        .kill_on_drop(false)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(RunnerError::Spawn)?;
    let pid = child.id().map(|id| id as i32).ok_or_else(|| {
        RunnerError::Spawn(std::io::Error::other(
            "conformance child exited before its pid could be read",
        ))
    })?;

    // Drain both pipes concurrently so a chatty pre-flight can never block
    // on a full pipe while we wait on the exit status.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_task = tokio::spawn(collect_capped(stdout_pipe));
    let stderr_task = tokio::spawn(collect_capped(stderr_pipe));

    // Cancellation safety: dropping this future mid-wait (client disconnect,
    // outer timeout) must not orphan the conformance group — the armed guard
    // group-kills and reaps on drop; every arm below defuses it explicitly.
    let mut guard = ChildGroupGuard::new(child, pid);

    match tokio::time::timeout(timeout, guard.child_mut().wait()).await {
        Ok(Ok(status)) => {
            drop(guard.defuse()); // reaped by wait(); nothing left to kill
            let stderr_bytes = stderr_task.await.unwrap_or_default();
            let stdout_bytes = stdout_task.await.unwrap_or_default();
            let duration_ms = started.elapsed().as_millis() as u64;
            if status.success() {
                tracing::info!(duration_ms, exit_code = 0, "session.conformance");
                return Ok(());
            }
            // Exit-by-signal has no code; -1 mirrors the timeout convention.
            let code = status.code().unwrap_or(-1);
            let mut combined = stderr_bytes;
            combined.extend_from_slice(&stdout_bytes);
            let stderr = truncate_output_lossy(&combined, error_capture_bytes);
            tracing::error!(duration_ms, exit_code = code, stderr = %stderr, "session.conformance failed");
            Err(RunnerError::ConformanceFailed { code, stderr })
        }
        Ok(Err(io)) => {
            // wait() itself failed: group-kill, reap, and stop the capture
            // tasks before surfacing the IO error (parity with the timeout
            // arm — no orphans, no zombies, no leaked tasks).
            let mut child = guard.defuse();
            kill_group_quiet(pid);
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            Err(RunnerError::Io(io))
        }
        Err(_elapsed) => {
            // Group-kill (conformance may have spawned its own children),
            // then ALWAYS reap our direct child — no zombies.
            let mut child = guard.defuse();
            kill_group_quiet(pid);
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            let secs = timeout.as_secs();
            tracing::error!(pid, timeout_secs = secs, "session.conformance timed out");
            Err(RunnerError::ConformanceFailed {
                code: -1,
                stderr: format!("conformance timed out after {secs}s"),
            })
        }
    }
}

/// Skip the rest of an oversized physical line: consume bytes (without
/// storing them) up to and including the next `\n`, or to EOF/error.
/// Returns the number of bytes discarded.
async fn discard_until_newline<R>(reader: &mut R) -> u64
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut dropped: u64 = 0;
    loop {
        let (consumed, found_newline) = {
            let available = match reader.fill_buf().await {
                Ok(buf) => buf,
                Err(_) => return dropped,
            };
            if available.is_empty() {
                return dropped; // EOF
            }
            match available.iter().position(|&byte| byte == b'\n') {
                Some(index) => (index + 1, true),
                None => (available.len(), false),
            }
        };
        reader.consume(consumed);
        dropped += consumed as u64;
        if found_newline {
            return dropped;
        }
    }
}

/// Read a pipe to EOF, byte-capped at [`CONFORMANCE_CAPTURE_LIMIT`].
async fn collect_capped<R>(pipe: Option<R>) -> Vec<u8>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut collected = Vec::new();
    if let Some(reader) = pipe {
        let _ = reader
            .take(CONFORMANCE_CAPTURE_LIMIT)
            .read_to_end(&mut collected)
            .await;
    }
    collected
}

/// Result of [`reap_with_grace`].
#[derive(Debug, Clone, Copy)]
pub struct ReapOutcome {
    /// True when escalation to SIGKILL was needed.
    pub escalated: bool,
    /// Exit status observed BEFORE any stop signal was sent: the child was
    /// already terminal when the reap began (e.g. a crash racing `stop()`),
    /// so the caller must CLASSIFY this exit instead of assuming a clean
    /// stop. `None` when the exit was caused by the reap's own signalling.
    pub pre_signal_exit: Option<std::process::ExitStatus>,
}

/// Stop and reap a process group: SIGTERM the group, poll `try_wait` every
/// 100 ms up to `grace`, escalate to SIGKILL, bounded re-poll, and ALWAYS
/// reap the held child (no zombies).
///
/// Returns `Ok(ReapOutcome)` once the direct child is reaped. An
/// already-dead group (`ESRCH`) is a no-op success carrying the observed
/// exit as `pre_signal_exit`. `Err(Signal)` only when even SIGKILL leaves
/// the child unreaped.
pub async fn reap_with_grace(
    child: &mut Child,
    pgid: i32,
    grace: Duration,
) -> Result<ReapOutcome, RunnerError> {
    let started = Instant::now();

    // Already exited (self-exit, out-of-band kill, or a previous reap —
    // tokio's Child caches the exit status): no signal needed at all. The
    // observed exit predates our signalling, so it is surfaced for the
    // caller to classify (a crash racing stop must stay a crash).
    if let Some(status) = child.try_wait().map_err(RunnerError::Io)? {
        tracing::debug!(pgid, exit = ?status, "session.stop: child already exited");
        return Ok(ReapOutcome {
            escalated: false,
            pre_signal_exit: Some(status),
        });
    }

    match signal_group(pgid, Signal::SIGTERM) {
        Ok(()) => {}
        // ESRCH: group already gone. EPERM: Darwin reports EPERM (not
        // ESRCH) when signalling a group whose members are all zombies —
        // tolerated; the try_wait poll below is the reaping authority.
        Err(nix::Error::ESRCH) | Err(nix::Error::EPERM) => {
            tracing::debug!(pgid, "session.stop: group not signalable (already gone)");
        }
        Err(err) => return Err(RunnerError::Signal(err)),
    }

    loop {
        if let Some(status) = child.try_wait().map_err(RunnerError::Io)? {
            tracing::info!(
                pgid,
                exit = ?status,
                reaped_in_ms = started.elapsed().as_millis() as u64,
                escalated_to_sigkill = false,
                "session.stop"
            );
            return Ok(ReapOutcome {
                escalated: false,
                pre_signal_exit: None,
            });
        }
        if started.elapsed() >= grace {
            break;
        }
        tokio::time::sleep(REAP_POLL_INTERVAL).await;
    }

    match signal_group(pgid, Signal::SIGKILL) {
        Ok(()) | Err(nix::Error::ESRCH) | Err(nix::Error::EPERM) => {}
        Err(err) => return Err(RunnerError::Signal(err)),
    }

    for _ in 0..SIGKILL_REAP_POLLS {
        if let Some(status) = child.try_wait().map_err(RunnerError::Io)? {
            tracing::info!(
                pgid,
                exit = ?status,
                reaped_in_ms = started.elapsed().as_millis() as u64,
                escalated_to_sigkill = true,
                "session.stop"
            );
            return Ok(ReapOutcome {
                escalated: true,
                pre_signal_exit: None,
            });
        }
        tokio::time::sleep(REAP_POLL_INTERVAL).await;
    }

    tracing::error!(pgid, "session.stop: process group survived SIGKILL");
    Err(RunnerError::Signal(nix::Error::ETIMEDOUT))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;

    /// Write an executable `/bin/sh` stub script and return its path.
    fn stub(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
        path
    }

    /// Poll `predicate` every 25 ms for up to ~20 s. The wide budget keeps
    /// these real-`sh`-child drain tests reliable under a saturated
    /// full-workspace run (many parallel engine spawns + containers), where
    /// the first drained line can take well over a few seconds to surface.
    async fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
        for _ in 0..800 {
            if predicate() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }

    // ---- OutputBuffer --------------------------------------------------------

    #[test]
    fn output_buffer_keeps_the_newest_tail_within_cap() {
        let buffer = OutputBuffer::new(32);
        for i in 0..100 {
            buffer.append_line(&format!("line-{i:03}"));
        }
        let snapshot = buffer.snapshot();
        assert!(snapshot.len() <= 32, "got {} bytes", snapshot.len());
        assert!(snapshot.contains("line-099"), "newest line must survive");
        assert!(!snapshot.contains("line-000"), "oldest must be trimmed");
    }

    #[test]
    fn output_buffer_trims_at_a_char_boundary() {
        let buffer = OutputBuffer::new(10);
        buffer.append_line("ααααααααα"); // 9 x 2-byte chars + \n = 19 bytes
        let snapshot = buffer.snapshot();
        assert!(snapshot.len() <= 10);
        // String invariants prove validity; the content is the tail.
        assert!(snapshot.ends_with("\n"));
    }

    #[test]
    fn output_buffer_clones_share_the_same_storage() {
        let buffer = OutputBuffer::new(1024);
        let writer = buffer.clone();
        writer.append_line("shared");
        assert!(buffer.snapshot().contains("shared"));
    }

    // ---- ready / panic scans ---------------------------------------------------

    #[test]
    fn is_ready_requires_both_markers() {
        let runtime_only = "INFO fkst_framework::supervise: event runtime running handles=3\n";
        let consumer_only =
            "INFO consumer started dept=hello reliable_queues=[\"tick\"] ephemeral_queues=[]\n";
        let both = format!("{runtime_only}{consumer_only}");

        // The half-alive guard: runtime-running alone is NOT ready (A3).
        assert!(!is_ready(runtime_only));
        assert!(!is_ready(consumer_only));
        assert!(is_ready(&both));
        assert!(!is_ready(""));
    }

    #[test]
    fn is_ready_matches_the_real_engine_ansi_styled_lines() {
        // Verbatim raw stderr from the pinned engine image (field names are
        // ANSI-styled separately from their values, so `handles=`/`dept=`
        // never appear contiguously in the raw stream).
        let raw = "\u{1b}[2m2026-06-11T05:03:55.974159Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m \
                   \u{1b}[2mfkst_framework::supervise\u{1b}[0m\u{1b}[2m:\u{1b}[0m event runtime \
                   running \u{1b}[3mhandles\u{1b}[0m\u{1b}[2m=\u{1b}[0m2\n\
                   \u{1b}[2m2026-06-11T05:03:55.974194Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m \
                   \u{1b}[2mfkst_framework::supervise::consumer\u{1b}[0m\u{1b}[2m:\u{1b}[0m \
                   consumer started \u{1b}[3mdept\u{1b}[0m\u{1b}[2m=\u{1b}[0mhello\n";
        assert!(
            is_ready(raw),
            "ANSI-styled real-engine output must be ready"
        );
        // The first (runtime-running) line alone stays NOT ready.
        let first_line_only = raw.lines().next().unwrap();
        assert!(!is_ready(first_line_only));
    }

    #[test]
    fn is_panicked_detects_panic_lines() {
        let panic_line = "thread 'main' (236) panicked at crates/x/src/consumer.rs:59:14:\n";
        assert!(is_panicked(panic_line));
        assert!(!is_panicked("event runtime running handles=3\n"));
    }

    // ---- is_pid_alive ------------------------------------------------------------

    #[tokio::test]
    async fn is_pid_alive_tracks_a_real_process_lifecycle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = stub(dir.path(), "sleeper.sh", "sleep 30");
        let mut child = Command::new(&script)
            .process_group(0)
            .kill_on_drop(false)
            .spawn()
            .expect("spawn sleeper");
        let pid = child.id().expect("pid") as i32;

        assert!(is_pid_alive(pid), "live process must be alive");

        signal_group(pid, Signal::SIGKILL).expect("kill group");
        child.wait().await.expect("reap");
        assert!(!is_pid_alive(pid), "reaped process must be gone (ESRCH)");
    }

    #[test]
    fn is_pid_alive_treats_eperm_as_alive() {
        // PID 1 exists but is not signalable by an unprivileged test (EPERM).
        assert!(is_pid_alive(1));
    }

    // ---- spawn_supervise --------------------------------------------------------

    #[tokio::test]
    async fn spawn_supervise_wires_args_env_and_own_process_group() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        // The stub leaks the parent var if env_remove is missing.
        std::env::set_var("FKST_PACKAGE_ROOT", "/should/be/removed");
        let script = stub(
            dir.path(),
            "fake-supervise.sh",
            r#"echo "args: $*" >&2
echo "rt: $FKST_RUNTIME_ROOT" >&2
echo "durable: $FKST_DURABLE_ROOT" >&2
echo "pkgroot: ${FKST_PACKAGE_ROOT:-UNSET}" >&2
echo "pkgroots: ${FKST_PACKAGE_ROOTS:-UNSET}" >&2
echo "event runtime running handles=3" >&2
echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
sleep 30"#,
        );

        let mut spawned =
            spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn stub supervise");
        std::env::remove_var("FKST_PACKAGE_ROOT");

        // Own process group: PGID == PID.
        let pgid = nix::unistd::getpgid(Some(Pid::from_raw(spawned.pid))).expect("getpgid");
        assert_eq!(pgid.as_raw(), spawned.pid, "child must lead its own group");

        // The drain task must surface the stub's stderr.
        let stderr = spawned.output.clone();
        assert!(
            wait_until(|| is_ready(&stderr.snapshot())).await,
            "ready markers must arrive via the stderr buffer"
        );

        let snapshot = spawned.output.snapshot();
        let expected_args = format!(
            "args: supervise --project-root {} --package-root {} --framework-bin {}",
            pkg.path().display(),
            pkg.path().display(),
            script.display()
        );
        assert!(snapshot.contains(&expected_args), "got:\n{snapshot}");
        assert!(snapshot.contains(&format!("rt: {}", rt.path().display())));
        assert!(snapshot.contains(&format!("durable: {}/durable", rt.path().display())));
        assert!(
            snapshot.contains("pkgroot: UNSET"),
            "FKST_PACKAGE_ROOT must be removed from the child env"
        );
        assert!(
            snapshot.contains("pkgroots: UNSET"),
            "FKST_PACKAGE_ROOTS must be removed from the child env"
        );

        let outcome = reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap");
        assert!(!outcome.escalated, "sh dies on SIGTERM without escalation");
        assert!(
            outcome.pre_signal_exit.is_none(),
            "exit was caused by the reap's own SIGTERM"
        );
        assert!(!is_pid_alive(spawned.pid));
    }

    #[tokio::test]
    async fn stdout_tap_delivers_line_framed_raised_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        // RAISED traffic goes to STDOUT; the ready markers stay on stderr —
        // exactly the engine's split.
        let script = stub(
            dir.path(),
            "raiser.sh",
            r#"echo "RAISED: eyJkZXB0IjoiaGVsbG8ifQ=="
echo "plain chatter line"
echo "event runtime running handles=3" >&2
sleep 30"#,
        );

        let mut spawned = spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn raiser");
        let first = tokio::time::timeout(Duration::from_secs(20), spawned.stdout_lines.recv())
            .await
            .expect("first stdout line within 20s")
            .expect("channel open");
        assert_eq!(first, b"RAISED: eyJkZXB0IjoiaGVsbG8ifQ==".to_vec());
        let second = tokio::time::timeout(Duration::from_secs(20), spawned.stdout_lines.recv())
            .await
            .expect("second stdout line within 20s")
            .expect("channel open");
        assert_eq!(second, b"plain chatter line".to_vec());

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap raiser");
    }

    #[tokio::test]
    async fn stdout_tap_leaves_the_engine_invocation_unchanged() {
        // CANON hard rule: tapping stdout is OBSERVATION ONLY. This asserts
        // the exact `supervise` argv and env wiring (the issue #17 contract)
        // with the tap active — byte-identical to the pre-tap invocation.
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(
            dir.path(),
            "contract.sh",
            r#"echo "argv-raw: $*"
echo "rt: $FKST_RUNTIME_ROOT"
echo "durable: $FKST_DURABLE_ROOT"
echo "pkgroot: ${FKST_PACKAGE_ROOT:-UNSET}"
echo "pkgroots: ${FKST_PACKAGE_ROOTS:-UNSET}"
sleep 30"#,
        );

        let mut spawned = spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn");
        let mut lines: Vec<String> = Vec::new();
        while lines.len() < 5 {
            let line = tokio::time::timeout(Duration::from_secs(20), spawned.stdout_lines.recv())
                .await
                .expect("stdout line within 20s")
                .expect("channel open");
            lines.push(String::from_utf8(line).expect("utf8 stub output"));
        }
        let joined = lines.join("\n");
        let expected_args = format!(
            "argv-raw: supervise --project-root {} --package-root {} --framework-bin {}",
            pkg.path().display(),
            pkg.path().display(),
            script.display()
        );
        assert!(joined.contains(&expected_args), "argv changed:\n{joined}");
        assert!(joined.contains(&format!("rt: {}", rt.path().display())));
        assert!(joined.contains(&format!("durable: {}/durable", rt.path().display())));
        assert!(joined.contains("pkgroot: UNSET"));
        assert!(joined.contains("pkgroots: UNSET"));

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap");
    }

    #[tokio::test]
    async fn stdout_drain_caps_a_newline_free_blast_and_keeps_flowing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        // A 3 MiB newline-free stdout blast exceeds the 2 MiB per-line cap:
        // it must arrive truncated (bounded memory) and later lines must
        // still flow.
        let script = stub(
            dir.path(),
            "stdout-blaster.sh",
            r#"head -c 3145728 /dev/zero | tr '\0' x
printf '\n'
echo "after-blast"
sleep 30"#,
        );

        let mut spawned = spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn");
        let blast = tokio::time::timeout(Duration::from_secs(30), spawned.stdout_lines.recv())
            .await
            .expect("blast line within 30s")
            .expect("channel open");
        assert_eq!(blast.len(), STDOUT_LINE_CAP_BYTES, "truncated at the cap");
        let after = tokio::time::timeout(Duration::from_secs(20), spawned.stdout_lines.recv())
            .await
            .expect("post-blast line within 20s")
            .expect("channel open");
        assert_eq!(after, b"after-blast".to_vec());

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap");
    }

    #[tokio::test]
    async fn dropped_stdout_receiver_never_stalls_the_engine() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        // Far more lines than the channel capacity, then a breadcrumb file:
        // with NO consumer the drain must keep the pipe flowing so the stub
        // reaches the end of its output.
        let done = dir.path().join("done");
        let script = stub(
            dir.path(),
            "chatty.sh",
            &format!(
                r#"i=0
while [ $i -lt 5000 ]; do echo "line $i"; i=$((i+1)); done
echo done > {}
sleep 30"#,
                done.display()
            ),
        );

        let mut spawned = spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn");
        drop(std::mem::replace(
            &mut spawned.stdout_lines,
            tokio::sync::mpsc::channel(1).1,
        ));
        assert!(
            wait_until(|| done.exists()).await,
            "engine must finish its output with no stdout consumer"
        );

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap");
    }

    #[tokio::test]
    async fn stderr_drain_caps_a_newline_free_blast() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        // 200 000 'x' bytes with NO newline, then a normal line: without the
        // length-capped read the drain task would buffer the entire blast.
        let script = stub(
            dir.path(),
            "blaster.sh",
            r#"head -c 200000 /dev/zero | tr '\0' x >&2
printf '\n' >&2
echo "after-blast" >&2
sleep 30"#,
        );

        let mut spawned = spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn blaster");
        let stderr = spawned.output.clone();
        assert!(
            wait_until(|| stderr.snapshot().contains("after-blast")).await,
            "post-blast output must still flow through the drain"
        );

        let snapshot = spawned.output.snapshot();
        assert!(
            snapshot.contains("[line truncated: "),
            "oversized line must carry the dropped-bytes note:\n{snapshot}"
        );
        // No single stored line may exceed the cap (plus the short note).
        for line in snapshot.lines() {
            assert!(
                line.len() <= OUTPUT_LINE_CAP_BYTES + 64,
                "line of {} bytes exceeds the per-line cap",
                line.len()
            );
        }

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap blaster");
    }

    #[tokio::test]
    async fn spawn_supervise_with_missing_binary_is_a_spawn_error() {
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let err = spawn_supervise(
            Path::new("/definitely/missing/fkst-framework"),
            pkg.path(),
            rt.path(),
        )
        .expect_err("missing binary must fail to spawn");
        assert!(matches!(err, RunnerError::Spawn(_)));
    }

    /// Issue #50 regression: the REAL `fkst-framework supervise` writes its
    /// readiness markers to STDOUT, not stderr. The drain must merge stdout
    /// into the scanned buffer, or the marker-gated ready-wait times out
    /// forever. This stub prints both markers on STDOUT only — it FAILS on the
    /// stderr-only wiring and PASSES once stdout is drained too.
    #[tokio::test]
    async fn spawn_supervise_surfaces_ready_markers_emitted_on_stdout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        // Markers go to STDOUT (no `>&2`) — exactly how supervise emits them.
        let script = stub(
            dir.path(),
            "stdout-ready.sh",
            r#"echo "event runtime running handles=3"
echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]"
sleep 30"#,
        );

        let mut spawned =
            spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn stdout-ready stub");
        let output = spawned.output.clone();
        assert!(
            wait_until(|| is_ready(&output.snapshot())).await,
            "ready markers emitted on STDOUT must reach the merged buffer"
        );

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap stdout-ready stub");
        assert!(!is_pid_alive(spawned.pid));
    }

    /// Both streams feed the same buffer, so markers on STDERR also still
    /// reach ready (back-compat with any engine build that logged to stderr).
    #[tokio::test]
    async fn spawn_supervise_surfaces_ready_markers_emitted_on_stderr() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(
            dir.path(),
            "stderr-ready.sh",
            r#"echo "event runtime running handles=3" >&2
echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
sleep 30"#,
        );

        let mut spawned =
            spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn stderr-ready stub");
        let output = spawned.output.clone();
        assert!(
            wait_until(|| is_ready(&output.snapshot())).await,
            "ready markers emitted on STDERR must reach the merged buffer"
        );

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap stderr-ready stub");
    }

    /// A panic on STDERR must remain visible after the merge — the panic scan
    /// reads the same buffer the (stdout) ready markers land in.
    #[tokio::test]
    async fn spawn_supervise_surfaces_panic_emitted_on_stderr() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(
            dir.path(),
            "panic.sh",
            r#"echo "event runtime running handles=3"
echo "thread 'main' (1) panicked at src/consumer.rs:1:1:" >&2
sleep 30"#,
        );

        let mut spawned =
            spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn panic stub");
        let output = spawned.output.clone();
        assert!(
            wait_until(|| is_panicked(&output.snapshot())).await,
            "a panic on STDERR must reach the merged buffer"
        );
        // The half-alive guard still holds: a panic without the consumer line
        // is never ready.
        assert!(!is_ready(&spawned.output.snapshot()));

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap panic stub");
    }

    /// A3 half-alive guard across the stream split: `event runtime running` on
    /// STDOUT with NO `consumer started` ANYWHERE must never read as ready,
    /// even though the runtime-running marker is now drained from stdout.
    #[tokio::test]
    async fn spawn_supervise_marker_split_across_streams_is_not_spuriously_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        // runtime-running on stdout; a DECOY on stderr that is NOT the
        // consumer marker. `consumer started` appears nowhere.
        let script = stub(
            dir.path(),
            "half-alive.sh",
            r#"echo "event runtime running handles=3"
echo "WARN department consumer thread exited" >&2
sleep 30"#,
        );

        let mut spawned =
            spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn half-alive stub");
        let output = spawned.output.clone();
        // Wait until the runtime-running marker has definitely been drained.
        assert!(
            wait_until(|| output.snapshot().contains("event runtime running")).await,
            "runtime-running marker must be drained from stdout"
        );
        // Give the stderr drain a moment too, then assert NOT ready.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !is_ready(&spawned.output.snapshot()),
            "runtime-running alone (consumer line absent) must NOT be ready"
        );

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap half-alive stub");
    }

    // ---- reap_with_grace ----------------------------------------------------------

    #[tokio::test]
    async fn reap_with_grace_kills_the_whole_group_including_grandchildren() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        // The stub spawns a long-lived grandchild in the SAME group and
        // reports its pid, then blocks.
        let script = stub(
            dir.path(),
            "group-parent.sh",
            r#"sleep 60 &
echo "grandchild: $!" >&2
wait"#,
        );

        let mut spawned = spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn group");
        let stderr = spawned.output.clone();
        assert!(
            wait_until(|| stderr.snapshot().contains("grandchild: ")).await,
            "stub must report its grandchild pid"
        );
        let snapshot = spawned.output.snapshot();
        let grandchild: i32 = snapshot
            .lines()
            .find_map(|line| line.strip_prefix("grandchild: "))
            .expect("grandchild line")
            .trim()
            .parse()
            .expect("grandchild pid");
        assert!(is_pid_alive(grandchild), "grandchild must start alive");

        reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("reap group");

        assert!(!is_pid_alive(spawned.pid), "group leader must be gone");
        let grandchild_gone = wait_until(|| !is_pid_alive(grandchild)).await;
        assert!(grandchild_gone, "grandchild must die with the group");
        // The whole group is gone: killpg now reports ESRCH.
        assert_eq!(
            signal_group(spawned.pid, Signal::SIGTERM),
            Err(nix::Error::ESRCH)
        );
    }

    #[tokio::test]
    async fn reap_with_grace_escalates_to_sigkill_when_sigterm_is_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(
            dir.path(),
            "term-ignorer.sh",
            r#"trap '' TERM
echo "trap installed" >&2
while true; do sleep 1; done"#,
        );

        let mut spawned = spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn ignorer");
        // Only signal once the stub has confirmed its TERM trap is live.
        let stderr = spawned.output.clone();
        assert!(
            wait_until(|| stderr.snapshot().contains("trap installed")).await,
            "stub must confirm its trap before the test signals"
        );

        let outcome = reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_millis(300))
            .await
            .expect("reap must escalate, not fail");
        assert!(outcome.escalated, "TERM-ignoring child requires SIGKILL");
        assert!(!is_pid_alive(spawned.pid));
    }

    #[tokio::test]
    async fn reap_with_grace_is_a_no_op_success_when_already_dead() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(dir.path(), "instant-exit.sh", "exit 0");

        let mut spawned = spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn");
        // Let the stub exit on its own; it stays a zombie until reaped (no
        // try_wait yet), which on Darwin makes its group EPERM-unsignalable.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let outcome = reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(5))
            .await
            .expect("already-dead group is a no-op success");
        assert!(!outcome.escalated);

        // Second reap on the already-reaped child is still Ok (idempotent),
        // and the cached exit — which predates this reap's (non-)signalling —
        // is surfaced deterministically for the caller to classify.
        let again = reap_with_grace(&mut spawned.child, spawned.pid, Duration::from_secs(1))
            .await
            .expect("double reap must stay Ok");
        assert!(!again.escalated);
        assert!(
            again.pre_signal_exit.is_some(),
            "an exit observed before signalling must be surfaced"
        );
    }

    // ---- run_conformance -------------------------------------------------------------

    #[tokio::test]
    async fn run_conformance_succeeds_on_exit_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(dir.path(), "conf-pass.sh", "echo PASS; exit 0");

        run_conformance(
            &script,
            pkg.path(),
            rt.path(),
            Duration::from_secs(10),
            8192,
        )
        .await
        .expect("exit 0 must pass");
    }

    #[tokio::test]
    async fn run_conformance_captures_failure_output_with_code() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(
            dir.path(),
            "conf-fail.sh",
            r#"echo "FAIL department-non-empty host graph contains no departments" >&2
echo "FAIL on stdout too"
exit 1"#,
        );

        let err = run_conformance(
            &script,
            pkg.path(),
            rt.path(),
            Duration::from_secs(10),
            8192,
        )
        .await
        .expect_err("exit 1 must fail");
        match err {
            RunnerError::ConformanceFailed { code, stderr } => {
                assert_eq!(code, 1);
                assert!(stderr.contains("FAIL department-non-empty"), "{stderr}");
                assert!(stderr.contains("FAIL on stdout too"), "{stderr}");
            }
            other => panic!("expected ConformanceFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_conformance_preserves_exit_code_two() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(
            dir.path(),
            "conf-sdk-err.sh",
            r#"echo "[framework] startup error: canonicalize --project-root" >&2
exit 2"#,
        );

        let err = run_conformance(
            &script,
            pkg.path(),
            rt.path(),
            Duration::from_secs(10),
            8192,
        )
        .await
        .expect_err("exit 2 must fail");
        match err {
            RunnerError::ConformanceFailed { code, stderr } => {
                assert_eq!(code, 2);
                assert!(stderr.contains("startup error"));
            }
            other => panic!("expected ConformanceFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_conformance_truncates_captured_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(
            dir.path(),
            "conf-chatty.sh",
            r#"i=0
while [ $i -lt 200 ]; do echo "noise noise noise noise noise" >&2; i=$((i+1)); done
exit 1"#,
        );

        let err = run_conformance(&script, pkg.path(), rt.path(), Duration::from_secs(10), 64)
            .await
            .expect_err("exit 1 must fail");
        match err {
            RunnerError::ConformanceFailed { stderr, .. } => {
                assert!(stderr.len() <= 64, "got {} bytes", stderr.len());
            }
            other => panic!("expected ConformanceFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_conformance_times_out_and_group_kills() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let pid_file = dir.path().join("conf.pid");
        let script = stub(
            dir.path(),
            "conf-hang.sh",
            &format!("echo $$ > {}\nsleep 60", pid_file.display()),
        );

        let err = run_conformance(&script, pkg.path(), rt.path(), Duration::from_secs(2), 8192)
            .await
            .expect_err("hang must time out");
        match err {
            RunnerError::ConformanceFailed { code, stderr } => {
                assert_eq!(code, -1, "timeout uses code -1");
                assert!(stderr.contains("timed out"), "{stderr}");
            }
            other => panic!("expected ConformanceFailed, got {other:?}"),
        }

        // The hung conformance group must be dead. Under pathological host
        // load the stub may have been SIGKILLed before writing its pid
        // breadcrumb — then there is no live group to assert against (the
        // group-kill mechanics are covered by the reap tests).
        match fs::read_to_string(&pid_file) {
            Ok(raw) => {
                let pid: i32 = raw.trim().parse().expect("pid");
                assert!(
                    wait_until(move || !is_pid_alive(pid)).await,
                    "timed-out conformance must be group-killed"
                );
            }
            Err(_) => eprintln!("NOTE: stub killed before writing its pid breadcrumb"),
        }
    }

    #[tokio::test]
    async fn dropping_run_conformance_mid_wait_kills_and_reaps_the_group() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let pid_file = dir.path().join("conf.pid");
        let script = stub(
            dir.path(),
            "conf-hang.sh",
            &format!("echo $$ > {}\nsleep 60", pid_file.display()),
        );

        // Drop the in-flight future mid-wait (an outer select! racing it),
        // exactly the axum-disconnect shape.
        {
            let fut = run_conformance(
                &script,
                pkg.path(),
                rt.path(),
                Duration::from_secs(60),
                8192,
            );
            tokio::pin!(fut);
            tokio::select! {
                res = &mut fut => panic!("conformance must still be in flight, got {res:?}"),
                _ = async {
                    while !pid_file.exists() {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                } => {}
            }
        } // <- future (and its armed guard) dropped here

        let conf_pid: i32 = fs::read_to_string(&pid_file)
            .expect("pid breadcrumb")
            .trim()
            .parse()
            .expect("pid");
        // is_pid_alive turns false only once the child is REAPED (a zombie
        // still answers kill(pid, 0)), so this asserts kill AND reap.
        assert!(
            wait_until(move || !is_pid_alive(conf_pid)).await,
            "dropped conformance future must kill and reap its group"
        );
    }

    #[tokio::test]
    async fn defused_guard_never_kills_the_child() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let script = stub(dir.path(), "sleeper.sh", "sleep 30");

        let spawned = spawn_supervise(&script, pkg.path(), rt.path()).expect("spawn");
        let pid = spawned.pid;
        let guard = ChildGroupGuard::new(spawned.child, pid);
        let mut child = guard.defuse(); // ownership handed back: drop is a no-op
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(is_pid_alive(pid), "defused guard must not kill the child");

        reap_with_grace(&mut child, pid, Duration::from_secs(5))
            .await
            .expect("cleanup reap");
        assert!(!is_pid_alive(pid));
    }

    #[tokio::test]
    async fn run_conformance_with_missing_binary_is_a_spawn_error() {
        let pkg = tempfile::tempdir().expect("pkg dir");
        let rt = tempfile::tempdir().expect("rt dir");
        let err = run_conformance(
            Path::new("/definitely/missing/fkst-framework"),
            pkg.path(),
            rt.path(),
            Duration::from_secs(1),
            8192,
        )
        .await
        .expect_err("missing binary must fail to spawn");
        assert!(matches!(err, RunnerError::Spawn(_)));
    }
}
