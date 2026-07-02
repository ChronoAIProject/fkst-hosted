//! Session pod log-streaming (log-streaming Wave 1 + Wave 2).
//!
//! A session pod's stdout/stderr is a stream of secrets waiting to happen: the
//! injected App token, the LLM key, per-user env values, and whatever a subprocess
//! (git, codex) chooses to echo. Before ANY of that leaves the pod boundary it must
//! pass through the [`redact`] redactor, which carries the hard no-leak guarantee.
//!
//! Wave 1 is the pure, exhaustively tested redactor LIBRARY ([`redact`]). Wave 2 is
//! the effectful IN-POD collector that captures the full session log tree, redacts
//! every record, and pushes it to a per-session GitHub branch
//! (`fkst-logs/issue-<N>`). The collector is spawned from the `run-substrate`
//! driver ONLY when `FKST_LOG_STREAMING=1`; it is best-effort and MUST NOT crash or
//! block the engine — the session keeps running even if streaming fails entirely.
//!
//! The pieces are decomposed so each concern is unit-testable in isolation:
//! [`classify`] maps a log source into the on-branch tree, [`tail`] tracks the
//! incremental read offset of a growing file, [`seed`] derives the known-secret
//! table from the mounted creds, [`instance`] computes the per-pod instance id +
//! the `README`/`meta.json` shapes, [`tee`] splits the supervise child stream so
//! `kubectl logs` stays byte-for-byte intact, [`gitbranch`] hides the git
//! commit/push sequence behind a trait, and [`collector`] wires them together.

pub mod bundle;
pub mod classify;
pub mod collector;
pub mod gitbranch;
pub mod instance;
pub mod redact;
pub mod seed;
pub mod tail;
pub mod tee;

pub use classify::LogClass;
pub use collector::{spawn_collector, CollectorConfig, CollectorRecord, LogStreamHandle};

// --- injected env keys (single source of truth) ------------------------------
// The launcher (writer, `k8s::session_launcher`) STAMPS these onto the session pod
// only when log streaming is opted in; the driver (reader, `session_pod::driver`)
// READS them. Centralizing the names here means the two can never disagree.

/// Set to `1` on the pod when the session opted into log streaming; the driver
/// gates the whole collector on it.
pub const ENV_LOG_STREAMING: &str = "FKST_LOG_STREAMING";
/// The per-session log branch the collector pushes to (`fkst-logs/issue-<N>`).
pub const ENV_LOG_BRANCH: &str = "FKST_LOG_BRANCH";
/// The trigger issue number (README link + `meta.json`).
pub const ENV_TRIGGER_ISSUE: &str = "FKST_TRIGGER_ISSUE";
/// The registration config-hash (recorded in `meta.json`).
pub const ENV_CONFIG_HASH: &str = "FKST_CONFIG_HASH";
/// Downward-API pod UID (`metadata.uid`); feeds the instance id + `meta.json`.
pub const ENV_POD_UID: &str = "FKST_POD_UID";
/// Downward-API pod name (`metadata.name`); the instance-id fallback when no UID.
pub const ENV_POD_NAME: &str = "FKST_POD_NAME";
/// Optional flush cadence override (seconds).
pub const ENV_FLUSH_SECS: &str = "FKST_LOG_FLUSH_SECS";
/// Optional flush cadence override (bytes).
pub const ENV_FLUSH_BYTES: &str = "FKST_LOG_FLUSH_BYTES";

/// Default flush cadence: every 20 seconds.
pub const DEFAULT_FLUSH_SECS: u64 = 20;
/// Default flush cadence: every 256 KiB of buffered (redacted) log.
pub const DEFAULT_FLUSH_BYTES: usize = 262_144;

/// The value the launcher sets [`ENV_LOG_STREAMING`] to when streaming is on. The
/// driver treats exactly this value (trimmed) as "enabled"; anything else is off.
pub const LOG_STREAMING_ENABLED: &str = "1";

/// Derive the deterministic per-session log branch from the trigger issue number.
/// One issue = one session = one branch, so a revived pod (new instance) reuses the
/// SAME branch and only ADDS a new instance dir.
pub fn log_branch_for_issue(trigger_issue: i64) -> String {
    format!("fkst-logs/issue-{trigger_issue}")
}

#[cfg(test)]
mod mod_tests {
    use super::*;

    #[test]
    fn branch_name_is_derived_from_the_issue_number() {
        assert_eq!(log_branch_for_issue(7), "fkst-logs/issue-7");
        assert_eq!(log_branch_for_issue(1234), "fkst-logs/issue-1234");
    }

    #[test]
    fn branch_name_has_the_stable_prefix() {
        assert!(log_branch_for_issue(42).starts_with("fkst-logs/issue-"));
    }
}
