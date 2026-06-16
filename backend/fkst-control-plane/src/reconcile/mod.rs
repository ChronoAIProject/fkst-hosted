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
//! # The sweep targets ONLY runtime dirs (`fkst-rt-*`) — the fenceable class
//!
//! The sweep deletes **only `fkst-rt-*` runtime dirs**, because they are the
//! only class it can fence safely: a session's `runtime_dir` is the exact value
//! persisted on the Mongo `SessionDoc.runtime_dir` field, so a live session's
//! runtime dir is always in the live set and is never removed. Each runtime dir
//! holds the engine's `durable/` (redb) state and the child logs — the real
//! disk weight — so reclaiming it captures ~all of the disk value.
//!
//! **`fkst-pkg-*` package dirs are intentionally NOT deleted.** The package dir
//! path is *not* persisted anywhere on the session document (only the runtime
//! dir is), so it **cannot be mapped back to a live session** and therefore
//! cannot be fenced. Deleting a package dir on mtime alone would be a
//! wrong-deletion bug: a session that has been running longer than `min_age`
//! still has its package dir on disk older than `min_age`, and the engine
//! **re-reads the materialized package tree during the run** (it re-invokes
//! `fkst-framework run --package-root <pkg>` per event), so removing it
//! mid-run would break the live session. Package dirs are KB-scale and bounded,
//! so leaking them on a hard kill is an accepted, minor cost. They are counted
//! as `skipped_unfenceable` and logged, never removed.
//!
//! A future change that persists the package-dir path on the session doc (or
//! nests the package dir *under* the runtime dir so one fence covers both) can
//! lift package dirs into the swept class; until then they are left alone.
//!
//! The runtime-dir sweep is **fenced** against two independent facts so an
//! in-flight session's dir is never removed:
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

pub use config::ReconcileConfig;

use crate::engine::EngineConfig;
// The runtime-dir prefix + age helper are engine-side facts now owned by the
// `fkst-engine` crate (issue #151); this module only READS them, it never
// redefines them.
use crate::engine::{dir_age, RUNTIME_DIR_PREFIX};

/// Package-dir prefix — scanned and counted but **never deleted**: the package
/// dir path is not persisted on the session doc, so it cannot be fenced, and
/// the engine re-reads the package tree during a run (see the module doc).
/// (Kept in sync with [`crate::engine::materialize`].)
const PACKAGE_DIR_PREFIX: &str = "fkst-pkg-";

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
    /// Runtime dirs (`fkst-rt-*`) that were swept (or, in dry-run, that WOULD
    /// be swept).
    pub swept: Vec<PathBuf>,
    /// Skipped because the runtime dir belongs to a live (non-terminal)
    /// session.
    pub skipped_live: usize,
    /// Skipped because the runtime dir's mtime is within `min_age` (an
    /// in-flight dir that the safety threshold protects).
    pub skipped_too_new: usize,
    /// Skipped because the entry is a package dir (`fkst-pkg-*`), which cannot
    /// be mapped to a live session via Mongo (its path is not persisted) and
    /// so is never deleted. See the module doc.
    pub skipped_unfenceable: usize,
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
/// each **runtime dir** (`fkst-rt-*`) that is (a) not in `live_runtime_dirs`
/// (the fencing set of canonical `runtime_dir` paths held by non-terminal
/// sessions) AND (b) older than `min_age` relative to `now`.
///
/// - Both `fkst-rt-*` and `fkst-pkg-*` entries are scanned, but **only
///   runtime dirs are eligible for deletion**. Package dirs (`fkst-pkg-*`)
///   cannot be fenced (their path is not persisted on the session doc) and are
///   counted as `skipped_unfenceable`, never removed — see the module doc.
/// - Everything else under `temp_root` is ignored.
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
        let is_runtime = name.starts_with(RUNTIME_DIR_PREFIX);
        let is_package = name.starts_with(PACKAGE_DIR_PREFIX);
        if !is_runtime && !is_package {
            continue; // not an engine temp dir — ignore.
        }

        let path = entry.path();
        report.scanned += 1;

        // Package dirs are unfenceable: their path is not persisted on the
        // session doc, so they can never be mapped back to a live session.
        // Count and log, but NEVER delete (deleting a live session's package
        // dir mid-run would break it — see the module doc).
        if is_package {
            report.skipped_unfenceable += 1;
            tracing::debug!(
                path = %path.display(),
                "reconcile.sweep.skip: not mappable to a live session via Mongo (fkst-pkg dir)"
            );
            continue;
        }

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

/// Build the OS-TRUTH live fencing set — the canonical+lexical paths of every
/// `fkst-rt-*` dir whose owner breadcrumb is present and whose owner is still
/// alive & leads its own group (#136) — capture `now`, and call the pure
/// [`sweep_orphan_runtime_dirs`]. This REPLACES the old Mongo `live_runtime_dirs`
/// query: a worker's runtime truth is the OS itself (live processes + on-disk
/// breadcrumbs), never a database document.
///
/// Pod-scoping is unnecessary for this LOCAL temp-dir class: each pod/worker has
/// its own filesystem and only ever sees its own temp dirs. The sweep is
/// fail-open and infallible (a missing/un-readable temp_root yields an empty
/// live set + an empty report), so no `Result`.
pub fn reconcile_orphans(engine_config: &EngineConfig, cfg: &ReconcileConfig) -> SweepReport {
    let temp_root = engine_config.temp_root.as_path();
    let live_runtime_dirs = crate::engine::os_truth_live_set(temp_root);

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
        skipped_unfenceable = report.skipped_unfenceable,
        errors = report.error_count(),
        dry_run = cfg.dry_run,
        "reconcile.done"
    );
    report
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
    fn never_sweeps_a_pkg_dir_it_cannot_fence() {
        // A package dir's path is not persisted on the session doc, so it
        // cannot be mapped to a live session. Even an OLD orphan fkst-pkg dir
        // must be SKIPPED (skipped_unfenceable), never deleted — deleting a
        // live session's package dir mid-run would break it.
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
        assert_eq!(report.scanned, 1);
        assert_eq!(report.swept_count(), 0, "package dirs are never swept");
        assert_eq!(report.skipped_unfenceable, 1);
        assert!(pkg.exists(), "an unfenceable package dir must survive");
    }

    #[test]
    fn pkg_dir_is_skipped_even_in_dry_run() {
        let root = tempfile::tempdir().expect("root");
        let pkg = make_dir(root.path(), "fkst-pkg-demo-dry");
        age_dir(&pkg, 600);

        let report = sweep_orphan_runtime_dirs(
            root.path(),
            &empty_live(),
            Duration::from_secs(300),
            SystemTime::now(),
            true, // dry-run
        );
        assert_eq!(
            report.swept_count(),
            0,
            "package dirs never appear in swept"
        );
        assert_eq!(report.skipped_unfenceable, 1);
        assert!(pkg.exists());
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
        // A removable runtime orphan.
        let good = make_dir(root.path(), "fkst-rt-good");
        age_dir(&good, 600);
        // An un-removable runtime orphan: a non-empty dir made read-only so
        // remove_dir_all fails on the contained entry. (Must be a RUNTIME dir
        // — package dirs are skipped before any removal is attempted.)
        let bad_parent = make_dir(root.path(), "fkst-rt-bad");
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

        assert_eq!(report.scanned, 2, "both runtime dirs scanned");
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
