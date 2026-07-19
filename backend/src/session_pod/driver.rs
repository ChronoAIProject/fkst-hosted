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

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

use secrecy::{ExposeSecret, SecretString};
use tokio::process::Command;

use crate::reserved_env::{is_reserved_env_key, LLM_ENV_KEY};
use crate::session_spec::creds::CredsLayout;

use super::codex::{render_codex_config, CodexShellEnv};
use super::creds_gate::{
    creds_wait_timeout_from_env, wait_for_creds_complete, CREDS_POLL_INTERVAL,
};
use super::creds_helper::{git_config_entries, materialize_helper_script, GitConfigEntry};
use super::log_stream::collector::{collector_config_from_env, spawn_collector};
use super::plan::{
    build_supervise_args, plan_clones, read_substrate_env, substrate_child_env, SubstrateEnv,
};
use super::supervise::{exec_supervise, FRAMEWORK_BIN};

/// The `gh` PATH shim source, materialized at runtime early on PATH so a bare `gh`
/// reads the rotating token (§5.2). Never overwrites the real `/usr/bin/gh`.
const GH_SHIM_SCRIPT: &str = include_str!("gh-shim.sh");
/// The shim filename (must be exactly `gh` so it shadows the real one on PATH).
const GH_SHIM_NAME: &str = "gh";
/// Subdirs the driver creates under the (writable) runtime root.
const PLATFORM_SUBDIR: &str = "platform";
const PROJECT_SUBDIR: &str = "project";
const GITCRED_SUBDIR: &str = "gitcred";
/// Repo-local workflow catalog. workflow-writer authors new `fkst.workflow.v1`
/// files here (as PRs to the target repo) and github-devloop-workflow's
/// `workflow_select` loads + runs them from here. Defaulted to the target repo's
/// `.fkst/packages/` (set-if-absent, so an operator-pinned session value wins).
const WORKFLOW_CATALOG_ROOT_ENV: &str = "FKST_WORKFLOW_CATALOG_ROOT";
const WORKFLOW_CATALOG_SUBDIR: &str = ".fkst/packages";
/// Writable per-session tool dir, put on the FRONT of PATH and exposed as
/// `FKST_ENV_BIN` (see [`crate::install::TOOL_DIR_ENV`]) so a named-environment
/// install step can drop a tool binary the workflow then calls by bare name.
const ENV_BIN_SUBDIR: &str = "env-bin";
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

    // 1b. Gate on the credentials-complete sentinel BEFORE reading any credential:
    //     the writer creates it LAST, so its presence proves the whole set is on disk.
    //     In k8s-customized mode it rides the atomic Secret mount → the first check
    //     passes (~0ms). A writer that never finishes trips the timeout and we abort
    //     engine start rather than run with a half-written credential set.
    match wait_for_creds_complete(
        &creds.creds_complete(),
        creds_wait_timeout_from_env(),
        CREDS_POLL_INTERVAL,
    )
    .await
    {
        Ok(waited) => tracing::info!(
            wait_ms = waited.as_millis() as u64,
            "run-substrate: credentials-complete sentinel present; starting engine"
        ),
        Err(timeout) => {
            tracing::error!(
                sentinel = %timeout.sentinel.display(),
                elapsed_ms = timeout.elapsed.as_millis() as u64,
                "run-substrate: credentials not complete before timeout; aborting engine start"
            );
            return Err(timeout.to_string());
        }
    }

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
    let mut git_entries = git_config_entries(&helper_path);
    // The devloop packages commit with `git`; without an author/committer identity
    // the commit fails ("Author identity unknown"). Stamp the App bot as the git
    // identity in CONFIG form (applies to every git invocation regardless of how the
    // devloop shells out). The email is the App bot's GitHub `noreply`, so the
    // commit attributes to the App that authored the push. Skipped when no bot login
    // was injected (git then falls back to its own error, surfaced in the pod log).
    if !env.bot_login.is_empty() {
        git_entries.push(GitConfigEntry {
            key: "user.name".to_string(),
            value: env.bot_login.clone(),
        });
        git_entries.push(GitConfigEntry {
            key: "user.email".to_string(),
            value: format!("{}@users.noreply.github.com", env.bot_login),
        });
    }
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

    // 4b. The framework's host-root workspace discovery walks UP from --project-root
    //     for a `fkst.workspace.toml` and fails CLOSED without one. The target repo
    //     is a plain repo with no fkst workspace, so write a minimal manifest
    //     declaring zero units: the host root owns no departments, while each
    //     platform `--package-root` resolves its `libraries/*` from the platform
    //     clone's OWN `fkst.workspace.toml` (walk-up from that package root). Verified
    //     against fkst-substrate `path_resolver.rs` host-root discovery.
    let workspace_manifest = project_root.join("fkst.workspace.toml");
    std::fs::write(&workspace_manifest, "[workspace]\nunits = []\n")
        .map_err(|e| format!("write {}: {e}", workspace_manifest.display()))?;

    // 5. (CODEX_HOME/config.toml is rendered in step 6c below — after the
    //    named-environment install — so its shell-environment policy can expose the
    //    env-bin PATH + profile variables to codex's own shell commands.)

    // 6. Build the supervise argv + the child env.
    let args = build_supervise_args(
        &project_root.to_string_lossy(),
        &platform_root.to_string_lossy(),
        &plan.package_paths,
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
    // Repo-local workflow catalog: point both workflow-writer (which authors new
    // workflow files here as PRs) and the github-devloop-workflow host (which loads
    // + runs them) at the target repo's `.fkst/packages/`. Set-if-absent so an
    // operator/session-pinned FKST_WORKFLOW_CATALOG_ROOT wins over this default.
    let workflow_catalog_root = workflow_catalog_root_default(&project_root);
    default_env(
        &mut child_env,
        WORKFLOW_CATALOG_ROOT_ENV,
        &workflow_catalog_root.to_string_lossy(),
    );

    // Provision the named-environment profile: run its ordered install commands in
    // the pod BEFORE supervise, with the injected env available and a writable
    // env-bin on PATH. This is what makes a profile that installs a tool (e.g. an
    // encoder) actually work at run time, not only pass PUT-time validation. Fail
    // closed — a broken install must not start a half-provisioned engine.
    run_env_install_commands(&creds, &mut child_env, runtime_root).await?;

    // 6c. Render CODEX_HOME/config.toml NOW that child_env is final: its
    //     [shell_environment_policy] exposes the named-environment's env-bin PATH +
    //     FKST_ENV_BIN + non-secret variables to codex's own shell commands, so a
    //     profile-installed tool (e.g. ffmpeg) is reachable from inside codex, not
    //     only by the supervise process. (The API key rides LLM_ENV_KEY, never the
    //     toml itself.)
    //
    //     The env-profile's SECRET values ride `user_env` too (the session injects
    //     them via child_env), but they must NEVER be written into the plaintext
    //     config.toml — so expose only the NON-SECRET variables to codex, per the
    //     mounted secret-key manifest.
    let codex_variables = non_secret_variables(&creds, &user_env);
    render_codex(env, &child_env, &codex_variables)?;

    // 6b. Log streaming: spawn the in-pod collector BEFORE supervise so it captures
    //     the whole run. Streaming is unconditional — the collector redacts every
    //     record and uploads a `tar.gz` bundle to chrono-storage as the mounted
    //     storage SA (or, absent storage creds, captures without uploading). It reads
    //     only the mounted creds it already has and adds no new credential. The
    //     collector runs on its own thread; a failure inside it can never crash or
    //     block supervise.
    let log_stream = spawn_collector(collector_config_from_env(
        env.repo.clone(),
        plan.platform_repo.git_ref.clone(),
        runtime_root.to_path_buf(),
        Path::new(&env.codex_home).to_path_buf(),
        Path::new(&env.creds_dir).to_path_buf(),
    ));
    let log_sender = Some(log_stream.sender());
    tracing::info!("run-substrate: in-pod log streaming enabled");
    // Seed `fkst-hosted/driver.log` with the driver's own launch record (the engine
    // ref + repo it is about to supervise) so the bundle captures the fkst-hosted
    // side of the run, not only the supervise/codex output.
    log_stream.emit_driver(format!(
        "run-substrate: supervising {} at engine ref {}",
        env.repo, plan.platform_repo.git_ref
    ));

    // 7. exec supervise, forwarding SIGTERM to its group for a graceful drain, and
    //    (when streaming) tee'ing its stdout/stderr into the collector.
    let code = exec_supervise(args, child_env, log_sender).await;

    // 7b. Record the exit into driver.log, then signal end-of-stream + wait (bounded)
    //     for the collector's final flush so a revived pod does not race the last
    //     upload. Non-fatal regardless of `code`.
    log_stream.emit_driver("run-substrate: supervise exited; finalizing logs");
    log_stream.shutdown().await;
    code
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

/// The subset of `user_env` that is SAFE to write into the codex `config.toml`:
/// the profile's non-secret variables, with the secret-valued keys removed per the
/// mounted [`CredsLayout::secret_keys`] manifest. The secrets themselves still ride
/// `user_env` (the session injects them into its process env) — they are only kept
/// out of the plaintext config here.
///
/// Fails CLOSED: an unreadable or corrupt manifest exposes NO variables to codex
/// (an empty map) rather than risk leaking a secret. An ABSENT manifest means the
/// profile declared no secrets, so every variable is exposed.
fn non_secret_variables(
    creds: &CredsLayout,
    user_env: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let secret_keys = match read_secret_keys(creds) {
        Some(keys) => keys,
        None => return BTreeMap::new(),
    };
    user_env
        .iter()
        .filter(|(key, _)| !secret_keys.contains(key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Read the mounted secret-key manifest ([`CredsLayout::secret_keys`], a JSON array
/// of env-var names). `Some(set)` on success (an empty set when the manifest is
/// ABSENT — no secrets); `None` when the manifest exists but cannot be read or
/// parsed, signalling the caller to fail closed.
fn read_secret_keys(creds: &CredsLayout) -> Option<BTreeSet<String>> {
    let path = creds.secret_keys();
    match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<Vec<String>>(&raw) {
            Ok(keys) => Some(keys.into_iter().collect()),
            Err(error) => {
                tracing::error!(error = %error, "run-substrate: corrupt secret-keys manifest; exposing NO variables to codex (fail closed)");
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(BTreeSet::new()),
        Err(error) => {
            tracing::error!(error = %error, "run-substrate: unreadable secret-keys manifest; exposing NO variables to codex (fail closed)");
            None
        }
    }
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
///
/// `child_env` is the FINAL supervise env (after the named-environment install), and
/// `user_env` the profile's non-secret variables. Both feed the rendered
/// `[shell_environment_policy]` so codex's shell commands see the profile's tools
/// and variables — secrets are deliberately not among them.
fn render_codex(
    env: &SubstrateEnv,
    child_env: &[(String, String)],
    user_env: &BTreeMap<String, String>,
) -> Result<(), String> {
    let home = Path::new(&env.codex_home);
    create_dir_idempotent(home)?;
    // Best-effort tighten to 0700 (the config references the env_key, not the key
    // itself, but the home dir is still session-private).
    if let Err(error) =
        std::fs::set_permissions(home, std::fs::Permissions::from_mode(CODEX_HOME_MODE))
    {
        tracing::warn!(error = %error, "run-substrate: could not chmod CODEX_HOME to 0700");
    }
    // Pull the env-bin-prepended PATH + FKST_ENV_BIN from the FINAL child_env
    // (last write wins → search from the back); empty string when unset (no profile
    // tools), which the renderer treats as "nothing to expose".
    let lookup = |key: &str| -> &str {
        child_env
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    };
    let shell_env = CodexShellEnv {
        path: lookup(PATH_ENV),
        tool_dir: lookup(crate::install::TOOL_DIR_ENV),
        variables: user_env,
    };
    let toml = render_codex_config(
        &env.llm_model,
        &env.llm_base_url,
        &env.llm_wire_api,
        LLM_ENV_KEY,
        Some(&shell_env),
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

/// Insert-or-replace `key` in the ordered env vec.
fn upsert_env(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(slot) = env.iter_mut().find(|(k, _)| k == key) {
        slot.1 = value.to_string();
    } else {
        env.push((key.to_string(), value.to_string()));
    }
}

/// Insert `key=value` only when the child env does not already carry `key`, so a
/// value the operator pinned on the session (via its env) wins over a computed
/// default. Unlike [`upsert_env`], an existing binding is left untouched.
fn default_env(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if !env.iter().any(|(k, _)| k == key) {
        env.push((key.to_string(), value.to_string()));
    }
}

/// The default repo-local workflow-catalog root for a session: the target repo's
/// `.fkst/packages/` under the cloned project root. workflow-writer writes new
/// `fkst.workflow.v1` files here and github-devloop-workflow loads + runs them.
fn workflow_catalog_root_default(project_root: &Path) -> PathBuf {
    project_root.join(WORKFLOW_CATALOG_SUBDIR)
}

/// Run the named-environment profile's ordered install commands inside the pod.
/// A writable env-bin dir is created under the runtime root, placed at the FRONT
/// of PATH, and exposed as `FKST_ENV_BIN` so an install step can drop a tool
/// binary there that the workflow later calls by bare name. Each command runs via
/// `sh -c` with the already-assembled child env, so the profile's own
/// variables/secrets are visible to the install step. A missing install file is a
/// no-op; a non-zero exit fails the launch closed (a broken install must not start
/// a half-provisioned engine). The commands were bounded + validated at PUT time.
async fn run_env_install_commands(
    creds: &CredsLayout,
    child_env: &mut Vec<(String, String)>,
    runtime_root: &Path,
) -> Result<(), String> {
    let path = creds.install_commands();
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("read install commands {}: {error}", path.display())),
    };
    let commands: Vec<String> =
        serde_json::from_str(&raw).map_err(|error| format!("parse install commands: {error}"))?;
    if commands.is_empty() {
        return Ok(());
    }

    let env_bin = runtime_root.join(ENV_BIN_SUBDIR);
    create_dir_idempotent(&env_bin)?;
    prepend_path(child_env, &env_bin);
    upsert_env(
        child_env,
        crate::install::TOOL_DIR_ENV,
        &env_bin.to_string_lossy(),
    );

    tracing::info!(
        count = commands.len(),
        env_bin = %env_bin.display(),
        "run-substrate: running environment install commands"
    );
    for (idx, command) in commands.iter().enumerate() {
        // bash (not sh): matches the PUT-time validator's `bash -c` contract so an
        // install command validated once behaves identically in the session.
        let output = Command::new("bash")
            .arg("-c")
            .arg(command)
            .envs(child_env.iter().map(|(k, v)| (k.clone(), v.clone())))
            // Run in the on-PATH env-bin so a relative install (`-o ffmpeg`) also
            // lands where the workflow can call it, matching $FKST_ENV_BIN.
            .current_dir(&env_bin)
            .output()
            .await
            .map_err(|error| format!("spawn install command #{idx}: {error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let tail = stderr
                .get(stderr.len().saturating_sub(2048)..)
                .unwrap_or(&stderr);
            return Err(format!(
                "environment install command #{idx} failed (exit {:?}): {tail}",
                output.status.code()
            ));
        }
        tracing::info!(index = idx, "run-substrate: environment install command ok");
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_catalog_root_defaults_to_repo_local_fkst_packages() {
        let project = Path::new("/var/lib/fkst/runtime/project");
        assert_eq!(
            workflow_catalog_root_default(project),
            PathBuf::from("/var/lib/fkst/runtime/project/.fkst/packages"),
        );
    }

    #[test]
    fn default_env_inserts_when_absent() {
        let mut env = vec![("PATH".to_string(), "/usr/bin".to_string())];
        default_env(&mut env, WORKFLOW_CATALOG_ROOT_ENV, "/p/.fkst/packages");
        let got = env.iter().find(|(k, _)| k == WORKFLOW_CATALOG_ROOT_ENV);
        assert_eq!(got.map(|(_, v)| v.as_str()), Some("/p/.fkst/packages"));
    }

    #[test]
    fn default_env_leaves_an_operator_pinned_value_untouched() {
        // A value already on the session env (operator/`### Environment`) must win.
        let mut env = vec![(
            WORKFLOW_CATALOG_ROOT_ENV.to_string(),
            "/custom/location".to_string(),
        )];
        default_env(&mut env, WORKFLOW_CATALOG_ROOT_ENV, "/p/.fkst/packages");
        let vals: Vec<&str> = env
            .iter()
            .filter(|(k, _)| k == WORKFLOW_CATALOG_ROOT_ENV)
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            vals,
            vec!["/custom/location"],
            "operator value must win, no duplicate"
        );
    }

    #[test]
    fn non_secret_variables_exposes_all_when_no_secret_manifest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let creds = CredsLayout::new(dir.path());
        let user_env = BTreeMap::from([
            ("BRAND_COLOR".to_string(), "0x0B5FFF".to_string()),
            ("FONT_FILE".to_string(), "OpenSans.ttf".to_string()),
        ]);
        // No manifest on disk ⇒ the profile declared no secrets ⇒ expose every var.
        assert_eq!(non_secret_variables(&creds, &user_env), user_env);
    }

    #[test]
    fn non_secret_variables_excludes_the_manifest_secret_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let creds = CredsLayout::new(dir.path());
        std::fs::write(creds.secret_keys(), r#"["WATERMARK_TOKEN"]"#).expect("write manifest");
        let user_env = BTreeMap::from([
            ("BRAND_COLOR".to_string(), "0x0B5FFF".to_string()),
            ("WATERMARK_TOKEN".to_string(), "shh".to_string()),
        ]);
        let got = non_secret_variables(&creds, &user_env);
        assert!(got.contains_key("BRAND_COLOR"));
        assert!(
            !got.contains_key("WATERMARK_TOKEN"),
            "a secret-valued key must never reach the codex config"
        );
    }

    #[test]
    fn non_secret_variables_fails_closed_on_a_corrupt_manifest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let creds = CredsLayout::new(dir.path());
        std::fs::write(creds.secret_keys(), "not json at all").expect("write manifest");
        let user_env = BTreeMap::from([("BRAND_COLOR".to_string(), "0x0B5FFF".to_string())]);
        // An unparseable manifest exposes NOTHING rather than risk leaking a secret.
        assert!(non_secret_variables(&creds, &user_env).is_empty());
    }
}
