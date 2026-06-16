//! Integration test for the boot-time orphan temp-dir reconciliation, now
//! fenced by OS TRUTH (issue #136): a real temp_root with planted `fkst-rt-*` /
//! `fkst-pkg-*` dirs, where "live" means a present owner breadcrumb whose owner
//! process is still alive & leads its own group — NOT a Mongo `sessions` query.
//!
//! No datastore: these tests need no Docker / Mongo and always run.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use fkst_control_plane::engine::breadcrumb::{write_owner_breadcrumb, OwnerBreadcrumb};
use fkst_control_plane::engine::is_pid_alive;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::reconcile::{reconcile_orphans, ReconcileConfig};

/// Plant a `fkst-*` dir under `base` with a marker file, aged `age_secs` old.
fn plant_dir(base: &Path, name: &str, age_secs: u64) -> PathBuf {
    let path = base.join(name);
    fs::create_dir(&path).expect("create planted dir");
    fs::write(path.join("marker"), b"x").expect("marker");
    age(&path, age_secs);
    path
}

/// Set a dir's mtime to `age_secs` in the past (call AFTER any file writes,
/// which would otherwise bump the dir mtime back to now).
fn age(path: &Path, age_secs: u64) {
    let when = SystemTime::now() - Duration::from_secs(age_secs);
    let ft = filetime::FileTime::from_system_time(when);
    filetime::set_file_mtime(path, ft).expect("set mtime");
}

/// Spawn a real `sleep` child leading its own process group; returns (pid, child).
/// The child is held so the caller can kill it at the end of the test.
async fn spawn_grouped_sleeper() -> (i32, tokio::process::Child) {
    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg("sleep 30")
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sleeper");
    let pid = child.id().expect("pid") as i32;
    (pid, child)
}

fn owner_for(pid: i32) -> OwnerBreadcrumb {
    OwnerBreadcrumb {
        session_id: "live-sess".to_string(),
        pid,
        pgid: pid,
        goal_id: None,
        run_nonce: "live-nonce".to_string(),
        worker_id: "w".to_string(),
    }
}

fn config(root: &Path) -> (EngineConfig, ReconcileConfig) {
    (
        EngineConfig {
            temp_root: root.to_path_buf(),
            ..EngineConfig::default()
        },
        ReconcileConfig {
            min_age: Duration::from_secs(300),
            dry_run: false,
        },
    )
}

#[tokio::test]
async fn boot_sweep_spares_live_owner_and_sweeps_orphans() {
    let temp_root = tempfile::tempdir().expect("temp_root");
    let root = temp_root.path();

    // 1. A LIVE-owner runtime dir (breadcrumb -> a real alive process) -> spared
    //    by the live fence even though it is old.
    let (live_pid, mut live_child) = spawn_grouped_sleeper().await;
    let live_dir = root.join("fkst-rt-live");
    fs::create_dir(&live_dir).unwrap();
    write_owner_breadcrumb(&live_dir, &owner_for(live_pid)).unwrap();
    age(&live_dir, 600); // age AFTER the breadcrumb write
    assert!(is_pid_alive(live_pid));

    // 2. A runtime dir with NO breadcrumb -> orphan -> swept (old enough).
    let no_bc_rt = plant_dir(root, "fkst-rt-orphan", 600);

    // 3. A PACKAGE dir -> unfenceable, NEVER deleted.
    let pkg = plant_dir(root, "fkst-pkg-demo-orphan", 600);

    // 4. A too-new runtime orphan -> spared by the mtime fence.
    let fresh = plant_dir(root, "fkst-rt-fresh", 5);

    // 5. A non-fkst dir -> never scanned.
    let unrelated = root.join("unrelated");
    fs::create_dir(&unrelated).unwrap();

    let (engine_config, reconcile_config) = config(root);
    let report = reconcile_orphans(&engine_config, &reconcile_config);

    assert_eq!(report.scanned, 4, "four fkst-* dirs scanned");
    assert_eq!(report.swept_count(), 1, "one genuine runtime orphan swept");
    assert_eq!(
        report.skipped_live, 1,
        "the live-owner runtime dir is spared"
    );
    assert_eq!(report.skipped_too_new, 1, "the fresh runtime dir is spared");
    assert_eq!(report.skipped_unfenceable, 1, "the package dir is spared");
    assert!(report.errors.is_empty());

    assert!(live_dir.exists(), "live-owner runtime dir survives");
    assert!(fresh.exists(), "too-new runtime dir survives");
    assert!(pkg.exists(), "unfenceable package dir survives");
    assert!(unrelated.exists(), "non-fkst dir untouched");
    assert!(!no_bc_rt.exists(), "breadcrumb-less runtime orphan swept");

    let swept: HashSet<PathBuf> = report.swept.into_iter().collect();
    assert!(swept.contains(&no_bc_rt));

    let _ = live_child.start_kill();
}

#[tokio::test]
async fn boot_sweep_is_idempotent_on_a_clean_root() {
    let temp_root = tempfile::tempdir().expect("temp_root");
    let orphan = plant_dir(temp_root.path(), "fkst-rt-orphan", 600);
    let (engine_config, reconcile_config) = config(temp_root.path());

    let first = reconcile_orphans(&engine_config, &reconcile_config);
    assert_eq!(first.swept_count(), 1);
    assert!(!orphan.exists());

    let second = reconcile_orphans(&engine_config, &reconcile_config);
    assert_eq!(second.scanned, 0, "clean root has nothing to scan");
    assert_eq!(second.swept_count(), 0, "second pass is a no-op");
}

#[tokio::test]
async fn boot_sweep_dry_run_records_but_deletes_nothing() {
    let temp_root = tempfile::tempdir().expect("temp_root");
    let rt_orphan = plant_dir(temp_root.path(), "fkst-rt-dry", 600);
    let pkg_orphan = plant_dir(temp_root.path(), "fkst-pkg-demo-dry", 600);

    let engine_config = EngineConfig {
        temp_root: temp_root.path().to_path_buf(),
        ..EngineConfig::default()
    };
    let reconcile_config = ReconcileConfig {
        min_age: Duration::from_secs(300),
        dry_run: true,
    };

    let report = reconcile_orphans(&engine_config, &reconcile_config);
    assert_eq!(
        report.swept_count(),
        1,
        "dry-run records the would-sweep orphan"
    );
    assert_eq!(
        report.skipped_unfenceable, 1,
        "the package dir is never a candidate"
    );
    assert!(
        rt_orphan.exists(),
        "dry-run must NOT delete the runtime dir"
    );
    assert!(pkg_orphan.exists(), "package dir is never deleted");
}
