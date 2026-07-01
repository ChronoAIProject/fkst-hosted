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

    pub(super) fn pod_session_max_lifetime_secs() -> u64 {
        // Hard ceiling on a single session pod's wall-clock lifetime. 0 = unbounded
        // (a session runs until it goes idle or its trigger closes).
        0
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
}

impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            substrate_trigger_label: defaults::substrate_trigger_label(),
            github_bot_login: None,
            reconcile_interval_secs: defaults::reconcile_interval_secs(),
            pod_full_resync_interval_secs: defaults::pod_full_resync_interval_secs(),
            session_idle_grace_secs: defaults::session_idle_grace_secs(),
            pod_min_lifetime_secs: defaults::pod_min_lifetime_secs(),
            pod_termination_grace_secs: defaults::pod_termination_grace_secs(),
            pod_token_refresh_secs: defaults::pod_token_refresh_secs(),
            pod_session_max_lifetime_secs: defaults::pod_session_max_lifetime_secs(),
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_apply_when_nothing_is_set() {
        let config = ReconcileConfig::from_vars(&vars(&[])).expect("defaults should deserialize");
        assert_eq!(config.substrate_trigger_label, "fkst-substrate-trigger");
        assert_eq!(config.github_bot_login, None);
        assert_eq!(config.reconcile_interval_secs, 30);
        assert_eq!(config.pod_full_resync_interval_secs, 600);
        assert_eq!(config.session_idle_grace_secs, 300);
        assert_eq!(config.pod_min_lifetime_secs, 120);
        assert_eq!(config.pod_termination_grace_secs, 60);
        assert_eq!(config.pod_token_refresh_secs, 2700);
        assert_eq!(config.pod_session_max_lifetime_secs, 0);
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = ReconcileConfig::from_vars(&vars(&[])).expect("defaults");
        let from_default = ReconcileConfig::default();
        assert_eq!(
            from_default.substrate_trigger_label,
            from_env.substrate_trigger_label
        );
        assert_eq!(from_default.github_bot_login, from_env.github_bot_login);
        assert_eq!(
            from_default.reconcile_interval_secs,
            from_env.reconcile_interval_secs
        );
        assert_eq!(
            from_default.pod_full_resync_interval_secs,
            from_env.pod_full_resync_interval_secs
        );
        assert_eq!(
            from_default.session_idle_grace_secs,
            from_env.session_idle_grace_secs
        );
        assert_eq!(
            from_default.pod_min_lifetime_secs,
            from_env.pod_min_lifetime_secs
        );
        assert_eq!(
            from_default.pod_termination_grace_secs,
            from_env.pod_termination_grace_secs
        );
        assert_eq!(
            from_default.pod_token_refresh_secs,
            from_env.pod_token_refresh_secs
        );
        assert_eq!(
            from_default.pod_session_max_lifetime_secs,
            from_env.pod_session_max_lifetime_secs
        );
    }

    #[test]
    fn every_knob_is_overridable() {
        let config = ReconcileConfig::from_vars(&vars(&[
            ("FKST_SUBSTRATE_TRIGGER_LABEL", "fkst-run"),
            ("FKST_GITHUB_BOT_LOGIN", "fkst-bot"),
            ("FKST_RECONCILE_INTERVAL_SECS", "15"),
            ("FKST_POD_FULL_RESYNC_INTERVAL_SECS", "1200"),
            ("FKST_SESSION_IDLE_GRACE_SECS", "600"),
            ("FKST_POD_MIN_LIFETIME_SECS", "240"),
            ("FKST_POD_TERMINATION_GRACE_SECS", "90"),
            ("FKST_POD_TOKEN_REFRESH_SECS", "1800"),
            ("FKST_POD_SESSION_MAX_LIFETIME_SECS", "86400"),
        ]))
        .expect("overrides should deserialize");
        assert_eq!(config.substrate_trigger_label, "fkst-run");
        assert_eq!(config.github_bot_login.as_deref(), Some("fkst-bot"));
        assert_eq!(config.reconcile_interval_secs, 15);
        assert_eq!(config.pod_full_resync_interval_secs, 1200);
        assert_eq!(config.session_idle_grace_secs, 600);
        assert_eq!(config.pod_min_lifetime_secs, 240);
        assert_eq!(config.pod_termination_grace_secs, 90);
        assert_eq!(config.pod_token_refresh_secs, 1800);
        assert_eq!(config.pod_session_max_lifetime_secs, 86400);
    }

    #[test]
    fn blank_bot_login_is_coerced_to_none() {
        let config =
            ReconcileConfig::from_vars(&vars(&[("FKST_GITHUB_BOT_LOGIN", "   ")])).expect("blank");
        assert_eq!(config.github_bot_login, None);
    }

    #[test]
    fn zero_cadence_bounds_are_config_errors_naming_the_var() {
        for var in [
            "FKST_RECONCILE_INTERVAL_SECS",
            "FKST_POD_FULL_RESYNC_INTERVAL_SECS",
            "FKST_SESSION_IDLE_GRACE_SECS",
            "FKST_POD_TOKEN_REFRESH_SECS",
        ] {
            let err = ReconcileConfig::from_vars(&vars(&[(var, "0")])).expect_err("zero must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }

    #[test]
    fn token_refresh_at_or_over_the_ttl_is_a_config_error() {
        // At the TTL boundary: a refresh that fires exactly at expiry is too late.
        let at = ReconcileConfig::from_vars(&vars(&[("FKST_POD_TOKEN_REFRESH_SECS", "3600")]))
            .expect_err("at TTL must fail");
        assert!(at.to_string().contains("FKST_POD_TOKEN_REFRESH_SECS"));
        // Over the TTL.
        let over = ReconcileConfig::from_vars(&vars(&[("FKST_POD_TOKEN_REFRESH_SECS", "7200")]))
            .expect_err("over TTL must fail");
        assert!(over.to_string().contains("FKST_POD_TOKEN_REFRESH_SECS"));
    }

    #[test]
    fn zero_valued_shield_and_lifetime_knobs_are_allowed() {
        // A zero min-lifetime / termination-grace / max-lifetime are all valid
        // (no shield / no drain / unbounded) — they must NOT fail closed.
        let config = ReconcileConfig::from_vars(&vars(&[
            ("FKST_POD_MIN_LIFETIME_SECS", "0"),
            ("FKST_POD_TERMINATION_GRACE_SECS", "0"),
            ("FKST_POD_SESSION_MAX_LIFETIME_SECS", "0"),
        ]))
        .expect("zero shields are valid");
        assert_eq!(config.pod_min_lifetime_secs, 0);
        assert_eq!(config.pod_termination_grace_secs, 0);
        assert_eq!(config.pod_session_max_lifetime_secs, 0);
    }

    #[test]
    fn non_numeric_interval_is_a_config_error() {
        let err = ReconcileConfig::from_vars(&vars(&[("FKST_RECONCILE_INTERVAL_SECS", "soon")]))
            .expect_err("non-numeric must fail");
        assert!(matches!(err, AppError::Config(_)));
    }
}
