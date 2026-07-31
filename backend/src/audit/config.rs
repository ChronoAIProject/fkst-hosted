//! Typed configuration for the audit pipeline and its self-hosted PostHog
//! capture sink (`FKST_POSTHOG_*` + `FKST_DEPLOYMENT_ENVIRONMENT`).
//!
//! One envy pass over the `FKST_POSTHOG_` prefix plus one over the bare `FKST_`
//! prefix (for `FKST_DEPLOYMENT_ENVIRONMENT`), mirroring the extracted-module,
//! fail-closed style of [`crate::storage::config`] and [`crate::osb_config`].
//! [`crate::config::Config::from_vars`] owns the single `vars` snapshot and hands
//! it here, so nothing reads the process environment twice.
//!
//! Two deliberate policy choices:
//!
//! - **Numeric bounds are validated unconditionally**, even with the feature off.
//!   A nonsensical batch size or an inverted retry window is an operator mistake
//!   that must surface at deploy time, not at the moment someone flips
//!   `FKST_POSTHOG_ENABLED=true` in production (the same reasoning as
//!   `FKST_POD_RATE_POOLS`/`FKST_POD_MODE` in [`crate::config`]).
//! - **Host/token shape is validated only when enabled**, because the spec's
//!   contract is "when enabled, host and project token are required": a deploy
//!   that never talks to PostHog must not be blocked by a half-staged host value.
//!
//! Secret hygiene: the project token is a [`SecretString`] and the hand-written
//! `Debug` renders it as `<redacted>`, so an accidental `{:?}` on the audit config
//! — or on the [`crate::config::Config`] embedding it — can never spill the
//! capture credential into a log. The *query*-side credentials
//! (`FKST_POSTHOG_PROJECT_ID` / `FKST_POSTHOG_QUERY_API_KEY`) are deliberately
//! absent here: they belong to the read/HogQL issue, not to capture.

use secrecy::SecretString;
use serde::Deserialize;

use crate::error::AppError;

/// Prefix shared by every PostHog capture variable. envy drops every field it
/// does not recognize, so this pass sees only the `FKST_POSTHOG_*` keys.
const POSTHOG_ENV_PREFIX: &str = "FKST_POSTHOG_";

/// Prefix for the deployment-environment name (`FKST_DEPLOYMENT_ENVIRONMENT`).
/// Bare `FKST_`, shared with the other bare passes; envy ignores the rest.
const DEPLOYMENT_ENV_PREFIX: &str = "FKST_";

/// The deployment environments in which a plaintext `http://` PostHog host is
/// tolerated. Everywhere else the capture request carries the project token, so
/// TLS is mandatory. Matched ASCII-case-insensitively after trimming.
const PLAINTEXT_HOST_ENVIRONMENTS: [&str; 2] = ["test", "local"];

/// Hard ceiling on `FKST_POSTHOG_MAX_RETRIES`. Worst-case delivery latency is
/// `max_retries * retry_max_ms`; an unbounded value would let one sick batch
/// monopolise the worker far past any shutdown deadline.
const MAX_RETRIES_CEILING: u32 = 20;

/// Floor for `FKST_POSTHOG_MAX_EVENT_BYTES`. A cap below this could not hold even
/// a minimal record's mandatory identifiers, so every event would be rejected as
/// oversized — a silent, total capture outage.
const MIN_EVENT_BYTES: usize = 4_096;

/// Default values, shared by the serde defaults and [`AuditConfig::default`].
mod defaults {
    pub(super) fn capture_timeout_ms() -> u64 {
        2_000
    }

    pub(super) fn batch_size() -> usize {
        100
    }

    pub(super) fn flush_interval_ms() -> u64 {
        1_000
    }

    pub(super) fn queue_capacity() -> usize {
        10_000
    }

    pub(super) fn max_retries() -> u32 {
        5
    }

    pub(super) fn retry_initial_ms() -> u64 {
        100
    }

    pub(super) fn retry_max_ms() -> u64 {
        5_000
    }

    pub(super) fn shutdown_flush_secs() -> u64 {
        10
    }

    pub(super) fn max_event_bytes() -> usize {
        // 64 KiB. Overflow is a delivery error plus a metric, never a silent
        // truncation of arbitrary JSON (which would corrupt the record).
        65_536
    }
}

/// The `FKST_POSTHOG_*` variables. Numerics carry serde defaults so an unset
/// environment loads the documented defaults; the presence/shape policy is
/// applied in [`AuditConfig::from_vars`].
#[derive(Debug, Deserialize)]
struct PosthogVars {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    project_token: Option<String>,
    #[serde(default = "defaults::capture_timeout_ms")]
    capture_timeout_ms: u64,
    #[serde(default = "defaults::batch_size")]
    batch_size: usize,
    #[serde(default = "defaults::flush_interval_ms")]
    flush_interval_ms: u64,
    #[serde(default = "defaults::queue_capacity")]
    queue_capacity: usize,
    #[serde(default = "defaults::max_retries")]
    max_retries: u32,
    #[serde(default = "defaults::retry_initial_ms")]
    retry_initial_ms: u64,
    #[serde(default = "defaults::retry_max_ms")]
    retry_max_ms: u64,
    #[serde(default = "defaults::shutdown_flush_secs")]
    shutdown_flush_secs: u64,
    #[serde(default = "defaults::max_event_bytes")]
    max_event_bytes: usize,
}

/// The bare `FKST_` variables this module owns (just the environment name).
#[derive(Debug, Deserialize)]
struct DeploymentVars {
    #[serde(default)]
    deployment_environment: Option<String>,
}

/// Resolved audit/PostHog-capture configuration. Always present on
/// [`crate::config::Config`]; `enabled == false` (the default) means the no-op
/// sink is installed and no worker is started.
#[derive(Clone)]
pub struct AuditConfig {
    /// Master switch. Env: `FKST_POSTHOG_ENABLED`. Default false — the control
    /// plane then makes no network call and keeps its existing behaviour.
    pub enabled: bool,
    /// Self-hosted PostHog base URL with any trailing slash normalized away.
    /// Env: `FKST_POSTHOG_HOST`. Required when enabled; HTTPS outside an
    /// explicitly `test`/`local` deployment environment; embedded userinfo is
    /// rejected (a credential in a URL leaks through every error/proxy log).
    pub host: Option<String>,
    /// PostHog project (write) token, sent as the capture payload's `api_key`.
    /// Env: `FKST_POSTHOG_PROJECT_TOKEN`. Required when enabled. A
    /// [`SecretString`]; never logged, redacted in `Debug`, and never handed to
    /// the frontend.
    pub project_token: SecretString,
    /// Per-capture-request HTTP timeout. Env:
    /// `FKST_POSTHOG_CAPTURE_TIMEOUT_MS`. Default 2000.
    pub capture_timeout_ms: u64,
    /// Events per capture batch. Env: `FKST_POSTHOG_BATCH_SIZE`. Default 100.
    pub batch_size: usize,
    /// How long a partially-filled batch waits before being sent anyway. Env:
    /// `FKST_POSTHOG_FLUSH_INTERVAL_MS`. Default 1000.
    pub flush_interval_ms: u64,
    /// Bounded admission queue depth. Env: `FKST_POSTHOG_QUEUE_CAPACITY`.
    /// Default 10000. Overflow drops the newest event with a metric — audit
    /// pressure must never block a product request.
    pub queue_capacity: usize,
    /// Maximum retry attempts after the first delivery failure. Env:
    /// `FKST_POSTHOG_MAX_RETRIES`. Default 5, ceiling [`MAX_RETRIES_CEILING`].
    pub max_retries: u32,
    /// First backoff delay. Env: `FKST_POSTHOG_RETRY_INITIAL_MS`. Default 100.
    pub retry_initial_ms: u64,
    /// Backoff ceiling, and the cap applied to a server-supplied `Retry-After`.
    /// Env: `FKST_POSTHOG_RETRY_MAX_MS`. Default 5000.
    pub retry_max_ms: u64,
    /// Deadline for the graceful drain at shutdown. Env:
    /// `FKST_POSTHOG_SHUTDOWN_FLUSH_SECS`. Default 10.
    pub shutdown_flush_secs: u64,
    /// Maximum serialized size of ONE projected event. Env:
    /// `FKST_POSTHOG_MAX_EVENT_BYTES`. Default 65536 (64 KiB), floor
    /// [`MIN_EVENT_BYTES`].
    pub max_event_bytes: usize,
    /// Deployment environment name stamped on every event as
    /// `service_environment`, and the switch that permits a plaintext host.
    /// Env: `FKST_DEPLOYMENT_ENVIRONMENT`. Never a secret; empty when unset.
    pub environment: String,
    /// Build/version identifier stamped on every event as `service_version`.
    /// Taken from the crate version — there is no environment override, so the
    /// running binary can never misreport itself.
    pub service_version: String,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: None,
            project_token: SecretString::from(String::new()),
            capture_timeout_ms: defaults::capture_timeout_ms(),
            batch_size: defaults::batch_size(),
            flush_interval_ms: defaults::flush_interval_ms(),
            queue_capacity: defaults::queue_capacity(),
            max_retries: defaults::max_retries(),
            retry_initial_ms: defaults::retry_initial_ms(),
            retry_max_ms: defaults::retry_max_ms(),
            shutdown_flush_secs: defaults::shutdown_flush_secs(),
            max_event_bytes: defaults::max_event_bytes(),
            environment: String::new(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

// Manual `Debug` that renders the project token as `<redacted>` (the config-module
// convention, mirroring `ChronoStorageConfig` / `OpensandboxConfig`).
impl std::fmt::Debug for AuditConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditConfig")
            .field("enabled", &self.enabled)
            .field("host", &self.host)
            .field("project_token", &"<redacted>")
            .field("capture_timeout_ms", &self.capture_timeout_ms)
            .field("batch_size", &self.batch_size)
            .field("flush_interval_ms", &self.flush_interval_ms)
            .field("queue_capacity", &self.queue_capacity)
            .field("max_retries", &self.max_retries)
            .field("retry_initial_ms", &self.retry_initial_ms)
            .field("retry_max_ms", &self.retry_max_ms)
            .field("shutdown_flush_secs", &self.shutdown_flush_secs)
            .field("max_event_bytes", &self.max_event_bytes)
            .field("environment", &self.environment)
            .field("service_version", &self.service_version)
            .finish()
    }
}

impl AuditConfig {
    /// Deserialize the audit configuration from environment-style pairs.
    ///
    /// Testable seam: unit tests feed explicit pairs instead of mutating the
    /// process environment; shares the caller's already-collected `vars`
    /// snapshot (see [`crate::config::Config::from_vars`]).
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<Self, AppError> {
        let raw: PosthogVars = envy::prefixed(POSTHOG_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| {
                AppError::Config(format!("FKST_POSTHOG_* configuration is invalid: {e}"))
            })?;
        let deployment: DeploymentVars = envy::prefixed(DEPLOYMENT_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| {
                AppError::Config(format!("FKST_DEPLOYMENT_ENVIRONMENT is invalid: {e}"))
            })?;
        let environment = non_blank(deployment.deployment_environment).unwrap_or_default();

        // Bounds first, unconditionally (see the module docs).
        at_least("FKST_POSTHOG_CAPTURE_TIMEOUT_MS", raw.capture_timeout_ms, 1)?;
        at_least("FKST_POSTHOG_FLUSH_INTERVAL_MS", raw.flush_interval_ms, 1)?;
        at_least("FKST_POSTHOG_RETRY_INITIAL_MS", raw.retry_initial_ms, 1)?;
        at_least(
            "FKST_POSTHOG_SHUTDOWN_FLUSH_SECS",
            raw.shutdown_flush_secs,
            1,
        )?;
        at_least("FKST_POSTHOG_BATCH_SIZE", raw.batch_size as u64, 1)?;
        at_least("FKST_POSTHOG_QUEUE_CAPACITY", raw.queue_capacity as u64, 1)?;
        at_least(
            "FKST_POSTHOG_MAX_EVENT_BYTES",
            raw.max_event_bytes as u64,
            MIN_EVENT_BYTES as u64,
        )?;
        if raw.retry_max_ms < raw.retry_initial_ms {
            return Err(AppError::Config(format!(
                "FKST_POSTHOG_RETRY_MAX_MS ({}) must be >= FKST_POSTHOG_RETRY_INITIAL_MS ({})",
                raw.retry_max_ms, raw.retry_initial_ms
            )));
        }
        if raw.max_retries > MAX_RETRIES_CEILING {
            return Err(AppError::Config(format!(
                "FKST_POSTHOG_MAX_RETRIES must be at most {MAX_RETRIES_CEILING} (got {})",
                raw.max_retries
            )));
        }
        // A batch larger than the queue could never be filled from the queue and
        // would make the flush interval the only send trigger — almost certainly
        // a typo, so name it rather than silently degrading throughput.
        if raw.batch_size > raw.queue_capacity {
            return Err(AppError::Config(format!(
                "FKST_POSTHOG_BATCH_SIZE ({}) must not exceed FKST_POSTHOG_QUEUE_CAPACITY ({})",
                raw.batch_size, raw.queue_capacity
            )));
        }

        let host = non_blank(raw.host);
        let project_token = non_blank(raw.project_token);
        let host = if raw.enabled {
            let host = host.ok_or_else(|| {
                AppError::Config(
                    "FKST_POSTHOG_HOST must be set when FKST_POSTHOG_ENABLED=true".to_string(),
                )
            })?;
            if project_token.is_none() {
                return Err(AppError::Config(
                    "FKST_POSTHOG_PROJECT_TOKEN must be set when FKST_POSTHOG_ENABLED=true"
                        .to_string(),
                ));
            }
            Some(normalize_host(&host, &environment)?)
        } else {
            // Disabled: keep whatever was staged (normalized only when it parses)
            // without judging it, so a half-prepared rollout cannot fail an
            // unrelated deploy.
            host.map(|h| h.trim_end_matches('/').to_string())
        };

        Ok(Self {
            enabled: raw.enabled,
            host,
            project_token: SecretString::from(project_token.unwrap_or_default()),
            capture_timeout_ms: raw.capture_timeout_ms,
            batch_size: raw.batch_size,
            flush_interval_ms: raw.flush_interval_ms,
            queue_capacity: raw.queue_capacity,
            max_retries: raw.max_retries,
            retry_initial_ms: raw.retry_initial_ms,
            retry_max_ms: raw.retry_max_ms,
            shutdown_flush_secs: raw.shutdown_flush_secs,
            max_event_bytes: raw.max_event_bytes,
            environment,
            service_version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }

    /// The `/capture/` endpoint for a single event, or `None` when no host is
    /// configured.
    pub fn capture_url(&self) -> Option<String> {
        self.host.as_ref().map(|host| format!("{host}/capture/"))
    }

    /// The `/batch/` endpoint for a multi-event payload, or `None` when no host
    /// is configured.
    pub fn batch_url(&self) -> Option<String> {
        self.host.as_ref().map(|host| format!("{host}/batch/"))
    }
}

/// Trim a raw env value; a blank string is treated as absent so a stray empty
/// ConfigMap value never masquerades as a real setting.
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Reject a numeric knob below its floor, naming the variable.
fn at_least(var: &str, value: u64, floor: u64) -> Result<(), AppError> {
    if value < floor {
        return Err(AppError::Config(format!(
            "{var} must be at least {floor} (got {value})"
        )));
    }
    Ok(())
}

/// Normalize + validate `FKST_POSTHOG_HOST`: strip trailing slashes, require a
/// parseable `http`/`https` URL with a host, forbid embedded userinfo, and
/// require HTTPS outside a `test`/`local` deployment environment.
fn normalize_host(raw: &str, environment: &str) -> Result<String, AppError> {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::Config(
            "FKST_POSTHOG_HOST must not be blank when FKST_POSTHOG_ENABLED=true".to_string(),
        ));
    }
    let url = reqwest::Url::parse(trimmed).map_err(|e| {
        AppError::Config(format!(
            "FKST_POSTHOG_HOST must be a valid URL when FKST_POSTHOG_ENABLED=true ({e})"
        ))
    })?;
    if url.host_str().is_none() {
        return Err(AppError::Config(
            "FKST_POSTHOG_HOST must include a host when FKST_POSTHOG_ENABLED=true".to_string(),
        ));
    }
    // A credential embedded in the URL would be copied into every reqwest error,
    // proxy access log, and metric label derived from the host.
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Config(
            "FKST_POSTHOG_HOST must not embed userinfo credentials".to_string(),
        ));
    }
    let plaintext_allowed = PLAINTEXT_HOST_ENVIRONMENTS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(environment));
    match url.scheme() {
        "https" => {}
        "http" if plaintext_allowed => {}
        "http" => {
            return Err(AppError::Config(format!(
                "FKST_POSTHOG_HOST must use https outside a {} deployment \
                 (FKST_DEPLOYMENT_ENVIRONMENT={environment:?}); the project token \
                 rides every capture request",
                PLAINTEXT_HOST_ENVIRONMENTS.join("/")
            )))
        }
        other => {
            return Err(AppError::Config(format!(
                "FKST_POSTHOG_HOST must use http(s) (got scheme {other:?})"
            )))
        }
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
