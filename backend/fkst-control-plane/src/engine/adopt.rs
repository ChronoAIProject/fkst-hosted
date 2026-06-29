//! OS-truth re-adopt: on worker restart, scan `temp_root` for engine runtime
//! dirs, re-adopt the ones whose owner process is still alive (and leads its own
//! group, and whose breadcrumb is intact), and age-fence-reap dead-owner /
//! orphan dirs (#136). This is the database-free replacement for the Mongo
//! `live_runtime_dirs` fence: "live = owner pid alive & leads its group &
//! breadcrumb present", not a `sessions` query.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use nix::unistd::{getpgid, Pid};

use crate::engine::breadcrumb::{read_owner_breadcrumb, OwnerBreadcrumb};
use crate::engine::process::is_pid_alive;
use crate::engine::runner::{RunningSession, SessionRunner};
use crate::engine::runtime::{dir_age, RUNTIME_DIR_PREFIX};

/// Is the breadcrumb's owner still genuinely live? Alive, leading its own group
/// (PID reuse into a different group fails this), and a valid pid.
fn owner_is_live(bc: &OwnerBreadcrumb) -> bool {
    bc.pid > 0
        && is_pid_alive(bc.pid)
        && matches!(getpgid(Some(Pid::from_raw(bc.pid))), Ok(p) if p.as_raw() == bc.pgid)
}

/// Outcome of one re-adopt scan over `temp_root`.
#[derive(Default)]
pub struct AdoptReport {
    /// Re-adopted live engines (the worker re-registers each + resumes status).
    pub adopted: Vec<RunningSession>,
    /// Dead-owner / orphan / unreadable dirs removed under the age fence.
    pub reaped: Vec<PathBuf>,
    /// Dirs left in place (too-new, removal error, or another worker's live dir).
    pub skipped: Vec<PathBuf>,
}

/// Scan `temp_root` for `fkst-rt-*` dirs and classify each: re-adopt a live
/// owner, age-fence-reap a dead-owner / orphan / unreadable dir. `worker_id` is
/// a defense-in-depth filter (the primary fence is liveness): a live dir whose
/// breadcrumb was written by a DIFFERENT worker is skipped (neither adopted nor
/// reaped). An empty `worker_id` adopts on liveness alone.
pub fn scan_and_adopt(
    temp_root: &Path,
    worker_id: &str,
    runner: &SessionRunner,
    min_age: Duration,
    now: SystemTime,
) -> AdoptReport {
    let mut report = AdoptReport::default();
    let Ok(entries) = std::fs::read_dir(temp_root) else {
        return report;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(RUNTIME_DIR_PREFIX) {
            continue;
        }
        let dir = entry.path();
        match read_owner_breadcrumb(&dir) {
            Ok(Some(bc)) if owner_is_live(&bc) => {
                if !worker_id.is_empty() && !bc.worker_id.is_empty() && bc.worker_id != worker_id {
                    // A live engine that another worker owns — never reap it,
                    // never adopt it (liveness is primary; this is defense in
                    // depth against a shared filesystem we never expect).
                    tracing::debug!(dir = %dir.display(), "live dir owned by another worker; skipping");
                    report.skipped.push(dir);
                    continue;
                }
                let pid = bc.pid;
                let session_id = bc.session_id.clone();
                report
                    .adopted
                    .push(runner.adopt(dir.clone(), Vec::new(), bc));
                // NEVER log the nonce.
                tracing::info!(pid, session_id = %session_id, dir = %dir.display(), "worker.adopt: re-adopted live engine");
            }
            // Dead owner, no breadcrumb, or malformed: age-fence-reap.
            _ => reap_if_old(&dir, min_age, now, &mut report),
        }
    }
    tracing::info!(
        adopted = report.adopted.len(),
        reaped = report.reaped.len(),
        skipped = report.skipped.len(),
        "worker.adopt scan complete"
    );
    report
}

/// Remove `dir` only if it is older than `min_age` (a dir created the same
/// instant a sibling is mid-spawn must not be deleted). Per-entry error
/// isolation: a failed removal is recorded as skipped and the scan continues.
fn reap_if_old(dir: &Path, min_age: Duration, now: SystemTime, report: &mut AdoptReport) {
    match dir_age(dir, now) {
        Ok(age) if age >= min_age => match std::fs::remove_dir_all(dir) {
            Ok(()) => report.reaped.push(dir.to_path_buf()),
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "dead-owner dir removal failed");
                report.skipped.push(dir.to_path_buf());
            }
        },
        _ => report.skipped.push(dir.to_path_buf()),
    }
}

/// The OS-truth live fencing set for the reconcile sweep (#136), REPLACING the
/// Mongo `live_runtime_dirs` query: a dir's canonical AND lexical path are in
/// the set iff its owner breadcrumb is present, well-formed, and its owner is
/// still alive & leads its own group.
pub fn os_truth_live_set(temp_root: &Path) -> HashSet<PathBuf> {
    let mut live = HashSet::new();
    let Ok(entries) = std::fs::read_dir(temp_root) else {
        return live;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(RUNTIME_DIR_PREFIX) {
            continue;
        }
        let dir = entry.path();
        if let Ok(Some(bc)) = read_owner_breadcrumb(&dir) {
            if owner_is_live(&bc) {
                // Insert BOTH forms so the sweep's membership check matches
                // regardless of which it computes (mirrors live_runtime_dirs).
                let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
                live.insert(canonical);
                live.insert(dir);
            }
        }
    }
    live
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::breadcrumb::write_owner_breadcrumb;
    use crate::engine::config::EngineConfig;

    fn runner(temp_root: &Path) -> SessionRunner {
        SessionRunner::new(EngineConfig {
            temp_root: temp_root.to_path_buf(),
            ..EngineConfig::default()
        })
    }

    fn write_owner(dir: &Path, pid: i32, pgid: i32, worker_id: &str) {
        std::fs::create_dir_all(dir).unwrap();
        write_owner_breadcrumb(
            dir,
            &OwnerBreadcrumb {
                session_id: "s".to_string(),
                pid,
                pgid,
                goal_id: None,
                run_nonce: "n".to_string(),
                worker_id: worker_id.to_string(),
            },
        )
        .unwrap();
    }

    /// Spawn a real `sleep` leading its own group; tokio reaps the zombie.
    async fn spawn_grouped_sleeper() -> i32 {
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 30")
            .process_group(0)
            .kill_on_drop(false)
            .spawn()
            .expect("spawn")
            .id()
            .expect("pid") as i32
    }

    /// A definitely-dead pid: spawn + reap a child, reuse its (now-free) pid.
    async fn dead_pid() -> i32 {
        let mut child = tokio::process::Command::new("true").spawn().unwrap();
        let pid = child.id().unwrap() as i32;
        let _ = child.wait().await;
        // Brief settle so the pid is reaped (is_pid_alive false).
        for _ in 0..50 {
            if !is_pid_alive(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        pid
    }

    #[tokio::test]
    async fn adopts_a_live_breadcrumbed_dir() {
        let temp_root = tempfile::tempdir().unwrap();
        let runner = runner(temp_root.path());
        let pid = spawn_grouped_sleeper().await;
        let dir = temp_root.path().join("fkst-rt-live");
        write_owner(&dir, pid, pid, "w");

        let report = scan_and_adopt(
            temp_root.path(),
            "w",
            &runner,
            Duration::ZERO,
            SystemTime::now(),
        );
        assert_eq!(report.adopted.len(), 1);
        assert!(report.reaped.is_empty());
        assert!(dir.exists(), "live dir is not reaped");

        // Kill it; now it is a dead owner and (older than 0) reaped.
        crate::engine::process::signal_group(pid, nix::sys::signal::Signal::SIGKILL).unwrap();
        for _ in 0..100 {
            if !is_pid_alive(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let report = scan_and_adopt(
            temp_root.path(),
            "w",
            &runner,
            Duration::ZERO,
            SystemTime::now(),
        );
        assert!(report.adopted.is_empty(), "dead owner is not adopted");
        assert_eq!(report.reaped, vec![dir.clone()]);
    }

    #[tokio::test]
    async fn dead_owner_old_dir_is_reaped() {
        let temp_root = tempfile::tempdir().unwrap();
        let runner = runner(temp_root.path());
        let pid = dead_pid().await;
        let dir = temp_root.path().join("fkst-rt-dead");
        write_owner(&dir, pid, pid, "w");
        let report = scan_and_adopt(
            temp_root.path(),
            "w",
            &runner,
            Duration::ZERO,
            SystemTime::now(),
        );
        assert_eq!(report.reaped, vec![dir]);
    }

    #[tokio::test]
    async fn dead_owner_fresh_dir_is_skipped() {
        let temp_root = tempfile::tempdir().unwrap();
        let runner = runner(temp_root.path());
        let pid = dead_pid().await;
        let dir = temp_root.path().join("fkst-rt-fresh");
        write_owner(&dir, pid, pid, "w");
        // min_age in the far future => the dir is "too new".
        let report = scan_and_adopt(
            temp_root.path(),
            "w",
            &runner,
            Duration::from_secs(3600),
            SystemTime::now(),
        );
        assert!(report.reaped.is_empty());
        assert!(dir.exists(), "fresh dead-owner dir survives the age fence");
    }

    #[tokio::test]
    async fn pid_reuse_with_wrong_group_is_not_adopted() {
        let temp_root = tempfile::tempdir().unwrap();
        let runner = runner(temp_root.path());
        // PID 1 is alive but its pgid is NOT our recorded pgid (99999).
        let dir = temp_root.path().join("fkst-rt-reuse");
        write_owner(&dir, 1, 99999, "w");
        let report = scan_and_adopt(
            temp_root.path(),
            "w",
            &runner,
            Duration::ZERO,
            SystemTime::now(),
        );
        assert!(report.adopted.is_empty(), "wrong-group pid is not adopted");
        assert_eq!(report.reaped, vec![dir], "classified dead-owner and reaped");
    }

    #[tokio::test]
    async fn missing_breadcrumb_dir_is_orphan_not_adopted() {
        let temp_root = tempfile::tempdir().unwrap();
        let runner = runner(temp_root.path());
        let dir = temp_root.path().join("fkst-rt-orphan");
        std::fs::create_dir_all(&dir).unwrap(); // no owner.json
        let report = scan_and_adopt(
            temp_root.path(),
            "w",
            &runner,
            Duration::ZERO,
            SystemTime::now(),
        );
        assert!(report.adopted.is_empty());
        assert_eq!(report.reaped, vec![dir]);
    }

    #[tokio::test]
    async fn os_truth_live_set_contains_only_live_owners() {
        let temp_root = tempfile::tempdir().unwrap();
        let live_pid = spawn_grouped_sleeper().await;
        let live_dir = temp_root.path().join("fkst-rt-os-live");
        write_owner(&live_dir, live_pid, live_pid, "w");
        let dead = dead_pid().await;
        let dead_dir = temp_root.path().join("fkst-rt-os-dead");
        write_owner(&dead_dir, dead, dead, "w");

        let set = os_truth_live_set(temp_root.path());
        let canonical = live_dir.canonicalize().unwrap_or_else(|_| live_dir.clone());
        assert!(set.contains(&canonical) || set.contains(&live_dir));
        let dead_canonical = dead_dir.canonicalize().unwrap_or_else(|_| dead_dir.clone());
        assert!(!set.contains(&dead_canonical) && !set.contains(&dead_dir));

        crate::engine::process::signal_group(live_pid, nix::sys::signal::Signal::SIGKILL).unwrap();
    }
}
