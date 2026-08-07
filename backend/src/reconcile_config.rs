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
///
/// The bound is only half the invariant (#3410). It guarantees "the next rotation
/// tick lands before the token TTL elapses"; it says nothing about how much life the
/// token a session was HANDED actually had. Both halves are required:
///
/// 1. every session-bound token is minted at full TTL — delivery and rotation both go
///    through `GithubAppTokens::token_with_expiry_for_repo_forced`, never the shared
///    cache, which may serve a token with only its 5-minute expiry buffer left; and
/// 2. `pod_token_refresh_secs < INSTALLATION_TOKEN_TTL_SECS`, enforced below.
///
/// Together they give `delivered_ttl > pod_token_refresh_secs`: a session's token
/// always outlives the wait for the sweep that replaces it.
const INSTALLATION_TOKEN_TTL_SECS: u64 = 3600;

/// The default fkst-manifest an auto-seeded trigger references (epic #594 I9): the
/// composed default-workflows manifest bundling workflow-dev + security + writer.
/// A manifest reference is spelled with the same `owner/repo@ref:path` grammar as a
/// package reference; the reconciler's manifest expander fetches + expands it into a
/// package list, and the session's wake labels auto-discover from those packages'
/// `[github].work_labels`. Overridable via `FKST_DEFAULT_MANIFEST`; a blank override
/// disables the manifest-driven seed and falls back to the legacy packages+label body.
const DEFAULT_MANIFEST_REF: &str =
    "ChronoAIProject/fkst-hosted@packages:manifests/default-workflows.json";

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

    pub(super) fn startup_resync_retry_initial_secs() -> u64 {
        5
    }

    pub(super) fn startup_resync_retry_max_secs() -> u64 {
        60
    }

    pub(super) fn startup_resync_retry_jitter_percent() -> u64 {
        20
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
        vec!["ChronoAIProject/fkst-hosted@packages:packages/github-devloop-workflow".to_string()]
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

    pub(super) fn sandbox_inventory_max_source_items() -> usize {
        // Defensive ceiling on ONE live-inventory read (issue #5674). Sized far
        // above any realistic fleet: it exists to stop a runaway/foreign backend
        // from making the control plane allocate without bound, not to shape a
        // normal response. Exceeding it is a loud error, never a silent clip.
        5000
    }

    pub(super) fn sandbox_inventory_max_warnings() -> usize {
        // The companion ceiling on ONE snapshot's warnings (issue #5674). It is
        // deliberately far BELOW the item ceiling: warnings are diagnostic, and
        // a fleet-wide metadata regression should cost bounded memory. Overflow
        // is announced by a truncation marker, never silent — and a deployment
        // that raised the item ceiling can raise this one to match.
        crate::session_backend::inventory::DEFAULT_MAX_WARNINGS
    }

    pub(super) fn cron_min_interval_secs() -> u64 {
        // The tightest cadence a scheduled workflow may declare. Every firing
        // creates a run issue and boots a session pod, so an unbounded cadence is a
        // cost hazard rather than a feature. 15 minutes.
        900
    }

    pub(super) fn cron_max_runtime_secs() -> u64 {
        // The watchdog budget: how long one run may hold its schedule before the
        // control plane releases it. This is the ONLY thing that stops a hung run
        // pinning a schedule forever, so it is deliberately generous but finite.
        3600
    }

    pub(super) fn cron_max_jobs_per_creator() -> u32 {
        // Blast-radius guard on one creator's scheduled workflows per repository.
        20
    }

    pub(super) fn cron_history_pages() -> u32 {
        // How many 100-comment pages of a schedule issue's history the pass reads,
        // newest first. Two pages cover ~100 run records — far more than the cursor
        // and in-flight detection need, while bounding the cost of a long-lived
        // definition to two requests.
        2
    }

    pub(super) fn creds_watch_secs() -> u64 {
        // How often the credentials watch probes every live session for a missing
        // credential bundle. Deliberately FAST (unlike the health scrape) because
        // it races the session pod's own bounded creds wait: a pod replaced under a
        // surviving runtime aborts if delivery is not triggered inside that window.
        // The probe is a cheap backend/execd file check and costs no GitHub call,
        // so a tight cadence is affordable (issue #5927).
        30
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
    /// Optional provider namespace appended to every session work label.
    #[serde(default)]
    work_label_namespace: Option<String>,
    #[serde(default = "defaults::reconcile_interval_secs")]
    reconcile_interval_secs: u64,
    #[serde(default = "defaults::pod_full_resync_interval_secs")]
    pod_full_resync_interval_secs: u64,
    #[serde(default = "defaults::startup_resync_retry_initial_secs")]
    startup_resync_retry_initial_secs: u64,
    #[serde(default = "defaults::startup_resync_retry_max_secs")]
    startup_resync_retry_max_secs: u64,
    #[serde(default = "defaults::startup_resync_retry_jitter_percent")]
    startup_resync_retry_jitter_percent: u64,
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
    #[serde(default = "defaults::sandbox_inventory_max_source_items")]
    sandbox_inventory_max_source_items: usize,
    #[serde(default = "defaults::sandbox_inventory_max_warnings")]
    sandbox_inventory_max_warnings: usize,
    #[serde(default = "defaults::health_scrape_secs")]
    health_scrape_secs: u64,
    #[serde(default = "defaults::creds_watch_secs")]
    creds_watch_secs: u64,
    #[serde(default = "defaults::cron_min_interval_secs")]
    cron_min_interval_secs: u64,
    #[serde(default = "defaults::cron_max_runtime_secs")]
    cron_max_runtime_secs: u64,
    #[serde(default = "defaults::cron_max_jobs_per_creator")]
    cron_max_jobs_per_creator: u32,
    #[serde(default = "defaults::cron_history_pages")]
    cron_history_pages: u32,
    /// Auto-create a seed trigger issue when the App is installed on a repo.
    /// Default TRUE (I9): a fresh install auto-writes a manifest-driven trigger.
    #[serde(default = "defaults::seed_trigger_issue_on_install")]
    seed_trigger_issue_on_install: bool,
    /// Whitespace-separated `owner/repo@ref:path` package refs the seeded trigger
    /// issue loads. Unset → the github-devloop-workflow default.
    #[serde(default)]
    seed_packages: Option<String>,
    /// The default fkst-manifest `owner/repo@ref:path` ref a seeded trigger loads.
    /// Unset → the default-workflows manifest; a blank override → the legacy body.
    #[serde(default = "defaults::default_manifest")]
    default_manifest: Option<String>,
    /// Whitespace-separated `owner/repo@ref:path` refs EVERY session receives on top
    /// of what its trigger declares. Unset/blank -> none (feature off).
    #[serde(default)]
    mandatory_packages: Option<String>,
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
    /// Deployment/provider namespace appended to every logical GitHub work label.
    /// Env: `FKST_WORK_LABEL_NAMESPACE`. Unset/blank preserves logical labels.
    pub work_label_namespace: Option<String>,
    /// Reconcile-loop cadence, seconds. Env: `FKST_RECONCILE_INTERVAL_SECS`.
    /// Default 30; must be >= 1.
    pub reconcile_interval_secs: u64,
    /// Full pod-resync cadence, seconds. Env: `FKST_POD_FULL_RESYNC_INTERVAL_SECS`.
    /// Default 600; must be >= 1.
    pub pod_full_resync_interval_secs: u64,
    /// Initial retry delay after an incomplete full resync. Env:
    /// `FKST_STARTUP_RESYNC_RETRY_INITIAL_SECS`. Default 5; must be >= 1.
    pub startup_resync_retry_initial_secs: u64,
    /// Maximum retry delay after an incomplete full resync. Env:
    /// `FKST_STARTUP_RESYNC_RETRY_MAX_SECS`. Default 60; must be at least the
    /// configured initial delay.
    pub startup_resync_retry_max_secs: u64,
    /// Symmetric jitter around each retry delay, as a percentage. Env:
    /// `FKST_STARTUP_RESYNC_RETRY_JITTER_PERCENT`. Default 20; range 0..=100.
    pub startup_resync_retry_jitter_percent: u64,
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
    /// Defensive ceiling on the runtimes ONE live-inventory read may return. Env:
    /// `FKST_SANDBOX_INVENTORY_MAX_SOURCE_ITEMS`. Default 5000; must be >= 1.
    /// Exceeding it fails the read explicitly
    /// ([`crate::session_backend::BackendError::InventoryTooLarge`]) rather than
    /// returning a shortened list that would read as a complete fleet.
    pub sandbox_inventory_max_source_items: usize,
    /// Defensive ceiling on the warnings ONE live-inventory read may carry. Env:
    /// `FKST_SANDBOX_INVENTORY_MAX_WARNINGS`. Default 256; must be >= 1.
    /// Exceeding it appends one
    /// [`crate::session_backend::inventory::InventoryWarningCode::WarningsTruncated`]
    /// marker rather than failing the read: a snapshot whose diagnostics are
    /// clipped is still a correct fleet listing, which is the opposite trade-off
    /// from the item ceiling above.
    pub sandbox_inventory_max_warnings: usize,
    /// Session-health scrape cadence, seconds. Env: `FKST_HEALTH_SCRAPE_SECS`.
    /// Default 150; must be >= 1. How often the package-agnostic health scrape
    /// reads each live pod's status + recent framework logs to flag/clear a
    /// degraded session on its trigger issue.
    pub health_scrape_secs: u64,
    /// Credentials watch cadence, seconds. Env: `FKST_CREDS_WATCH_SECS`. Default
    /// 30; must be >= 1. How often every live session is probed for a missing
    /// credential bundle so a REPLACED pod gets re-delivery triggered before its
    /// own bounded wait expires (issue #5927). The probe costs no GitHub call.
    pub creds_watch_secs: u64,
    /// The tightest cadence a `fkst-scheduled-workflow` issue may declare, in
    /// seconds. Env: `FKST_CRON_MIN_INTERVAL_SECS`. Default 900; must be >= 60.
    ///
    /// Every firing creates a run issue and boots a session pod, so a one-minute
    /// cadence is a cost hazard, not a feature. A schedule tighter than this is
    /// latched invalid with a message naming the limit rather than silently slowed.
    pub cron_min_interval_secs: u64,
    /// The per-run watchdog budget, in seconds. Env: `FKST_CRON_MAX_RUNTIME_SECS`.
    /// Default 3600; must be >= 60.
    ///
    /// The ONLY thing that stops a hung run pinning its schedule forever: on expiry
    /// the control plane records a `timeout` run and drops the running latch, so the
    /// next slot can proceed.
    pub cron_max_runtime_secs: u64,
    /// Maximum accepted scheduled workflows per creator per repository. Env:
    /// `FKST_CRON_MAX_JOBS_PER_CREATOR`. Default 20; must be >= 1. Beyond it the
    /// lowest-numbered definitions win and the rest are latched invalid naming the
    /// cap — a blast-radius guard, not a licence tier.
    pub cron_max_jobs_per_creator: u32,
    /// How many 100-comment pages of a schedule issue's history the pass reads,
    /// newest first. Env: `FKST_CRON_HISTORY_PAGES`. Default 2; must be >= 1.
    pub cron_history_pages: u32,
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
    /// Package refs EVERY session gets, PREPENDED to whatever its trigger declares.
    /// Env: `FKST_MANDATORY_PACKAGES` (whitespace-separated `owner/repo@ref:path`).
    ///
    /// Deliberately NOT defaulted in code: empty means "feature off, behave exactly
    /// as before", and the deployed value lives in the ConfigMap so the baseline can
    /// change without shipping a binary.
    ///
    /// This is what makes session isolation structural rather than author-dependent.
    /// The rule ships in `libraries/devloop`, so a trigger declaring no devloop tree
    /// would otherwise run with no isolation at all (#5773).
    pub mandatory_packages: Vec<crate::goals::trigger_parse::PackageRef>,
}

impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            substrate_trigger_label: defaults::substrate_trigger_label(),
            github_bot_login: None,
            work_label_namespace: None,
            seed_trigger_issue_on_install: defaults::seed_trigger_issue_on_install(),
            seed_packages: defaults::seed_packages(),
            default_manifest: defaults::default_manifest(),
            mandatory_packages: Vec::new(),
            reconcile_interval_secs: defaults::reconcile_interval_secs(),
            pod_full_resync_interval_secs: defaults::pod_full_resync_interval_secs(),
            startup_resync_retry_initial_secs: defaults::startup_resync_retry_initial_secs(),
            startup_resync_retry_max_secs: defaults::startup_resync_retry_max_secs(),
            startup_resync_retry_jitter_percent: defaults::startup_resync_retry_jitter_percent(),
            session_idle_grace_secs: defaults::session_idle_grace_secs(),
            pod_min_lifetime_secs: defaults::pod_min_lifetime_secs(),
            pod_termination_grace_secs: defaults::pod_termination_grace_secs(),
            pod_token_refresh_secs: defaults::pod_token_refresh_secs(),
            pod_session_max_lifetime_secs: defaults::pod_session_max_lifetime_secs(),
            sandbox_inventory_max_source_items: defaults::sandbox_inventory_max_source_items(),
            sandbox_inventory_max_warnings: defaults::sandbox_inventory_max_warnings(),
            health_scrape_secs: defaults::health_scrape_secs(),
            creds_watch_secs: defaults::creds_watch_secs(),
            cron_min_interval_secs: defaults::cron_min_interval_secs(),
            cron_max_runtime_secs: defaults::cron_max_runtime_secs(),
            cron_max_jobs_per_creator: defaults::cron_max_jobs_per_creator(),
            cron_history_pages: defaults::cron_history_pages(),
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
        if env.startup_resync_retry_initial_secs == 0 {
            return Err(AppError::Config(
                "FKST_STARTUP_RESYNC_RETRY_INITIAL_SECS must be at least 1".to_string(),
            ));
        }
        if env.startup_resync_retry_max_secs < env.startup_resync_retry_initial_secs {
            return Err(AppError::Config(
                "FKST_STARTUP_RESYNC_RETRY_MAX_SECS must be greater than or equal to \
                 FKST_STARTUP_RESYNC_RETRY_INITIAL_SECS"
                    .to_string(),
            ));
        }
        if env.startup_resync_retry_jitter_percent > 100 {
            return Err(AppError::Config(
                "FKST_STARTUP_RESYNC_RETRY_JITTER_PERCENT must be between 0 and 100".to_string(),
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
        if env.creds_watch_secs == 0 {
            return Err(AppError::Config(
                "FKST_CREDS_WATCH_SECS must be at least 1".to_string(),
            ));
        }
        // A cadence bound below a minute would let a schedule fire faster than the
        // reconcile sweep observes it, so the clock could never keep up with its own
        // definition — and every firing costs a run issue plus a pod boot.
        if env.cron_min_interval_secs < 60 {
            return Err(AppError::Config(
                "FKST_CRON_MIN_INTERVAL_SECS must be at least 60".to_string(),
            ));
        }
        // A tiny watchdog budget would expire every real run mid-flight, which reads
        // to an operator as "scheduled workflows randomly time out".
        if env.cron_max_runtime_secs < 60 {
            return Err(AppError::Config(
                "FKST_CRON_MAX_RUNTIME_SECS must be at least 60".to_string(),
            ));
        }
        // Zero would reject every schedule in the deployment, disabling the feature
        // through what looks like a tuning knob.
        if env.cron_max_jobs_per_creator == 0 {
            return Err(AppError::Config(
                "FKST_CRON_MAX_JOBS_PER_CREATOR must be at least 1".to_string(),
            ));
        }
        // Zero pages would make every schedule recover an empty history and re-fire
        // its anchor slot on every sweep.
        if env.cron_history_pages == 0 {
            return Err(AppError::Config(
                "FKST_CRON_HISTORY_PAGES must be at least 1".to_string(),
            ));
        }
        // A zero ceiling would make every live-inventory read fail as oversize,
        // silently disabling the operations sandbox view — reject it outright
        // rather than let an empty ConfigMap value take the feature down.
        if env.sandbox_inventory_max_source_items == 0 {
            return Err(AppError::Config(
                "FKST_SANDBOX_INVENTORY_MAX_SOURCE_ITEMS must be at least 1".to_string(),
            ));
        }
        // A zero warning ceiling leaves no room even for the truncation marker,
        // so a snapshot would silently claim it had nothing to report.
        if env.sandbox_inventory_max_warnings == 0 {
            return Err(AppError::Config(
                "FKST_SANDBOX_INVENTORY_MAX_WARNINGS must be at least 1".to_string(),
            ));
        }
        // The token refresh must fire strictly inside the 1-hour installation-token
        // TTL, or a long-lived pod would carry an expired credential. Reject both a
        // zero cadence and one at/over the TTL. This bound is load-bearing only
        // BECAUSE session tokens are delivered at full TTL (#3410) — see the
        // INSTALLATION_TOKEN_TTL_SECS docs for the full two-part invariant.
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

        let work_label_namespace = env
            .work_label_namespace
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(namespace) = work_label_namespace.as_deref() {
            crate::reconcile::work_labels::validate_work_label_namespace(namespace).map_err(
                |error| {
                    AppError::Config(format!(
                        "{} {error}",
                        crate::reconcile::work_labels::WORK_LABEL_NAMESPACE_ENV
                    ))
                },
            )?;
        }

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

        // Mandatory packages: validated HERE and failed closed. A silently-dropped
        // mandatory ref would remove the isolation guarantee this knob exists to
        // provide, which is precisely the failure that must not be quiet. Unset or
        // blank yields an empty list -- feature off, effective sets unchanged.
        let mut mandatory_packages: Vec<crate::goals::trigger_parse::PackageRef> = Vec::new();
        for token in env
            .mandatory_packages
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
        {
            // Parsed with the TRIGGER parser, so a mandatory ref is held to exactly
            // the same shape rules as one an author writes -- and so the stored value
            // is the same PackageRef type the effective-set resolver consumes.
            let parsed = crate::goals::trigger_parse::parse_package_ref(token).map_err(|e| {
                AppError::Config(format!(
                    "FKST_MANDATORY_PACKAGES token {token:?} is invalid: {e}; expected \
                     whitespace-separated owner/repo@ref:path refs"
                ))
            })?;
            mandatory_packages.push(parsed);
        }

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
            work_label_namespace,
            reconcile_interval_secs: env.reconcile_interval_secs,
            pod_full_resync_interval_secs: env.pod_full_resync_interval_secs,
            startup_resync_retry_initial_secs: env.startup_resync_retry_initial_secs,
            startup_resync_retry_max_secs: env.startup_resync_retry_max_secs,
            startup_resync_retry_jitter_percent: env.startup_resync_retry_jitter_percent,
            session_idle_grace_secs: env.session_idle_grace_secs,
            pod_min_lifetime_secs: env.pod_min_lifetime_secs,
            pod_termination_grace_secs: env.pod_termination_grace_secs,
            pod_token_refresh_secs: env.pod_token_refresh_secs,
            pod_session_max_lifetime_secs: env.pod_session_max_lifetime_secs,
            sandbox_inventory_max_source_items: env.sandbox_inventory_max_source_items,
            sandbox_inventory_max_warnings: env.sandbox_inventory_max_warnings,
            health_scrape_secs: env.health_scrape_secs,
            creds_watch_secs: env.creds_watch_secs,
            cron_min_interval_secs: env.cron_min_interval_secs,
            cron_max_runtime_secs: env.cron_max_runtime_secs,
            cron_max_jobs_per_creator: env.cron_max_jobs_per_creator,
            cron_history_pages: env.cron_history_pages,
            seed_trigger_issue_on_install: env.seed_trigger_issue_on_install,
            seed_packages,
            mandatory_packages,
            default_manifest,
        })
    }
}

#[cfg(test)]
#[path = "reconcile_config_tests.rs"]
mod tests;
