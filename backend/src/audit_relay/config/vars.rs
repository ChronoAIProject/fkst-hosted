//! The raw environment shape and its documented defaults.
//!
//! Deserialization is separated from validation ([`super::bounds`]) and from the
//! resolved type ([`super::RelayConfig`]) so each half stays small and reviewable:
//! this file answers only "what variables exist and what do they default to",
//! which is the question an operator reading a manifest is actually asking.

use std::path::PathBuf;

use serde::Deserialize;

use crate::error::AppError;

/// Prefix for the relay's own variables.
const RELAY_ENV_PREFIX: &str = "FKST_AUDIT_RELAY_";
/// Prefix shared with the control plane's delivery configuration.
const AUDIT_ENV_PREFIX: &str = "FKST_AUDIT_";
/// Prefix shared with the control plane's capture/query configuration.
const POSTHOG_ENV_PREFIX: &str = "FKST_POSTHOG_";
/// Prefix for the deployment environment name.
const DEPLOYMENT_ENV_PREFIX: &str = "FKST_";

pub(super) mod defaults {
    use std::path::PathBuf;

    pub(in crate::audit_relay::config) fn bind_addr() -> String {
        "0.0.0.0:8090".to_string()
    }

    pub(in crate::audit_relay::config) fn db_path() -> PathBuf {
        PathBuf::from("/var/lib/fkst-audit/audit.sqlite3")
    }

    pub(in crate::audit_relay::config) fn max_body_bytes() -> usize {
        65_536
    }

    pub(in crate::audit_relay::config) fn max_records() -> u64 {
        5_000_000
    }

    pub(in crate::audit_relay::config) fn verification_delay_secs() -> u64 {
        30
    }

    pub(in crate::audit_relay::config) fn verification_max_age_secs() -> u64 {
        300
    }

    pub(in crate::audit_relay::config) fn verified_retention_days() -> u64 {
        7
    }

    pub(in crate::audit_relay::config) fn audit_retention_days() -> u64 {
        90
    }

    pub(in crate::audit_relay::config) fn incomplete_grace_secs() -> u64 {
        60
    }

    pub(in crate::audit_relay::config) fn busy_timeout_ms() -> u64 {
        5_000
    }

    pub(in crate::audit_relay::config) fn writer_queue_capacity() -> usize {
        512
    }

    pub(in crate::audit_relay::config) fn max_read_concurrency() -> usize {
        8
    }

    pub(in crate::audit_relay::config) fn max_read_rows() -> u32 {
        500
    }

    pub(in crate::audit_relay::config) fn max_range_days() -> u64 {
        400
    }

    pub(in crate::audit_relay::config) fn capture_batch_size() -> usize {
        100
    }

    pub(in crate::audit_relay::config) fn max_capture_attempts() -> u32 {
        8
    }

    pub(in crate::audit_relay::config) fn retry_initial_secs() -> u64 {
        5
    }

    pub(in crate::audit_relay::config) fn retry_max_secs() -> u64 {
        900
    }

    pub(in crate::audit_relay::config) fn worker_interval_secs() -> u64 {
        5
    }

    pub(in crate::audit_relay::config) fn verification_batch_size() -> usize {
        200
    }
}

/// The `FKST_AUDIT_RELAY_*` variables.
#[derive(Debug, Deserialize)]
pub(super) struct RelayVars {
    #[serde(default = "defaults::bind_addr")]
    pub(super) bind_addr: String,
    #[serde(default = "defaults::db_path")]
    pub(super) db_path: PathBuf,
    #[serde(default)]
    pub(super) write_token: Option<String>,
    #[serde(default)]
    pub(super) read_token: Option<String>,
    #[serde(default = "defaults::max_body_bytes")]
    pub(super) max_body_bytes: usize,
    #[serde(default = "defaults::max_records")]
    pub(super) max_records: u64,
    #[serde(default = "defaults::verification_delay_secs")]
    pub(super) verification_delay_secs: u64,
    #[serde(default = "defaults::verification_max_age_secs")]
    pub(super) verification_max_age_secs: u64,
    #[serde(default = "defaults::verified_retention_days")]
    pub(super) verified_retention_days: u64,
    #[serde(default = "defaults::audit_retention_days")]
    pub(super) audit_retention_days: u64,
    #[serde(default = "defaults::busy_timeout_ms")]
    pub(super) busy_timeout_ms: u64,
    #[serde(default = "defaults::writer_queue_capacity")]
    pub(super) writer_queue_capacity: usize,
    #[serde(default = "defaults::max_read_concurrency")]
    pub(super) max_read_concurrency: usize,
    #[serde(default = "defaults::max_read_rows")]
    pub(super) max_read_rows: u32,
    #[serde(default = "defaults::max_range_days")]
    pub(super) max_range_days: u64,
    #[serde(default = "defaults::capture_batch_size")]
    pub(super) capture_batch_size: usize,
    #[serde(default = "defaults::max_capture_attempts")]
    pub(super) max_capture_attempts: u32,
    #[serde(default = "defaults::retry_initial_secs")]
    pub(super) retry_initial_secs: u64,
    #[serde(default = "defaults::retry_max_secs")]
    pub(super) retry_max_secs: u64,
    #[serde(default = "defaults::worker_interval_secs")]
    pub(super) worker_interval_secs: u64,
    #[serde(default = "defaults::verification_batch_size")]
    pub(super) verification_batch_size: usize,
}

/// The grace the relay adds to a record's own completion deadline before it
/// synthesizes an incomplete terminal. Read from the SHARED
/// `FKST_AUDIT_INCOMPLETE_GRACE_SECS` so the writer and the closer agree.
#[derive(Debug, Deserialize)]
pub(super) struct SharedAuditVars {
    #[serde(default = "defaults::incomplete_grace_secs")]
    pub(super) incomplete_grace_secs: u64,
}

/// The `FKST_POSTHOG_*` half the relay needs (capture + verification).
#[derive(Debug, Deserialize)]
pub(super) struct RelayPosthogVars {
    #[serde(default)]
    pub(super) host: Option<String>,
    #[serde(default)]
    pub(super) project_token: Option<String>,
    #[serde(default)]
    pub(super) project_id: Option<String>,
    #[serde(default)]
    pub(super) query_api_key: Option<String>,
}

/// The bare `FKST_` variables this module reads.
#[derive(Debug, Deserialize)]
pub(super) struct DeploymentVars {
    #[serde(default)]
    pub(super) deployment_environment: Option<String>,
}

/// Every raw group, deserialized from one snapshot of the environment.
pub(super) struct RawVars {
    pub(super) relay: RelayVars,
    pub(super) shared: SharedAuditVars,
    pub(super) posthog: RelayPosthogVars,
    pub(super) deployment: DeploymentVars,
}

impl RawVars {
    /// Deserialize all four groups, naming the offending PREFIX on failure. The
    /// message never echoes a value: one of these groups carries secrets.
    pub(super) fn load(vars: &[(String, String)]) -> Result<Self, AppError> {
        Ok(Self {
            relay: envy::prefixed(RELAY_ENV_PREFIX)
                .from_iter(vars.iter().cloned())
                .map_err(|e| AppError::Config(format!("FKST_AUDIT_RELAY_* is invalid: {e}")))?,
            shared: envy::prefixed(AUDIT_ENV_PREFIX)
                .from_iter(vars.iter().cloned())
                .map_err(|e| AppError::Config(format!("FKST_AUDIT_* is invalid: {e}")))?,
            posthog: envy::prefixed(POSTHOG_ENV_PREFIX)
                .from_iter(vars.iter().cloned())
                .map_err(|e| AppError::Config(format!("FKST_POSTHOG_* is invalid: {e}")))?,
            deployment: envy::prefixed(DEPLOYMENT_ENV_PREFIX)
                .from_iter(vars.iter().cloned())
                .map_err(|e| {
                    AppError::Config(format!("FKST_DEPLOYMENT_ENVIRONMENT is invalid: {e}"))
                })?,
        })
    }
}
