//! Session pod log-streaming (log-streaming Wave 1 + Wave 2).
//!
//! A session pod's stdout/stderr is a stream of secrets waiting to happen: the
//! injected App token, the LLM key, per-user env values, and whatever a subprocess
//! (git, codex) chooses to echo. Before ANY of that leaves the pod boundary it must
//! pass through the [`redact`] redactor, which carries the hard no-leak guarantee.
//!
//! Wave 1 is the pure, exhaustively tested redactor LIBRARY ([`redact`]). Wave 2 is
//! the effectful IN-POD collector that captures the full session log tree, redacts
//! every record, folds the tree into a single `tar.gz`, and uploads it to
//! chrono-storage at `logs/<session_id>/latest.tar.gz` as the mounted storage
//! SA. The collector is spawned from the `run-substrate` driver on EVERY
//! session (streaming is unconditional); it is best-effort and MUST NOT crash or
//! block the engine — the session keeps running even if streaming fails entirely,
//! and a session whose control plane configured no chrono-storage simply produces no
//! bundle (the uploader is not spawned).
//!
//! The pieces are decomposed so each concern is unit-testable in isolation:
//! [`classify`] maps a log source into the tree, [`tail`] tracks the incremental
//! read offset of a growing file, [`seed`] derives the known-secret table from the
//! mounted creds, [`instance`] computes the per-pod instance id + the `meta.json`/
//! `README` shapes, [`tee`] splits the supervise child stream so `kubectl logs`
//! stays byte-for-byte intact, [`bundle`] tar+gzips the redacted tree, [`sink`]
//! hides the upload destination behind a trait, and [`collector`] wires them
//! together.

pub mod bundle;
pub mod classify;
pub mod collector;
pub mod instance;
pub mod redact;
pub mod seed;
pub mod sink;
pub mod tail;
pub mod tee;

pub use classify::LogClass;
pub use collector::{spawn_collector, CollectorConfig, CollectorRecord, LogStreamHandle};

// --- injected env keys (single source of truth) ------------------------------
// The launcher (writer, `k8s::session_launcher`) STAMPS these onto every session
// pod; the driver (reader, `session_pod::driver`) READS them. Centralizing the
// names here means the two can never disagree.

/// The deterministic session id; the collector uploads its bundle to
/// `logs/<session_id>/latest.tar.gz` (a revived pod overwrites its own object).
pub const ENV_SESSION_ID: &str = "FKST_SESSION_ID";
/// The trigger issue number (`README` link + `meta.json`).
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

/// Derive the chrono-storage object key the session's log bundle is uploaded to.
/// One session = one object; a revived pod (a new instance) overwrites the SAME
/// `latest.tar.gz`, so the key is stable across a session's pod lifetimes.
pub fn bundle_key(session_id: &str) -> String {
    format!("logs/{session_id}/latest.tar.gz")
}

#[cfg(test)]
mod mod_tests {
    use super::*;

    #[test]
    fn bundle_key_is_derived_from_the_session_id() {
        assert_eq!(bundle_key("abc123"), "logs/abc123/latest.tar.gz");
        assert_eq!(bundle_key("sess-7"), "logs/sess-7/latest.tar.gz");
    }

    #[test]
    fn bundle_key_has_the_stable_prefix_and_object_name() {
        let key = bundle_key("xyz");
        assert!(key.starts_with("logs/"));
        assert!(key.ends_with("/latest.tar.gz"));
    }
}
