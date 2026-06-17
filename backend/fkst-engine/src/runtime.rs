//! Engine runtime-dir naming + age helpers.
//!
//! These are engine-side facts: the session runner materializes each session's
//! runtime root as `fkst-rt-<rand>` under `EngineConfig::temp_root`, and both
//! the OS-truth re-adopt scan ([`crate::adopt`]) and the control-plane's
//! orphan-reconcile sweep fence on the same naming convention + mtime. They
//! live here (engine-side) so every consumer reads ONE definition of the
//! runtime-dir prefix and the age computation; the reconcile sweep only READS
//! them, it never redefines them.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// Runtime-dir prefix — the class the orphan sweep and re-adopt scan act on. A
/// runtime dir's path is the value persisted/observed for a live session, so it
/// is fully fenceable against the live set. (Kept in sync with
/// [`crate::runner`] — consumers only READ this naming convention.)
pub const RUNTIME_DIR_PREFIX: &str = "fkst-rt-";

/// Age of `path` relative to `now`, derived from its mtime. A future mtime
/// (clock skew) yields a zero age (treated as "fresh"), never a panic.
pub fn dir_age(path: &Path, now: SystemTime) -> Result<Duration, std::io::Error> {
    let mtime = std::fs::metadata(path)?.modified()?;
    Ok(now.duration_since(mtime).unwrap_or(Duration::ZERO))
}
