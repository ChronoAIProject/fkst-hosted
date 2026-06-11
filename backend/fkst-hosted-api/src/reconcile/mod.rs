//! Boot-time orphan temp-dir reconciliation (reduced scope, issue #26).
//!
//! # Why this is a temp-dir sweep, not a worktree/branch reconciler
//!
//! The original issue #26 spec ("clean stale worktrees & candidate branches in
//! the shared host repo after takeover") targets a surface that **does not
//! exist in landed v1**:
//!
//! - The #17 spike proved packages materialize as **plain temp directories** —
//!   no `git init`, no worktrees, no candidate branches (see
//!   [`crate::engine::materialize`] and [`crate::engine::runner`]; the only
//!   git-shaped artifact is a defensive `fkst.env` written into a plain dir).
//! - #25's progress journaling uses the **GitHub API** — nothing on local disk
//!   to reconcile.
//! - A dead pod's filesystem dies with the pod in Kubernetes, so there is no
//!   cross-pod shared host-repo working surface to clean up.
//!
//! The design-consensus gate escalated and the operator chose to **reduce
//! scope** to the only genuine orphan class that exists in v1.
//!
//! # The real orphan class: same-pod hard-kill temp-dir leftovers
//!
//! [`crate::engine::runner::SessionRunner`] materializes two temp dirs per
//! session under [`crate::engine::config::EngineConfig::temp_root`]:
//! - `fkst-pkg-<name>-<rand>` (the materialized package tree), and
//! - `fkst-rt-<rand>` (the runtime root, holding `durable/` and logs).
//!
//! Both are `TempDir` RAII guards, so **every normal path removes them**
//! (success, validation reject, conformance fail, startup timeout, stop, drop
//! — all covered by the runner's leak-scan tests). The ONLY way one leaks is a
//! **hard kill** (`SIGKILL` / OOM / pod `kill -9`) of a previous incarnation of
//! *this same pod*: RAII never runs, and the next boot of the same pod inherits
//! the orphaned dirs under the same `temp_root`.
//!
//! This module sweeps exactly those leftovers at boot, **fenced** against two
//! independent facts so an in-flight session's dir is never removed:
//!  1. the dir's canonical path is **not** in the live set (the `runtime_dir`
//!     values of non-terminal sessions in Mongo), and
//!  2. the dir's mtime is older than a configurable `min_age` safety threshold.
//!
//! # If a shared host-repo surface ever lands
//!
//! Re-introduce the spec's full worktree/branch reconciler (the
//! `EngineWorktreeLister` + `GitOps` + `LiveTruth` traits, the two-key liveness
//! intersection, the prefix-guarded branch deletion) at that point. Until then
//! that machinery would have nothing to act on.

pub mod config;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use bson::doc;

pub use config::ReconcileConfig;

use crate::db::Db;
use crate::engine::EngineConfig;
use crate::error::AppError;

/// Temp-dir name prefixes the engine uses (kept in sync with
/// [`crate::engine::materialize`] / [`crate::engine::runner`] — this module
/// only READS those naming conventions, it never changes them).
const ENGINE_DIR_PREFIXES: [&str; 2] = ["fkst-rt-", "fkst-pkg-"];

/// A single entry that could not be removed during the sweep (per-entry error
/// isolation: a failed `remove_dir_all` is recorded here, never aborts the
/// pass).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepError {
    /// The orphan dir the removal was attempted on.
    pub path: PathBuf,
    /// Human-readable reason (the `io::Error` rendering).
    pub reason: String,
}

/// Outcome of one orphan temp-dir sweep over `temp_root`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// How many candidate `fkst-*` entries were scanned.
    pub scanned: usize,
    /// Dirs that were swept (or, in dry-run, that WOULD be swept).
    pub swept: Vec<PathBuf>,
    /// Skipped because the dir belongs to a live (non-terminal) session.
    pub skipped_live: usize,
    /// Skipped because the dir's mtime is within `min_age` (an in-flight dir
    /// that the safety threshold protects).
    pub skipped_too_new: usize,
    /// Per-entry removal failures (isolated, non-fatal).
    pub errors: Vec<SweepError>,
}

impl SweepReport {
    /// How many dirs were swept (or would be, in dry-run).
    pub fn swept_count(&self) -> usize {
        self.swept.len()
    }

    /// How many per-entry removal failures occurred.
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }
}

/// Pure, testable core: scan `temp_root` for engine orphan temp dirs and remove
/// each one that is (a) not in `live_runtime_dirs` (the fencing set of
/// canonical `runtime_dir` paths held by non-terminal sessions) AND (b) older
/// than `min_age` relative to `now`.
///
/// - Only entries whose name starts with `fkst-rt-` or `fkst-pkg-` are
///   considered; everything else under `temp_root` is ignored.
/// - Per-entry error isolation: a directory that fails to canonicalize, stat,
///   or remove is recorded in [`SweepReport::errors`] and the pass continues.
/// - `dry_run` records what WOULD be swept without removing anything.
///
/// The set membership check uses the **canonical** path of each candidate so a
/// symlinked or `..`-laden `temp_root` still matches the canonical
/// `runtime_dir` values the live set is built from.
pub fn sweep_orphan_runtime_dirs(
    temp_root: &Path,
    live_runtime_dirs: &HashSet<PathBuf>,
    min_age: Duration,
    now: SystemTime,
    dry_run: bool,
) -> SweepReport {
    let mut report = SweepReport::default();

    let entries = match fs::read_dir(temp_root) {
        Ok(entries) => entries,
        Err(error) => {
            // A missing/unreadable temp_root is not fatal to startup: log and
            // return an empty report (nothing to sweep that we can see).
            tracing::warn!(
                temp_root = %temp_root.display(),
                error = %error,
                "reconcile.sweep: temp_root not readable; nothing swept"
            );
            return report;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.errors.push(SweepError {
                    path: temp_root.to_path_buf(),
                    reason: format!("dir entry read failed: {error}"),
                });
                continue;
            }
        };

        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !ENGINE_DIR_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            continue; // not an engine temp dir — ignore.
        }

        let path = entry.path();
        report.scanned += 1;

        // Fence 1: skip anything a live (non-terminal) session still owns.
        // Compare on the canonical path; a candidate that cannot canonicalize
        // (e.g. removed underneath us) is treated as not-live and judged by
        // age against its lexical path.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        if live_runtime_dirs.contains(&canonical) || live_runtime_dirs.contains(&path) {
            report.skipped_live += 1;
            tracing::debug!(
                path = %path.display(),
                "reconcile.sweep.skip: dir owned by a live session"
            );
            continue;
        }

        // Fence 2: skip anything younger than min_age (an in-flight dir whose
        // session may not yet have a persisted runtime_dir).
        match dir_age(&path, now) {
            Ok(age) if age < min_age => {
                report.skipped_too_new += 1;
                tracing::debug!(
                    path = %path.display(),
                    age_secs = age.as_secs(),
                    "reconcile.sweep.skip: dir younger than min_age"
                );
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                report.errors.push(SweepError {
                    path: path.clone(),
                    reason: format!("stat failed: {error}"),
                });
                continue;
            }
        }

        if dry_run {
            report.swept.push(path.clone());
            tracing::info!(
                path = %path.display(),
                "reconcile.sweep.dry_run: WOULD remove orphan dir"
            );
            continue;
        }

        match fs::remove_dir_all(&path) {
            Ok(()) => {
                report.swept.push(path.clone());
                tracing::info!(
                    path = %path.display(),
                    "reconcile.sweep.removed: orphan dir removed"
                );
            }
            Err(error) => {
                report.errors.push(SweepError {
                    path: path.clone(),
                    reason: format!("remove_dir_all failed: {error}"),
                });
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "reconcile.sweep.failed: orphan dir could not be removed"
                );
            }
        }
    }

    report
}

/// Age of `path` relative to `now`, derived from its mtime. A future mtime
/// (clock skew) yields a zero age (treated as "fresh"), never a panic.
fn dir_age(path: &Path, now: SystemTime) -> Result<Duration, std::io::Error> {
    let mtime = fs::metadata(path)?.modified()?;
    Ok(now.duration_since(mtime).unwrap_or(Duration::ZERO))
}

/// Async wrapper: query the `sessions` collection for the `runtime_dir` values
/// of non-terminal sessions (status NOT in `[stopped, failed]`, `runtime_dir`
/// not null), build the canonical live fencing set scoped to THIS
/// `temp_root`, capture `now`, and call the pure [`sweep_orphan_runtime_dirs`].
///
/// Pod-scoping is unnecessary for this LOCAL temp-dir class: each pod has its
/// own filesystem and only ever sees its own temp dirs. The live set is still
/// defensively filtered to `runtime_dir` values that live under `temp_root`,
/// so an unrelated path can never widen (or, via a non-canonicalizable entry,
/// distort) the fence.
pub async fn reconcile_orphans(
    db: &Db,
    engine_config: &EngineConfig,
    cfg: &ReconcileConfig,
) -> Result<SweepReport, AppError> {
    let temp_root = engine_config.temp_root.as_path();
    let live_runtime_dirs = live_runtime_dirs(db, temp_root).await?;

    tracing::info!(
        temp_root = %temp_root.display(),
        live_dirs = live_runtime_dirs.len(),
        min_age_secs = cfg.min_age.as_secs(),
        dry_run = cfg.dry_run,
        "reconcile.start"
    );

    let now = SystemTime::now();
    let report =
        sweep_orphan_runtime_dirs(temp_root, &live_runtime_dirs, cfg.min_age, now, cfg.dry_run);

    tracing::info!(
        scanned = report.scanned,
        swept = report.swept_count(),
        skipped_live = report.skipped_live,
        skipped_too_new = report.skipped_too_new,
        errors = report.error_count(),
        dry_run = cfg.dry_run,
        "reconcile.done"
    );
    Ok(report)
}

/// Build the live fencing set: the canonical `runtime_dir` paths of every
/// non-terminal session, restricted to those that live under `temp_root`.
async fn live_runtime_dirs(db: &Db, temp_root: &Path) -> Result<HashSet<PathBuf>, AppError> {
    use mongodb::options::FindOptions;

    // Non-terminal == status NOT in {stopped, failed}; runtime_dir present.
    let filter = doc! {
        "status": { "$nin": ["stopped", "failed"] },
        "runtime_dir": { "$ne": bson::Bson::Null },
    };
    let options = FindOptions::builder()
        .projection(doc! { "runtime_dir": 1, "_id": 0 })
        .build();

    // Project only runtime_dir; deserialize into a tiny shape so the query is
    // robust to the full SessionDoc evolving.
    #[derive(serde::Deserialize)]
    struct RuntimeDirOnly {
        runtime_dir: Option<String>,
    }

    let coll = db.collection::<RuntimeDirOnly>(crate::db::SESSIONS);
    let mut cursor = coll
        .find(filter)
        .with_options(options)
        .await
        .map_err(AppError::Mongo)?;

    let canonical_root = temp_root.canonicalize().ok();
    let mut live = HashSet::new();
    while cursor.advance().await.map_err(AppError::Mongo)? {
        let row: RuntimeDirOnly = cursor.deserialize_current().map_err(AppError::Mongo)?;
        let Some(runtime_dir) = row.runtime_dir else {
            continue;
        };
        let path = PathBuf::from(&runtime_dir);

        // Defensive scoping: only keep dirs that live under THIS temp_root.
        // Compare canonical-to-canonical when the root canonicalizes; fall
        // back to a lexical prefix check otherwise.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        let under_root = match &canonical_root {
            Some(root) => canonical.starts_with(root) || path.starts_with(temp_root),
            None => path.starts_with(temp_root),
        };
        if under_root {
            // Insert BOTH the canonical and lexical forms so the sweeper's
            // membership check matches regardless of which it computes.
            live.insert(canonical);
            live.insert(path);
        }
    }
    Ok(live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Create a `fkst-*` dir under `base` and return its path.
    fn make_dir(base: &Path, name: &str) -> PathBuf {
        let path = base.join(name);
        fs::create_dir(&path).expect("create dir");
        // Drop a file inside so remove_dir_all has real work.
        fs::write(path.join("marker"), b"x").expect("write marker");
        path
    }

    /// Set a dir's mtime to `secs` in the past relative to now.
    fn age_dir(path: &Path, secs: u64) {
        set_mtime(path, SystemTime::now() - Duration::from_secs(secs));
    }

    /// Set a dir's mtime to an explicit instant (filetime dev-dep; std offers
    /// no mtime setter).
    fn set_mtime(path: &Path, when: SystemTime) {
        let ft = filetime::FileTime::from_system_time(when);
        filetime::set_file_mtime(path, ft).expect("set_file_mtime");
    }

    fn empty_live() -> HashSet<PathBuf> {
        HashSet::new()
    }

    #[test]
    fn sweeps_an_orphan_old_dir() {
        let root = tempfile::tempdir().expect("root");
        let rt = make_dir(root.path(), "fkst-rt-abc123");
        age_dir(&rt, 600);

        let report = sweep_orphan_runtime_dirs(
            root.path(),
            &empty_live(),
            Duration::from_secs(300),
            SystemTime::now(),
            false,
        );
        assert_eq!(report.scanned, 1);
        assert_eq!(report.swept_count(), 1);
        assert_eq!(report.skipped_live, 0);
        assert_eq!(report.skipped_too_new, 0);
        assert!(report.errors.is_empty());
        assert!(!rt.exists(), "orphan dir must be removed");
    }

    #[test]
    fn sweeps_the_pkg_prefix_too() {
        let root = tempfile::tempdir().expect("root");
        let pkg = make_dir(root.path(), "fkst-pkg-demo-xyz");
        age_dir(&pkg, 600);

        let report = sweep_orphan_runtime_dirs(
            root.path(),
            &empty_live(),
            Duration::from_secs(300),
            SystemTime::now(),
            false,
        );
        assert_eq!(report.swept_count(), 1);
        assert!(!pkg.exists());
    }

    #[test]
    fn skips_a_dir_owned_by_a_live_session() {
        let root = tempfile::tempdir().expect("root");
        let rt = make_dir(root.path(), "fkst-rt-live");
        age_dir(&rt, 600);

        // The live set carries the canonical path (as the async wrapper builds).
        let mut live = empty_live();
        live.insert(rt.canonicalize().expect("canon"));

        let report = sweep_orphan_runtime_dirs(
            root.path(),
            &live,
            Duration::from_secs(300),
            SystemTime::now(),
            false,
        );
        assert_eq!(report.scanned, 1);
        assert_eq!(report.swept_count(), 0);
        assert_eq!(report.skipped_live, 1);
        assert!(rt.exists(), "a live session's dir must survive");
    }

    #[test]
    fn skips_a_dir_younger_than_min_age() {
        let root = tempfile::tempdir().expect("root");
        let rt = make_dir(root.path(), "fkst-rt-fresh");
        age_dir(&rt, 10); // 10s old

        let report = sweep_orphan_runtime_dirs(
            root.path(),
            &empty_live(),
            Duration::from_secs(300), // require 5min
            SystemTime::now(),
            false,
        );
        assert_eq!(report.scanned, 1);
        assert_eq!(report.swept_count(), 0);
        assert_eq!(report.skipped_too_new, 1);
        assert!(rt.exists(), "a freshly-created dir must survive");
    }

    #[test]
    fn ignores_non_fkst_dirs_and_files() {
        let root = tempfile::tempdir().expect("root");
        let other = make_dir(root.path(), "some-other-dir");
        age_dir(&other, 600);
        fs::write(root.path().join("fkst-rt-not-a-dir.txt"), b"file").expect("write file");
        let stray = root.path().join("fkst-rt-not-a-dir.txt");
        age_dir(&stray, 600);

        let report = sweep_orphan_runtime_dirs(
            root.path(),
            &empty_live(),
            Duration::from_secs(300),
            SystemTime::now(),
            false,
        );
        // The .txt file DOES start with fkst-rt- so it IS scanned, but
        // remove_dir_all on a file fails -> recorded as an error, never
        // touches the non-fkst dir.
        assert!(other.exists(), "non-fkst dir untouched");
        // The non-fkst dir is not even scanned.
        assert_eq!(report.scanned, 1, "only the fkst-prefixed entry is scanned");
    }

    #[test]
    fn dry_run_records_but_does_not_delete() {
        let root = tempfile::tempdir().expect("root");
        let rt = make_dir(root.path(), "fkst-rt-dry");
        age_dir(&rt, 600);

        let report = sweep_orphan_runtime_dirs(
            root.path(),
            &empty_live(),
            Duration::from_secs(300),
            SystemTime::now(),
            true, // dry-run
        );
        assert_eq!(report.swept_count(), 1, "dry-run records the would-sweep");
        assert!(report.swept.contains(&rt) || report.swept.iter().any(|p| p == &rt));
        assert!(rt.exists(), "dry-run must NOT delete");
    }

    #[test]
    fn per_entry_error_does_not_abort_the_pass() {
        let root = tempfile::tempdir().expect("root");
        // A removable orphan.
        let good = make_dir(root.path(), "fkst-rt-good");
        age_dir(&good, 600);
        // An un-removable orphan: a non-empty dir whose PARENT is made
        // read-only so remove_dir_all fails on the contained entry.
        let bad_parent = make_dir(root.path(), "fkst-pkg-bad");
        // Put a child in, then strip write perms on bad_parent so the child
        // cannot be unlinked.
        fs::write(bad_parent.join("locked"), b"y").expect("child");
        age_dir(&bad_parent, 600);
        let mut perms = fs::metadata(&bad_parent).expect("meta").permissions();
        perms.set_mode(0o555); // r-x: cannot unlink children
        fs::set_permissions(&bad_parent, perms).expect("chmod");

        let report = sweep_orphan_runtime_dirs(
            root.path(),
            &empty_live(),
            Duration::from_secs(300),
            SystemTime::now(),
            false,
        );

        // Restore perms so tempdir cleanup works.
        let mut perms = fs::metadata(&bad_parent).expect("meta").permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(&bad_parent, perms);

        assert_eq!(report.scanned, 2, "both fkst dirs scanned");
        assert_eq!(report.swept_count(), 1, "the good orphan is still swept");
        assert_eq!(report.error_count(), 1, "the bad orphan is isolated");
        assert!(!good.exists(), "the good orphan was removed");
        assert!(
            report.errors[0].path == bad_parent,
            "the error names the bad dir"
        );
    }

    #[test]
    fn missing_temp_root_yields_empty_report() {
        let report = sweep_orphan_runtime_dirs(
            Path::new("/definitely/not/here/fkst-temp"),
            &empty_live(),
            Duration::from_secs(300),
            SystemTime::now(),
            false,
        );
        assert_eq!(report.scanned, 0);
        assert_eq!(report.swept_count(), 0);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn future_mtime_is_treated_as_fresh_not_a_panic() {
        let root = tempfile::tempdir().expect("root");
        let rt = make_dir(root.path(), "fkst-rt-future");
        // mtime 1 hour in the FUTURE (clock skew).
        set_mtime(&rt, SystemTime::now() + Duration::from_secs(3600));

        let report = sweep_orphan_runtime_dirs(
            root.path(),
            &empty_live(),
            Duration::from_secs(300),
            SystemTime::now(),
            false,
        );
        assert_eq!(
            report.skipped_too_new, 1,
            "future mtime => zero age => fresh"
        );
        assert!(rt.exists());
    }
}
