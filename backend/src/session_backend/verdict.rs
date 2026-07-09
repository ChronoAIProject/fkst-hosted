//! Shared, kube-free env-validation verdict parsing (issue #419).
//!
//! Extracted VERBATIM out of `k8s/validation.rs` so BOTH the direct-Kubernetes
//! backend and the OpenSandbox backend parse the SAME single-line JSON verdict frame
//! into the SAME [`ValidationOutcome`] — byte-for-byte identical verdicts regardless
//! of which runtime executed the validation. No `kube` / `k8s_openapi` type appears
//! here (the effectful log READ stays in each backend); this is pure and exhaustively
//! unit-testable.
//!
//! The two conservative-`Failed` builders live here too so a timed-out or unparseable
//! run produces the identical detail sentence in either backend — the environment must
//! NEVER be persisted on an untrusted result, and the REST layer renders the same 422
//! whichever backend produced it.

use serde::Deserialize;

use super::ValidationOutcome;

/// The verdict frame the validator prints as its last stdout line (see
/// [`crate::install::verdict_frame`]). Optional fields let both the `ok` and `failed`
/// shapes deserialize into one struct.
#[derive(Deserialize)]
pub(crate) struct VerdictFrame {
    status: String,
    #[serde(default)]
    commands: Option<usize>,
    #[serde(default)]
    index: Option<u64>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    timed_out: Option<bool>,
    #[serde(default)]
    stderr_tail: Option<String>,
}

/// Parse a single verdict JSON line into a [`ValidationOutcome`]. `None` for a
/// non-JSON / empty / unrecognized-status line (pure + unit-tested).
pub(crate) fn parse_verdict_line(line: &str) -> Option<ValidationOutcome> {
    let frame: VerdictFrame = serde_json::from_str(line.trim()).ok()?;
    match frame.status.as_str() {
        "ok" => Some(ValidationOutcome::Passed {
            commands: frame.commands?,
        }),
        "failed" => Some(ValidationOutcome::Failed {
            failed_command_index: u32::try_from(frame.index?).unwrap_or(0),
            failed_command: frame.command.unwrap_or_default(),
            exit_code: frame.exit_code.unwrap_or(-1),
            timed_out: frame.timed_out.unwrap_or(false),
            stderr_tail: frame.stderr_tail.unwrap_or_default(),
        }),
        _ => None,
    }
}

/// The last non-empty (trimmed) line of `text`, or `None` if there is none. The
/// validator may emit tracing chatter before the frame, so only the final line counts.
pub(crate) fn last_non_empty_line(text: &str) -> Option<&str> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
}

/// The conservative `Failed` verdict for a validation run that did NOT reach a
/// terminal state before its deadline. Byte-identical across backends.
pub(crate) fn verdict_timed_out() -> ValidationOutcome {
    ValidationOutcome::Failed {
        failed_command_index: 0,
        failed_command: String::new(),
        exit_code: -1,
        timed_out: true,
        stderr_tail: "validation pod did not complete before the deadline".to_string(),
    }
}

/// The conservative `Failed` verdict for a run whose logs were readable but produced
/// NO trusted verdict frame (OOM / deadline-kill / anomaly). NOT an infra error — the
/// environment must never be persisted on an untrusted result. Byte-identical across
/// backends.
pub(crate) fn verdict_unparseable() -> ValidationOutcome {
    ValidationOutcome::Failed {
        failed_command_index: 0,
        failed_command: String::new(),
        exit_code: -1,
        timed_out: false,
        stderr_tail: "validation pod exceeded its limits".to_string(),
    }
}

#[cfg(test)]
#[path = "verdict_tests.rs"]
mod tests;
