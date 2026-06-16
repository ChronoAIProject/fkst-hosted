//! Integration test for the boot-time orphan temp-dir reconciliation
//! (issue #26, reduced scope): a real testcontainers Mongo + a real temp_root
//! with planted `fkst-rt-*` / `fkst-pkg-*` dirs + seeded sessions.
//!
//! Self-skips when Docker is unavailable so `cargo test` stays green on
//! runners without a Docker daemon (the established pattern).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use fkst_control_plane::config::Config;
use fkst_control_plane::db::Db;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::models::{SessionDoc, SessionStatus};
use fkst_control_plane::reconcile::{reconcile_orphans, ReconcileConfig};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;

/// True when a Docker daemon answers `docker info`.
fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Mongo image tag — pinned to the same major as the sibling suites.
const MONGO_TAG: &str = "7";

/// Start an ephemeral Mongo and build a connected `Db` over it.
async fn mongo_db() -> (ContainerAsync<Mongo>, Db) {
    let container = Mongo::default()
        .with_tag(MONGO_TAG)
        .start()
        .await
        .expect("start mongo");
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(27017)
        .await
        .expect("container port");
    let config = Config {
        mongodb_uri: format!("mongodb://{host}:{port}"),
        mongodb_server_selection_timeout_ms: 5000,
        ..Config::default()
    };
    let db = Db::connect(&config).await.expect("connect + ping");
    (container, db)
}

/// Plant a `fkst-*` dir under `base` with a marker file, aged `age_secs` old.
fn plant_dir(base: &Path, name: &str, age_secs: u64) -> PathBuf {
    let path = base.join(name);
    fs::create_dir(&path).expect("create planted dir");
    fs::write(path.join("marker"), b"x").expect("marker");
    let when = SystemTime::now() - Duration::from_secs(age_secs);
    let ft = filetime::FileTime::from_system_time(when);
    filetime::set_file_mtime(&path, ft).expect("set mtime");
    path
}

/// Build a session document with the given status and runtime_dir.
fn session(status: SessionStatus, runtime_dir: Option<&Path>) -> SessionDoc {
    SessionDoc {
        id: bson::Uuid::new(),
        package_name: "demo".to_string(),
        status,
        pod_id: Some("pod-test".to_string()),
        fencing_token: Some(1),
        pid: Some(1234),
        runtime_dir: runtime_dir.map(|p| p.to_string_lossy().to_string()),
        error: None,
        run_key: None,
        owner_user_id: None,
        org_id: None,
        package_names: vec![],
        goal_id: None,
        repo: None,
        env_scope: None,
        triggered_by: None,
        nyxid_key_id: None,
        nyxid_key_prefix: None,
        ornn_skills: None,
        created_at: bson::DateTime::now(),
        started_at: Some(bson::DateTime::now()),
        stopped_at: None,
    }
}

#[tokio::test]
async fn boot_sweep_removes_orphans_and_spares_live_and_fresh_dirs() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    db.ensure_indexes().await.expect("ensure indexes");

    let temp_root = tempfile::tempdir().expect("temp_root");
    let root = temp_root.path();

    // 1. A running session whose runtime_dir points at a planted dir -> must
    //    survive (fenced by the live set even though it is old).
    let live_dir = plant_dir(root, "fkst-rt-live", 600);
    db.sessions()
        .insert_one(session(SessionStatus::Running, Some(&live_dir)))
        .await
        .expect("insert running session");

    // 2. A stopped session whose runtime_dir points at a planted dir -> that
    //    dir is an orphan (terminal session) -> swept.
    let stopped_dir = plant_dir(root, "fkst-rt-stopped", 600);
    db.sessions()
        .insert_one(session(SessionStatus::Stopped, Some(&stopped_dir)))
        .await
        .expect("insert stopped session");

    // 3. A runtime dir with NO session at all -> swept (fenceable, orphan).
    let no_session_rt = plant_dir(root, "fkst-rt-orphan", 600);

    // 4. A PACKAGE dir with NO session at all -> survives: package dirs are
    //    unfenceable (their path is not persisted) and are NEVER deleted.
    let no_session_pkg = plant_dir(root, "fkst-pkg-demo-orphan", 600);

    // 5. A too-new runtime dir with no session -> survives (mtime fence).
    let fresh_dir = plant_dir(root, "fkst-rt-fresh", 5);

    // 6. A non-fkst dir -> never even scanned, always survives.
    let unrelated = root.join("unrelated");
    fs::create_dir(&unrelated).expect("unrelated dir");

    let engine_config = EngineConfig {
        temp_root: root.to_path_buf(),
        ..EngineConfig::default()
    };
    let reconcile_config = ReconcileConfig {
        min_age: Duration::from_secs(300),
        dry_run: false,
    };

    let report = reconcile_orphans(&db, &engine_config, &reconcile_config)
        .await
        .expect("reconcile pass");

    // Counts: scanned the 5 fkst-* dirs; swept the 2 genuine RUNTIME orphans;
    // spared the live runtime dir, the too-new one, and the package dir.
    assert_eq!(report.scanned, 5, "five fkst-* dirs scanned");
    assert_eq!(report.swept_count(), 2, "two genuine runtime orphans swept");
    assert_eq!(
        report.skipped_live, 1,
        "the running session's runtime dir is spared"
    );
    assert_eq!(report.skipped_too_new, 1, "the fresh runtime dir is spared");
    assert_eq!(
        report.skipped_unfenceable, 1,
        "the unfenceable package dir is spared"
    );
    assert!(report.errors.is_empty(), "no per-entry errors");

    // Disk reality matches the report.
    assert!(live_dir.exists(), "live session's runtime dir survives");
    assert!(fresh_dir.exists(), "too-new runtime dir survives");
    assert!(no_session_pkg.exists(), "unfenceable package dir survives");
    assert!(unrelated.exists(), "non-fkst dir untouched");
    assert!(!stopped_dir.exists(), "stopped session's runtime dir swept");
    assert!(!no_session_rt.exists(), "session-less runtime orphan swept");

    let swept: HashSet<PathBuf> = report.swept.into_iter().collect();
    assert!(swept.contains(&stopped_dir));
    assert!(swept.contains(&no_session_rt));
}

#[tokio::test]
async fn boot_sweep_is_idempotent_on_a_clean_root() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    db.ensure_indexes().await.expect("ensure indexes");

    let temp_root = tempfile::tempdir().expect("temp_root");
    let orphan = plant_dir(temp_root.path(), "fkst-rt-orphan", 600);

    let engine_config = EngineConfig {
        temp_root: temp_root.path().to_path_buf(),
        ..EngineConfig::default()
    };
    let reconcile_config = ReconcileConfig {
        min_age: Duration::from_secs(300),
        dry_run: false,
    };

    let first = reconcile_orphans(&db, &engine_config, &reconcile_config)
        .await
        .expect("first pass");
    assert_eq!(first.swept_count(), 1);
    assert!(!orphan.exists());

    // Second pass over the now-clean root: nothing to do.
    let second = reconcile_orphans(&db, &engine_config, &reconcile_config)
        .await
        .expect("second pass");
    assert_eq!(second.scanned, 0, "clean root has nothing to scan");
    assert_eq!(second.swept_count(), 0, "second pass is a no-op");
}

#[tokio::test]
async fn boot_sweep_dry_run_records_but_deletes_nothing() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    db.ensure_indexes().await.expect("ensure indexes");

    let temp_root = tempfile::tempdir().expect("temp_root");
    // A runtime orphan (the swept class) plus a package dir (never swept).
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

    let report = reconcile_orphans(&db, &engine_config, &reconcile_config)
        .await
        .expect("dry-run pass");
    assert_eq!(
        report.swept_count(),
        1,
        "dry-run records the would-sweep runtime orphan"
    );
    assert_eq!(
        report.skipped_unfenceable, 1,
        "the package dir is never a sweep candidate"
    );
    assert!(
        rt_orphan.exists(),
        "dry-run must NOT delete the runtime dir"
    );
    assert!(pkg_orphan.exists(), "package dir is never deleted");
}
