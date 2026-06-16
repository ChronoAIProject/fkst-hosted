//! Distribution-layer configuration: the lease pool identity/TTL
//! ([`PoolConfig`]) plus the renew / takeover-scan cadences, the takeover
//! grace window, and the optional per-pod placement cap. Loaded from the
//! environment through the same injectable key/value seam as the sibling
//! configs, validated fail-closed at startup.

use std::time::Duration;

use crate::leases::{PoolConfig, PoolError};

/// How often a holder renews its live lease. Must satisfy
/// `0 < renew && renew * 2 < lease_ttl`.
pub const ENV_RENEW_INTERVAL_SECS: &str = "FKST_LEASE_RENEW_INTERVAL_SECS";
/// How often the reaper scans for orphaned leases. Must be `> 0`.
pub const ENV_SCAN_INTERVAL_SECS: &str = "FKST_TAKEOVER_SCAN_INTERVAL_SECS";
/// Extra wait after `expires_at` before a takeover is attempted (clock-skew
/// buffer). May be `0`.
pub const ENV_GRACE_SECS: &str = "FKST_TAKEOVER_GRACE_SECS";
/// Optional per-pod active-session cap; `0` = no cap.
pub const ENV_MAX_LOAD: &str = "FKST_PLACEMENT_MAX_LOAD";

const DEFAULT_RENEW_INTERVAL_SECS: u64 = 10;
const DEFAULT_SCAN_INTERVAL_SECS: u64 = 5;
const DEFAULT_GRACE_SECS: u64 = 2;
const DEFAULT_MAX_LOAD: u64 = 0;

/// Sanity ceiling shared by every duration knob (24 hours) so downstream
/// millisecond arithmetic stays trivially within `i64` — mirrors the lease
/// TTL bound in [`crate::leases::config`].
const MAX_SECS: u64 = 86_400;

/// Configuration for the distribution + takeover layer.
#[derive(Debug, Clone)]
pub struct DistributionConfig {
    /// Pod identity and lease TTL (owned by the leases module; composed
    /// here so the two layers can never disagree on either value).
    pub pool: PoolConfig,
    /// Holder heartbeat cadence: how often a session driver renews its
    /// package lease. Default 10s.
    pub renew_interval: Duration,
    /// Reaper cadence: how often this pod scans for orphaned leases and
    /// unplaced sessions. Default 5s.
    pub scan_interval: Duration,
    /// Extra wait after lease expiry before a takeover is attempted, to
    /// absorb cross-pod clock skew (the lease store compares against the
    /// application clock, not Mongo server time). Default 2s; may be 0.
    pub grace: Duration,
    /// Per-pod active-session cap for placement; `0` means uncapped (a pod
    /// is never rejected on load). Default 0.
    pub max_load: u64,
}

impl DistributionConfig {
    /// Build (and fail-closed validate) a `DistributionConfig` from
    /// environment-style key/value pairs. The same pairs feed
    /// [`PoolConfig::from_vars`] so pod identity and lease TTL resolution
    /// stay single-sourced.
    pub fn from_vars(
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> Result<DistributionConfig, PoolError> {
        let vars: Vec<(String, String)> = vars.into_iter().collect();
        let pool = PoolConfig::from_vars(vars.clone())?;

        let mut renew_raw = None;
        let mut scan_raw = None;
        let mut grace_raw = None;
        let mut max_load_raw = None;
        for (key, value) in vars {
            match key.as_str() {
                ENV_RENEW_INTERVAL_SECS => renew_raw = Some(value),
                ENV_SCAN_INTERVAL_SECS => scan_raw = Some(value),
                ENV_GRACE_SECS => grace_raw = Some(value),
                ENV_MAX_LOAD => max_load_raw = Some(value),
                _ => {}
            }
        }

        let config = DistributionConfig {
            pool,
            renew_interval: Duration::from_secs(parse_u64(
                ENV_RENEW_INTERVAL_SECS,
                renew_raw,
                DEFAULT_RENEW_INTERVAL_SECS,
            )?),
            scan_interval: Duration::from_secs(parse_u64(
                ENV_SCAN_INTERVAL_SECS,
                scan_raw,
                DEFAULT_SCAN_INTERVAL_SECS,
            )?),
            grace: Duration::from_secs(parse_u64(ENV_GRACE_SECS, grace_raw, DEFAULT_GRACE_SECS)?),
            max_load: parse_u64(ENV_MAX_LOAD, max_load_raw, DEFAULT_MAX_LOAD)?,
        };
        config.validate()?;
        tracing::info!(
            pod = %config.pool.pod_id,
            lease_ttl_secs = config.pool.lease_ttl.as_secs(),
            renew_interval_secs = config.renew_interval.as_secs(),
            scan_interval_secs = config.scan_interval.as_secs(),
            grace_secs = config.grace.as_secs(),
            max_load = config.max_load,
            "distribution configured"
        );
        Ok(config)
    }

    /// Load the configuration from the process environment.
    pub fn load_from_env() -> Result<DistributionConfig, PoolError> {
        Self::from_vars(std::env::vars())
    }

    /// Fail-closed startup validation. Every violation is a
    /// [`PoolError::Config`] naming the offending environment variable so
    /// the non-zero exit is actionable:
    /// - pod identity must be non-empty after trimming;
    /// - the lease TTL must be `> 0` (already enforced by
    ///   [`PoolConfig::from_vars`]; re-checked for direct construction);
    /// - `0 < renew_interval` and `renew_interval * 2 < lease_ttl` (a holder
    ///   gets at least two renewal attempts per TTL window);
    /// - `scan_interval > 0`.
    pub fn validate(&self) -> Result<(), PoolError> {
        if self.pool.pod_id.trim().is_empty() {
            return Err(PoolError::Config(
                "FKST_POD_ID must resolve to a non-empty pod identity".to_string(),
            ));
        }
        if self.pool.lease_ttl.is_zero() {
            return Err(PoolError::Config(
                "FKST_LEASE_TTL_SECS must be at least 1 second".to_string(),
            ));
        }
        if self.renew_interval.is_zero() {
            return Err(PoolError::Config(format!(
                "{ENV_RENEW_INTERVAL_SECS} must be at least 1 second"
            )));
        }
        if self.renew_interval.as_secs().saturating_mul(2) >= self.pool.lease_ttl.as_secs() {
            return Err(PoolError::Config(format!(
                "{ENV_RENEW_INTERVAL_SECS} ({}s) doubled must stay under \
                 FKST_LEASE_TTL_SECS ({}s) so a holder gets at least two \
                 renewal attempts per TTL window",
                self.renew_interval.as_secs(),
                self.pool.lease_ttl.as_secs(),
            )));
        }
        if self.scan_interval.is_zero() {
            return Err(PoolError::Config(format!(
                "{ENV_SCAN_INTERVAL_SECS} must be at least 1 second"
            )));
        }
        Ok(())
    }
}

/// Parse a non-negative whole-seconds/count value with a default, rejecting
/// non-numeric and absurdly large (> 24h) values with an error naming the
/// variable. `0` is allowed here — zero-rejection is relationship-specific
/// and lives in [`DistributionConfig::validate`].
fn parse_u64(name: &'static str, raw: Option<String>, default: u64) -> Result<u64, PoolError> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    let value: u64 = raw.trim().parse().map_err(|_| {
        PoolError::Config(format!(
            "{name} must be a non-negative whole number, got {raw:?}"
        ))
    })?;
    if value > MAX_SECS {
        return Err(PoolError::Config(format!(
            "{name} must be at most {MAX_SECS}, got {value}"
        )));
    }
    Ok(value)
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

    fn base_vars() -> Vec<(String, String)> {
        vars(&[("FKST_POD_ID", "pod-test")])
    }

    #[test]
    fn defaults_load_and_validate() {
        let config = DistributionConfig::from_vars(base_vars()).expect("defaults load");
        assert_eq!(config.pool.pod_id, "pod-test");
        assert_eq!(config.pool.lease_ttl, Duration::from_secs(30));
        assert_eq!(config.renew_interval, Duration::from_secs(10));
        assert_eq!(config.scan_interval, Duration::from_secs(5));
        assert_eq!(config.grace, Duration::from_secs(2));
        assert_eq!(config.max_load, 0);
        config.validate().expect("default config is valid");
    }

    #[test]
    fn explicit_overrides_are_honored() {
        let mut pairs = base_vars();
        pairs.extend(vars(&[
            ("FKST_LEASE_TTL_SECS", "60"),
            ("FKST_LEASE_RENEW_INTERVAL_SECS", "20"),
            ("FKST_TAKEOVER_SCAN_INTERVAL_SECS", "7"),
            ("FKST_TAKEOVER_GRACE_SECS", "0"),
            ("FKST_PLACEMENT_MAX_LOAD", "3"),
        ]));
        let config = DistributionConfig::from_vars(pairs).expect("config loads");
        assert_eq!(config.pool.lease_ttl, Duration::from_secs(60));
        assert_eq!(config.renew_interval, Duration::from_secs(20));
        assert_eq!(config.scan_interval, Duration::from_secs(7));
        assert_eq!(config.grace, Duration::ZERO, "grace may be zero");
        assert_eq!(config.max_load, 3);
    }

    /// Each rejected combination must fail closed and name the offending
    /// env var so the startup ERROR is actionable.
    #[test]
    fn invalid_combinations_fail_closed_naming_the_var() {
        let cases: &[(&[(&str, &str)], &str)] = &[
            // renew == 0
            (
                &[("FKST_LEASE_RENEW_INTERVAL_SECS", "0")],
                "FKST_LEASE_RENEW_INTERVAL_SECS",
            ),
            // renew * 2 == ttl (boundary: must be strictly under)
            (
                &[
                    ("FKST_LEASE_TTL_SECS", "30"),
                    ("FKST_LEASE_RENEW_INTERVAL_SECS", "15"),
                ],
                "FKST_LEASE_RENEW_INTERVAL_SECS",
            ),
            // renew * 2 > ttl
            (
                &[
                    ("FKST_LEASE_TTL_SECS", "10"),
                    ("FKST_LEASE_RENEW_INTERVAL_SECS", "9"),
                ],
                "FKST_LEASE_RENEW_INTERVAL_SECS",
            ),
            // scan == 0
            (
                &[("FKST_TAKEOVER_SCAN_INTERVAL_SECS", "0")],
                "FKST_TAKEOVER_SCAN_INTERVAL_SECS",
            ),
            // non-numeric values
            (
                &[("FKST_LEASE_RENEW_INTERVAL_SECS", "soon")],
                "FKST_LEASE_RENEW_INTERVAL_SECS",
            ),
            (
                &[("FKST_TAKEOVER_GRACE_SECS", "-1")],
                "FKST_TAKEOVER_GRACE_SECS",
            ),
            (
                &[("FKST_PLACEMENT_MAX_LOAD", "lots")],
                "FKST_PLACEMENT_MAX_LOAD",
            ),
            // over the 24h sanity ceiling
            (
                &[("FKST_TAKEOVER_SCAN_INTERVAL_SECS", "86401")],
                "FKST_TAKEOVER_SCAN_INTERVAL_SECS",
            ),
        ];
        for (extra, var) in cases {
            let mut pairs = base_vars();
            pairs.extend(vars(extra));
            let err = DistributionConfig::from_vars(pairs)
                .expect_err(&format!("{extra:?} must fail closed"));
            assert!(matches!(err, PoolError::Config(_)), "{extra:?}: {err}");
            assert!(
                err.to_string().contains(var),
                "error must name {var}, got: {err}"
            );
        }
    }

    #[test]
    fn validate_rejects_blank_pod_identity_on_direct_construction() {
        let config = DistributionConfig {
            pool: PoolConfig {
                pod_id: "   ".to_string(),
                lease_ttl: Duration::from_secs(30),
            },
            renew_interval: Duration::from_secs(10),
            scan_interval: Duration::from_secs(5),
            grace: Duration::from_secs(2),
            max_load: 0,
        };
        let err = config.validate().expect_err("blank pod id must fail");
        assert!(err.to_string().contains("FKST_POD_ID"), "got: {err}");
    }

    #[test]
    fn max_load_zero_means_uncapped_and_is_the_default() {
        let config = DistributionConfig::from_vars(base_vars()).expect("config loads");
        assert_eq!(config.max_load, 0, "0 is the documented uncapped default");
    }
}
