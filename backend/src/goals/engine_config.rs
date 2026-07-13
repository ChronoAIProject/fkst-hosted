//! Parser + validator for the trigger issue's `### Engine Config` section
//! (issue #472): a per-session, ALLOWLISTED subset of the fkst-substrate
//! engine's typed config registry, written as `KEY=value` lines.
//!
//! Why an allowlist and not a passthrough: the per-user environment store
//! blanket-reserves the whole `FKST_` prefix (a raw passthrough would let a
//! trigger author redirect `FKST_GITHUB_TOKEN_FILE`, the state roots, …), so
//! the ONLY safe road for engine tunables is a control-plane-validated set
//! whose every value is bounded here, before a pod ever spawns. The validated
//! map is injected as pod env by the launcher and reaches the supervise child
//! through the driver's base-env layer.
//!
//! Validation mirrors — and deliberately tightens — the engine's own rules
//! (verified against fkst-substrate `config_registry.rs` / `rate_pool.rs` /
//! `supervise/graph_scan.rs`):
//! - integers parse as `u64` (overflow must 422 here, not fail pod-side) and
//!   carry control-plane policy caps the engine itself does not enforce;
//! - durations must be FULLY `<digits><s|m|h>` — the engine checks only the
//!   suffix and its supervisor `expect()`-panics on a non-numeric prefix — and
//!   are bounded after normalization to seconds (a suffix-blind bound would
//!   let `10080h` through while meaning to cap at 7 days);
//! - the retry pair is validated TOGETHER against effective values (engine
//!   defaults filled in), because supervise startup bails when
//!   `FKST_RETRY_DEFAULT_CAP < FKST_RETRY_DEFAULT_BASE` — a base-only raise
//!   would pass per-key checks and then kill every session at startup;
//! - rate pools use the same shape as the operator's `FKST_POD_RATE_POOLS`
//!   (the launcher later tighten-merges user pools against operator defaults).
//!
//! Secret hygiene: error messages echo only the offending key/value (engine
//! config is non-secret by construction); nothing is logged here.

use std::collections::BTreeMap;

use crate::error::AppError;
use crate::goals::section_parse::{non_empty_lines, strip_html_comments};

/// Hard ceiling for `FKST_CODEX_PERMIT_SLOTS` (global codex-subprocess
/// concurrency inside one session). A compile-time constant, deliberately NOT
/// operator-configurable: the section is re-validated on every reconcile tick,
/// so a dynamic cap that an operator later lowers would silently flip a
/// previously-valid LIVE session to invalid and destroy it. Engine default is
/// 20; users may set 1..=32.
pub const MAX_CODEX_PERMIT_SLOTS: u64 = 32;

/// Ceiling for the queue/backpressure integer keys (`FKST_QUEUE_CAPACITY`,
/// `FKST_MAX_IN_FLIGHT_PER_DEPT`, `FKST_DURABLE_ADMISSION_BURST_PER_DEPT`).
const MAX_QUEUE_SHAPE: u64 = 1024;

/// Ceiling for `FKST_RETRY_DEFAULT_MAX_ATTEMPTS`.
const MAX_RETRY_ATTEMPTS: u64 = 100;

/// Duration bound after normalization to seconds: 7 days. Matches the largest
/// engine default (`FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET` = 168h).
const MAX_DURATION_SECS: u64 = 604_800;

/// Engine defaults for the retry pair, used to validate the CROSS-FIELD rule
/// (`cap >= base`) when the author sets only one of the two.
const RETRY_BASE_DEFAULT_SECS: u64 = 60; // 60s
const RETRY_CAP_DEFAULT_SECS: u64 = 1_800; // 30m

const KEY_PERMIT_SLOTS: &str = "FKST_CODEX_PERMIT_SLOTS";
const KEY_RETRY_BASE: &str = "FKST_RETRY_DEFAULT_BASE";
const KEY_RETRY_CAP: &str = "FKST_RETRY_DEFAULT_CAP";
const RATE_POOL_PREFIX: &str = "FKST_RATE_POOL_";

/// The queue/backpressure trio sharing one integer rule.
const QUEUE_SHAPE_KEYS: [&str; 3] = [
    "FKST_QUEUE_CAPACITY",
    "FKST_MAX_IN_FLIGHT_PER_DEPT",
    "FKST_DURABLE_ADMISSION_BURST_PER_DEPT",
];

/// The duration-string keys sharing one rule (the retry pair additionally gets
/// the cross-field check).
const DURATION_KEYS: [&str; 4] = [
    KEY_RETRY_BASE,
    KEY_RETRY_CAP,
    "FKST_DEPARTMENT_DEFAULT_STALL_WINDOW",
    "FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET",
];

/// Parse + validate one `### Engine Config` section body into the validated
/// `KEY=value` map the launcher injects as session env. Empty/comment-only
/// section → empty map. Every violation is a 422 that names the section, the
/// offending key (or line), and the accepted rule — mirroring the package-ref
/// parser's self-correcting error style.
pub fn parse_engine_config(block: &str) -> Result<BTreeMap<String, String>, AppError> {
    let stripped = strip_html_comments(block);
    let mut config: BTreeMap<String, String> = BTreeMap::new();

    for line in non_empty_lines(&stripped) {
        let (key, value) = line.split_once('=').ok_or_else(|| {
            AppError::Unprocessable(format!(
                "the `### Engine Config` section has a malformed line {line:?}: expected \
                 one KEY=value per line"
            ))
        })?;
        let (key, value) = (key.trim(), value.trim());
        validate_entry(key, value)?;
        if config.insert(key.to_string(), value.to_string()).is_some() {
            return Err(AppError::Unprocessable(format!(
                "the `### Engine Config` section sets {key} more than once"
            )));
        }
    }

    validate_retry_pair(&config)?;
    Ok(config)
}

/// Per-key validation against the allowlist. Anything not allowlisted is a 422
/// naming the key — including `FKST_OUTPUT_LANG`, which gets a pointer to its
/// own dedicated section.
fn validate_entry(key: &str, value: &str) -> Result<(), AppError> {
    if key == KEY_PERMIT_SLOTS {
        return validate_u64_in(key, value, 1, MAX_CODEX_PERMIT_SLOTS);
    }
    if QUEUE_SHAPE_KEYS.contains(&key) {
        return validate_u64_in(key, value, 1, MAX_QUEUE_SHAPE);
    }
    if key == "FKST_RETRY_DEFAULT_MAX_ATTEMPTS" {
        return validate_u64_in(key, value, 1, MAX_RETRY_ATTEMPTS);
    }
    if DURATION_KEYS.contains(&key) {
        return parse_duration_secs(key, value).map(|_| ());
    }
    if let Some(name) = key.strip_prefix(RATE_POOL_PREFIX) {
        return validate_rate_pool(key, name, value);
    }
    if key == "FKST_OUTPUT_LANG" {
        return Err(AppError::Unprocessable(
            "the `### Engine Config` section must not set FKST_OUTPUT_LANG: use the \
             dedicated `### Output Language` section instead"
                .to_string(),
        ));
    }
    Err(AppError::Unprocessable(format!(
        "the `### Engine Config` section sets an unsupported key {key:?}: allowed keys are \
         {KEY_PERMIT_SLOTS}, {}, FKST_RETRY_DEFAULT_MAX_ATTEMPTS, {}, and {RATE_POOL_PREFIX}<NAME>",
        QUEUE_SHAPE_KEYS.join(", "),
        DURATION_KEYS.join(", "),
    )))
}

/// `value` must be a `u64` in `min..=max` (a 422 names the key + bound).
fn validate_u64_in(key: &str, value: &str, min: u64, max: u64) -> Result<(), AppError> {
    let n: u64 = value.parse().map_err(|_| {
        AppError::Unprocessable(format!(
            "the `### Engine Config` section sets {key} to {value:?}: must be an integer \
             in {min}..={max}"
        ))
    })?;
    if n < min || n > max {
        return Err(AppError::Unprocessable(format!(
            "the `### Engine Config` section sets {key} to {value:?}: must be in {min}..={max}"
        )));
    }
    Ok(())
}

/// Parse an engine duration string (`<digits><s|m|h>`, FULL shape — the engine
/// itself checks only the suffix and its supervisor panics on a non-numeric
/// prefix) and bound it to `1..=MAX_DURATION_SECS` normalized seconds.
fn parse_duration_secs(key: &str, value: &str) -> Result<u64, AppError> {
    let err = |detail: &str| {
        AppError::Unprocessable(format!(
            "the `### Engine Config` section sets {key} to {value:?}: {detail}"
        ))
    };
    let (digits, unit) = value.split_at(value.len().saturating_sub(1));
    let per_unit: u64 = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        _ => return Err(err("must be a duration like 30s / 5m / 2h")),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(err("must be a duration like 30s / 5m / 2h"));
    }
    let n: u64 = digits.parse().map_err(|_| err("the number is too large"))?;
    let secs = n
        .checked_mul(per_unit)
        .ok_or_else(|| err("the number is too large"))?;
    if secs == 0 || secs > MAX_DURATION_SECS {
        return Err(err("must normalize to 1 second ..= 7 days"));
    }
    Ok(secs)
}

/// A user rate pool: same shape as the operator's `FKST_POD_RATE_POOLS` tokens
/// (`NAME` uppercase, not ROOT; `<burst>,<refill_per_minute>` both positive
/// u64s). The launcher later tighten-merges these against operator defaults —
/// a user pool can only THROTTLE, never widen an operator bound.
fn validate_rate_pool(key: &str, name: &str, value: &str) -> Result<(), AppError> {
    let err = |detail: &str| {
        AppError::Unprocessable(format!(
            "the `### Engine Config` section sets {key} to {value:?}: {detail}"
        ))
    };
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(err("the pool NAME must match ^[A-Z0-9_]+$"));
    }
    if name == "ROOT" {
        return Err(err(
            "ROOT is reserved (FKST_RATE_POOL_ROOT is the platform-owned ledger dir)",
        ));
    }
    let (burst, refill) = value
        .split_once(',')
        .ok_or_else(|| err("expected <burst>,<refill_per_minute>"))?;
    for (part, which) in [(burst, "burst"), (refill, "refill_per_minute")] {
        let n: u64 = part.trim().parse().map_err(|_| {
            AppError::Unprocessable(format!(
                "the `### Engine Config` section sets {key} to {value:?}: the {which} must \
                 be a positive integer"
            ))
        })?;
        if n == 0 {
            return Err(err(&format!("the {which} must be >= 1")));
        }
    }
    Ok(())
}

/// The CROSS-FIELD retry rule: supervise startup bails when the effective
/// `FKST_RETRY_DEFAULT_CAP` is below the effective `FKST_RETRY_DEFAULT_BASE`
/// (engine defaults 60s / 30m fill the unset side). Validated here so a
/// base-only raise (e.g. `FKST_RETRY_DEFAULT_BASE=1h`) is a 422 at the issue —
/// not a session that passes registration and then dies at engine startup.
fn validate_retry_pair(config: &BTreeMap<String, String>) -> Result<(), AppError> {
    let base = match config.get(KEY_RETRY_BASE) {
        Some(v) => parse_duration_secs(KEY_RETRY_BASE, v)?,
        None => RETRY_BASE_DEFAULT_SECS,
    };
    let cap = match config.get(KEY_RETRY_CAP) {
        Some(v) => parse_duration_secs(KEY_RETRY_CAP, v)?,
        None => RETRY_CAP_DEFAULT_SECS,
    };
    if cap < base {
        return Err(AppError::Unprocessable(format!(
            "the `### Engine Config` section makes the effective {KEY_RETRY_CAP} \
             ({cap}s) smaller than the effective {KEY_RETRY_BASE} ({base}s); the engine \
             refuses to start a session with cap < base (unset sides default to 60s / 30m)"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "engine_config_tests.rs"]
mod tests;
