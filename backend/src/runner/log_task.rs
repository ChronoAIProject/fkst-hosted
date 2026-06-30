//! The runner-side glue that drives the best-effort session log (issue #291).
//!
//! [`super::log::SessionLog`] owns the commit/push plumbing; this module wires it
//! to the live engine: it decides WHEN logging is enabled (only for a real git
//! clone), takes the engine's line-framed stdout, and runs the debounced
//! append/checkpoint loop on a background task. Keeping it out of
//! [`super`]'s orchestration core keeps both files small and the seam obvious —
//! the log task is purely additive and never touches the supervise disposition.

use std::path::Path;
use std::time::{Duration, Instant};

use secrecy::SecretString;

use super::log::SessionLog;
use super::PreparedRepo;
use crate::engine::RunningSession;
use crate::session_spec::SessionSpec;

/// Debounce between log checkpoints: a busy session flushes at most this often,
/// and an idle one flushes once per interval. Short enough that an observer sees
/// near-live progress, long enough that each checkpoint commits a meaningful
/// chunk rather than one line at a time.
const CHECKPOINT_DEBOUNCE: Duration = Duration::from_secs(10);

/// Spawn the best-effort session-log task: take the engine's line-framed stdout
/// and stream it into the dedicated log branch. Returns `None` (logging off) when
/// the project is not a git clone (fixture tests), the stdout stream was already
/// taken, or the log writer could not be initialised — none of which is fatal.
pub(super) fn spawn_session_log(
    session: &mut RunningSession,
    prepared: &PreparedRepo,
    spec: &SessionSpec,
    github_token: &SecretString,
    temp_root: &Path,
) -> Option<tokio::task::JoinHandle<()>> {
    if !prepared.project_root.join(".git").exists() {
        tracing::warn!(
            session_id = %spec.session_id,
            "run-session: project root is not a git repo; session logging disabled"
        );
        return None;
    }
    let rx = session.take_stdout()?;
    let writer = match SessionLog::new(
        prepared.project_root.clone(),
        &spec.session_id,
        &spec.run_key,
        Some(github_token.clone()),
        temp_root,
    ) {
        Ok(writer) => writer,
        Err(error) => {
            tracing::warn!(error = %error, "run-session: failed to init session log; logging disabled");
            return None;
        }
    };
    Some(tokio::spawn(run_log_task(rx, writer)))
}

/// Drive the session log: append each engine stdout line, checkpoint on a
/// debounce, and flush a final checkpoint when the stdout stream closes. Every
/// failure is a warning only.
async fn run_log_task(mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>, mut writer: SessionLog) {
    writer.record("session started");
    let mut last = Instant::now();
    loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(bytes) => {
                    writer.append_line(&bytes);
                    if last.elapsed() >= CHECKPOINT_DEBOUNCE {
                        if let Err(error) = writer.checkpoint().await {
                            tracing::warn!(error = %error, "run-session: log checkpoint failed");
                        }
                        last = Instant::now();
                    }
                }
                None => break,
            },
            _ = tokio::time::sleep(CHECKPOINT_DEBOUNCE) => {
                if let Err(error) = writer.checkpoint().await {
                    tracing::warn!(error = %error, "run-session: log checkpoint failed");
                }
                last = Instant::now();
            }
        }
    }
    writer.record("session ended");
    if let Err(error) = writer.finish().await {
        tracing::warn!(error = %error, "run-session: final log checkpoint failed");
    }
}
