//! The in-pod `run-session` runner (milestone #9, issue #289).
//!
//! Pod-per-session execution runs ONE substrate engine session per Kubernetes
//! Job. The Job's process exit IS the session disposition — there is no
//! ClaimMap/CAS, no `/internal/v1` worker protocol, and no heartbeat. This
//! module is that process: it reads the non-secret [`SessionSpec`] and the
//! mounted credential files ([`CredsLayout`]), clones the goal repo, renders a
//! per-session `CODEX_HOME`, injects any pinned Ornn skills, drives the engine
//! to a terminal status, and maps that status onto the process exit code
//! (`0` on a clean completion, non-zero on any failure).
//!
//! ## Abstraction seam (testability)
//!
//! The only step that genuinely needs the network is the authenticated
//! `git clone`. It is injected behind the [`RepoSource`] trait so the orchestration
//! core ([`run_session_with`]) — validation, env-profile assembly, codex render,
//! `StartSpec` build, and the supervise loop — is unit-testable hermetically with
//! a local-fixture source. Production wires [`GitCloneRepoSource`]; the
//! network-clone primitive itself is covered by `engine::clone`'s own tests.

mod creds;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use secrecy::SecretString;
use tempfile::TempDir;

use crate::engine::{
    clone_repo_packages, EngineConfig, GoalContext, LiveStatus, RunnerError, SessionRunner,
    StartSpec,
};
use crate::models::RepoRef;
use crate::nyxid::{NyxIdClient, DEFAULT_GITHUB_PROXY_SLUG};
use crate::ornn::{inject_pins, OrnnClient, DEFAULT_ORNN_SLUG};
use crate::session_spec::{CredsLayout, SessionSpec};
use crate::sessions::codex_provider::{render_codex_config, ProviderChoice, DEFAULT_ENV_KEY};

/// Env var naming the SessionSpec JSON path; defaults to [`DEFAULT_SPEC_PATH`].
const SPEC_PATH_ENV: &str = "FKST_SESSION_SPEC_PATH";
/// Default mount path of the SessionSpec JSON inside the pod.
const DEFAULT_SPEC_PATH: &str = "/var/run/fkst/session-spec.json";
/// Env var naming the mounted credential dir; defaults to the [`CredsLayout`]
/// default mount.
const CREDS_DIR_ENV: &str = "FKST_SESSION_CREDS_DIR";

/// Operator-pinned codex model + chrono-llm base URL for the DEFAULT provider.
/// Read directly from the environment (not threaded through the HTTP `Config`):
/// the pod boots the runner path only, so loading the full server `Config`
/// — with its bind addr / auth / NyxID requirements — would be needless
/// coupling. The defaults MIRROR `crate::config`'s `codex_model` /
/// `chrono_llm_base_url` defaults so the two paths never diverge.
const CODEX_MODEL_ENV: &str = "FKST_HOSTED_CODEX_MODEL";
const DEFAULT_CODEX_MODEL: &str = "gpt-5-codex";
const CHRONO_LLM_BASE_URL_ENV: &str = "FKST_HOSTED_CHRONO_LLM_BASE_URL";
const DEFAULT_CHRONO_LLM_BASE_URL: &str = "https://nyx.chrono-ai.fun/api/v1/proxy/s/chrono-llm";

/// Env var holding the NyxID issuer base URL, injected into the engine child so
/// the substrate's NyxID-aware tooling can reach the issuer. Not a reserved
/// platform key, so it may ride the `env_profile`.
const NYXID_URL_ENV: &str = "NYXID_URL";

/// Cache TTL for the in-pod NyxID client built solely to fetch Ornn pins. The
/// pod is single-session and short-lived, so the value only needs to be
/// non-zero; org-list caching is irrelevant here.
const ORNN_NYXID_CACHE_TTL: Duration = Duration::from_secs(60);

/// Poll interval of the supervise loop. The engine's status is cheap (`try_wait`
/// / OS truth), so a half-second cadence keeps the loop responsive without
/// busy-spinning.
const SUPERVISE_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// A prepared project working tree + its resolved package roots, plus the RAII
/// guard that removes the on-disk tree when the session ends.
///
/// The guard is held opaquely (`Box<dyn Send>`): production stores the whole
/// [`crate::engine::ClonedRepo`] (whose `TempDir`s clean the clone + credential
/// dir on drop); a test fixture stores a unit (the fixture repo dir is owned by
/// the test). Either way the runner only needs the drop behaviour, never to read
/// the guard back.
pub struct PreparedRepo {
    /// `--project-root`: the canonicalized repo working-tree root.
    pub project_root: PathBuf,
    /// One canonicalized `<repo>/.fkst/packages/<name>` root per requested
    /// package, in request order.
    pub package_roots: Vec<PathBuf>,
    /// Held for the session lifetime; dropping it removes the working tree.
    _guard: Box<dyn Send>,
}

/// How the runner obtains the goal's working tree + package roots.
///
/// Injected so the network `git clone` is the ONLY stubbed step in
/// [`run_session_with`]'s unit tests — every other piece of the orchestration is
/// exercised for real against a local fixture tree.
#[async_trait::async_trait]
pub trait RepoSource: Send + Sync {
    /// Materialize `repo` under `base` (authenticating with `token`) and resolve
    /// the named package roots.
    async fn prepare(
        &self,
        repo: &RepoRef,
        token: &SecretString,
        package_names: &[String],
        base: &Path,
        framework_bin: &Path,
    ) -> Result<PreparedRepo, RunnerError>;
}

/// Production [`RepoSource`]: a real authenticated shallow `git clone` plus
/// package resolution via [`clone_repo_packages`].
pub struct GitCloneRepoSource;

#[async_trait::async_trait]
impl RepoSource for GitCloneRepoSource {
    async fn prepare(
        &self,
        repo: &RepoRef,
        token: &SecretString,
        package_names: &[String],
        base: &Path,
        framework_bin: &Path,
    ) -> Result<PreparedRepo, RunnerError> {
        let cloned = clone_repo_packages(base, repo, token, package_names, framework_bin).await?;
        Ok(PreparedRepo {
            project_root: cloned.project_root.clone(),
            package_roots: cloned.package_roots.clone(),
            // Hold the whole ClonedRepo: its TempDir guards remove the clone and
            // the transient credential dir when the session ends.
            _guard: Box::new(cloned),
        })
    }
}

/// Entry point for the `run-session` subcommand: read the spec path + creds dir
/// from the environment, load the engine config, and drive one session to
/// completion. Returns the process [`ExitCode`] (`0` clean, non-zero failure).
pub async fn run_session_from_env() -> ExitCode {
    let spec_path = std::env::var(SPEC_PATH_ENV).unwrap_or_else(|_| DEFAULT_SPEC_PATH.to_string());
    let creds_dir = std::env::var(CREDS_DIR_ENV)
        .unwrap_or_else(|_| crate::session_spec::creds::DEFAULT_CREDS_DIR.to_string());

    let engine_config = match EngineConfig::load_from_env() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(error = %error, "run-session: failed to load engine configuration");
            return ExitCode::FAILURE;
        }
    };

    let creds = CredsLayout::new(creds_dir);
    tracing::info!(
        spec_path = %spec_path,
        creds_dir = %creds.base().display(),
        "run-session: starting"
    );
    run_session(Path::new(&spec_path), &creds, engine_config).await
}

/// Drive one session to completion using the PRODUCTION git-clone source.
///
/// Reads + validates the spec at `spec_path` and the credential files under
/// `creds`; a missing/invalid spec or an empty GitHub token returns a non-zero
/// [`ExitCode`] without spawning anything. Delegates the orchestration to
/// [`run_session_with`] so the only difference from a test is the [`RepoSource`].
pub async fn run_session(
    spec_path: &Path,
    creds: &CredsLayout,
    engine_config: EngineConfig,
) -> ExitCode {
    run_session_with(&GitCloneRepoSource, spec_path, creds, engine_config).await
}

/// The testable orchestration core. See [`run_session`]; the only injected
/// dependency is `source` (the network clone), so this exercises validation, the
/// env-profile assembly, the codex render, the `StartSpec` build, and the
/// supervise loop for real.
async fn run_session_with(
    source: &dyn RepoSource,
    spec_path: &Path,
    creds: &CredsLayout,
    engine_config: EngineConfig,
) -> ExitCode {
    // 1. Load + validate the non-secret descriptor.
    let spec = match load_spec(spec_path) {
        Ok(spec) => spec,
        Err(error) => {
            tracing::error!(error = %error, "run-session: invalid session spec");
            return ExitCode::FAILURE;
        }
    };
    let session_id = spec.session_id.clone();
    let run_key = spec.run_key.clone();
    tracing::info!(
        session_id = %session_id,
        run_key = %run_key,
        owner = %spec.repo.owner,
        name = %spec.repo.name,
        issue_number = spec.issue_number,
        package_count = spec.package_names.len(),
        ornn_pins = spec.ornn_pins.len(),
        "run-session: spec loaded"
    );

    // 2. The GitHub App installation token is required: an empty/missing token
    //    means the engine could never clone or push, so abort loudly.
    let github_token = match creds::read_required_secret(&creds.github_token()) {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(session_id = %session_id, error = %error, "run-session: missing github token");
            return ExitCode::FAILURE;
        }
    };

    // 3. Goal context. The deterministic `session_id` is a UUID string; reuse it
    //    as the goal id so logs/goal files key on the same identity. A
    //    non-UUID id (defensive) falls back to a fresh one.
    let goal_id = match bson::Uuid::parse_str(&session_id) {
        Ok(uuid) => uuid,
        Err(_) => {
            tracing::warn!(session_id = %session_id, "run-session: session id is not a uuid; using a fresh goal id");
            bson::Uuid::new()
        }
    };
    let token_expires_at =
        SystemTime::now() + Duration::from_secs(engine_config.github_token_refresh_secs.max(1));
    let goal = GoalContext {
        goal_id,
        title: spec.goal.title.clone(),
        description: spec.goal.prompt.clone(),
        repo: spec.repo.clone(),
        github_token: github_token.clone(),
        token_expires_at,
    };

    // 4. Clone (or fixture) the repo + resolve its package roots.
    let prepared = match source
        .prepare(
            &spec.repo,
            &github_token,
            &spec.package_names,
            &engine_config.temp_root,
            &engine_config.framework_bin,
        )
        .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::error!(session_id = %session_id, error = %error, "run-session: repo preparation failed");
            return ExitCode::FAILURE;
        }
    };

    // 5. Per-session env: the NyxID token (the codex `env_key`) + the issuer URL,
    //    injected only when their files are present + non-empty.
    let mut env_profile: BTreeMap<String, SecretString> = BTreeMap::new();
    if let Some(token) = creds::read_optional_nonempty(&creds.nyxid_token()) {
        env_profile.insert(DEFAULT_ENV_KEY.to_string(), SecretString::from(token));
    }
    if let Some(url) = creds::read_optional_nonempty(&creds.nyxid_url()) {
        env_profile.insert(NYXID_URL_ENV.to_string(), SecretString::from(url));
    }

    // 6. Render the per-session CODEX_HOME (0700 dir + DEFAULT chrono-llm
    //    config.toml). The pod has no vault, so the runner renders the DEFAULT
    //    provider layer directly rather than resolving from vault entries.
    let codex_home = match prepare_codex_home(&engine_config.temp_root) {
        Ok(home) => home,
        Err(error) => {
            tracing::error!(session_id = %session_id, error = %error, "run-session: failed to render CODEX_HOME");
            return ExitCode::FAILURE;
        }
    };

    // 7. Inject pinned Ornn skills, if any. A pin set with no NyxID token/URL is
    //    a loud failure (the pins cannot be fetched as the user).
    if let Err(error) = inject_ornn_if_pinned(&spec, creds, codex_home.path()).await {
        tracing::error!(session_id = %session_id, error = %error, "run-session: ornn injection failed");
        return ExitCode::FAILURE;
    }

    // 8. Build the StartSpec for the repo-scoped, pod-per-session shape and start
    //    the engine.
    let start_spec = StartSpec {
        packages: Vec::new(),
        goal: Some(goal),
        env_profile,
        codex_home: Some(codex_home.path().to_path_buf()),
        project_root: Some(prepared.project_root.clone()),
        package_roots: prepared.package_roots.clone(),
        session_id: session_id.clone(),
        worker_id: run_key.clone(),
    };
    let runner = SessionRunner::new(engine_config);
    let mut session = match runner.start_with_spec(&start_spec).await {
        Ok(session) => session,
        Err(error) => {
            tracing::error!(session_id = %session_id, error = %error, "run-session: engine start failed");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        session_id = %session_id,
        run_key = %run_key,
        pid = session.pid,
        "run-session: engine ready; supervising to completion"
    );

    // 9. Supervise to a terminal status. `prepared` + `codex_home` are dropped
    //    AFTER the loop so their RAII guards keep the clone + CODEX_HOME on disk
    //    for the whole session lifetime.
    let disposition = supervise_to_completion(&runner, &mut session, &session_id, &run_key).await;
    drop(codex_home);
    drop(prepared);
    disposition
}

/// Poll the engine's status until it reaches a terminal state, mapping a clean
/// stop onto success and a failure/crash onto a non-zero exit. The pod's own
/// `activeDeadlineSeconds` / TTL bound the wall-clock, so the loop itself adds no
/// artificial cap.
async fn supervise_to_completion(
    runner: &SessionRunner,
    session: &mut crate::engine::RunningSession,
    session_id: &str,
    run_key: &str,
) -> ExitCode {
    loop {
        match runner.status(session) {
            LiveStatus::Running => tokio::time::sleep(SUPERVISE_POLL_INTERVAL).await,
            LiveStatus::Stopped => {
                tracing::info!(
                    session_id = %session_id,
                    run_key = %run_key,
                    "run-session: engine completed cleanly"
                );
                return ExitCode::SUCCESS;
            }
            LiveStatus::Failed { code, signal } => {
                tracing::error!(
                    session_id = %session_id,
                    run_key = %run_key,
                    ?code,
                    ?signal,
                    "run-session: engine failed"
                );
                return ExitCode::FAILURE;
            }
        }
    }
}

/// Read + deserialize the SessionSpec from `path`. Both the read and the parse
/// surface a non-secret, path-anchored message (the spec carries no credentials).
fn load_spec(path: &Path) -> Result<SessionSpec, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("read session spec {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse session spec {}: {error}", path.display()))
}

/// Create a fresh 0700 CODEX_HOME under `temp_root` and write the DEFAULT
/// chrono-llm `config.toml` into it. The directory shares the engine temp root so
/// it inherits the same cleanup/reconcile filesystem as the runtime dirs.
fn prepare_codex_home(temp_root: &Path) -> Result<TempDir, String> {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::Builder::new()
        .prefix("fkst-codex-")
        .tempdir_in(temp_root)
        .map_err(|error| format!("create CODEX_HOME: {error}"))?;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("chmod CODEX_HOME to 0700: {error}"))?;

    let model = std::env::var(CODEX_MODEL_ENV).unwrap_or_else(|_| DEFAULT_CODEX_MODEL.to_string());
    let base_url = std::env::var(CHRONO_LLM_BASE_URL_ENV)
        .unwrap_or_else(|_| DEFAULT_CHRONO_LLM_BASE_URL.to_string());
    let config_toml = render_codex_config(&ProviderChoice::DefaultChronoLlm, &model, &base_url)
        .map_err(|error| format!("render codex config: {error}"))?;
    std::fs::write(dir.path().join("config.toml"), config_toml)
        .map_err(|error| format!("write config.toml: {error}"))?;
    Ok(dir)
}

/// Inject the spec's pinned Ornn skills into `codex_home`, if any. A no-op when
/// there are no pins. Pins present without a mounted NyxID token + URL is a loud
/// failure: the pins are fetched AS the user and cannot be resolved otherwise.
async fn inject_ornn_if_pinned(
    spec: &SessionSpec,
    creds: &CredsLayout,
    codex_home: &Path,
) -> Result<(), String> {
    if spec.ornn_pins.is_empty() {
        return Ok(());
    }

    let token = creds::read_optional_nonempty(&creds.nyxid_token())
        .ok_or_else(|| "ornn pins are present but no NyxID token is mounted".to_string())?;
    let url = creds::read_optional_nonempty(&creds.nyxid_url())
        .ok_or_else(|| "ornn pins are present but no NyxID URL is mounted".to_string())?;

    let nyxid = NyxIdClient::new(&url, DEFAULT_GITHUB_PROXY_SLUG, ORNN_NYXID_CACHE_TTL)
        .map_err(|error| format!("build NyxID client for ornn: {error}"))?;
    let ornn = OrnnClient::with_nyxid(nyxid, DEFAULT_ORNN_SLUG)
        .map_err(|error| format!("build ornn client: {error}"))?;
    inject_pins(
        &ornn,
        &SecretString::from(token),
        codex_home,
        &spec.ornn_pins,
    )
    .await
    .map_err(|error| format!("inject ornn pins: {error}"))?;
    tracing::info!(
        pin_count = spec.ornn_pins.len(),
        "run-session: ornn pins injected"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
