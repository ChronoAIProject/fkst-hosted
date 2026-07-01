//! The effectful `run-substrate` driver (issue #359 §5): the in-pod entrypoint
//! that fetches the workspace packages + the target repo, wires the ROTATING
//! GitHub token into both `git` (a credential helper) and `gh` (a PATH shim),
//! renders the codex config, and execs `fkst-framework supervise` — forwarding
//! SIGTERM so a reconciler pod-delete drains supervise gracefully.
//!
//! Every DECISION-shaped step lives in [`super::plan`] (pure, unit-tested); this
//! module is the thin I/O shell whose full end-to-end correctness is verified on a
//! live cluster. Secret hygiene: the App token is NEVER read into a variable here —
//! the helper + shim read the mounted rotating file per-op so a control-plane token
//! rotation (§5.4) is always picked up; only the static LLM key + user-env values
//! are read (into `SecretString` / a plaintext map) and never logged.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{ExitCode, Stdio};

use nix::sys::signal::Signal;
use secrecy::{ExposeSecret, SecretString};
use tokio::process::Command;
use tokio::signal::unix::{signal, SignalKind};

use crate::engine::materialize_helper_script;
use crate::engine::process::signal_group;
use crate::engine::{git_config_entries, GitConfigEntry};
use crate::reserved_env::{is_reserved_env_key, LLM_ENV_KEY};
use crate::session_spec::creds::CredsLayout;
use crate::sessions::codex_provider::render_codex_config;

use super::plan::{
    build_supervise_args, exit_status_to_code, plan_clones, read_substrate_env,
    substrate_child_env, SubstrateEnv,
};

/// The bundled substrate binary the session execs (image-baked, §Dockerfile).
const FRAMEWORK_BIN: &str = "/usr/local/bin/fkst-framework";
/// The `gh` PATH shim source, materialized at runtime early on PATH so a bare `gh`
/// reads the rotating token (§5.2). Never overwrites the real `/usr/bin/gh`.
const GH_SHIM_SCRIPT: &str = include_str!("gh-shim.sh");
/// The shim filename (must be exactly `gh` so it shadows the real one on PATH).
const GH_SHIM_NAME: &str = "gh";
/// Subdirs the driver creates under the (writable) runtime root.
const PLATFORM_SUBDIR: &str = "platform";
const PROJECT_SUBDIR: &str = "project";
const GITCRED_SUBDIR: &str = "gitcred";
const SHIM_SUBDIR: &str = "binshim";
/// Env var the credential helper + gh shim read the mounted token path from.
const TOKEN_FILE_ENV: &str = "FKST_GITHUB_TOKEN_FILE";
const PATH_ENV: &str = "PATH";
/// Owner-only rwx for the codex home (session-private).
const CODEX_HOME_MODE: u32 = 0o700;
/// Owner rwx + group/other rx for the executable gh shim.
const SHIM_MODE: u32 = 0o755;

/// Entry point for the `run-substrate` subcommand: read the injected env and drive
/// the session pod to `supervise`, returning the supervise child's [`ExitCode`].
/// A launch-time failure (bad env, missing key, clone failure) is logged and
/// returns [`ExitCode::FAILURE`] without exec'ing supervise.
pub async fn run_substrate_from_env() -> ExitCode {
    let env = match read_substrate_env() {
        Ok(env) => env,
        Err(error) => {
            tracing::error!(error = %error, "run-substrate: invalid environment");
            return ExitCode::FAILURE;
        }
    };
    match run_substrate(&env).await {
        Ok(code) => code,
        Err(error) => {
            tracing::error!(error = %error, "run-substrate: launch failed");
            ExitCode::FAILURE
        }
    }
}

/// The launch sequence: idempotent roots, creds + git/gh wiring, fetch, codex
/// render, then exec supervise. Returns the supervise child's exit code.
async fn run_substrate(env: &SubstrateEnv) -> Result<ExitCode, String> {
    let runtime_root = Path::new(&env.runtime_root);
    let durable_root = Path::new(&env.durable_root);
    let creds = CredsLayout::new(&env.creds_dir);
    let token_file = creds.github_token();

    tracing::info!(
        repo = %env.repo,
        package_count = env.package_refs.len(),
        work_label = %env.work_label,
        durable_root = %env.durable_root,
        runtime_root = %env.runtime_root,
        "run-substrate: starting"
    );

    // 1. Idempotent roots (create-if-absent). A container restart under
    //    restartPolicy:Always MUST resume durable delivery state, never wipe it.
    create_dir_idempotent(durable_root)?;
    create_dir_idempotent(runtime_root)?;

    // 2. Read the static secrets. The github-token is deliberately NOT read into a
    //    variable — the helper + shim read the mounted file per-op so the
    //    control-plane token rotation is always picked up.
    let llm_api_key = read_trimmed_secret(&creds.llm_api_key())?;
    let user_env = read_user_env(&creds);

    // 3. git credentials: materialize the helper into a WRITABLE dir (the creds
    //    mount is read-only 0400) and point it — and the gh shim — at the mounted
    //    rotating token file. git uses the credential helper; gh has no helper hook,
    //    so a separate PATH shim exports GH_TOKEN from the same file. Both are
    //    needed because they authenticate by different mechanisms.
    let gitcred_dir = runtime_root.join(GITCRED_SUBDIR);
    create_dir_idempotent(&gitcred_dir)?;
    let helper_path = materialize_helper_script(&gitcred_dir)
        .map_err(|error| format!("materialize git credential helper: {error}"))?;
    let git_entries = git_config_entries(&helper_path);
    let shim_dir = runtime_root.join(SHIM_SUBDIR);
    install_gh_shim(&shim_dir)?;

    // 4. Fetch: the one workspace repo (all refs share it in v1) into
    //    <runtime>/platform at its ref, and the target repo (default branch) into
    //    <runtime>/project. Both authenticate via the credential helper (public
    //    repos succeed regardless; a private target uses the App token).
    let plan = plan_clones(&env.package_refs)?;
    let platform_root = runtime_root.join(PLATFORM_SUBDIR);
    let project_root = runtime_root.join(PROJECT_SUBDIR);
    let workspace_url = format!(
        "https://github.com/{}/{}.git",
        plan.platform_repo.owner, plan.platform_repo.repo
    );
    git_clone(
        &workspace_url,
        Some(&plan.platform_repo.git_ref),
        &platform_root,
        &git_entries,
        &token_file,
    )
    .await?;
    let target_url = format!("https://github.com/{}.git", env.repo);
    git_clone(&target_url, None, &project_root, &git_entries, &token_file).await?;

    // 5. Render CODEX_HOME/config.toml (the API key rides LLM_ENV_KEY, never the
    //    toml itself).
    render_codex(env)?;

    // 6. Build the supervise argv + the child env.
    let args = build_supervise_args(
        &project_root.to_string_lossy(),
        &platform_root.to_string_lossy(),
        &plan.platform_packages,
        &env.durable_root,
        &env.runtime_root,
        FRAMEWORK_BIN,
    );
    let mut child_env = substrate_child_env(
        std::env::vars().collect(),
        &user_env,
        llm_api_key.expose_secret(),
        &git_entries,
        &env.codex_home,
        &env.durable_root,
        &env.runtime_root,
    );
    // The helper + shim both read the mounted rotating token from this path.
    upsert_env(
        &mut child_env,
        TOKEN_FILE_ENV,
        &token_file.to_string_lossy(),
    );
    // Prepend the shim dir so our `gh` wins over /usr/bin/gh on PATH.
    prepend_path(&mut child_env, &shim_dir);

    // 7. exec supervise, forwarding SIGTERM to its group for a graceful drain.
    exec_supervise(args, child_env).await
}

/// Create `dir` (and parents) idempotently — an existing dir is not an error and
/// is NOT wiped.
fn create_dir_idempotent(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("create dir {}: {error}", dir.display()))
}

/// Read a required credential file into a [`SecretString`], trimming the trailing
/// newline a Secret write leaves. Only the path (non-secret) appears in an error.
fn read_trimmed_secret(path: &Path) -> Result<SecretString, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("read credential file {}: {error}", path.display()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("credential file {} is empty", path.display()));
    }
    Ok(SecretString::from(trimmed.to_string()))
}

/// Read the mounted `userenv.<KEY>` files into a plaintext map, dropping any key
/// the platform owns (a warn per rejected key). An unreadable individual file is
/// logged and skipped — optional user env never aborts the launch.
fn read_user_env(creds: &CredsLayout) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let files = match creds.user_env_files() {
        Ok(files) => files,
        Err(error) => {
            tracing::warn!(error = %error, "run-substrate: could not list user env files");
            return map;
        }
    };
    for (key, path) in files {
        if is_reserved_env_key(&key) {
            tracing::warn!(key = %key, "run-substrate: dropping reserved user env key");
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(value) => {
                map.insert(key, value.strip_suffix('\n').unwrap_or(&value).to_string());
            }
            Err(error) => {
                tracing::warn!(key = %key, error = %error, "run-substrate: skipping unreadable user env file")
            }
        }
    }
    map
}

/// Materialize the executable `gh` PATH shim into `shim_dir`.
fn install_gh_shim(shim_dir: &Path) -> Result<(), String> {
    create_dir_idempotent(shim_dir)?;
    let path = shim_dir.join(GH_SHIM_NAME);
    std::fs::write(&path, GH_SHIM_SCRIPT)
        .map_err(|error| format!("write gh shim {}: {error}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(SHIM_MODE))
        .map_err(|error| format!("chmod gh shim: {error}"))?;
    Ok(())
}

/// Render the operator-pinned codex `config.toml` into `CODEX_HOME`.
fn render_codex(env: &SubstrateEnv) -> Result<(), String> {
    let home = Path::new(&env.codex_home);
    create_dir_idempotent(home)?;
    // Best-effort tighten to 0700 (the config references the env_key, not the key
    // itself, but the home dir is still session-private).
    if let Err(error) =
        std::fs::set_permissions(home, std::fs::Permissions::from_mode(CODEX_HOME_MODE))
    {
        tracing::warn!(error = %error, "run-substrate: could not chmod CODEX_HOME to 0700");
    }
    let toml = render_codex_config(
        &env.llm_model,
        &env.llm_base_url,
        &env.llm_wire_api,
        LLM_ENV_KEY,
    );
    std::fs::write(home.join("config.toml"), toml)
        .map_err(|error| format!("write codex config.toml: {error}"))
}

/// Shallow `git clone` of `url` into `dest`, authenticating via the credential
/// helper wired through `GIT_CONFIG_*` (the token stays in the mounted file, never
/// in argv or `.git/config`). `git_ref` (branch/tag) selects a `--single-branch`
/// shallow checkout; `None` clones the default branch. An existing clone is reused
/// (idempotent restart).
async fn git_clone(
    url: &str,
    git_ref: Option<&str>,
    dest: &Path,
    git_entries: &[GitConfigEntry],
    token_file: &Path,
) -> Result<(), String> {
    if dest.join(".git").is_dir() {
        tracing::info!(dest = %dest.display(), "run-substrate: clone already present; reusing");
        return Ok(());
    }
    let mut command = Command::new("git");
    command.arg("clone").arg("--depth").arg("1");
    if let Some(git_ref) = git_ref {
        // --branch accepts a branch OR a tag. // verify live: an arbitrary commit
        // SHA needs init+fetch+checkout (see backend/Dockerfile); branch/tag covered.
        command.arg("--single-branch").arg("--branch").arg(git_ref);
    }
    command
        .arg(url)
        .arg(dest)
        // The helper resolves the token from the mounted file at credential time.
        .env(TOKEN_FILE_ENV, token_file)
        // Never let git drop into an interactive prompt that would hang the pod.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env("GIT_CONFIG_COUNT", git_entries.len().to_string());
    for (i, entry) in git_entries.iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{i}"), &entry.key);
        command.env(format!("GIT_CONFIG_VALUE_{i}"), &entry.value);
    }

    let output = command
        .output()
        .await
        .map_err(|error| format!("spawn git clone {url}: {error}"))?;
    if !output.status.success() {
        // stderr may carry the failure reason but NEVER the token (it is in the
        // mounted file, not the argv/url).
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(url = %url, code = ?output.status.code(), stderr = %stderr, "run-substrate: git clone failed");
        return Err(format!(
            "git clone {url} failed (code {:?})",
            output.status.code()
        ));
    }
    tracing::info!(url = %url, dest = %dest.display(), "run-substrate: cloned");
    Ok(())
}

/// Spawn `fkst-framework supervise` with the built argv + env in its OWN process
/// group and supervise it, forwarding SIGTERM/SIGINT to the child's group so the
/// reconciler's pod-delete (SIGTERM) drains supervise + its descendants (codex,
/// git) gracefully. Returns the child's exit code as this process's [`ExitCode`].
async fn exec_supervise(args: Vec<String>, env: Vec<(String, String)>) -> Result<ExitCode, String> {
    let mut command = Command::new(FRAMEWORK_BIN);
    command
        .args(&args)
        // The child env is the FULL environment (built from `std::env::vars()` +
        // overrides), so clear-then-set makes it deterministic and drops nothing
        // unexpected.
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        // stdout/stderr inherit (default) so the agent output flows to pod logs.
        // A new process group (pgid == child pid) lets us signal the whole tree.
        .process_group(0)
        .kill_on_drop(false);
    tracing::info!(bin = FRAMEWORK_BIN, args = ?args, "run-substrate: exec supervise");

    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn {FRAMEWORK_BIN}: {error}"))?;
    let pid = child
        .id()
        .ok_or_else(|| "supervise child exited before yielding a pid".to_string())?;

    let mut sigterm = signal(SignalKind::terminate())
        .map_err(|error| format!("install SIGTERM handler: {error}"))?;
    let mut sigint = signal(SignalKind::interrupt())
        .map_err(|error| format!("install SIGINT handler: {error}"))?;

    let status = loop {
        tokio::select! {
            // biased: always check child exit first so a race between exit and a
            // signal never re-signals a dead group.
            biased;
            result = child.wait() => break result.map_err(|error| format!("await supervise: {error}"))?,
            _ = sigterm.recv() => forward_signal(pid, Signal::SIGTERM),
            _ = sigint.recv() => forward_signal(pid, Signal::SIGTERM),
        }
    };

    let code = exit_status_to_code(status.code());
    tracing::info!(code, "run-substrate: supervise exited");
    Ok(ExitCode::from(code))
}

/// Forward `signal` to the supervise child's process GROUP (it is
/// `process_group(0)`, so pgid == pid). `ESRCH` (already gone) is a benign no-op.
fn forward_signal(pid: u32, signal: Signal) {
    match signal_group(pid as i32, signal) {
        Ok(()) => tracing::info!(
            pid,
            ?signal,
            "run-substrate: forwarded signal to supervise group"
        ),
        Err(nix::Error::ESRCH) => {}
        Err(error) => tracing::warn!(pid, %error, "run-substrate: could not forward signal"),
    }
}

/// Insert-or-replace `key` in the ordered env vec.
fn upsert_env(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(slot) = env.iter_mut().find(|(k, _)| k == key) {
        slot.1 = value.to_string();
    } else {
        env.push((key.to_string(), value.to_string()));
    }
}

/// Prepend `dir` to the child env's `PATH` so a bare `gh` resolves to the shim.
fn prepend_path(env: &mut Vec<(String, String)>, dir: &Path) {
    let dir = dir.to_string_lossy();
    let existing = env
        .iter()
        .find(|(k, _)| k == PATH_ENV)
        .map(|(_, v)| v.clone());
    let new_path = match existing {
        Some(path) if !path.is_empty() => format!("{dir}:{path}"),
        _ => dir.to_string(),
    };
    upsert_env(env, PATH_ENV, &new_path);
}
