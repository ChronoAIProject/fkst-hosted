//! Pure planners for the Model B `run-substrate` in-pod entrypoint (issue #359 §5).
//!
//! Split from the effectful [`super::driver`] so the launch DECISIONS — reading the
//! injected `FKST_*` env into a [`SubstrateEnv`], grouping the fetched package refs
//! into a single-workspace [`ClonePlan`], building the exact `supervise` argv, and
//! folding the supervise child env (git-cred wiring + LLM key + userenv with
//! reserved-key filtering) — are unit-testable with ZERO cluster / network /
//! process side effects. The driver is the thin I/O shell around these.

use std::collections::BTreeMap;

use crate::engine::GitConfigEntry;
use crate::goals::package_ref::{package_name_from_path, parse_package_ref, PackageRef};
use crate::reserved_env::{is_reserved_env_key, LLM_ENV_KEY};

// --- injected env keys (mirror `k8s::session_launcher`'s writer-side consts so
// the pod reader can never disagree with the launcher on a name) ---------------
const GITHUB_REPO_ENV: &str = "FKST_GITHUB_REPO";
const PACKAGE_ROOTS_ENV: &str = "FKST_SESSION_PACKAGE_ROOTS";
const WORK_LABEL_ENV: &str = "FKST_SESSION_WORK_LABEL";
const BOT_LOGIN_ENV: &str = "FKST_GITHUB_BOT_LOGIN";
const LLM_MODEL_ENV: &str = "FKST_LLM_MODEL";
const LLM_BASE_URL_ENV: &str = "FKST_LLM_BASE_URL";
const LLM_WIRE_API_ENV: &str = "FKST_LLM_WIRE_API";
const DURABLE_ROOT_ENV: &str = "FKST_DURABLE_ROOT";
const RUNTIME_ROOT_ENV: &str = "FKST_RUNTIME_ROOT";
const CREDS_DIR_ENV: &str = "FKST_SESSION_CREDS_DIR";
const CODEX_HOME_ENV: &str = "CODEX_HOME";

/// `git config` count key + the LLM env key the child reads its API key from.
const GIT_CONFIG_COUNT_ENV: &str = "GIT_CONFIG_COUNT";

// --- LLM defaults (mirror `config::defaults` + `runner`'s defaults so the pod and
// the HTTP config never diverge on the operator-pinned provider) ---------------
const DEFAULT_LLM_MODEL: &str = "gpt-5-codex";
const DEFAULT_LLM_BASE_URL: &str = "https://nyx.chrono-ai.fun/api/v1/proxy/s/chrono-llm";
/// MUST default to `chat`: chrono-llm serves only `/chat/completions`; `responses`
/// 502s (a verified bug). Never default to `responses`.
const DEFAULT_LLM_WIRE_API: &str = "chat";

/// The `supervise` subcommand token.
const SUPERVISE_SUBCOMMAND: &str = "supervise";

/// The non-secret launch inputs the `run-substrate` entrypoint reads from the
/// pod-injected `FKST_*` env. Non-secret: a `{:?}` of it can never leak a token
/// (the creds live in the mounted Secret, read separately by the driver).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubstrateEnv {
    /// `owner/name` of the target repo the session works.
    pub repo: String,
    /// The fully-qualified package refs to fetch (≥1, parsed from
    /// `FKST_SESSION_PACKAGE_ROOTS`).
    pub package_refs: Vec<PackageRef>,
    /// The session's work label (control-plane spawn/idle gate + poll prefix).
    pub work_label: String,
    /// The App bot login (git author/committer + github-proxy identity).
    pub bot_login: String,
    /// Operator-pinned LLM provider (feeds the codex `config.toml` render).
    pub llm_model: String,
    pub llm_base_url: String,
    pub llm_wire_api: String,
    /// Durable delivery-state root (fixed; the observe socket derives from it).
    pub durable_root: String,
    /// Per-restart scratch/runtime root.
    pub runtime_root: String,
    /// Mounted creds Secret volume base dir.
    pub creds_dir: String,
    /// Codex config/home dir.
    pub codex_home: String,
}

/// Read the injected env into a [`SubstrateEnv`] from the process environment.
pub fn read_substrate_env() -> Result<SubstrateEnv, String> {
    read_substrate_env_from(|key| std::env::var(key).ok())
}

/// Testable core of [`read_substrate_env`]: reads via `get` (injected in tests as a
/// map lookup) so it needs no `std::env` mutation. A required var that is unset or
/// blank → `Err`; the `FKST_LLM_*` trio defaults to the operator-pinned values when
/// absent (mirroring the HTTP config), everything else is required.
pub(crate) fn read_substrate_env_from(
    get: impl Fn(&str) -> Option<String>,
) -> Result<SubstrateEnv, String> {
    let required = |key: &str| -> Result<String, String> {
        match get(key) {
            Some(value) if !value.trim().is_empty() => Ok(value),
            _ => Err(format!("required env var {key} is unset or empty")),
        }
    };
    let with_default = |key: &str, default: &str| -> String {
        match get(key) {
            Some(value) if !value.trim().is_empty() => value,
            _ => default.to_string(),
        }
    };

    let repo = required(GITHUB_REPO_ENV)?;
    // `owner/name` shape guard — the launcher always sets `<owner>/<name>`; a
    // malformed value would mis-clone the target repo, so fail loudly here.
    if repo.split('/').count() != 2 || repo.split('/').any(|segment| segment.is_empty()) {
        return Err(format!(
            "{GITHUB_REPO_ENV} {repo:?} must be exactly `owner/name`"
        ));
    }

    let roots_raw = required(PACKAGE_ROOTS_ENV)?;
    let mut package_refs = Vec::new();
    for token in roots_raw.split_whitespace() {
        package_refs.push(parse_package_ref(token)?);
    }
    if package_refs.is_empty() {
        return Err(format!("{PACKAGE_ROOTS_ENV} lists no package refs"));
    }

    Ok(SubstrateEnv {
        repo,
        package_refs,
        work_label: required(WORK_LABEL_ENV)?,
        bot_login: required(BOT_LOGIN_ENV)?,
        llm_model: with_default(LLM_MODEL_ENV, DEFAULT_LLM_MODEL),
        llm_base_url: with_default(LLM_BASE_URL_ENV, DEFAULT_LLM_BASE_URL),
        llm_wire_api: with_default(LLM_WIRE_API_ENV, DEFAULT_LLM_WIRE_API),
        durable_root: required(DURABLE_ROOT_ENV)?,
        runtime_root: required(RUNTIME_ROOT_ENV)?,
        creds_dir: required(CREDS_DIR_ENV)?,
        codex_home: required(CODEX_HOME_ENV)?,
    })
}

/// The single workspace repo `(owner, repo, git_ref)` all v1 package refs must
/// share (cloned once into `<runtime>/platform`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRepo {
    pub owner: String,
    pub repo: String,
    pub git_ref: String,
}

/// The resolved clone plan: the one workspace repo to fetch + the platform-package
/// names to activate under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClonePlan {
    pub platform_repo: WorkspaceRepo,
    pub platform_packages: Vec<String>,
}

/// Group the package refs into a single-workspace [`ClonePlan`].
///
/// **v1 constraint:** every ref must share ONE `(owner, repo, git_ref)` — a lone
/// workspace repo whose clone brings the sibling `libraries/*` + `fkst.lock` a
/// workspace package needs (issue #359 §5.3). More than one distinct
/// `(owner,repo,git_ref)` → `Err` (multi-workspace fetch is a documented
/// follow-up). Each platform-package name is the LAST path segment of a ref's
/// `path` (`packages/github-devloop` → `github-devloop`), preserving ref order.
pub fn plan_clones(refs: &[PackageRef]) -> Result<ClonePlan, String> {
    let first = refs
        .first()
        .ok_or_else(|| "no package refs to plan".to_string())?;
    let platform_repo = WorkspaceRepo {
        owner: first.owner.clone(),
        repo: first.repo.clone(),
        git_ref: first.git_ref.clone(),
    };

    let mut platform_packages = Vec::with_capacity(refs.len());
    for candidate in refs {
        if candidate.owner != platform_repo.owner
            || candidate.repo != platform_repo.repo
            || candidate.git_ref != platform_repo.git_ref
        {
            return Err(format!(
                "all packages must currently come from one workspace repo \
                 ({}/{}@{}), but {}/{}@{} differs; multi-workspace fetch is a follow-up",
                platform_repo.owner,
                platform_repo.repo,
                platform_repo.git_ref,
                candidate.owner,
                candidate.repo,
                candidate.git_ref,
            ));
        }
        platform_packages.push(package_name_from_path(&candidate.path).to_string());
    }
    Ok(ClonePlan {
        platform_repo,
        platform_packages,
    })
}

/// Build the exact `fkst-framework supervise` argv (issue #359 §5.3):
/// `["supervise", "--project-root", <p>, "--platform-root", <p>,
///   "--platform-packages", <names>, "--durable-root", <d>, "--runtime-root", <r>,
///   "--framework-bin", <bin>]`.
///
/// `--platform-packages` is ONE argument: the names joined by a single space
/// (matches the fkst-packages host-run contract). // verify live: confirm the
/// space-joined single-arg form against the running `fkst-framework supervise`
/// before the first cluster run.
pub fn build_supervise_args(
    project_root: &str,
    platform_root: &str,
    platform_packages: &[String],
    durable_root: &str,
    runtime_root: &str,
    framework_bin: &str,
) -> Vec<String> {
    vec![
        SUPERVISE_SUBCOMMAND.to_string(),
        "--project-root".to_string(),
        project_root.to_string(),
        "--platform-root".to_string(),
        platform_root.to_string(),
        "--platform-packages".to_string(),
        platform_packages.join(" "),
        "--durable-root".to_string(),
        durable_root.to_string(),
        "--runtime-root".to_string(),
        runtime_root.to_string(),
        "--framework-bin".to_string(),
        framework_bin.to_string(),
    ]
}

/// Assemble the env for the supervise child: the process env, PLUS the git-cred
/// `GIT_CONFIG_*` wiring, the `LLM_API_KEY`, `CODEX_HOME`, and
/// `FKST_DURABLE_ROOT`/`FKST_RUNTIME_ROOT`, with the issue author's `user_env`
/// folded in under `is_reserved_env_key` filtering.
///
/// Layering is load-bearing: `user_env` is folded FIRST (dropping any reserved /
/// `FKST_*` / git-cred / allow-listed host key), then the platform vars are written
/// LAST so they always win. `LLM_API_KEY` is NOT in the reserved table, so this
/// last-writer-wins step is what guarantees a `userenv.LLM_API_KEY` can never
/// shadow the real key.
pub fn substrate_child_env(
    base: Vec<(String, String)>,
    user_env: &BTreeMap<String, String>,
    llm_api_key: &str,
    git_config_entries: &[GitConfigEntry],
    codex_home: &str,
    durable_root: &str,
    runtime_root: &str,
) -> Vec<(String, String)> {
    // A BTreeMap keeps the result deterministic (stable ordering aids tests) and
    // de-duplicates keys as we layer.
    let mut env: BTreeMap<String, String> = base.into_iter().collect();

    for (key, value) in user_env {
        if is_reserved_env_key(key) {
            continue;
        }
        env.insert(key.clone(), value.clone());
    }

    env.insert(
        GIT_CONFIG_COUNT_ENV.to_string(),
        git_config_entries.len().to_string(),
    );
    for (i, entry) in git_config_entries.iter().enumerate() {
        env.insert(format!("GIT_CONFIG_KEY_{i}"), entry.key.clone());
        env.insert(format!("GIT_CONFIG_VALUE_{i}"), entry.value.clone());
    }

    env.insert(LLM_ENV_KEY.to_string(), llm_api_key.to_string());
    env.insert(CODEX_HOME_ENV.to_string(), codex_home.to_string());
    env.insert(DURABLE_ROOT_ENV.to_string(), durable_root.to_string());
    env.insert(RUNTIME_ROOT_ENV.to_string(), runtime_root.to_string());

    env.into_iter().collect()
}

/// Map the supervised child's exit into this process's exit code (returned as a
/// `u8` so it is trivially unit-testable): a clean exit (0) stays 0; any non-zero
/// code is preserved (truncated to a byte, but a byte-0 non-zero code is forced to
/// 1 so a failure never masquerades as success); a signal-kill (`None`) is 1.
///
/// A SIGTERM-forwarded graceful stop still surfaces the child's OWN disposition —
/// the reconciler kills only when idle, so a clean supervise drain returns 0.
pub(crate) fn exit_status_to_code(code: Option<i32>) -> u8 {
    match code {
        Some(0) => 0,
        Some(nonzero) => {
            let byte = (nonzero & 0xff) as u8;
            if byte == 0 {
                1
            } else {
                byte
            }
        }
        None => 1,
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
