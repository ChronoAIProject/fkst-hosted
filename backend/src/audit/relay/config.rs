//! The control plane's audit-delivery configuration (`FKST_AUDIT_*`).
//!
//! ```text
//! FKST_AUDIT_DELIVERY_MODE=disabled|best_effort|required
//! FKST_AUDIT_RELAY_URL=
//! FKST_AUDIT_RELAY_WRITE_TOKEN=              # secret
//! FKST_AUDIT_RELAY_READ_TOKEN=               # secret
//! FKST_AUDIT_RELAY_START_TIMEOUT_MS=1000
//! FKST_AUDIT_RELAY_COMPLETION_TIMEOUT_MS=5000
//! FKST_AUDIT_INCOMPLETE_GRACE_SECS=<request timeout + safety margin>
//! ```
//!
//! ## The three modes, and what each one promises
//!
//! - **`disabled`** (the default) — no relay call at all. Delivery is whatever
//!   [`crate::audit::sink`] is configured to be, which preserves today's
//!   behaviour exactly for a deployment that has not adopted the relay.
//! - **`best_effort`** — the relay is called, and a failure is counted, logged,
//!   and then IGNORED; the record falls back to the local sink so a relay outage
//!   does not silently lose it (PostHog deduplicates on the event uuid, so the
//!   overlap is safe). A response is never altered.
//! - **`required`** — the production mode. The start must be durably acknowledged
//!   BEFORE any audited handler runs, and the terminal event must be committed
//!   before the response is released. Failures become `503`s.
//!
//! ## `FKST_AUDIT_INCOMPLETE_GRACE_SECS` is shared on purpose
//!
//! The control plane writes `completion_deadline_at = started_at + grace` and the
//! relay closes a start at `completion_deadline_at + grace`. Both read the SAME
//! variable, so the writer and the closer cannot be configured to disagree; the
//! relay's second application of it is clock-skew tolerance, and erring long only
//! ever delays an incomplete record — never invents one for a request that was
//! still running.
//!
//! ## …and it must cover the LONGEST request the deployment allows
//!
//! That last sentence is only true if the grace is at least as long as the
//! request budget. It is not derivable here — the ceiling that actually applies
//! to an audited route is the `/api/v1` subtree's `TimeoutLayer`, which is built
//! from `FKST_ENV_VALIDATE_DEADLINE_SECS`, not from
//! `FKST_HOSTED_REQUEST_TIMEOUT_SECS` — so [`AuditDeliveryConfig::ensure_grace_covers`]
//! is called from [`crate::config::Config::from_vars`], where both are known.
//! Configure it too small and the relay force-closes a request that is STILL
//! RUNNING as `incomplete`; the real completion then conflicts and, in `required`
//! mode, the caller gets a `503` for work that actually succeeded. That is a
//! fabricated terminal state, which the epic forbids in either direction, so it
//! is refused at boot rather than discovered at 3am.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::error::AppError;

/// Prefix for every variable in this module.
const AUDIT_ENV_PREFIX: &str = "FKST_AUDIT_";

/// Ceiling on the pre-handler acknowledgement budget. Every audited request pays
/// it before its handler runs, so an unbounded value would let a sick relay hold
/// the whole API open.
const START_TIMEOUT_CEILING_MS: u64 = 10_000;
/// Ceiling on the completion budget. A response is held until it elapses.
const COMPLETION_TIMEOUT_CEILING_MS: u64 = 30_000;
/// Ceiling on the shared grace, matching a generous request timeout plus margin.
const GRACE_CEILING_SECS: u64 = 3_600;
/// How far the grace must exceed the longest request the deployment allows.
///
/// It absorbs the clock skew between the replica that stamped `started_at` and
/// the relay that evaluates the deadline, plus the completion call's own budget.
/// Anything smaller and a request finishing at the very edge of its timeout races
/// the sweep that would close it as `incomplete`.
const GRACE_SAFETY_MARGIN_SECS: u64 = 30;

/// How hard the control plane tries to make a record durable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AuditDeliveryMode {
    /// No relay call. The configured sink is the only delivery.
    #[default]
    Disabled,
    /// Call the relay; never let a failure change a response.
    BestEffort,
    /// Durable start before the handler, durable completion before the response.
    Required,
}

impl AuditDeliveryMode {
    pub const ALL: [AuditDeliveryMode; 3] = [
        AuditDeliveryMode::Disabled,
        AuditDeliveryMode::BestEffort,
        AuditDeliveryMode::Required,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AuditDeliveryMode::Disabled => "disabled",
            AuditDeliveryMode::BestEffort => "best_effort",
            AuditDeliveryMode::Required => "required",
        }
    }

    /// Parse the closed wire spelling.
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == normalized)
    }

    /// Whether this mode calls the relay at all.
    pub fn uses_relay(self) -> bool {
        !matches!(self, AuditDeliveryMode::Disabled)
    }
}

mod defaults {
    pub(super) fn start_timeout_ms() -> u64 {
        1_000
    }

    pub(super) fn completion_timeout_ms() -> u64 {
        5_000
    }

    pub(super) fn incomplete_grace_secs() -> u64 {
        60
    }
}

/// The raw `FKST_AUDIT_*` variables.
#[derive(Debug, Deserialize)]
struct DeliveryVars {
    #[serde(default)]
    delivery_mode: Option<String>,
    #[serde(default)]
    relay_url: Option<String>,
    #[serde(default)]
    relay_write_token: Option<String>,
    #[serde(default)]
    relay_read_token: Option<String>,
    #[serde(default = "defaults::start_timeout_ms")]
    relay_start_timeout_ms: u64,
    #[serde(default = "defaults::completion_timeout_ms")]
    relay_completion_timeout_ms: u64,
    #[serde(default = "defaults::incomplete_grace_secs")]
    incomplete_grace_secs: u64,
}

/// Resolved audit-delivery configuration. Always present on
/// [`crate::config::Config`].
#[derive(Clone)]
pub struct AuditDeliveryConfig {
    pub mode: AuditDeliveryMode,
    /// Relay base URL (ClusterIP service). Required unless the mode is disabled.
    pub relay_url: Option<String>,
    /// The write credential. Secret; redacted in `Debug`, never logged.
    pub write_token: SecretString,
    /// The read credential used by the relay activity source. Optional: capture
    /// must keep working while an operator stages the read secret.
    pub read_token: SecretString,
    pub start_timeout_ms: u64,
    pub completion_timeout_ms: u64,
    pub incomplete_grace_secs: u64,
}

impl Default for AuditDeliveryConfig {
    fn default() -> Self {
        Self {
            mode: AuditDeliveryMode::Disabled,
            relay_url: None,
            write_token: SecretString::from(String::new()),
            read_token: SecretString::from(String::new()),
            start_timeout_ms: defaults::start_timeout_ms(),
            completion_timeout_ms: defaults::completion_timeout_ms(),
            incomplete_grace_secs: defaults::incomplete_grace_secs(),
        }
    }
}

// Hand-written so neither credential can reach a log through a `{:?}`.
impl std::fmt::Debug for AuditDeliveryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditDeliveryConfig")
            .field("mode", &self.mode)
            .field("relay_url", &self.relay_url)
            .field("write_token", &"<redacted>")
            .field("read_token", &"<redacted>")
            .field("start_timeout_ms", &self.start_timeout_ms)
            .field("completion_timeout_ms", &self.completion_timeout_ms)
            .field("incomplete_grace_secs", &self.incomplete_grace_secs)
            .finish()
    }
}

impl AuditDeliveryConfig {
    /// Deserialize + validate from environment-style pairs.
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<Self, AppError> {
        let raw: DeliveryVars = envy::prefixed(AUDIT_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(format!("FKST_AUDIT_* configuration is invalid: {e}")))?;

        // Bounds first and unconditionally: a nonsensical budget is an operator
        // mistake that must surface at deploy time, not the moment somebody flips
        // the mode to `required` in production.
        between(
            "FKST_AUDIT_RELAY_START_TIMEOUT_MS",
            raw.relay_start_timeout_ms,
            1,
            START_TIMEOUT_CEILING_MS,
        )?;
        between(
            "FKST_AUDIT_RELAY_COMPLETION_TIMEOUT_MS",
            raw.relay_completion_timeout_ms,
            1,
            COMPLETION_TIMEOUT_CEILING_MS,
        )?;
        between(
            "FKST_AUDIT_INCOMPLETE_GRACE_SECS",
            raw.incomplete_grace_secs,
            1,
            GRACE_CEILING_SECS,
        )?;

        let mode = match non_blank(raw.delivery_mode) {
            None => AuditDeliveryMode::Disabled,
            Some(value) => AuditDeliveryMode::parse(&value).ok_or_else(|| {
                AppError::Config(
                    "FKST_AUDIT_DELIVERY_MODE must be disabled, best_effort, or required"
                        .to_string(),
                )
            })?,
        };
        let relay_url = non_blank(raw.relay_url)
            .map(|url| normalize_url(&url))
            .transpose()?;
        let write_token = non_blank(raw.relay_write_token).unwrap_or_default();

        // Fail closed: a mode that promises durability with no relay to talk to
        // would refuse every audited request at runtime instead of at boot.
        if mode.uses_relay() {
            if relay_url.is_none() {
                return Err(AppError::Config(format!(
                    "FKST_AUDIT_RELAY_URL must be set when FKST_AUDIT_DELIVERY_MODE={}",
                    mode.as_str()
                )));
            }
            if write_token.is_empty() {
                return Err(AppError::Config(format!(
                    "FKST_AUDIT_RELAY_WRITE_TOKEN must be set when FKST_AUDIT_DELIVERY_MODE={}",
                    mode.as_str()
                )));
            }
        }

        Ok(Self {
            mode,
            relay_url,
            write_token: SecretString::from(write_token),
            read_token: SecretString::from(non_blank(raw.relay_read_token).unwrap_or_default()),
            start_timeout_ms: raw.relay_start_timeout_ms,
            completion_timeout_ms: raw.relay_completion_timeout_ms,
            incomplete_grace_secs: raw.incomplete_grace_secs,
        })
    }

    /// Refuse a grace that does not cover the longest request the deployment
    /// allows, so a still-running request can never be closed as `incomplete`.
    ///
    /// `longest_request` is the widest ceiling an audited route can run under —
    /// see the module docs for why it cannot be derived from this module's own
    /// variables. Only checked for modes that use the relay: `disabled` never
    /// writes a `completion_deadline_at`, so its grace is inert and an unrelated
    /// deployment must not fail to boot over it.
    pub fn ensure_grace_covers(
        &self,
        longest_request: std::time::Duration,
    ) -> Result<(), AppError> {
        if !self.mode.uses_relay() {
            return Ok(());
        }
        let required = longest_request
            .as_secs()
            .saturating_add(GRACE_SAFETY_MARGIN_SECS);
        if self.incomplete_grace_secs < required {
            return Err(AppError::Config(format!(
                "FKST_AUDIT_INCOMPLETE_GRACE_SECS ({}) must be at least {required} — the longest \
                 audited request may run for {}s, and a smaller grace lets the relay close a \
                 still-running request as `incomplete`",
                self.incomplete_grace_secs,
                longest_request.as_secs(),
            )));
        }
        tracing::debug!(
            incomplete_grace_secs = self.incomplete_grace_secs,
            longest_request_secs = longest_request.as_secs(),
            "audit delivery: the incomplete grace covers the request budget"
        );
        Ok(())
    }

    /// Whether the WRITE half is usable (a URL and a write token).
    pub fn write_configured(&self) -> bool {
        self.relay_url.is_some() && !self.write_token.expose_secret().is_empty()
    }

    /// Whether the READ half is usable (a URL and a read token). Independent of
    /// the delivery mode: a `best_effort` deployment may still merge relay rows.
    pub fn read_configured(&self) -> bool {
        self.relay_url.is_some() && !self.read_token.expose_secret().is_empty()
    }
}

fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn between(var: &str, value: u64, floor: u64, ceiling: u64) -> Result<(), AppError> {
    if value < floor || value > ceiling {
        return Err(AppError::Config(format!(
            "{var} must be between {floor} and {ceiling} (got {value})"
        )));
    }
    Ok(())
}

/// Normalize the relay base URL: strip a trailing slash, require a parseable
/// `http`/`https` URL with a host, and refuse embedded userinfo.
///
/// The userinfo check is not optional even though the relay's credential rides a
/// header: a credential parked in a URL is copied into every reqwest error, proxy
/// access log, and `Debug` dump of the config.
fn normalize_url(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim_end_matches('/');
    let url = reqwest::Url::parse(trimmed)
        .map_err(|e| AppError::Config(format!("FKST_AUDIT_RELAY_URL must be a valid URL ({e})")))?;
    if url.host_str().is_none() {
        return Err(AppError::Config(
            "FKST_AUDIT_RELAY_URL must include a host".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Config(
            "FKST_AUDIT_RELAY_URL must not embed userinfo credentials".to_string(),
        ));
    }
    match url.scheme() {
        "http" | "https" => Ok(trimmed.to_string()),
        other => Err(AppError::Config(format!(
            "FKST_AUDIT_RELAY_URL must use http(s) (got scheme {other:?})"
        ))),
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
