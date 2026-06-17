//! The worker's engine executor (issue #151, increment 4).
//!
//! [`execute_dispatch`] spawns ONE engine from a controller-resolved
//! [`ResolvedDispatch`], mirroring the control-plane driver's start sequence
//! exactly (see the module docs in `super`). It returns the running engine plus
//! the on-disk guards (the clone working tree + the CODEX_HOME dir) that the
//! worker must hold for the session's lifetime — the driver-side
//! `_clone_guard` / `_codex_home_guard` made worker-owned.
//!
//! Out of scope for this increment (the NEXT one): the supervise loop, status
//! reporting, and credential refresh. This only spawns and registers.
//!
//! Secrets (`github_token`, the env profile values, the goal prompt, the mint
//! nonce) are `SecretString`s exposed ONLY at their write/use set-sites and are
//! NEVER logged.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use secrecy::ExposeSecret;
use tempfile::TempDir;

use fkst_engine::goal_token::write_nonce_file;
use fkst_engine::{
    EngineConfig, GoalContext, RunnerError, RunningSession, SessionRunner, StartSpec,
};
use fkst_journal::{Journaler, Transition};
use fkst_shared::protocol::{OrnnPlan, OrnnSource, ResolvedDispatch};

use super::journal::{self, start_session_journaler};
use super::{ClonedHandle, Cloner, RealCloner};

/// Owner-only permission for the CODEX_HOME dir and its `config.toml`, matching
/// the control-plane `prepare_codex_home`'s 0700 dir / 0600 file modes.
const CODEX_HOME_MODE: u32 = 0o700;
const CODEX_CONFIG_MODE: u32 = 0o600;

/// The on-disk guards the worker owns for a session's lifetime. Dropping it
/// removes the clone working tree (and the transient clone credential dir) and
/// the CODEX_HOME dir; the engine itself is stopped explicitly (the supervise
/// loop), never on drop. Held by the supervise task so the dirs outlive the
/// engine — the worker-owned mirror of the driver's `_clone_guard` /
/// `_codex_home_guard`.
#[derive(Debug)]
pub struct SessionGuards {
    /// Held for the session lifetime; dropping it removes the clone working tree
    /// and the transient clone credential dir (mirrors `_clone_guard`).
    _clone: ClonedHandle,
    /// The per-session CODEX_HOME dir guard, `Some` only when a CODEX_HOME was
    /// rendered (config and/or ornn present). Dropping it removes the dir
    /// (mirrors `_codex_home_guard`).
    _codex_home: Option<TempDir>,
}

impl SessionGuards {
    /// No guards — the re-adopt path: an adopted engine's dirs were created by
    /// the DEAD worker (there is no `TempDir` to re-wrap), so its `runtime_dir` /
    /// package dirs are cleaned explicitly by `RunningSession::stop` instead.
    pub fn none() -> Self {
        Self {
            _clone: ClonedHandle::new(std::path::PathBuf::new(), Vec::new(), Box::new(())),
            _codex_home: None,
        }
    }
}

/// A spawned, running engine plus the on-disk guards the worker owns for the
/// session's lifetime and the per-session journaler (`None` when journaling is
/// off). Dropping it removes the clone working tree and the CODEX_HOME dir; the
/// engine itself is stopped explicitly later (the supervise loop), never on
/// drop. `running` is `pub` so the caller can register it and supervise it;
/// [`Self::into_parts`] hands the running engine + its guards + the journaler to
/// the supervise task.
pub struct ExecutedSession {
    /// The live engine handle (`SessionRunner::start_with_spec` output).
    pub running: RunningSession,
    /// The on-disk guards (clone tree + CODEX_HOME), moved into the supervise
    /// task so the dirs outlive the engine.
    guards: SessionGuards,
    /// The per-session journaler, `Some` only when the dispatch carried a
    /// `JournalPlan`. Moved into the supervise loop, which journals the engine's
    /// RAISED stdout + lifecycle transitions through it.
    journaler: Option<Journaler>,
}

// Hand-written so the journaler (which is NOT `Debug` and holds a `SecretString`
// token) cannot leak via a derived `Debug`; only its presence is rendered.
impl std::fmt::Debug for ExecutedSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutedSession")
            .field("running", &self.running)
            .field("guards", &self.guards)
            .field("journaling", &self.journaler.is_some())
            .finish()
    }
}

impl ExecutedSession {
    /// Split into the running engine, its on-disk guards, and the per-session
    /// journaler, so the supervise task can own all three (it `&mut`-drives the
    /// engine, holds the guards for the dirs' lifetime, and journals through the
    /// journaler).
    pub fn into_parts(self) -> (RunningSession, SessionGuards, Option<Journaler>) {
        (self.running, self.guards, self.journaler)
    }
}

/// Failure modes of [`execute_dispatch`]. Each wraps the underlying domain error
/// so the caller can log the cause; none renders a secret.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// The repo clone / package resolution failed.
    #[error("clone failed: {0}")]
    Clone(#[source] RunnerError),
    /// A host-side filesystem operation (CODEX_HOME dir, config.toml, nonce)
    /// failed.
    #[error("io error: {0}")]
    Io(#[source] std::io::Error),
    /// A resolved Ornn skill could not be fetched or installed.
    #[error("ornn injection failed: {0}")]
    Ornn(String),
    /// The dispatch carried a malformed value the worker must reject at the
    /// trust boundary (e.g. an unparseable goal id or a bad base64 zip).
    #[error("invalid dispatch: {0}")]
    InvalidDispatch(String),
    /// The engine failed to start (conformance / startup / spawn).
    #[error("engine start failed: {0}")]
    Start(#[source] RunnerError),
}

/// Spawn the engine for `dispatch` and return the running session.
///
/// Production entry point: uses the real [`RealCloner`] (a verbatim
/// `clone_repo_packages`), so the dormant prod dispatch path is byte-identical to
/// the control-plane driver's clone. `http` is used ONLY for a direct, no-auth
/// fetch of an [`OrnnSource::PresignedUrl`] skill zip (the egress-free escape
/// hatch); inline [`OrnnSource::ZipB64`] skills never touch the network.
pub async fn execute_dispatch(
    cfg: &EngineConfig,
    dispatch: &ResolvedDispatch,
    http: &reqwest::Client,
) -> Result<ExecutedSession, ExecError> {
    execute_dispatch_with(cfg, dispatch, http, &RealCloner).await
}

/// The cloner-injected core, so the unit suite can drive every step except the
/// real `git clone` offline. The production [`execute_dispatch`] passes
/// [`RealCloner`].
pub(crate) async fn execute_dispatch_with(
    cfg: &EngineConfig,
    dispatch: &ResolvedDispatch,
    http: &reqwest::Client,
    cloner: &dyn Cloner,
) -> Result<ExecutedSession, ExecError> {
    // (1) The session runtime base: the engine temp root the runner already uses
    // for its `fkst-rt-*` / `fkst-codex-*` dirs. The clone, CODEX_HOME, and the
    // runner's runtime dir all live on this one filesystem so they share cleanup
    // semantics (mirrors the driver, which clones into `runner.temp_root()`).
    let base = cfg.temp_root.as_path();

    // (2) Clone the goal repo + resolve the named packages. The token rides a
    // 0600 credential file, never the argv (engine::clone) — and never a log.
    let cloned = cloner
        .clone_packages(
            base,
            &dispatch.clone_spec.repo,
            &dispatch.github_token,
            &dispatch.clone_spec.package_roots,
            &cfg.framework_bin,
        )
        .await
        .map_err(ExecError::Clone)?;

    // (2b) Start the per-session journaler (#151 i6c) and journal the first
    // lifecycle transition (Validating). `None` when the dispatch carries no
    // `JournalPlan` (journaling off) — then every journal call below is a no-op.
    // The fingerprint root is the FIRST cloned package dir (request order), the
    // run's primary package (mirrors the driver). Journaling is NEVER
    // load-bearing: a journaler start failure proceeds unjournaled.
    let mut journaler =
        start_session_journaler(dispatch, cloned.package_roots.first().map(|p| p.as_path())).await;
    journal::journal_lifecycle(&mut journaler, Transition::Validating).await;

    // (3) Per-session CODEX_HOME (#112/#114/#182): rendered when the dispatch
    // carries codex config OR an ornn plan OR the cloned repo ships a
    // `.fkst/AGENTS.md` base — exactly `prepare_codex_home`'s widened gate.
    // Order inside mirrors the driver: 0700 dir, then 0600 config.toml, then each
    // ornn skill install, then the composed AGENTS.md (repo base above Ornn
    // blocks). `cloned.project_root` is the worker-owned working tree, so the
    // repo base is read with ZERO extra GitHub call and NO new dispatch field.
    let codex_home =
        prepare_codex_home(base, dispatch, cloned.project_root.as_path(), http).await?;
    let codex_home_path = codex_home.as_ref().map(|guard| guard.path().to_path_buf());

    // (4) Build the GoalContext from the resolved dispatch. `description` is the
    // engine prompt (a SecretString) exposed only here to land in goal.json; the
    // token is cloned into the context and surfaces only at the 0600 file write.
    let goal_id = bson::Uuid::parse_str(&dispatch.goal.goal_id)
        .map_err(|e| ExecError::InvalidDispatch(format!("goal_id is not a uuid: {e}")))?;
    let goal_ctx = GoalContext {
        goal_id,
        title: dispatch.goal.title.clone(),
        description: dispatch.goal.description.expose_secret().to_string(),
        repo: dispatch.goal.repo.clone(),
        github_token: dispatch.github_token.clone(),
        token_expires_at: unix_ms_to_system_time(dispatch.github_token_expires_at_unix_ms),
    };

    // (6) Build the StartSpec. Repo-scoped: `packages` stays empty; the runner
    // points the engine at the clone (`project_root` + `package_roots`). Field
    // values match the driver's `(C)` construction exactly.
    let spec = StartSpec {
        packages: Vec::new(),
        goal: Some(goal_ctx),
        env_profile: dispatch.env_profile.clone(),
        codex_home: codex_home_path,
        project_root: Some(cloned.project_root.clone()),
        package_roots: cloned.package_roots.clone(),
        session_id: dispatch.session_id.clone(),
        worker_id: dispatch.worker_id.clone(),
    };

    // (7) Start the engine. `start_with_spec` itself writes the 0600 token file +
    // goal.json from the GoalContext, generates+writes its own `.mint-nonce`, and
    // writes the owner breadcrumb (session_id is non-empty) — so the executor does
    // NOT re-do those; it only overwrites the nonce below with the controller's.
    let running = SessionRunner::new(cfg.clone())
        .start_with_spec(&spec)
        .await
        .map_err(ExecError::Start)?;

    // (5) Overwrite `<runtime_dir>/.mint-nonce` (0600) with the controller's
    // nonce. The runtime dir is the runner's, known only now; the engine's
    // credential helper presents THIS nonce, and the controller authenticates a
    // refresh request against the same value — so it must be the dispatch's, not
    // the runner's locally-generated placeholder. Reuses the engine nonce writer.
    write_nonce_file(&running.runtime_dir, dispatch.mint_nonce.expose_secret())
        .map_err(runner_io_to_exec)?;

    // The engine is spawned: journal the Spawned{pid} lifecycle (mirrors the
    // driver, which journals it right after start returns). No-op when off.
    journal::journal_lifecycle(&mut journaler, Transition::Spawned { pid: running.pid }).await;

    tracing::info!(
        session_id = %dispatch.session_id,
        worker_id = %dispatch.worker_id,
        pid = running.pid,
        has_codex_home = codex_home.is_some(),
        "worker spawned engine for dispatch"
    );

    Ok(ExecutedSession {
        running,
        guards: SessionGuards {
            _clone: cloned,
            _codex_home: codex_home,
        },
        journaler,
    })
}

/// Render the per-session CODEX_HOME, or `None` when the dispatch carries no
/// codex config, no ornn plan, AND the cloned repo ships no `.fkst/AGENTS.md`
/// base (the widened #182 gate). Creates a 0700 dir under `base`, writes a 0600
/// `config.toml` when present, installs each ornn skill, then writes the composed
/// `AGENTS.md` — the repo base first (verbatim), the Ornn marker blocks below it.
///
/// The repo base is read from `project_root` (the worker-owned cloned working
/// tree) through `fkst_engine::read_repo_agents_md`, which is containment-guarded
/// via `safe_join` and size-capped; its content is never logged.
async fn prepare_codex_home(
    base: &Path,
    dispatch: &ResolvedDispatch,
    project_root: &Path,
    http: &reqwest::Client,
) -> Result<Option<TempDir>, ExecError> {
    // Read the repo's `.fkst/AGENTS.md` base FIRST so it can both widen the gate
    // (a base alone now warrants a CODEX_HOME) and seed AGENTS.md below. A
    // containment escape is a trust-boundary rejection (InvalidDispatch); an IO
    // failure maps through the existing runner-IO mapper.
    let repo_base = match fkst_engine::read_repo_agents_md(project_root) {
        Ok(base) => base,
        Err(RunnerError::InvalidPackage(message)) => {
            return Err(ExecError::InvalidDispatch(format!(
                "repo .fkst/AGENTS.md rejected: {message}"
            )));
        }
        Err(other) => return Err(runner_io_to_exec(other)),
    };

    // Widened gate (#182): a repo base alone now also produces a CODEX_HOME.
    if dispatch.codex_config_toml.is_none() && dispatch.ornn.is_none() && repo_base.is_none() {
        return Ok(None);
    }

    // 0700 dir under the engine temp root (same filesystem as the runtime dirs).
    let guard = tempfile::Builder::new()
        .prefix("fkst-codex-")
        .tempdir_in(base)
        .map_err(ExecError::Io)?;
    std::fs::set_permissions(
        guard.path(),
        std::fs::Permissions::from_mode(CODEX_HOME_MODE),
    )
    .map_err(ExecError::Io)?;

    // 0600 config.toml when codex is configured.
    if let Some(config_toml) = &dispatch.codex_config_toml {
        let config_path = guard.path().join("config.toml");
        std::fs::write(&config_path, config_toml.as_bytes()).map_err(ExecError::Io)?;
        std::fs::set_permissions(
            &config_path,
            std::fs::Permissions::from_mode(CODEX_CONFIG_MODE),
        )
        .map_err(ExecError::Io)?;
    }

    // Ornn install: each skill (fetched per its source), the same order
    // `inject_pins` applies (skills first). The install implementation lives in
    // `fkst-engine`. The marker-block appends are NOT written here — they are
    // composed below, on top of the repo base, so precedence lives in one place.
    let ornn_tail = if let Some(plan) = &dispatch.ornn {
        install_ornn_plan(guard.path(), plan, http).await?;
        plan.agents_md_appends.join("\n\n")
    } else {
        String::new()
    };

    // Compose AGENTS.md: the repo base first (verbatim), then the Ornn marker
    // blocks below it (#182). `compose_agents_md` is the single assembly rule
    // shared with the in-process driver, so both paths emit identical bytes. Only
    // write when the body is non-empty — a config-only CODEX_HOME (no base, no
    // ornn) leaves AGENTS.md absent, exactly as before #182.
    let body = fkst_engine::compose_agents_md(repo_base.as_deref(), &ornn_tail);
    if !body.is_empty() {
        std::fs::write(guard.path().join("AGENTS.md"), body).map_err(ExecError::Io)?;
    }

    Ok(Some(guard))
}

/// Install every resolved Ornn skill into `codex_home/skills/<name>/`. A
/// presigned-URL skill is fetched DIRECTLY (no auth) — the URL is sensitive and
/// is never logged; an inline base64 zip is decoded in-process (no network). The
/// skillset instruction blocks are composed into AGENTS.md by the caller
/// (`prepare_codex_home`), layered below any repo `.fkst/AGENTS.md` base (#182).
async fn install_ornn_plan(
    codex_home: &Path,
    plan: &OrnnPlan,
    http: &reqwest::Client,
) -> Result<(), ExecError> {
    for skill in &plan.skills {
        let zip = match &skill.source {
            OrnnSource::ZipB64(b64) => base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| {
                    ExecError::InvalidDispatch(format!(
                        "ornn skill {:?}: bad base64 zip: {e}",
                        skill.name
                    ))
                })?,
            OrnnSource::PresignedUrl(url) => fetch_presigned_zip(http, url)
                .await
                .map_err(|e| ExecError::Ornn(format!("skill {:?}: {e}", skill.name)))?,
        };
        fkst_engine::install_skill(codex_home, &skill.name, &zip)
            .map_err(|e| ExecError::Ornn(format!("skill {:?}: {e}", skill.name)))?;
    }
    Ok(())
}

/// Fetch a presigned skill zip directly (no auth header) and return its bytes.
/// The URL is sensitive (it grants read of the object) and is NEVER logged.
async fn fetch_presigned_zip(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("presigned fetch transport error: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("presigned fetch returned {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("presigned fetch body error: {e}"))?;
    Ok(bytes.to_vec())
}

/// Convert a non-negative unix-ms timestamp to a `SystemTime` (saturating at the
/// epoch for a non-positive value, so a clock-skewed dispatch never panics).
fn unix_ms_to_system_time(unix_ms: i64) -> SystemTime {
    if unix_ms <= 0 {
        return SystemTime::UNIX_EPOCH;
    }
    SystemTime::UNIX_EPOCH + Duration::from_millis(unix_ms as u64)
}

/// Map a runner IO error (from the nonce write) to [`ExecError::Io`], preserving
/// the underlying `io::Error` when present.
fn runner_io_to_exec(error: RunnerError) -> ExecError {
    match error {
        RunnerError::Io(io) => ExecError::Io(io),
        other => ExecError::Io(std::io::Error::other(other.to_string())),
    }
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
