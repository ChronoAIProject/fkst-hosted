//! Pure planners for the Model B `run-substrate` in-pod entrypoint (issue #359 §5).
//!
//! Split from the effectful [`super::driver`] so the launch DECISIONS — reading the
//! injected `FKST_*` env into a [`SubstrateEnv`], grouping the fetched package refs
//! into a multi-workspace [`ClonePlan`], building the exact `supervise` argv, and
//! folding the supervise child env (git-cred wiring + LLM key + userenv with
//! reserved-key filtering) — are unit-testable with ZERO cluster / network /
//! process side effects. The driver is the thin I/O shell around these.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::delivery_grants::{
    DeliveryGrant, DeliveryGrantPolicy, ResolvedDeliveryGrant, SESSION_DELIVERY_GRANTS_ENV,
};
use crate::goals::package_ref::{parse_package_ref, PackageRef};
use crate::reserved_env::{is_reserved_env_key, GIT_TRACE_SILENCING_ENV, LLM_ENV_KEY};

use super::creds_helper::GitConfigEntry;

// --- injected env keys (mirror `k8s::session_launcher`'s writer-side consts so
// the pod reader can never disagree with the launcher on a name) ---------------
const GITHUB_REPO_ENV: &str = "FKST_GITHUB_REPO";
const PACKAGE_ROOTS_ENV: &str = "FKST_SESSION_PACKAGE_ROOTS";
const WORK_LABEL_ENV: &str = "FKST_SESSION_WORK_LABEL";
const BOT_LOGIN_ENV: &str = "FKST_GITHUB_BOT_LOGIN";
const LLM_MODEL_ENV: &str = "FKST_LLM_MODEL";
const LLM_BASE_URL_ENV: &str = "FKST_LLM_BASE_URL";
const LLM_WIRE_API_ENV: &str = "FKST_LLM_WIRE_API";
const LLM_REASONING_EFFORT_ENV: &str = "FKST_LLM_REASONING_EFFORT";
const DURABLE_ROOT_ENV: &str = "FKST_DURABLE_ROOT";
const RUNTIME_ROOT_ENV: &str = "FKST_RUNTIME_ROOT";
const CREDS_DIR_ENV: &str = "FKST_SESSION_CREDS_DIR";
const CODEX_HOME_ENV: &str = "CODEX_HOME";
const DEVLOOP_INTEGRATION_BRANCH_ENV: &str = "FKST_DEVLOOP_INTEGRATION_BRANCH";
const DEVLOOP_UPSTREAM_BRANCH_ENV: &str = "FKST_DEVLOOP_UPSTREAM_BRANCH";

/// `git config` count key + the LLM env key the child reads its API key from.
const GIT_CONFIG_COUNT_ENV: &str = "GIT_CONFIG_COUNT";

// --- LLM defaults (mirror `config::defaults` + `runner`'s defaults so the pod and
// the HTTP config never diverge on the operator-pinned provider) ---------------
const DEFAULT_LLM_MODEL: &str = "gpt-5.6-sol";
const DEFAULT_LLM_BASE_URL: &str = "https://llm.aelf.dev/v1";
/// Defaults to `responses`: codex 0.139+ rejects `wire_api = "chat"` at config
/// load, and the LLM backend (verified on llm.aelf.dev) serves the `/responses`
/// API. Overridden per-deploy via `FKST_LLM_WIRE_API`.
const DEFAULT_LLM_WIRE_API: &str = "responses";
/// The codex `model_reasoning_effort` (issue #3393): the platform default is the
/// deepest tier; the launcher injects the effective (config-or-trigger) value.
const DEFAULT_LLM_REASONING_EFFORT: &str = "max";

/// The `supervise` subcommand token.
const SUPERVISE_SUBCOMMAND: &str = "supervise";
/// Legacy primary platform checkout root under `FKST_RUNTIME_ROOT`.
const PRIMARY_PLATFORM_SUBDIR: &str = "platform";
/// Additional package workspaces are cloned under this root, keyed by repo/ref hash.
const ADDITIONAL_PLATFORM_SUBDIR: &str = "platforms";

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
    pub llm_reasoning_effort: String,
    /// Durable delivery-state root (fixed; the observe socket derives from it).
    pub durable_root: String,
    /// Per-restart scratch/runtime root.
    pub runtime_root: String,
    /// Mounted creds Secret volume base dir.
    pub creds_dir: String,
    /// Codex config/home dir.
    pub codex_home: String,
    /// Target branch to clone. `None` preserves the legacy default-branch clone
    /// for callers outside the hosted session launcher.
    pub target_branch: Option<String>,
    /// Upstream (source) branch the devloop rolls completed target work into.
    /// Read so the clone can ALSO fetch this ref: the shallow `--single-branch`
    /// clone otherwise leaves `refs/remotes/origin/<source>` absent, and the
    /// devloop's rollup/sync scans resolve ranges against it on every branch
    /// tick. `None` for callers outside the hosted launcher.
    pub source_branch: Option<String>,
    /// Exact operator grants for this lifecycle repository. Empty preserves the
    /// historical single-repository worker contract.
    pub delivery_grants: Vec<DeliveryGrant>,
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
    let optional = |key: &str| match get(key) {
        Some(value) if !value.trim().is_empty() => Some(value),
        _ => None,
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

    let delivery_grants = DeliveryGrantPolicy::parse_session_value(
        get(SESSION_DELIVERY_GRANTS_ENV).as_deref(),
        &repo,
    )?;

    Ok(SubstrateEnv {
        repo,
        package_refs,
        work_label: required(WORK_LABEL_ENV)?,
        bot_login: required(BOT_LOGIN_ENV)?,
        llm_model: with_default(LLM_MODEL_ENV, DEFAULT_LLM_MODEL),
        llm_base_url: with_default(LLM_BASE_URL_ENV, DEFAULT_LLM_BASE_URL),
        llm_wire_api: with_default(LLM_WIRE_API_ENV, DEFAULT_LLM_WIRE_API),
        llm_reasoning_effort: with_default(LLM_REASONING_EFFORT_ENV, DEFAULT_LLM_REASONING_EFFORT),
        durable_root: required(DURABLE_ROOT_ENV)?,
        runtime_root: required(RUNTIME_ROOT_ENV)?,
        creds_dir: required(CREDS_DIR_ENV)?,
        codex_home: required(CODEX_HOME_ENV)?,
        target_branch: optional(DEVLOOP_INTEGRATION_BRANCH_ENV),
        source_branch: optional(DEVLOOP_UPSTREAM_BRANCH_ENV),
        delivery_grants,
    })
}

/// One additional checkout the driver must clone. Grants that exactly match the
/// lifecycle or platform checkout are resolved to those existing roots instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryClone {
    pub repository: String,
    pub branch: String,
    pub root: PathBuf,
}

/// Pure cross-repository checkout plan consumed by the effectful driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryCheckoutPlan {
    pub resolved_grants: Vec<ResolvedDeliveryGrant>,
    pub clones: Vec<DeliveryClone>,
}

/// Resolve every grant to an exact checkout. Repository comparison follows
/// GitHub's case-insensitive identity; branches remain case-sensitive. Distinct
/// repo/branch pairs get one deterministic runtime root and one clone operation.
#[allow(clippy::too_many_arguments)]
pub fn plan_delivery_checkouts(
    grants: &[DeliveryGrant],
    lifecycle_repo: &str,
    lifecycle_branch: Option<&str>,
    project_root: &Path,
    platform_repo: &str,
    platform_branch: &str,
    platform_root: &Path,
    runtime_root: &Path,
) -> DeliveryCheckoutPlan {
    let mut resolved_grants = Vec::with_capacity(grants.len());
    let mut clones = Vec::new();
    let mut seen = BTreeSet::new();

    for grant in grants {
        let root = if checkout_matches(
            &grant.implementation_repo,
            &grant.implementation_branch,
            platform_repo,
            Some(platform_branch),
        ) {
            platform_root.to_path_buf()
        } else if checkout_matches(
            &grant.implementation_repo,
            &grant.implementation_branch,
            lifecycle_repo,
            lifecycle_branch,
        ) {
            project_root.to_path_buf()
        } else {
            let identity = format!(
                "{}\0{}",
                grant.implementation_repo.to_ascii_lowercase(),
                grant.implementation_branch
            );
            let digest = Sha256::digest(identity.as_bytes());
            let suffix: String = digest
                .iter()
                .take(8)
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let root = runtime_root.join("delivery").join(suffix);
            if seen.insert(identity) {
                clones.push(DeliveryClone {
                    repository: grant.implementation_repo.clone(),
                    branch: grant.implementation_branch.clone(),
                    root: root.clone(),
                });
            }
            root
        };

        resolved_grants.push(ResolvedDeliveryGrant {
            lifecycle_repo: grant.lifecycle_repo.clone(),
            lifecycle_issue: grant.lifecycle_issue,
            implementation_repo: grant.implementation_repo.clone(),
            implementation_branch: grant.implementation_branch.clone(),
            implementation_root: root.to_string_lossy().into_owned(),
        });
    }

    DeliveryCheckoutPlan {
        resolved_grants,
        clones,
    }
}

fn checkout_matches(
    repository: &str,
    branch: &str,
    checkout_repo: &str,
    checkout_branch: Option<&str>,
) -> bool {
    repository.eq_ignore_ascii_case(checkout_repo) && checkout_branch == Some(branch)
}

/// One package workspace repo `(owner, repo, git_ref)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRepo {
    pub owner: String,
    pub repo: String,
    pub git_ref: String,
}

/// One package workspace checkout to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceClone {
    pub repo: WorkspaceRepo,
    pub root: PathBuf,
}

/// The resolved package clone plan: one checkout per distinct workspace plus every
/// concrete `--package-root` path in the original effective-package order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClonePlan {
    pub workspaces: Vec<WorkspaceClone>,
    pub package_roots: Vec<PathBuf>,
}

/// Group package refs into package workspace checkouts.
///
/// Each distinct `(owner, repo, git_ref)` is cloned once. The first workspace keeps
/// the historical `<runtime>/platform` root for backward-compatible paths and
/// delivery-grant reuse; later workspaces live under `<runtime>/platforms/<hash>`.
/// The returned `package_roots` preserve the input order, so explicit package refs
/// still win any ordering-sensitive behavior before manifest-expanded refs.
pub fn plan_clones(refs: &[PackageRef], runtime_root: &Path) -> Result<ClonePlan, String> {
    if refs.is_empty() {
        return Err("no package refs to plan".to_string());
    }

    type WorkspaceKey = (String, String, String);
    let mut seen: BTreeMap<WorkspaceKey, usize> = BTreeMap::new();
    let mut workspaces: Vec<WorkspaceClone> = Vec::new();
    let mut package_roots = Vec::with_capacity(refs.len());

    for candidate in refs {
        let key = (
            candidate.owner.to_ascii_lowercase(),
            candidate.repo.to_ascii_lowercase(),
            candidate.git_ref.clone(),
        );
        let index = match seen.get(&key) {
            Some(index) => *index,
            None => {
                let index = workspaces.len();
                let root = workspace_root(runtime_root, index, &key);
                workspaces.push(WorkspaceClone {
                    repo: WorkspaceRepo {
                        owner: candidate.owner.clone(),
                        repo: candidate.repo.clone(),
                        git_ref: candidate.git_ref.clone(),
                    },
                    root,
                });
                seen.insert(key, index);
                index
            }
        };
        package_roots.push(
            workspaces[index]
                .root
                .join(candidate.path.trim_start_matches('/')),
        );
    }
    Ok(ClonePlan {
        workspaces,
        package_roots,
    })
}

fn workspace_root(runtime_root: &Path, index: usize, key: &(String, String, String)) -> PathBuf {
    if index == 0 {
        return runtime_root.join(PRIMARY_PLATFORM_SUBDIR);
    }
    let identity = format!("{}\0{}\0{}", key.0, key.1, key.2);
    let digest = Sha256::digest(identity.as_bytes());
    let suffix: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    runtime_root.join(ADDITIONAL_PLATFORM_SUBDIR).join(suffix)
}

/// Build the exact `fkst-framework supervise` argv. The real CLI (verified
/// against `crates/fkst-framework/src/main.rs`) accepts ONLY `--project-root`,
/// repeatable `--package-root`, and `--framework-bin` — there is no
/// `--platform-root`/`--platform-packages`/`--durable-root`/`--runtime-root`. The
/// durable + runtime roots are read from the `FKST_DURABLE_ROOT`/
/// `FKST_RUNTIME_ROOT` env instead (set on the child by [`substrate_child_env`]).
///
/// Each `package_root` is already resolved under its owning workspace checkout.
pub fn build_supervise_args(
    project_root: &str,
    package_roots: &[PathBuf],
    framework_bin: &str,
) -> Vec<String> {
    let mut args = vec![
        SUPERVISE_SUBCOMMAND.to_string(),
        "--project-root".to_string(),
        project_root.to_string(),
    ];
    for path in package_roots {
        args.push("--package-root".to_string());
        args.push(path.to_string_lossy().into_owned());
    }
    args.push("--framework-bin".to_string());
    args.push(framework_bin.to_string());
    args
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
///
/// The same last-writer-wins step force-sets the [`GIT_TRACE_SILENCING_ENV`] toggles
/// to `"0"` (Layer 0 log hardening) so git/GCM never trace `Authorization:`/`password`
/// lines into the streamed pod log, and a user `env_profile` cannot re-enable them.
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

    // Layer 0: silence git/GCM tracing so a rotating token never trace-leaks into the
    // streamed pod log. Written last so a user `env_profile` value can never win.
    for (key, value) in GIT_TRACE_SILENCING_ENV {
        env.insert((*key).to_string(), (*value).to_string());
    }

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
