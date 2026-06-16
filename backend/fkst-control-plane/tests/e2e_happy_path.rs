//! End-to-end happy path (issue #21 exit criterion, re-pointed for #115):
//! a package now lives in the user's repo at `<repo>/.fkst/packages/<name>/`,
//! so this drives the REAL `SessionRunner` over the repo-scoped load path —
//! `--project-root <repo>` + `--package-root <repo>/.fkst/packages/<name>` —
//! against the REAL bundled engine, with no Mongo store and no HTTP package or
//! classic-session-create endpoints (both removed in #115).
//!
//! Self-skipping, honestly: the test engages only when a real `fkst-framework`
//! is available. The engine resolves via `tests/support/mod.rs`
//! (`FKST_ENGINE_BIN`, then `/usr/local/bin/fkst-framework`, then — Linux only —
//! Docker extraction from `FKST_ENGINE_IMAGE`); on hosts without one (e.g. macOS
//! without `FKST_ENGINE_BIN`) it prints a SKIP line and returns. NOTE: no CI job
//! currently provides the engine image to `cargo test`, so the green gate here
//! is compile + clean self-skip; the full run engages on engine-capable hosts.
//!
//! The package content is read FROM DISK out of the shared fixture
//! `backend/tests/fixtures/e2e-minimal/departments/hello/main.lua` and laid out
//! under a temp repo's `.fkst/packages/` exactly as the driver's clone produces.

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use fkst_control_plane::engine::{EngineConfig, LiveStatus, SessionRunner, StartSpec};
use support::require_engine;

/// How long the engine must stay Running after start to count as stable.
const POLL_STABILITY_WINDOW: Duration = Duration::from_secs(2);

/// Interval between liveness polls.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The single source of fixture truth, read from disk (never inlined).
fn fixture_lua() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/e2e-minimal/departments/hello/main.lua");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read fixture {}: {err}", path.display()))
}

/// Lay the fixture package under a temp repo's `.fkst/packages/<name>/`,
/// mirroring the driver's clone output. The raiser is needed so the engine has
/// a producer for the department's `tick` consumer.
fn write_fixture_repo(name: &str) -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("repo dir");
    let pkg = repo.path().join(".fkst").join("packages").join(name);
    std::fs::create_dir_all(pkg.join("departments").join("hello")).expect("mkdir dept");
    std::fs::create_dir_all(pkg.join("raisers")).expect("mkdir raisers");
    std::fs::write(
        pkg.join("departments").join("hello").join("main.lua"),
        fixture_lua(),
    )
    .expect("write main.lua");
    std::fs::write(
        pkg.join("raisers").join("tick.lua"),
        "return {\n  type = \"cron\",\n  interval = \"1s\",\n  produces = \"tick\",\n}\n",
    )
    .expect("write raiser");
    repo
}

#[tokio::test]
async fn e2e_repo_scoped_package_runs_then_stops_against_the_real_engine() {
    let engine_bin = require_engine!();

    let temp_root = tempfile::tempdir().expect("temp root");
    let engine = EngineConfig {
        framework_bin: engine_bin,
        temp_root: temp_root.path().to_path_buf(),
        ..EngineConfig::default()
    };
    let runner = SessionRunner::new(engine);

    // -- 1. lay the package in a repo's .fkst/packages/ (driver clone output) --
    let repo = write_fixture_repo("e2e-minimal");
    let package_root = repo
        .path()
        .join(".fkst")
        .join("packages")
        .join("e2e-minimal");

    // -- 2. start the engine over the repo-scoped load path ------------------
    let spec = StartSpec {
        packages: Vec::new(),
        goal: None,
        env_profile: std::collections::BTreeMap::new(),
        codex_home: None,
        project_root: Some(repo.path().to_path_buf()),
        package_roots: vec![package_root],
    };
    let mut session = runner
        .start_with_spec(&spec)
        .await
        .expect("real engine must start the repo-scoped package");

    // -- 3. confirm running (start_with_spec already waited for ready) -------
    assert_eq!(runner.status(&mut session), LiveStatus::Running);

    // Liveness is stable across a short window (no immediate uncommanded exit):
    // poll a few times, each tick must still be Running.
    let deadline = Instant::now() + POLL_STABILITY_WINDOW;
    while Instant::now() < deadline {
        assert_eq!(
            runner.status(&mut session),
            LiveStatus::Running,
            "engine left Running unexpectedly during the stability window"
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    // -- 4. stop cleanly -----------------------------------------------------
    runner.stop(&mut session).await.expect("stop");
    assert_eq!(runner.status(&mut session), LiveStatus::Stopped);
}
