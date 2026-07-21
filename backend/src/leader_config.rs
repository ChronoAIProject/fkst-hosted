//! Fail-closed configuration for Kubernetes Lease leader election.
//!
//! Leader election is optional so a single-process development deployment keeps
//! the historical behavior. When enabled, every contender must have a unique
//! holder identity (the canonical Deployment injects its pod name through the
//! downward API) and the timing relationship is strictly `retry < renew < lease`.

use serde::Deserialize;

use crate::error::AppError;

const LEADER_ENV_PREFIX: &str = "FKST_LEADER_";
const MAX_LEASE_DURATION_SECS: u64 = 300;
const MAX_HOLDER_IDENTITY_BYTES: usize = 128;

mod defaults {
    pub(super) fn lease_name() -> String {
        "fkst-control-plane-reconciler".to_string()
    }

    pub(super) fn lease_duration_secs() -> u64 {
        30
    }

    pub(super) fn renew_deadline_secs() -> u64 {
        20
    }

    pub(super) fn retry_period_secs() -> u64 {
        5
    }
}

#[derive(Debug, Deserialize)]
struct LeaderVars {
    #[serde(default)]
    election_enabled: bool,
    #[serde(default = "defaults::lease_name")]
    lease_name: String,
    #[serde(default)]
    identity: Option<String>,
    #[serde(default = "defaults::lease_duration_secs")]
    lease_duration_secs: u64,
    #[serde(default = "defaults::renew_deadline_secs")]
    renew_deadline_secs: u64,
    #[serde(default = "defaults::retry_period_secs")]
    retry_period_secs: u64,
}

/// Resolved leader-election settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaderElectionConfig {
    /// Whether Kubernetes Lease ownership gates reconcile side effects.
    pub enabled: bool,
    /// Stable namespaced `coordination.k8s.io/v1` Lease name.
    pub lease_name: String,
    /// Unique contender identity. Required when election is enabled.
    pub identity: Option<String>,
    /// Validity advertised in the Lease after a successful renewal.
    pub lease_duration_secs: u64,
    /// Maximum local time without a confirmed renewal before leadership is lost.
    pub renew_deadline_secs: u64,
    /// Delay between acquisition or renewal attempts.
    pub retry_period_secs: u64,
}

impl Default for LeaderElectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            lease_name: defaults::lease_name(),
            identity: None,
            lease_duration_secs: defaults::lease_duration_secs(),
            renew_deadline_secs: defaults::renew_deadline_secs(),
            retry_period_secs: defaults::retry_period_secs(),
        }
    }
}

impl LeaderElectionConfig {
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<LeaderElectionConfig, AppError> {
        let env: LeaderVars = envy::prefixed(LEADER_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|error| AppError::Config(error.to_string()))?;

        let lease_name = env.lease_name.trim().to_string();
        if !is_dns_subdomain(&lease_name) {
            return Err(AppError::Config(
                "FKST_LEADER_LEASE_NAME must be a non-empty DNS-1123 subdomain no longer than 253 bytes"
                    .to_string(),
            ));
        }

        let identity = env
            .identity
            .map(|identity| identity.trim().to_string())
            .filter(|identity| !identity.is_empty());
        if env.election_enabled && identity.is_none() {
            return Err(AppError::Config(
                "FKST_LEADER_IDENTITY must be set when FKST_LEADER_ELECTION_ENABLED=true"
                    .to_string(),
            ));
        }
        if identity
            .as_ref()
            .is_some_and(|identity| identity.len() > MAX_HOLDER_IDENTITY_BYTES)
        {
            return Err(AppError::Config(format!(
                "FKST_LEADER_IDENTITY must be at most {MAX_HOLDER_IDENTITY_BYTES} bytes"
            )));
        }
        if identity
            .as_ref()
            .is_some_and(|identity| !is_dns_subdomain(identity))
        {
            return Err(AppError::Config(
                "FKST_LEADER_IDENTITY must be a DNS-1123 pod name".to_string(),
            ));
        }

        if !(3..=MAX_LEASE_DURATION_SECS).contains(&env.lease_duration_secs) {
            return Err(AppError::Config(format!(
                "FKST_LEADER_LEASE_DURATION_SECS must be between 3 and {MAX_LEASE_DURATION_SECS}"
            )));
        }
        if env.retry_period_secs == 0 {
            return Err(AppError::Config(
                "FKST_LEADER_RETRY_PERIOD_SECS must be at least 1".to_string(),
            ));
        }
        if env.renew_deadline_secs == 0 {
            return Err(AppError::Config(
                "FKST_LEADER_RENEW_DEADLINE_SECS must be at least 1".to_string(),
            ));
        }
        if env.retry_period_secs >= env.renew_deadline_secs
            || env.renew_deadline_secs >= env.lease_duration_secs
        {
            return Err(AppError::Config(
                "leader election timings must satisfy FKST_LEADER_RETRY_PERIOD_SECS < FKST_LEADER_RENEW_DEADLINE_SECS < FKST_LEADER_LEASE_DURATION_SECS"
                    .to_string(),
            ));
        }

        Ok(LeaderElectionConfig {
            enabled: env.election_enabled,
            lease_name,
            identity,
            lease_duration_secs: env.lease_duration_secs,
            renew_deadline_secs: env.renew_deadline_secs,
            retry_period_secs: env.retry_period_secs,
        })
    }
}

fn is_dns_subdomain(value: &str) -> bool {
    if value.is_empty() || value.len() > 253 {
        return false;
    }
    value.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && label
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && label
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn disabled_defaults_preserve_single_process_mode() {
        assert_eq!(
            LeaderElectionConfig::from_vars(&vars(&[])).expect("defaults"),
            LeaderElectionConfig::default()
        );
    }

    #[test]
    fn enabled_configuration_requires_and_trims_identity() {
        let config = LeaderElectionConfig::from_vars(&vars(&[
            ("FKST_LEADER_ELECTION_ENABLED", "true"),
            ("FKST_LEADER_IDENTITY", "  fkst-control-plane-abc  "),
        ]))
        .expect("enabled config");
        assert!(config.enabled);
        assert_eq!(config.identity.as_deref(), Some("fkst-control-plane-abc"));

        let error =
            LeaderElectionConfig::from_vars(&vars(&[("FKST_LEADER_ELECTION_ENABLED", "true")]))
                .expect_err("missing identity");
        assert!(error.to_string().contains("FKST_LEADER_IDENTITY"));
    }

    #[test]
    fn timings_must_be_positive_ordered_and_bounded() {
        for (key, value) in [
            ("FKST_LEADER_RETRY_PERIOD_SECS", "0"),
            ("FKST_LEADER_RENEW_DEADLINE_SECS", "0"),
            ("FKST_LEADER_LEASE_DURATION_SECS", "2"),
            ("FKST_LEADER_LEASE_DURATION_SECS", "301"),
        ] {
            let error =
                LeaderElectionConfig::from_vars(&vars(&[(key, value)])).expect_err("invalid bound");
            assert!(error.to_string().contains(key), "{key}: {error}");
        }

        for pairs in [
            vec![
                ("FKST_LEADER_RETRY_PERIOD_SECS", "20"),
                ("FKST_LEADER_RENEW_DEADLINE_SECS", "20"),
            ],
            vec![
                ("FKST_LEADER_RENEW_DEADLINE_SECS", "30"),
                ("FKST_LEADER_LEASE_DURATION_SECS", "30"),
            ],
        ] {
            let error =
                LeaderElectionConfig::from_vars(&vars(&pairs)).expect_err("unordered timings");
            assert!(error.to_string().contains("RETRY_PERIOD_SECS"));
            assert!(error.to_string().contains("RENEW_DEADLINE_SECS"));
            assert!(error.to_string().contains("LEASE_DURATION_SECS"));
        }
    }

    #[test]
    fn lease_name_and_identity_fail_closed_without_echoing_values() {
        let bad_name = "NOT_A_LEASE";
        let error = LeaderElectionConfig::from_vars(&vars(&[("FKST_LEADER_LEASE_NAME", bad_name)]))
            .expect_err("invalid lease name");
        assert!(error.to_string().contains("FKST_LEADER_LEASE_NAME"));
        assert!(!error.to_string().contains(bad_name));

        let long_identity = "x".repeat(MAX_HOLDER_IDENTITY_BYTES + 1);
        let error = LeaderElectionConfig::from_vars(
            vec![("FKST_LEADER_IDENTITY".to_string(), long_identity.clone())].as_slice(),
        )
        .expect_err("long identity");
        assert!(error.to_string().contains("FKST_LEADER_IDENTITY"));
        assert!(!error.to_string().contains(&long_identity));
    }
}
