//! Issue #182 tests: seeding the repo's `.fkst/AGENTS.md` as the verbatim base
//! of the per-session `CODEX_HOME/AGENTS.md`, with Ornn marker blocks layered
//! below it. Split out of `executor_tests.rs` (included via a nested `#[path]`)
//! so each test file stays under the 500-line budget. Reuses the parent module's
//! `FakeCloner` / `dispatch_with` / `config` / `engine_stub` support via `super`.

use super::*;

/// Run a dispatch through a fake cloner and return the composed AGENTS.md body
/// (or `None` when no CODEX_HOME was rendered). Stops the engine cleanly. Shared
/// by the #182 layering/degenerate-case tests so each stays focused on its body
/// assertion.
async fn agents_md_for(
    cfg: &EngineConfig,
    dispatch: &ResolvedDispatch,
    http: &reqwest::Client,
    cloner: &FakeCloner,
) -> Option<String> {
    let mut session = execute_dispatch_with(cfg, dispatch, http, cloner)
        .await
        .expect("dispatch executes");
    let body = session.guards._codex_home.as_ref().map(|home| {
        let path = home.path().join("AGENTS.md");
        if path.is_file() {
            Some(std::fs::read_to_string(path).expect("AGENTS.md"))
        } else {
            None
        }
    });
    let runner = SessionRunner::new(cfg.clone());
    runner.stop(&mut session.running).await.expect("stop");
    // Flatten: outer `Some` = a CODEX_HOME existed; inner `Some` = AGENTS.md
    // present. We collapse both "no CODEX_HOME" and "CODEX_HOME but no AGENTS.md"
    // callers handle distinctly via the guard, so return the inner directly.
    body.flatten()
}

/// #182 repo base + Ornn: AGENTS.md is the repo base VERBATIM, then exactly one
/// blank line, then the Ornn marker-block tail — the fixed precedence.
#[tokio::test]
async fn execute_dispatch_layers_ornn_below_repo_base() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let bin = engine_stub(stub_dir.path());
    let cfg = config(&bin, temp_root.path());
    let http = reqwest::Client::new();

    let plan = OrnnPlan {
        agents_md_appends: vec![
            "<!-- ornn-skillset:x BEGIN -->\nbody\n<!-- ornn-skillset:x END -->".into(),
        ],
        skills: Vec::new(),
    };
    let dispatch = dispatch_with(BTreeMap::new(), Some(plan));
    let cloner = FakeCloner::with_repo_base("# Repo rules\nFollow them.\n");

    let agents = agents_md_for(&cfg, &dispatch, &http, &cloner)
        .await
        .expect("AGENTS.md written");
    assert_eq!(
        agents,
        "# Repo rules\nFollow them.\n\n\
         <!-- ornn-skillset:x BEGIN -->\nbody\n<!-- ornn-skillset:x END -->\n",
        "repo base verbatim, one blank line, then the Ornn block"
    );
}

/// #182 base-only: a repo `.fkst/AGENTS.md` with no codex config and no Ornn
/// STILL yields a CODEX_HOME (widened gate) whose AGENTS.md is just the base.
#[tokio::test]
async fn execute_dispatch_repo_base_only_seeds_agents_md() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let bin = engine_stub(stub_dir.path());
    let cfg = config(&bin, temp_root.path());
    let http = reqwest::Client::new();

    let mut dispatch = dispatch_with(BTreeMap::new(), None);
    dispatch.codex_config_toml = None; // No config, no Ornn — base alone.
    let cloner = FakeCloner::with_repo_base("# Base only\nrules\n");

    let agents = agents_md_for(&cfg, &dispatch, &http, &cloner)
        .await
        .expect("AGENTS.md written from base alone");
    assert_eq!(agents, "# Base only\nrules\n");
}

/// #182 oversize: a repo base larger than the 256 KiB cap is truncated (never
/// panics on a boundary) and still produces a CODEX_HOME from the base alone.
#[tokio::test]
async fn execute_dispatch_truncates_oversize_repo_base() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let bin = engine_stub(stub_dir.path());
    let cfg = config(&bin, temp_root.path());
    let http = reqwest::Client::new();

    let mut dispatch = dispatch_with(BTreeMap::new(), None);
    dispatch.codex_config_toml = None;
    let cloner = FakeCloner::with_oversize_repo_base();

    let agents = agents_md_for(&cfg, &dispatch, &http, &cloner)
        .await
        .expect("AGENTS.md written");
    // Truncated to the cap (ASCII body, so the boundary is exactly the cap) plus
    // a single trailing newline added by `compose_agents_md`.
    assert_eq!(
        agents.len(),
        CAP_BYTES + 1,
        "truncated to cap + trailing \\n"
    );
    assert!(agents.ends_with('\n'));
    assert!(agents.trim_end_matches('\n').bytes().all(|b| b == b'x'));
}

/// #182 containment: a `.fkst` symlink that escapes the cloned tree is rejected
/// at the trust boundary (`InvalidDispatch`); no engine is spawned and the
/// escaped file is never read.
#[tokio::test]
async fn execute_dispatch_rejects_fkst_symlink_escape() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let bin = engine_stub(stub_dir.path());
    let cfg = config(&bin, temp_root.path());
    let http = reqwest::Client::new();

    let dispatch = dispatch_with(BTreeMap::new(), None);
    let cloner = FakeCloner::with_escaping_fkst();

    let err = execute_dispatch_with(&cfg, &dispatch, &http, &cloner)
        .await
        .expect_err("escaping .fkst must be rejected");
    assert!(matches!(err, ExecError::InvalidDispatch(_)), "got {err:?}");
}
