//! Typed configuration for the Model B reconciler (issue #359 §4).
//!
//! A single envy pass over the bare `FKST_*` prefix, mirroring the defaults +
//! fail-closed style of [`crate::config`] and [`crate::env_config`]. These knobs
//! bound the reconcile cadence, the pod-liveness clocks the pure planner
//! ([`crate::reconcile::desired::plan_repo`]) reads, and the per-pod token/lifetime
//! bounds the effectful reconciler (PR5b) will enforce.
//!
//! ADDITIVE: this is the config SURFACE only. Nothing reads these values yet, and
//! — deliberately — none of the new keys introduce a fail-closed that would break
//! an already-running Model-A deploy: every bound has a sensible default and only
//! a genuinely nonsensical override (a zero cadence, or a token refresh that never
//! fires before the 1-hour installation-token expiry) is rejected. The
//! dispatch-on / bot-login requirement is enforced at the PR6 flip, NOT here.
//!
//! Prefix note: this pass reads the bare `FKST_` prefix, so it deliberately shares
//! the namespace with [`crate::config`]'s webhook/`FKST_POD_*` passes. envy drops
//! every field it does not recognize, so each struct sees only its own keys and
//! the passes never collide (a `FKST_POD_MIN_LIFETIME_SECS` lands here as
//! `pod_min_lifetime_secs`; a `FKST_POD_DISPATCH` is ignored here and read by the
//! pod-dispatch pass instead).

use serde::Deserialize;

use crate::error::AppError;

/// Prefix shared by every reconciler configuration variable. Bare `FKST_` so the
/// keys read naturally (`FKST_RECONCILE_INTERVAL_SECS`, `FKST_POD_MIN_LIFETIME_SECS`).
const RECONCILE_ENV_PREFIX: &str = "FKST_";

/// An installation token lives one hour; a refresh that never fires inside that
/// window would let a long-lived session pod run on an expired credential. The
/// refresh cadence must sit strictly below it.
const INSTALLATION_TOKEN_TTL_SECS: u64 = 3600;

/// The default fkst-manifest an auto-seeded trigger references (epic #594 I9): the
/// composed default-workflows manifest bundling workflow-dev + security + writer.
/// A manifest reference is spelled with the same `owner/repo@ref:path` grammar as a
/// package reference; the reconciler's manifest expander fetches + expands it into a
/// package list, and the session's wake labels auto-discover from those packages'
/// `[github].work_labels`. Overridable via `FKST_DEFAULT_MANIFEST`; a blank override
/// disables the manifest-driven seed and falls back to the legacy packages+label body.
const DEFAULT_MANIFEST_REF: &str =
    "ChronoAIProject/fkst-packages@fkst-hosted:manifests/default-workflows.json";

/// Default values, shared by serde defaults and [`ReconcileConfig::default`].
mod defaults {
    pub(super) fn substrate_trigger_label() -> String {
        // The Issue-Form label a Model B trigger issue carries. Model A keeps its
        // own `FKST_WEBHOOK_TRIGGER_LABEL` until the PR6 flip; this is separate.
        "fkst-substrate-trigger".to_string()
    }

    pub(super) fn reconcile_interval_secs() -> u64 {
        // How often the reconcile loop wakes to diff desired vs live state.
        30
    }

    pub(super) fn pod_full_resync_interval_secs() -> u64 {
        // How often a full pod list (not just the incremental diff) is resynced.
        600
    }

    pub(super) fn session_idle_grace_secs() -> u64 {
        // How long a live pod may sit non-pending before it is idle-killed.
        300
    }

    pub(super) fn pod_min_lifetime_secs() -> u64 {
        // A newly-spawned pod is shielded from idle-kill for this long so a slow
        // startup is not mistaken for idleness.
        120
    }

    pub(super) fn pod_termination_grace_secs() -> u64 {
        // The pod `terminationGracePeriodSeconds` the reconciler will honour when
        // it deletes a pod (drain window before SIGKILL).
        60
    }

    pub(super) fn pod_token_refresh_secs() -> u64 {
        // How often a long-lived pod's installation token is refreshed. Must sit
        // strictly below the 1-hour token TTL. 45 minutes.
        2700
    }

    pub(super) fn seed_packages() -> Vec<String> {
        // The default package(s) an auto-seeded trigger issue loads when
        // FKST_SEED_PACKAGES is unset: the composed github-devloop-workflow root.
        vec!["ChronoAIProject/fkst-packages@dev:packages/github-devloop-workflow".to_string()]
    }

    pub(super) fn seed_trigger_issue_on_install() -> bool {
        // ON by default (epic #594 I9): a successful App install auto-writes ONE
        // manifest-driven trigger issue into every newly-installed repo (subject to
        // the idempotency skip). Set FKST_SEED_TRIGGER_ISSUE_ON_INSTALL=false to
        // disable the auto-seed entirely.
        true
    }

    pub(super) fn default_manifest() -> Option<String> {
        // The default fkst-manifest a seeded trigger references (I9). Unset →
        // Some(the default-workflows ref); a blank override → None (legacy body).
        Some(super::DEFAULT_MANIFEST_REF.to_string())
    }

    pub(super) fn pod_session_max_lifetime_secs() -> u64 {
        // Hard ceiling on a single session pod's wall-clock lifetime. 0 = unbounded
        // (a session runs until it goes idle or its trigger closes).
        0
    }

    pub(super) fn health_scrape_secs() -> u64 {
        // How often the package-agnostic session-health scrape reads each live
        // pod's status + recent framework logs to flag/clear a degraded session.
        // Deliberately slower than the reconcile sweep: it only relays a signal
        // (no lifecycle effect), and the recurring-warn threshold needs a few log
        // cycles to accrue, so a ~2.5-minute cadence keeps the GitHub read/comment
        // budget low while still catching a green-but-idle pod within minutes.
        150
    }
}

/// Bare `FKST_*`-prefixed variables (Model B reconciler).
#[derive(Debug, Deserialize)]
struct ReconcileVars {
    #[serde(default = "defaults::substrate_trigger_label")]
    substrate_trigger_label: String,
    /// The bot's GitHub login. `None` (the default) until the PR6 flip wires the
    /// dispatch-on requirement; a blank override is coerced to `None`.
    #[serde(default)]
    github_bot_login: Option<String>,
    #[serde(default = "defaults::reconcile_interval_secs")]
    reconcile_interval_secs: u64,
    #[serde(default = "defaults::pod_full_resync_interval_secs")]
    pod_full_resync_interval_secs: u64,
    #[serde(default = "defaults::session_idle_grace_secs")]
    session_idle_grace_secs: u64,
    #[serde(default = "defaults::pod_min_lifetime_secs")]
    pod_min_lifetime_secs: u64,
    #[serde(default = "defaults::pod_termination_grace_secs")]
    pod_termination_grace_secs: u64,
    #[serde(default = "defaults::pod_token_refresh_secs")]
    pod_token_refresh_secs: u64,
    #[serde(default = "defaults::pod_session_max_lifetime_secs")]
    pod_session_max_lifetime_secs: u64,
    #[serde(default = "defaults::health_scrape_secs")]
    health_scrape_secs: u64,
    /// Auto-create a seed trigger issue when the App is installed on a repo.
    /// Default TRUE (I9): a fresh install auto-writes a manifest-driven trigger.
    #[serde(default = "defaults::seed_trigger_issue_on_install")]
    seed_trigger_issue_on_install: bool,
    /// Operator opt-in for the R3 work-issue authority gate. Default false =
    /// today's permissive behavior (any author may raise work).
    #[serde(default)]
    enforce_work_issue_authz: bool,
    /// Whitespace-separated `owner/repo@ref:path` package refs the seeded trigger
    /// issue loads. Unset → the github-devloop-workflow default.
    #[serde(default)]
    seed_packages: Option<String>,
    /// The default fkst-manifest `owner/repo@ref:path` ref a seeded trigger loads.
    /// Unset → the default-workflows manifest; a blank override → the legacy body.
    #[serde(default = "defaults::default_manifest")]
    default_manifest: Option<String>,
}

/// Model B reconciler configuration (issue #359 §4). Config surface only — no
/// behaviour reads these yet (PR5b wires the loop; PR6 flips Model B on).
#[derive(Clone, Debug)]
pub struct ReconcileConfig {
    /// The Issue-Form label a Model B trigger issue carries. Env:
    /// `FKST_SUBSTRATE_TRIGGER_LABEL`. Default `fkst-substrate-trigger`.
    pub substrate_trigger_label: String,
    /// The bot's GitHub login. Env: `FKST_GITHUB_BOT_LOGIN`. Default `None`
    /// (blank coerced to `None`); the dispatch-on requirement is a PR6 concern.
    pub github_bot_login: Option<String>,
    /// Reconcile-loop cadence, seconds. Env: `FKST_RECONCILE_INTERVAL_SECS`.
    /// Default 30; must be >= 1.
    pub reconcile_interval_secs: u64,
    /// Full pod-resync cadence, seconds. Env: `FKST_POD_FULL_RESYNC_INTERVAL_SECS`.
    /// Default 600; must be >= 1.
    pub pod_full_resync_interval_secs: u64,
    /// Idle grace before a non-pending live pod is killed, seconds. Env:
    /// `FKST_SESSION_IDLE_GRACE_SECS`. Default 300; must be >= 1.
    pub session_idle_grace_secs: u64,
    /// Minimum pod lifetime shielding a fresh pod from idle-kill, seconds. Env:
    /// `FKST_POD_MIN_LIFETIME_SECS`. Default 120; 0 = no shield.
    pub pod_min_lifetime_secs: u64,
    /// Pod termination grace (drain window before SIGKILL), seconds. Env:
    /// `FKST_POD_TERMINATION_GRACE_SECS`. Default 60.
    pub pod_termination_grace_secs: u64,
    /// Installation-token refresh cadence for a long-lived pod, seconds. Env:
    /// `FKST_POD_TOKEN_REFRESH_SECS`. Default 2700; must be >= 1 and < 3600 (the
    /// token TTL), or a pod would run on an expired credential.
    pub pod_token_refresh_secs: u64,
    /// Hard ceiling on one session pod's wall-clock lifetime, seconds. Env:
    /// `FKST_POD_SESSION_MAX_LIFETIME_SECS`. Default 0 = unbounded.
    pub pod_session_max_lifetime_secs: u64,
    /// Session-health scrape cadence, seconds. Env: `FKST_HEALTH_SCRAPE_SECS`.
    /// Default 150; must be >= 1. How often the package-agnostic health scrape
    /// reads each live pod's status + recent framework logs to flag/clear a
    /// degraded session on its trigger issue.
    pub health_scrape_secs: u64,
    /// When true, auto-create ONE seed trigger issue the first time the App is
    /// installed on a repo with no open trigger issue. Env:
    /// `FKST_SEED_TRIGGER_ISSUE_ON_INSTALL`. **Default TRUE (epic #594 I9)** — a
    /// behaviour change: a successful App install now writes a trigger issue into
    /// every newly-installed repo. When [`Self::default_manifest`] is set (the
    /// default), that trigger is manifest-driven (a `### Manifest` reference, no
    /// `### Packages`/`### Work Label` — the manifest supplies the packages and the
    /// wake labels auto-discover). Set the env to `false` to disable the auto-seed
    /// entirely.
    pub seed_trigger_issue_on_install: bool,
    /// Operator opt-in for the R3 work-issue AUTHORITY gate (epic #572). Env:
    /// `FKST_ENFORCE_WORK_ISSUE_AUTHZ`. Default false = today's permissive behavior:
    /// any GitHub user who opens a work-label issue has it picked up. When true, the
    /// reconciler fetches the repo's admin/org-owner set and only a session's
    /// **author ∪ Session Collaborators ∪ repo admins/org owners** may raise work for
    /// it — anyone else is visibly rejected (comment + `fkst-unauthorized` latch) and
    /// never picked up. Enforcement FAILS OPEN on any admin-lookup error (a lookup
    /// blip must never lock out work). The flag being off is byte-identical to
    /// pre-R3 behavior (no admin fetch, no author filtering, no reject).
    pub enforce_work_issue_authz: bool,
    /// The `### Packages` refs an auto-seeded trigger issue lists (one per line).
    /// Env: `FKST_SEED_PACKAGES` (whitespace-separated). Default: the
    /// github-devloop-workflow root. Never empty (a blank env value falls back to
    /// the default). Used ONLY for the legacy (no-manifest) seed body — when
    /// [`Self::default_manifest`] is set, the manifest supplies the packages and
    /// this list is not rendered.
    pub seed_packages: Vec<String>,
    /// The default fkst-manifest reference (`owner/repo@ref:path`) a seeded trigger
    /// loads under `### Manifest`. Env: `FKST_DEFAULT_MANIFEST`. Default:
    /// `Some(the default-workflows manifest)` (epic #594 I9). A blank env value →
    /// `None`, which makes the seeder fall back to the legacy packages+label body.
    /// When `Some`, the seed body carries ONLY `### Manifest` (no `### Packages`,
    /// no `### Work Label`): the manifest supplies the package set and the session's
    /// wake labels auto-discover from those packages' `[github].work_labels`.
    pub default_manifest: Option<String>,
}

impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            substrate_trigger_label: defaults::substrate_trigger_label(),
            github_bot_login: None,
            seed_trigger_issue_on_install: defaults::seed_trigger_issue_on_install(),
            enforce_work_issue_authz: false,
            seed_packages: defaults::seed_packages(),
            default_manifest: defaults::default_manifest(),
            reconcile_interval_secs: defaults::reconcile_interval_secs(),
            pod_full_resync_interval_secs: defaults::pod_full_resync_interval_secs(),
            session_idle_grace_secs: defaults::session_idle_grace_secs(),
            pod_min_lifetime_secs: defaults::pod_min_lifetime_secs(),
            pod_termination_grace_secs: defaults::pod_termination_grace_secs(),
            pod_token_refresh_secs: defaults::pod_token_refresh_secs(),
            pod_session_max_lifetime_secs: defaults::pod_session_max_lifetime_secs(),
            health_scrape_secs: defaults::health_scrape_secs(),
        }
    }
}

impl ReconcileConfig {
    /// Deserialize a `ReconcileConfig` from environment-style key/value pairs.
    ///
    /// Testable seam: unit tests feed explicit pairs instead of mutating the
    /// process environment. Shares the caller's already-collected `vars` snapshot
    /// (see [`crate::config::Config::from_vars`]).
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<ReconcileConfig, AppError> {
        let env: ReconcileVars = envy::prefixed(RECONCILE_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;

        // Fail closed only on the genuinely nonsensical bounds, each naming its
        // variable. A zero cadence would spin the reconcile loop or the resync
        // with no delay; a zero idle grace would kill every non-pending pod on the
        // first sweep. The other duration knobs (min lifetime, termination grace,
        // max lifetime) are legitimately zero-valued (no shield / no drain /
        // unbounded), and `u64` already rejects negatives at parse time.
        if env.reconcile_interval_secs == 0 {
            return Err(AppError::Config(
                "FKST_RECONCILE_INTERVAL_SECS must be at least 1".to_string(),
            ));
        }
        if env.pod_full_resync_interval_secs == 0 {
            return Err(AppError::Config(
                "FKST_POD_FULL_RESYNC_INTERVAL_SECS must be at least 1".to_string(),
            ));
        }
        if env.session_idle_grace_secs == 0 {
            return Err(AppError::Config(
                "FKST_SESSION_IDLE_GRACE_SECS must be at least 1".to_string(),
            ));
        }
        if env.health_scrape_secs == 0 {
            return Err(AppError::Config(
                "FKST_HEALTH_SCRAPE_SECS must be at least 1".to_string(),
            ));
        }
        // The token refresh must fire strictly inside the 1-hour installation-token
        // TTL, or a long-lived pod would carry an expired credential. Reject both a
        // zero cadence and one at/over the TTL.
        if env.pod_token_refresh_secs == 0 {
            return Err(AppError::Config(
                "FKST_POD_TOKEN_REFRESH_SECS must be at least 1".to_string(),
            ));
        }
        if env.pod_token_refresh_secs >= INSTALLATION_TOKEN_TTL_SECS {
            return Err(AppError::Config(format!(
                "FKST_POD_TOKEN_REFRESH_SECS must be less than {INSTALLATION_TOKEN_TTL_SECS} \
                 (the installation-token TTL)"
            )));
        }

        // A blank bot login is meaningless; treat it as unset so a stray empty
        // ConfigMap value does not masquerade as a real login.
        let github_bot_login = env
            .github_bot_login
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // Whitespace-separated package refs; a blank/all-whitespace value falls
        // back to the default so a stray empty ConfigMap value cannot seed an
        // issue with an empty `### Packages` section (which the parser rejects).
        let seed_packages = env
            .seed_packages
            .as_deref()
            .map(|raw| {
                raw.split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(defaults::seed_packages);

        // Default manifest ref: an absent env value keeps the built-in default (the
        // serde default already put it here); a blank/all-whitespace override is
        // coerced to `None` so a stray empty ConfigMap value cleanly DISABLES the
        // manifest-driven seed (the seeder then renders the legacy packages+label
        // body) rather than emitting an unparseable empty `### Manifest`.
        let default_manifest = env
            .default_manifest
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        Ok(ReconcileConfig {
            substrate_trigger_label: env.substrate_trigger_label,
            github_bot_login,
            reconcile_interval_secs: env.reconcile_interval_secs,
            pod_full_resync_interval_secs: env.pod_full_resync_interval_secs,
            session_idle_grace_secs: env.session_idle_grace_secs,
            pod_min_lifetime_secs: env.pod_min_lifetime_secs,
            pod_termination_grace_secs: env.pod_termination_grace_secs,
            pod_token_refresh_secs: env.pod_token_refresh_secs,
            pod_session_max_lifetime_secs: env.pod_session_max_lifetime_secs,
            health_scrape_secs: env.health_scrape_secs,
            seed_trigger_issue_on_install: env.seed_trigger_issue_on_install,
            enforce_work_issue_authz: env.enforce_work_issue_authz,
            seed_packages,
            default_manifest,
        })
    }
}

#[cfg(test)]
#[path = "reconcile_config_tests.rs"]
mod tests;
