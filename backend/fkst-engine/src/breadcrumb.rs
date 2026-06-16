//! On-disk owner/exit breadcrumbs that let a RESTARTED worker re-adopt its
//! still-live engine children from OS truth alone (database-free pivot, #136).
//!
//! Both files live at the TOP LEVEL of a runtime dir (`<runtime_dir>/owner.json`,
//! `<runtime_dir>/exit.json`) — siblings of the engine's `durable/` + `goal.json`,
//! NEVER under `durable/` (the engine owns that subtree). Writes are atomic
//! (temp file + `rename`, atomic on the same filesystem — all runtime dirs are
//! under one `temp_root`) and `0600` (no secrets are in them; owner-only for
//! hygiene).
//!
//! SECURITY: the breadcrumb carries NO secret (secrets live in controller memory
//! per the pivot). `run_nonce` is treated as opaque and is NEVER logged.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::RunnerError;

/// Owner-breadcrumb file name (top level of the runtime dir).
pub const OWNER_BREADCRUMB_FILE: &str = "owner.json";
/// Exit-breadcrumb file name (top level of the runtime dir).
pub const EXIT_BREADCRUMB_FILE: &str = "exit.json";

/// Identifies which still-live engine process owns a runtime dir, for re-adopt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerBreadcrumb {
    /// The controller-assigned session id from the dispatch (string — the
    /// database-free path does NOT reintroduce `bson::Uuid` here).
    pub session_id: String,
    /// The supervise PID (== PGID at spawn; the engine is `process_group(0)`).
    pub pid: i32,
    /// The supervise process-group id at spawn (== pid); recorded separately so
    /// adoption can verify the process still LEADS its own group.
    pub pgid: i32,
    /// Present for goal sessions, `None` for classic sessions.
    pub goal_id: Option<String>,
    /// A fresh 128-bit random nonce minted per spawn — the PID-reuse guard: a
    /// recycled PID that happens to lead its own group will not match this.
    /// OPAQUE — never logged.
    pub run_nonce: String,
    /// The worker pod identity that wrote this breadcrumb (diagnostics + a
    /// defense-in-depth "only re-adopt what I wrote"; the primary fence is
    /// liveness).
    pub worker_id: String,
}

/// The observed terminal disposition of an engine process, written by whoever
/// reaps it so a worker that dies right after the engine exits still has the
/// truth on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitBreadcrumb {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub observed_at_rfc3339: String,
}

impl ExitBreadcrumb {
    /// Build an exit breadcrumb stamped with the current time (RFC3339).
    pub fn observed(code: Option<i32>, signal: Option<i32>) -> Self {
        Self {
            code,
            signal,
            observed_at_rfc3339: bson::DateTime::now()
                .try_to_rfc3339_string()
                .unwrap_or_default(),
        }
    }
}

/// Write `bc` to `<runtime_dir>/owner.json` atomically, mode `0600`.
pub fn write_owner_breadcrumb(runtime_dir: &Path, bc: &OwnerBreadcrumb) -> Result<(), RunnerError> {
    atomic_write_json(&runtime_dir.join(OWNER_BREADCRUMB_FILE), bc)?;
    // Log pid/session_id for traceability — NEVER the nonce (opaque secret-like).
    tracing::debug!(
        runtime_dir = %runtime_dir.display(),
        pid = bc.pid,
        session_id = %bc.session_id,
        "wrote owner breadcrumb"
    );
    Ok(())
}

/// Write `bc` to `<runtime_dir>/exit.json` atomically, mode `0600`.
pub fn write_exit_breadcrumb(runtime_dir: &Path, bc: &ExitBreadcrumb) -> Result<(), RunnerError> {
    atomic_write_json(&runtime_dir.join(EXIT_BREADCRUMB_FILE), bc)
}

/// Read the owner breadcrumb: absent -> `None` (pre-breadcrumb / partial dir),
/// malformed -> `Err` (caller classifies as `Unreadable` and reaps under the
/// age fence — corruption is never silently treated as absence).
pub fn read_owner_breadcrumb(runtime_dir: &Path) -> Result<Option<OwnerBreadcrumb>, RunnerError> {
    read_json(&runtime_dir.join(OWNER_BREADCRUMB_FILE))
}

/// Read the exit breadcrumb: absent -> `None`, malformed -> `Err`.
pub fn read_exit_breadcrumb(runtime_dir: &Path) -> Result<Option<ExitBreadcrumb>, RunnerError> {
    read_json(&runtime_dir.join(EXIT_BREADCRUMB_FILE))
}

/// Atomic JSON write: serialize, write a `0600` `<path>.tmp`, then `rename` it
/// over the final path (atomic on the same filesystem).
fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), RunnerError> {
    let json = serde_json::to_vec_pretty(value)
        .map_err(|e| RunnerError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    let mut tmp = path.to_path_buf().into_os_string();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&json)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read + parse a JSON file: absent -> `Ok(None)`, malformed -> `Err`.
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, RunnerError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RunnerError::Io(e)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| RunnerError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn sample_owner() -> OwnerBreadcrumb {
        OwnerBreadcrumb {
            session_id: "sess-1".to_string(),
            pid: 4242,
            pgid: 4242,
            goal_id: Some("goal-1".to_string()),
            run_nonce: "deadbeef".to_string(),
            worker_id: "worker-0".to_string(),
        }
    }

    #[test]
    fn owner_breadcrumb_round_trips_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let bc = sample_owner();
        write_owner_breadcrumb(dir.path(), &bc).unwrap();
        let back = read_owner_breadcrumb(dir.path()).unwrap();
        assert_eq!(back, Some(bc));
        // Mode is 0600 and no *.tmp leaked.
        let meta = std::fs::metadata(dir.path().join(OWNER_BREADCRUMB_FILE)).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert!(!dir
            .path()
            .join(format!("{OWNER_BREADCRUMB_FILE}.tmp"))
            .exists());
    }

    #[test]
    fn absent_owner_breadcrumb_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_owner_breadcrumb(dir.path()).unwrap(), None);
    }

    #[test]
    fn malformed_owner_breadcrumb_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(OWNER_BREADCRUMB_FILE), b"{ not json").unwrap();
        assert!(read_owner_breadcrumb(dir.path()).is_err());
    }

    #[test]
    fn exit_breadcrumb_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let bc = ExitBreadcrumb {
            code: Some(0),
            signal: None,
            observed_at_rfc3339: "2026-06-16T00:00:00Z".to_string(),
        };
        write_exit_breadcrumb(dir.path(), &bc).unwrap();
        assert_eq!(read_exit_breadcrumb(dir.path()).unwrap(), Some(bc));
    }

    #[test]
    fn exit_breadcrumb_observed_stamps_a_timestamp() {
        let bc = ExitBreadcrumb::observed(Some(1), None);
        assert_eq!(bc.code, Some(1));
        assert!(!bc.observed_at_rfc3339.is_empty());
    }
}
