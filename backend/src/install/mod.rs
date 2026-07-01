//! Shared, in-pod install-command runner + the `validate-env` subcommand
//! (issue #338 §3.2 + §3.4).
//!
//! A named environment carries an ordered list of shell install commands. Two
//! call sites execute them: the isolated env-VALIDATION pod (wired here via the
//! `validate-env` subcommand) and, later, the SESSION's pre-agent install step.
//! Both reuse the pure [`run_ordered`] engine so the two paths can never diverge
//! in how a command is spawned, ordered, deadline-bounded, or reported.
//!
//! ## Verdict frame
//!
//! The validation pod is orchestrated by the control plane, which reads the pod
//! logs and parses the LAST stdout line as a single-line JSON verdict frame (see
//! [`verdict_frame`]). Every command's own stdout/stderr is captured through
//! pipes and NEVER inherited to this process's stdout, so command chatter can
//! never contaminate the frame the control plane parses.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{ExitCode, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// Fixed mount path of the validation spec inside the validation pod. The
/// control plane writes a [`ValidateSpec`] JSON here before the pod boots.
const VALIDATE_SPEC_PATH: &str = "/var/run/fkst/validate/validate-spec.json";

/// In-pod cap on how many trailing stderr bytes a failed command surfaces in the
/// verdict frame. Mirrors `FKST_ENV_INSTALL_STDERR_TAIL_BYTES`'s default so the
/// pod bounds the frame the same way the control-plane config documents.
const IN_POD_STDERR_TAIL_BYTES: usize = 4096;

/// The spec the validation pod mounts at [`VALIDATE_SPEC_PATH`].
///
/// `install` is the ordered command list, `variables` the environment injected
/// into every command, and `deadline_secs` the whole-sequence wall-clock. All
/// three are non-secret (variables are user-declared plaintext env for the
/// isolated validation pod, never platform credentials).
#[derive(Debug, Deserialize)]
pub struct ValidateSpec {
    /// Shell install commands, executed in order under `bash -c`.
    pub install: Vec<String>,
    /// Environment injected into every command (added to the inherited env).
    pub variables: BTreeMap<String, String>,
    /// Hard wall-clock for the whole sequence, in seconds.
    pub deadline_secs: u64,
}

/// The outcome of running an environment's ordered install commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every command exited zero. `commands` is how many ran.
    Ok { commands: usize },
    /// A command failed (non-zero exit) or the sequence hit its deadline. The
    /// remaining commands did NOT run.
    Failed {
        /// 1-based index of the offending command.
        index: usize,
        /// The offending command text (verbatim).
        command: String,
        /// The command's exit code, or `-1` when it timed out / could not be
        /// awaited / spawned.
        exit_code: i32,
        /// True only when the whole-sequence deadline elapsed while this command
        /// was still running.
        timed_out: bool,
        /// Lossy-UTF8 tail of the command's stderr (empty on timeout, where the
        /// partial pipe cannot be recovered).
        stderr_tail: String,
    },
}

/// Serializable shape of the `Ok` verdict frame. A `#[derive(Serialize)]` struct
/// (not `serde_json::json!`) is used deliberately: it preserves field
/// declaration order so the emitted line matches the exact byte sequence the
/// control-plane parser expects (`serde_json`'s `Value` object would reorder or
/// alphabetize keys).
#[derive(Serialize)]
struct OkFrame {
    status: &'static str,
    commands: usize,
}

/// Serializable shape of the `failed` verdict frame. See [`OkFrame`] for why a
/// struct is used instead of `json!` — field order is load-bearing here.
#[derive(Serialize)]
struct FailedFrame<'a> {
    status: &'static str,
    index: usize,
    command: &'a str,
    exit_code: i32,
    timed_out: bool,
    stderr_tail: &'a str,
}

/// Run `commands` IN ORDER, each under `bash -c`, in a single shared temp working
/// directory, inheriting the current process env PLUS `variables`.
///
/// Each command's stdout+stderr is captured through pipes (never inherited), the
/// exit code recorded, and only the LAST `stderr_tail_bytes` bytes of stderr
/// retained. The sequence STOPS at the first non-zero exit and returns
/// [`Verdict::Failed`]; the later commands do not run. `overall_deadline` bounds
/// the whole sequence: it is enforced as an absolute deadline so each command
/// gets only the time remaining in the shared budget, and a command still running
/// when the budget is exhausted yields `Failed { timed_out: true }`.
pub async fn run_ordered(
    commands: &[String],
    variables: &BTreeMap<String, String>,
    overall_deadline: Duration,
    stderr_tail_bytes: usize,
) -> Verdict {
    // One scratch working dir shared by the ordered commands, so a later command
    // sees the side effects (created files, installed tooling) of the earlier
    // ones — mirroring how the real install step runs them as one sequence.
    let workdir = match tempfile::Builder::new().prefix("fkst-validate-").tempdir() {
        Ok(dir) => dir,
        Err(error) => {
            // Cannot even create a scratch dir: still return a machine-readable
            // verdict rather than panicking the pod.
            return Verdict::Failed {
                index: 1,
                command: commands.first().cloned().unwrap_or_default(),
                exit_code: -1,
                timed_out: false,
                stderr_tail: format!("could not create working dir: {error}"),
            };
        }
    };

    // Absolute deadline for the whole sequence. Computing `remaining` per command
    // guarantees the total wall-clock can never exceed `overall_deadline`, even
    // across many fast commands, while still telling us WHICH command was running
    // when the budget ran out.
    let deadline = Instant::now() + overall_deadline;

    for (i, command) in commands.iter().enumerate() {
        let index = i + 1; // 1-based for the verdict frame.
        let remaining = deadline.saturating_duration_since(Instant::now());

        let mut cmd = Command::new("bash");
        cmd.arg("-c")
            .arg(command)
            .current_dir(workdir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Kill the child if the timeout drops its wait future, so a slow
            // command cannot outlive the sequence's deadline.
            .kill_on_drop(true);
        for (key, value) in variables {
            cmd.env(key, value);
        }

        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(error) => {
                return Verdict::Failed {
                    index,
                    command: command.clone(),
                    exit_code: -1,
                    timed_out: false,
                    stderr_tail: format!("failed to spawn command: {error}"),
                };
            }
        };

        match tokio::time::timeout(remaining, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                if !output.status.success() {
                    return Verdict::Failed {
                        index,
                        command: command.clone(),
                        exit_code: output.status.code().unwrap_or(-1),
                        timed_out: false,
                        stderr_tail: tail_lossy(&output.stderr, stderr_tail_bytes),
                    };
                }
            }
            Ok(Err(error)) => {
                return Verdict::Failed {
                    index,
                    command: command.clone(),
                    exit_code: -1,
                    timed_out: false,
                    stderr_tail: format!("failed to await command: {error}"),
                };
            }
            Err(_elapsed) => {
                // The shared deadline elapsed while this command was running; the
                // child is killed on drop. The partial pipe cannot be recovered
                // once its read future is dropped, so surface an empty tail — the
                // `timed_out` flag is the signal that matters.
                return Verdict::Failed {
                    index,
                    command: command.clone(),
                    exit_code: -1,
                    timed_out: true,
                    stderr_tail: String::new(),
                };
            }
        }
    }

    Verdict::Ok {
        commands: commands.len(),
    }
}

/// Keep only the last `max` bytes of `bytes` and decode them lossily. A UTF-8
/// character split by the truncation boundary is replaced (never panics).
fn tail_lossy(bytes: &[u8], max: usize) -> String {
    let start = bytes.len().saturating_sub(max);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

/// Serialize a `failed` verdict frame with the exact, order-stable field layout.
fn failure_frame(
    index: usize,
    command: &str,
    exit_code: i32,
    timed_out: bool,
    stderr_tail: &str,
) -> String {
    serde_json::to_string(&FailedFrame {
        status: "failed",
        index,
        command,
        exit_code,
        timed_out,
        stderr_tail,
    })
    // A struct of primitives + strings cannot fail to serialize; surface loudly
    // if that invariant is ever broken.
    .expect("failed-verdict frame serialization cannot fail")
}

/// The EXACT one-line JSON the pod prints to stdout for `verdict`. The control
/// plane parses the last stdout line, so this is a contract:
/// - `Ok` -> `{"status":"ok","commands":N}`
/// - `Failed` -> `{"status":"failed","index":I,"command":"…","exit_code":C,"timed_out":B,"stderr_tail":"…"}`
pub fn verdict_frame(v: &Verdict) -> String {
    match v {
        Verdict::Ok { commands } => serde_json::to_string(&OkFrame {
            status: "ok",
            commands: *commands,
        })
        .expect("ok-verdict frame serialization cannot fail"),
        Verdict::Failed {
            index,
            command,
            exit_code,
            timed_out,
            stderr_tail,
        } => failure_frame(*index, command, *exit_code, *timed_out, stderr_tail),
    }
}

/// Read + deserialize the [`ValidateSpec`] at `path`. Surfaces a non-secret,
/// path-anchored message (the spec carries no credentials).
fn load_validate_spec(path: &Path) -> Result<ValidateSpec, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse {}: {error}", path.display()))
}

/// Testable core of [`run_validate_env`]: read the spec at `path`, run its
/// commands, and return the `(verdict_frame, success)` pair. An unreadable or
/// malformed spec yields the `index:0` read-failure frame and `success = false`
/// without spawning anything.
async fn run_validate_spec_at(path: &Path, stderr_tail_bytes: usize) -> (String, bool) {
    let spec = match load_validate_spec(path) {
        Ok(spec) => spec,
        Err(error) => {
            let frame = failure_frame(
                0,
                "",
                -1,
                false,
                &format!("could not read validate spec: {error}"),
            );
            return (frame, false);
        }
    };

    let verdict = run_ordered(
        &spec.install,
        &spec.variables,
        Duration::from_secs(spec.deadline_secs),
        stderr_tail_bytes,
    )
    .await;
    let success = matches!(verdict, Verdict::Ok { .. });
    (verdict_frame(&verdict), success)
}

/// Entry point for the `validate-env` subcommand: read the mounted
/// [`ValidateSpec`], execute its install commands, print the verdict frame as the
/// FINAL stdout line, and exit `SUCCESS`/`FAILURE` accordingly.
pub async fn run_validate_env() -> ExitCode {
    tracing::info!(path = VALIDATE_SPEC_PATH, "validate-env: starting");
    let (frame, success) =
        run_validate_spec_at(Path::new(VALIDATE_SPEC_PATH), IN_POD_STDERR_TAIL_BYTES).await;
    tracing::info!(success, "validate-env: install commands finished");
    // The frame MUST be the last stdout line (after any tracing line): the
    // control plane parses exactly that line. Command output was captured via
    // pipes and never inherited, so it cannot precede or corrupt this frame.
    println!("{frame}");
    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests;
