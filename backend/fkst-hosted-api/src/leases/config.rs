//! Lease-pool configuration: pod identity and lease TTL, loaded from the
//! environment through an injectable key/value seam (mirrors
//! [`crate::config::Config::from_vars`] so unit tests never mutate the real
//! process environment).

use std::time::Duration;

use super::error::PoolError;

/// Explicit pod identity; highest precedence.
pub const ENV_POD_ID: &str = "FKST_POD_ID";
/// Kubernetes injects the pod name here; used when [`ENV_POD_ID`] is
/// unset/empty.
pub const ENV_HOSTNAME: &str = "HOSTNAME";
/// Lease TTL in seconds (`expires_at = now + ttl`). Must parse to `>= 1`.
pub const ENV_LEASE_TTL_SECS: &str = "FKST_LEASE_TTL_SECS";

/// Default lease TTL when [`ENV_LEASE_TTL_SECS`] is unset.
const DEFAULT_LEASE_TTL_SECS: u64 = 30;

/// Sanity ceiling for [`ENV_LEASE_TTL_SECS`] (24 hours). Anything larger is
/// a misconfiguration: it would defeat expired-lease takeover and opens an
/// unchecked-overflow corner in millisecond arithmetic.
const MAX_LEASE_TTL_SECS: u64 = 86_400;

/// Configuration for the lease coordination layer.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Stable, process-lifetime pod identity. Opaque: only ever used as a
    /// typed BSON value in `doc!` filters, never string-concatenated into a
    /// query. Two different processes never share an identity (the uuid
    /// fallback guarantees uniqueness in non-K8s runs).
    pub pod_id: String,
    /// Lease TTL: on every successful acquire/renew,
    /// `expires_at = now + lease_ttl`.
    pub lease_ttl: Duration,
}

impl PoolConfig {
    /// Build a `PoolConfig` from environment-style key/value pairs.
    ///
    /// Pod identity resolution (first non-empty after trimming wins; the
    /// chain cannot fail because the last step always yields a value):
    /// 1. `FKST_POD_ID`
    /// 2. `HOSTNAME` (Kubernetes sets this to the pod name)
    /// 3. `local-<uuid v4>` fallback, WARN-logged (no stable K8s identity).
    ///
    /// The lease TTL comes from `FKST_LEASE_TTL_SECS` (default 30) and must
    /// be within `1..=86_400` seconds: a zero TTL would make every lease
    /// instantly dead and a multi-day TTL would defeat takeover, so `0`,
    /// non-numeric, and over-bound values fail closed with
    /// [`PoolError::Config`].
    pub fn from_vars(
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> Result<PoolConfig, PoolError> {
        let mut pod_id = None;
        let mut hostname = None;
        let mut ttl_raw = None;
        for (key, value) in vars {
            match key.as_str() {
                ENV_POD_ID => pod_id = Some(value),
                ENV_HOSTNAME => hostname = Some(value),
                ENV_LEASE_TTL_SECS => ttl_raw = Some(value),
                _ => {}
            }
        }
        let config = PoolConfig {
            pod_id: resolve_pod_id(pod_id, hostname),
            lease_ttl: parse_lease_ttl(ttl_raw)?,
        };
        tracing::info!(
            pod = %config.pod_id,
            lease_ttl_secs = config.lease_ttl.as_secs(),
            "lease pool configured"
        );
        Ok(config)
    }

    /// Load the configuration from the process environment.
    pub fn load_from_env() -> Result<PoolConfig, PoolError> {
        Self::from_vars(std::env::vars())
    }
}

/// Resolve the pod identity with the documented precedence. Cannot fail: the
/// uuid v4 fallback always yields a (process-unique) value.
fn resolve_pod_id(pod_id: Option<String>, hostname: Option<String>) -> String {
    for (source, value) in [(ENV_POD_ID, pod_id), (ENV_HOSTNAME, hostname)] {
        if let Some(value) = value {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                tracing::debug!(source, pod = trimmed, "pod identity resolved");
                return trimmed.to_string();
            }
        }
    }
    let fallback = format!("local-{}", bson::Uuid::new());
    tracing::warn!(
        pod = %fallback,
        "no stable pod identity (FKST_POD_ID and HOSTNAME unset/empty); \
         falling back to a process-lifetime random identity"
    );
    fallback
}

/// Parse the lease TTL, defaulting to [`DEFAULT_LEASE_TTL_SECS`]; rejects
/// zero, non-numeric, and absurdly large values (> [`MAX_LEASE_TTL_SECS`])
/// with an error naming the variable. The upper bound keeps every downstream
/// millis computation (`now + ttl`, reap margins) trivially within `i64` —
/// no unchecked-overflow corner — and a multi-day "lease" would defeat
/// takeover anyway.
fn parse_lease_ttl(raw: Option<String>) -> Result<Duration, PoolError> {
    let Some(raw) = raw else {
        return Ok(Duration::from_secs(DEFAULT_LEASE_TTL_SECS));
    };
    let secs: u64 = raw.trim().parse().map_err(|_| {
        PoolError::Config(format!(
            "{ENV_LEASE_TTL_SECS} must be a whole number of seconds, got {raw:?}"
        ))
    })?;
    if secs == 0 {
        return Err(PoolError::Config(format!(
            "{ENV_LEASE_TTL_SECS} must be at least 1 second"
        )));
    }
    if secs > MAX_LEASE_TTL_SECS {
        return Err(PoolError::Config(format!(
            "{ENV_LEASE_TTL_SECS} must be at most {MAX_LEASE_TTL_SECS} seconds (24h), got {secs}"
        )));
    }
    Ok(Duration::from_secs(secs))
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
    fn pod_id_precedence() {
        // FKST_POD_ID wins over HOSTNAME.
        let config = PoolConfig::from_vars(vars(&[
            ("FKST_POD_ID", "explicit-pod"),
            ("HOSTNAME", "k8s-pod-name"),
        ]))
        .expect("config loads");
        assert_eq!(config.pod_id, "explicit-pod");

        // HOSTNAME is used when FKST_POD_ID is absent.
        let config =
            PoolConfig::from_vars(vars(&[("HOSTNAME", "k8s-pod-name")])).expect("config loads");
        assert_eq!(config.pod_id, "k8s-pod-name");

        // Whitespace-only FKST_POD_ID falls through to HOSTNAME, and the
        // winning value is trimmed.
        let config = PoolConfig::from_vars(vars(&[
            ("FKST_POD_ID", "   "),
            ("HOSTNAME", "  k8s-pod-name  "),
        ]))
        .expect("config loads");
        assert_eq!(config.pod_id, "k8s-pod-name");

        // Neither set: `local-<uuid v4>` fallback (the WARN path); the
        // suffix must parse as a valid uuid.
        let config = PoolConfig::from_vars(vars(&[])).expect("config loads");
        let suffix = config
            .pod_id
            .strip_prefix("local-")
            .expect("fallback identity must be prefixed `local-`");
        bson::Uuid::parse_str(suffix).expect("fallback suffix must be a valid uuid");

        // Two resolutions never share a fallback identity.
        let other = PoolConfig::from_vars(vars(&[])).expect("config loads");
        assert_ne!(
            config.pod_id, other.pod_id,
            "fallback identity must be unique"
        );
    }

    #[test]
    fn ttl_parsing() {
        // Default when unset.
        let config = PoolConfig::from_vars(vars(&[])).expect("config loads");
        assert_eq!(config.lease_ttl, Duration::from_secs(30));

        // Explicit override is honored.
        let config =
            PoolConfig::from_vars(vars(&[("FKST_LEASE_TTL_SECS", "10")])).expect("config loads");
        assert_eq!(config.lease_ttl, Duration::from_secs(10));

        // Zero is rejected (would make every lease instantly dead) and the
        // error names the variable.
        let err = PoolConfig::from_vars(vars(&[("FKST_LEASE_TTL_SECS", "0")]))
            .expect_err("zero TTL must fail");
        assert!(matches!(err, PoolError::Config(_)));
        assert!(
            err.to_string().contains("FKST_LEASE_TTL_SECS"),
            "error must name the env var, got: {err}"
        );

        // The 24h ceiling itself is accepted...
        let config = PoolConfig::from_vars(vars(&[("FKST_LEASE_TTL_SECS", "86400")]))
            .expect("max TTL loads");
        assert_eq!(config.lease_ttl, Duration::from_secs(86_400));

        // ...but anything above it is rejected and the error names the
        // variable (closes the unchecked-millis-overflow corner).
        let err = PoolConfig::from_vars(vars(&[("FKST_LEASE_TTL_SECS", "86401")]))
            .expect_err("over-bound TTL must fail");
        assert!(matches!(err, PoolError::Config(_)));
        assert!(
            err.to_string().contains("FKST_LEASE_TTL_SECS"),
            "error must name the env var, got: {err}"
        );

        // Non-numeric is rejected and the error names the variable.
        let err = PoolConfig::from_vars(vars(&[("FKST_LEASE_TTL_SECS", "abc")]))
            .expect_err("non-numeric TTL must fail");
        assert!(matches!(err, PoolError::Config(_)));
        assert!(
            err.to_string().contains("FKST_LEASE_TTL_SECS"),
            "error must name the env var, got: {err}"
        );

        // Negative values are non-parseable as u64 and rejected too.
        let err = PoolConfig::from_vars(vars(&[("FKST_LEASE_TTL_SECS", "-5")]))
            .expect_err("negative TTL must fail");
        assert!(matches!(err, PoolError::Config(_)));
    }
}
