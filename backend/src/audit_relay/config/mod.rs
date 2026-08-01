//! Typed configuration for the `fkst-audit-relay` deployable
//! (`FKST_AUDIT_RELAY_*` plus the `FKST_POSTHOG_*` credentials it delivers with).
//!
//! It follows the same fail-closed, bounds-first style as
//! [`crate::audit::config`]: every numeric knob is validated at startup so an
//! inverted retention window or a nonsensical batch size surfaces at deploy time
//! rather than the first night a backlog builds. The three concerns are three
//! files — [`vars`] is the environment shape and its defaults, [`bounds`] is the
//! validation, and this file is the resolved type — because one file holding all
//! three grew past the repository's size limit and read as one long function.
//!
//! ## Two tokens, never one
//!
//! The write token admits records; the read token reads the deployment's entire
//! recent audit trail. They are separate variables, separate
//! [`SecretString`]s, and separately compared, because collapsing them would
//! silently promote a low-value ingestion credential into the key to everyone's
//! activity (the same argument [`crate::operations::config`] makes about the
//! PostHog project token). Neither is ever logged, rendered in `Debug`, or
//! returned in a response, and a deployment that configures the same value for
//! both is refused at startup.
//!
//! ## Retention is ordered, and only verified rows are ever purged
//!
//! `FKST_AUDIT_RELAY_VERIFIED_RETENTION_DAYS` bounds the dedup/query overlap
//! window after a row is proven query-visible in PostHog; those rows — and only
//! those — are purged. Unverified, incomplete, and dead-letter rows are NEVER
//! auto-deleted: they are the records whose delivery could not be proven, which
//! makes them the last thing an audit trail may discard.
//! `FKST_AUDIT_RELAY_AUDIT_RETENTION_DAYS` is the documented floor an operator
//! must keep them for before remediating by hand, and is validated to be at
//! least the verified window so the two can never be configured inside out.
//!
//! ## The delivery host is judged by the same rule the control plane uses
//!
//! `FKST_POSTHOG_HOST` goes through [`crate::audit::host::normalize`], the
//! shared rule, rather than a local trim. The relay is the process that carries
//! the project capture token on every batch, so a `http://` host here ships that
//! credential in cleartext and a `https://svc:<token>@…` host puts it in the
//! ConfigMap that exists to hold no credential (epic `OPS-02`). Unlike the
//! control plane there is no lenient "staged" path: an unset host is the
//! outbox-only shape, so a host that IS set is always about to be dialled.

mod bounds;
mod vars;

use std::net::SocketAddr;
use std::path::PathBuf;

use secrecy::{ExposeSecret, SecretString};

use crate::error::AppError;

use bounds::{non_blank, required_secret};
use vars::RawVars;

/// Resolved relay configuration.
#[derive(Clone)]
pub struct RelayConfig {
    /// Env: `FKST_AUDIT_RELAY_BIND_ADDR`. Default `0.0.0.0:8090`.
    pub bind_addr: SocketAddr,
    /// Env: `FKST_AUDIT_RELAY_DB_PATH`. The parent directory is created `0700`
    /// and the database file kept `0600`.
    pub db_path: PathBuf,
    /// Env: `FKST_AUDIT_RELAY_WRITE_TOKEN`. Admits records. Secret.
    pub write_token: SecretString,
    /// Env: `FKST_AUDIT_RELAY_READ_TOKEN`. Reads the recent trail. Secret, and
    /// deliberately NOT the write token.
    pub read_token: SecretString,
    /// Env: `FKST_AUDIT_RELAY_MAX_BODY_BYTES`. Default 65536.
    pub max_body_bytes: usize,
    /// Env: `FKST_AUDIT_RELAY_MAX_RECORDS`. Capacity guard: past it, ingress is
    /// refused with a bounded error instead of filling the volume.
    pub max_records: u64,
    /// Env: `FKST_AUDIT_RELAY_VERIFICATION_DELAY_SECS`. How long after capture
    /// acceptance the verification read may first run. Default 30.
    pub verification_delay_secs: u64,
    /// Env: `FKST_AUDIT_RELAY_VERIFICATION_MAX_AGE_SECS`. Past this, an accepted
    /// but still-absent event is re-captured with the SAME uuid and alerted on.
    pub verification_max_age_secs: u64,
    /// Env: `FKST_AUDIT_RELAY_VERIFIED_RETENTION_DAYS`. Default 7.
    pub verified_retention_days: u64,
    /// Env: `FKST_AUDIT_RELAY_AUDIT_RETENTION_DAYS`. Default 90. A documented
    /// floor for operator remediation — the relay never auto-deletes these rows.
    pub audit_retention_days: u64,
    /// Env: `FKST_AUDIT_INCOMPLETE_GRACE_SECS`. Added to each record's own
    /// deadline before an incomplete terminal is synthesized. Default 60.
    ///
    /// The CONTROL PLANE is where this value is checked against the request
    /// budget it has to outlast (see
    /// [`crate::audit::relay::AuditDeliveryConfig::ensure_grace_covers`]); the
    /// relay cannot see that budget and deliberately does not guess at one.
    pub incomplete_grace_secs: u64,
    /// SQLite `busy_timeout`, bounded. Env: `FKST_AUDIT_RELAY_BUSY_TIMEOUT_MS`.
    pub busy_timeout_ms: u64,
    /// Depth of the single writer's command queue. Env:
    /// `FKST_AUDIT_RELAY_WRITER_QUEUE_CAPACITY`.
    pub writer_queue_capacity: usize,
    /// Concurrent scoped reads. Env: `FKST_AUDIT_RELAY_MAX_READ_CONCURRENCY`.
    pub max_read_concurrency: usize,
    /// Hard ceiling on one read page. Env: `FKST_AUDIT_RELAY_MAX_READ_ROWS`.
    pub max_read_rows: u32,
    /// Widest `to - from` one read may span. Env:
    /// `FKST_AUDIT_RELAY_MAX_RANGE_DAYS`.
    pub max_range_days: u64,
    /// Records per capture batch. Env: `FKST_AUDIT_RELAY_CAPTURE_BATCH_SIZE`.
    pub capture_batch_size: usize,
    /// Attempts before a record dead-letters. Env:
    /// `FKST_AUDIT_RELAY_MAX_CAPTURE_ATTEMPTS`.
    pub max_capture_attempts: u32,
    /// First retry delay. Env: `FKST_AUDIT_RELAY_RETRY_INITIAL_SECS`.
    pub retry_initial_secs: u64,
    /// Retry backoff ceiling. Env: `FKST_AUDIT_RELAY_RETRY_MAX_SECS`.
    pub retry_max_secs: u64,
    /// Cadence of the delivery/verification/closer sweep. Env:
    /// `FKST_AUDIT_RELAY_WORKER_INTERVAL_SECS`.
    pub worker_interval_secs: u64,
    /// Event ids per verification query. Env:
    /// `FKST_AUDIT_RELAY_VERIFICATION_BATCH_SIZE`. Verification is BATCHED: one
    /// HogQL request per event would make verification cost more than capture.
    pub verification_batch_size: usize,
    /// Env: `FKST_POSTHOG_HOST`. `None` disables delivery (records accumulate
    /// durably and readiness stays true — that is the point of an outbox).
    pub posthog_host: Option<String>,
    /// Env: `FKST_POSTHOG_PROJECT_TOKEN`. Secret.
    pub posthog_project_token: SecretString,
    /// Env: `FKST_POSTHOG_PROJECT_ID`.
    pub posthog_project_id: Option<String>,
    /// Env: `FKST_POSTHOG_QUERY_API_KEY`. Secret; without it capture still runs
    /// but nothing can be VERIFIED, and rows correctly stay `posthog_accepted`.
    pub posthog_query_api_key: SecretString,
    /// Env: `FKST_DEPLOYMENT_ENVIRONMENT`. Never a secret.
    pub environment: String,
}

// Hand-written so neither token can reach a log through a `{:?}` on the config
// or on anything embedding it.
impl std::fmt::Debug for RelayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayConfig")
            .field("bind_addr", &self.bind_addr)
            .field("db_path", &self.db_path)
            .field("write_token", &"<redacted>")
            .field("read_token", &"<redacted>")
            .field("max_body_bytes", &self.max_body_bytes)
            .field("max_records", &self.max_records)
            .field("verification_delay_secs", &self.verification_delay_secs)
            .field("verification_max_age_secs", &self.verification_max_age_secs)
            .field("verified_retention_days", &self.verified_retention_days)
            .field("audit_retention_days", &self.audit_retention_days)
            .field("incomplete_grace_secs", &self.incomplete_grace_secs)
            .field("busy_timeout_ms", &self.busy_timeout_ms)
            .field("writer_queue_capacity", &self.writer_queue_capacity)
            .field("max_read_concurrency", &self.max_read_concurrency)
            .field("max_read_rows", &self.max_read_rows)
            .field("max_range_days", &self.max_range_days)
            .field("capture_batch_size", &self.capture_batch_size)
            .field("max_capture_attempts", &self.max_capture_attempts)
            .field("retry_initial_secs", &self.retry_initial_secs)
            .field("retry_max_secs", &self.retry_max_secs)
            .field("worker_interval_secs", &self.worker_interval_secs)
            .field("verification_batch_size", &self.verification_batch_size)
            .field("posthog_host", &self.posthog_host)
            .field("posthog_project_token", &"<redacted>")
            .field("posthog_project_id", &self.posthog_project_id)
            .field("posthog_query_api_key", &"<redacted>")
            .field("environment", &self.environment)
            .finish()
    }
}

impl RelayConfig {
    /// Deserialize + validate from environment-style pairs.
    pub fn from_vars(vars: &[(String, String)]) -> Result<Self, AppError> {
        let raw = RawVars::load(vars)?;
        // Bounds first and unconditionally: an operator mistake must surface at
        // deploy time, not at the first request that trips it.
        bounds::validate(&raw.relay, &raw.shared)?;

        let bind_addr = raw
            .relay
            .bind_addr
            .trim()
            .parse::<SocketAddr>()
            .map_err(|_| {
                AppError::Config(
                    "FKST_AUDIT_RELAY_BIND_ADDR must be a host:port socket address".to_string(),
                )
            })?;

        let write_token = required_secret("FKST_AUDIT_RELAY_WRITE_TOKEN", raw.relay.write_token)?;
        let read_token = required_secret("FKST_AUDIT_RELAY_READ_TOKEN", raw.relay.read_token)?;
        // Same value for both would make the write credential a read credential.
        // Refused loudly: it is the exact mistake the two-token split exists for.
        if write_token == read_token {
            return Err(AppError::Config(
                "FKST_AUDIT_RELAY_WRITE_TOKEN and FKST_AUDIT_RELAY_READ_TOKEN must differ"
                    .to_string(),
            ));
        }

        let environment = non_blank(raw.deployment.deployment_environment).unwrap_or_default();
        // The relay has no "staged host" state: a configured host IS the
        // delivery target, so it gets the full shared rule (TLS unless the
        // deployment names itself test/local, and never embedded userinfo). The
        // relay is the process that actually carries the project capture token
        // on every batch, which is precisely why it may not be the lenient one.
        let posthog_host = non_blank(raw.posthog.host)
            .map(|host| crate::audit::host::normalize(&host, &environment))
            .transpose()?;

        Ok(Self {
            bind_addr,
            db_path: raw.relay.db_path,
            write_token: SecretString::from(write_token),
            read_token: SecretString::from(read_token),
            max_body_bytes: raw.relay.max_body_bytes,
            max_records: raw.relay.max_records,
            verification_delay_secs: raw.relay.verification_delay_secs,
            verification_max_age_secs: raw.relay.verification_max_age_secs,
            verified_retention_days: raw.relay.verified_retention_days,
            audit_retention_days: raw.relay.audit_retention_days,
            incomplete_grace_secs: raw.shared.incomplete_grace_secs,
            busy_timeout_ms: raw.relay.busy_timeout_ms,
            writer_queue_capacity: raw.relay.writer_queue_capacity,
            max_read_concurrency: raw.relay.max_read_concurrency,
            max_read_rows: raw.relay.max_read_rows,
            max_range_days: raw.relay.max_range_days,
            capture_batch_size: raw.relay.capture_batch_size,
            max_capture_attempts: raw.relay.max_capture_attempts,
            retry_initial_secs: raw.relay.retry_initial_secs,
            retry_max_secs: raw.relay.retry_max_secs,
            worker_interval_secs: raw.relay.worker_interval_secs,
            verification_batch_size: raw.relay.verification_batch_size,
            posthog_host,
            posthog_project_token: SecretString::from(
                non_blank(raw.posthog.project_token).unwrap_or_default(),
            ),
            posthog_project_id: non_blank(raw.posthog.project_id),
            posthog_query_api_key: SecretString::from(
                non_blank(raw.posthog.query_api_key).unwrap_or_default(),
            ),
            environment,
        })
    }

    /// Read the process environment.
    pub fn load_from_env() -> Result<Self, AppError> {
        let vars: Vec<(String, String)> = std::env::vars().collect();
        Self::from_vars(&vars)
    }

    /// Whether PostHog capture is configured. `false` keeps the relay a pure
    /// durable outbox: records commit and accumulate, readiness stays true, and
    /// the backlog gauges are how an operator sees it.
    pub fn capture_configured(&self) -> bool {
        self.posthog_host.is_some() && !self.posthog_project_token.expose_secret().is_empty()
    }

    /// Whether query VERIFICATION is configured. Without it, rows legitimately
    /// stop at `posthog_accepted` — the relay never renames acceptance.
    pub fn verification_configured(&self) -> bool {
        self.posthog_host.is_some()
            && self.posthog_project_id.is_some()
            && !self.posthog_query_api_key.expose_secret().is_empty()
    }

    /// The project-scoped HogQL endpoint used for verification.
    pub fn query_url(&self) -> Option<String> {
        let host = self.posthog_host.as_ref()?;
        let project_id = self.posthog_project_id.as_ref()?;
        Some(format!("{host}/api/projects/{project_id}/query/"))
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
