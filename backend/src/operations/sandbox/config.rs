//! Typed configuration for the live sandbox inventory read path
//! (`FKST_OPERATIONS_SANDBOX_*`).
//!
//! ## Two ceilings, because they protect two different things
//!
//! `FKST_SANDBOX_INVENTORY_MAX_SOURCE_ITEMS` (issue #5674, on
//! [`crate::reconcile_config::ReconcileConfig`]) is the DEFENSIVE ceiling: it
//! bounds what one process will allocate from a runaway or foreign backend, and
//! it is evaluated on the complete fleet — before any authorization exists.
//!
//! `FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS` is the PUBLIC ceiling: it bounds
//! one serialized response and is evaluated only on rows the caller is already
//! authorized to see. Keeping them apart is what makes "a regular user is never
//! failed because of fleet rows they cannot see" expressible at all — a single
//! shared ceiling would fail exactly that caller.
//!
//! Neither ever truncates. Exceeding either is a stable
//! `503 sandbox_inventory_too_large` carrying NO count, because a count derived
//! from a fleet the caller cannot see is itself a hidden-row signal.
//!
//! ## The bounded route budget
//!
//! `FKST_OPERATIONS_SANDBOX_TIMEOUT_MS` bounds the one backend list. It sits
//! below the deployment's global request ceiling
//! (`FKST_HOSTED_REQUEST_TIMEOUT_SECS`) on purpose: an inventory read that
//! outlives the request it serves would burn a backend round trip for a client
//! that has already gone, and the honest answer to a slow fleet read is an
//! explicit `503`, not a hung request.

use std::time::Duration;

use serde::Deserialize;

use crate::error::AppError;

/// The `FKST_OPERATIONS_` envy prefix. envy drops every field it does not
/// recognize, so this pass reads only the sandbox half of the namespace.
const OPERATIONS_ENV_PREFIX: &str = "FKST_OPERATIONS_";

/// Hard ceiling on `FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS`. One response is
/// materialized and serialized in memory; an unbounded value would let a single
/// caller pin an arbitrary amount of heap per concurrent request.
const RESULT_ITEMS_CEILING: usize = 50_000;

/// Hard ceiling on `FKST_OPERATIONS_SANDBOX_TIMEOUT_MS`. The route also sits
/// under the global request timeout; a budget above this could never be observed.
const TIMEOUT_CEILING_MS: u64 = 60_000;

/// Defaults, shared by the serde defaults and [`SandboxInventoryConfig::default`].
mod defaults {
    pub(super) fn sandbox_max_result_items() -> usize {
        // The issue's stated default. Deliberately equal to the source ceiling:
        // an operator who has not thought about either gets one consistent
        // number, and the two only diverge when someone deliberately tunes them.
        5_000
    }

    pub(super) fn sandbox_timeout_ms() -> u64 {
        // One namespace-scoped Pod LIST, or one paginated sandbox walk. Five
        // seconds is generous for both and still far below the global request
        // ceiling, so a stuck backend fails as `503` rather than as a timeout the
        // caller cannot interpret.
        5_000
    }
}

/// The `FKST_OPERATIONS_SANDBOX_*` variables.
#[derive(Debug, Deserialize)]
struct SandboxVars {
    #[serde(default = "defaults::sandbox_max_result_items")]
    sandbox_max_result_items: usize,
    #[serde(default = "defaults::sandbox_timeout_ms")]
    sandbox_timeout_ms: u64,
}

/// Resolved sandbox-inventory configuration. Always present on
/// [`crate::config::Config`]: the endpoint itself is unconditional, and whether a
/// deployment can answer depends on the runtime backend, not on this block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxInventoryConfig {
    /// The largest AUTHORIZED, filter-matching result one response may carry.
    /// Env: `FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS`. Default 5000.
    pub max_result_items: usize,
    /// The bounded budget for the one backend list. Env:
    /// `FKST_OPERATIONS_SANDBOX_TIMEOUT_MS`. Default 5000.
    pub timeout_ms: u64,
}

impl Default for SandboxInventoryConfig {
    fn default() -> Self {
        Self {
            max_result_items: defaults::sandbox_max_result_items(),
            timeout_ms: defaults::sandbox_timeout_ms(),
        }
    }
}

impl SandboxInventoryConfig {
    /// Deserialize from environment-style pairs, sharing the caller's single
    /// `vars` snapshot (see [`crate::config::Config::from_vars`]).
    ///
    /// Bounds are validated unconditionally: a zero result ceiling would make
    /// every inventory read fail as oversize, silently taking the operations
    /// sandbox view down, and that is an operator mistake which must surface at
    /// deploy time rather than the first time somebody opens `/operations`.
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<Self, AppError> {
        let raw: SandboxVars = envy::prefixed(OPERATIONS_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| {
                AppError::Config(format!(
                    "FKST_OPERATIONS_SANDBOX_* configuration is invalid: {e}"
                ))
            })?;

        between(
            "FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS",
            raw.sandbox_max_result_items as u64,
            1,
            RESULT_ITEMS_CEILING as u64,
        )?;
        between(
            "FKST_OPERATIONS_SANDBOX_TIMEOUT_MS",
            raw.sandbox_timeout_ms,
            1,
            TIMEOUT_CEILING_MS,
        )?;

        Ok(Self {
            max_result_items: raw.sandbox_max_result_items,
            timeout_ms: raw.sandbox_timeout_ms,
        })
    }

    /// The bounded budget for the one backend list.
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

/// Reject an out-of-range numeric setting, naming the variable.
fn between(name: &str, value: u64, min: u64, max: u64) -> Result<(), AppError> {
    if (min..=max).contains(&value) {
        return Ok(());
    }
    Err(AppError::Config(format!(
        "{name} must be between {min} and {max}"
    )))
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
