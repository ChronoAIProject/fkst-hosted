//! Typed configuration for the historical-activity READ path
//! (`FKST_POSTHOG_PROJECT_ID` / `FKST_POSTHOG_QUERY_*` /
//! `FKST_POSTHOG_ACTIVITY_*`).
//!
//! It is a second envy pass over the same `FKST_POSTHOG_` prefix
//! [`crate::audit::config`] already reads, deliberately kept in its own module:
//! capture and query are two credentials with two blast radii, and the one rule
//! that matters here is that they never become interchangeable.
//!
//! ## Why the read key is its own variable, never the project token
//!
//! `FKST_POSTHOG_PROJECT_TOKEN` is a WRITE credential that ships in every capture
//! request and is, by PostHog's design, semi-public. The query API is a READ
//! credential over the complete deployment audit dataset — every actor, every
//! session, every argument. Reusing the token as a read key would silently turn a
//! low-value ingestion credential into the key to the whole trail, so this module
//! never falls back to it: with `FKST_POSTHOG_QUERY_API_KEY` unset the activity
//! endpoint answers a stable `503 audit_query_not_configured` instead.
//!
//! Prefer a project-scoped Query Read secret where the deployment's PostHog
//! supports one; otherwise a dedicated minimum-scope service-account personal API
//! key. Either way it belongs in a Kubernetes Secret/ExternalSecret and never in a
//! ConfigMap, a log, a metric, or a response (epic `OPS-02`).
//!
//! ## Bounds are validated unconditionally
//!
//! Exactly as [`crate::audit::config`] argues: an inverted limit or a nonsensical
//! range cap is an operator mistake that must surface at deploy time, not the
//! first time somebody opens `/operations`.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::error::AppError;

/// Prefix shared with the capture pass. envy drops every field it does not
/// recognize, so the two passes read disjoint halves of the same namespace.
const POSTHOG_ENV_PREFIX: &str = "FKST_POSTHOG_";

/// Hard ceiling on `FKST_POSTHOG_ACTIVITY_MAX_LIMIT`. A page is materialized in
/// memory and serialized in one response; an unbounded value would let one query
/// pin an arbitrary amount of heap per concurrent caller.
const PAGE_LIMIT_CEILING: u32 = 1_000;

/// Hard ceiling on `FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS`. Beyond this a single
/// query scans more of the events table than any interactive request should.
const RANGE_DAYS_CEILING: u64 = 400;

/// Hard ceiling on `FKST_POSTHOG_QUERY_TIMEOUT_MS`. The route also sits under the
/// global request timeout; a query budget above it could never be observed.
const QUERY_TIMEOUT_CEILING_MS: u64 = 60_000;

/// Default values, shared by the serde defaults and [`ActivityQueryConfig::default`].
mod defaults {
    pub(super) fn query_timeout_ms() -> u64 {
        5_000
    }

    pub(super) fn activity_max_range_days() -> u64 {
        30
    }

    pub(super) fn activity_default_limit() -> u32 {
        100
    }

    pub(super) fn activity_max_limit() -> u32 {
        200
    }
}

/// The read-side `FKST_POSTHOG_*` variables.
#[derive(Debug, Deserialize)]
struct QueryVars {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    query_api_key: Option<String>,
    #[serde(default = "defaults::query_timeout_ms")]
    query_timeout_ms: u64,
    #[serde(default = "defaults::activity_max_range_days")]
    activity_max_range_days: u64,
    #[serde(default = "defaults::activity_default_limit")]
    activity_default_limit: u32,
    #[serde(default = "defaults::activity_max_limit")]
    activity_max_limit: u32,
}

/// Resolved activity-query configuration. Always present on
/// [`crate::config::Config`]; [`ActivityQueryConfig::is_configured`] is false when
/// the project id or the read key is missing, which is what the endpoint's
/// `503 audit_query_not_configured` answer is derived from.
#[derive(Clone)]
pub struct ActivityQueryConfig {
    /// PostHog numeric project id. Env: `FKST_POSTHOG_PROJECT_ID`. Not a secret,
    /// but host-internal — never handed to a browser.
    pub project_id: Option<String>,
    /// The query READ key sent as `Authorization: Bearer`. Env:
    /// `FKST_POSTHOG_QUERY_API_KEY`. A [`SecretString`]; redacted in `Debug`,
    /// never logged, never in a response.
    pub query_api_key: SecretString,
    /// Per-query HTTP budget. Env: `FKST_POSTHOG_QUERY_TIMEOUT_MS`. Default 5000.
    pub query_timeout_ms: u64,
    /// Widest `to - from` a caller may request. Env:
    /// `FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS`. Default 30.
    pub activity_max_range_days: u64,
    /// Page size when `limit` is omitted. Env:
    /// `FKST_POSTHOG_ACTIVITY_DEFAULT_LIMIT`. Default 100.
    pub activity_default_limit: u32,
    /// Largest accepted `limit`. Env: `FKST_POSTHOG_ACTIVITY_MAX_LIMIT`.
    /// Default 200.
    pub activity_max_limit: u32,
}

impl Default for ActivityQueryConfig {
    fn default() -> Self {
        Self {
            project_id: None,
            query_api_key: SecretString::from(String::new()),
            query_timeout_ms: defaults::query_timeout_ms(),
            activity_max_range_days: defaults::activity_max_range_days(),
            activity_default_limit: defaults::activity_default_limit(),
            activity_max_limit: defaults::activity_max_limit(),
        }
    }
}

// Hand-written `Debug` so the read key can never reach a log through a `{:?}` on
// the config, the state, or anything embedding them.
impl std::fmt::Debug for ActivityQueryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityQueryConfig")
            .field("project_id", &self.project_id)
            .field("query_api_key", &"<redacted>")
            .field("query_timeout_ms", &self.query_timeout_ms)
            .field("activity_max_range_days", &self.activity_max_range_days)
            .field("activity_default_limit", &self.activity_default_limit)
            .field("activity_max_limit", &self.activity_max_limit)
            .finish()
    }
}

impl ActivityQueryConfig {
    /// Deserialize from environment-style pairs, sharing the caller's single
    /// `vars` snapshot (see [`crate::config::Config::from_vars`]).
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<Self, AppError> {
        let raw: QueryVars = envy::prefixed(POSTHOG_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| {
                AppError::Config(format!(
                    "FKST_POSTHOG_* activity-query configuration is invalid: {e}"
                ))
            })?;

        between(
            "FKST_POSTHOG_QUERY_TIMEOUT_MS",
            raw.query_timeout_ms,
            1,
            QUERY_TIMEOUT_CEILING_MS,
        )?;
        between(
            "FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS",
            raw.activity_max_range_days,
            1,
            RANGE_DAYS_CEILING,
        )?;
        between(
            "FKST_POSTHOG_ACTIVITY_MAX_LIMIT",
            u64::from(raw.activity_max_limit),
            1,
            u64::from(PAGE_LIMIT_CEILING),
        )?;
        between(
            "FKST_POSTHOG_ACTIVITY_DEFAULT_LIMIT",
            u64::from(raw.activity_default_limit),
            1,
            u64::from(raw.activity_max_limit),
        )?;

        let project_id = non_blank(raw.project_id)
            .map(|id| validate_project_id(&id))
            .transpose()?;
        Ok(Self {
            project_id,
            query_api_key: SecretString::from(non_blank(raw.query_api_key).unwrap_or_default()),
            query_timeout_ms: raw.query_timeout_ms,
            activity_max_range_days: raw.activity_max_range_days,
            activity_default_limit: raw.activity_default_limit,
            activity_max_limit: raw.activity_max_limit,
        })
    }

    /// Whether BOTH read credentials are present. A half-configured pair is
    /// treated as unconfigured rather than as a startup failure: the capture side
    /// must keep working while an operator stages the read secret.
    pub fn is_configured(&self) -> bool {
        self.project_id.is_some() && !self.query_api_key.expose_secret().is_empty()
    }

    /// The project-scoped query endpoint under `host`, or `None` when the
    /// deployment has no project id.
    pub fn query_url(&self, host: &str) -> Option<String> {
        let project_id = self.project_id.as_ref()?;
        Some(format!(
            "{}/api/projects/{project_id}/query/",
            host.trim_end_matches('/')
        ))
    }
}

/// Trim a raw env value; a blank string is absent, so a stray empty ConfigMap
/// value never masquerades as a configured credential.
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Reject a numeric knob outside its documented window, naming the variable.
fn between(var: &str, value: u64, floor: u64, ceiling: u64) -> Result<(), AppError> {
    if value < floor || value > ceiling {
        return Err(AppError::Config(format!(
            "{var} must be between {floor} and {ceiling} (got {value})"
        )));
    }
    Ok(())
}

/// A PostHog project id is a bare number or a short opaque token. It is
/// interpolated into a URL PATH, so anything that could escape the segment — a
/// slash, a dot-segment, a query/fragment marker, whitespace — is refused at
/// startup rather than sanitized at request time.
fn validate_project_id(value: &str) -> Result<String, AppError> {
    let ok = value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(AppError::Config(
            "FKST_POSTHOG_PROJECT_ID must be a short alphanumeric/-/_ identifier".to_string(),
        ));
    }
    Ok(value.to_string())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
