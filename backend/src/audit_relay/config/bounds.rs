//! Startup validation: every numeric knob, every ordering rule, one place.
//!
//! It runs BEFORE anything is resolved, and unconditionally — a nonsensical
//! retention window or an inverted retry budget is an operator mistake that must
//! surface at deploy time rather than the first night a backlog builds. Each
//! message names the variable and the accepted range; none of them echoes a
//! secret, because none of the values checked here is one.

use crate::error::AppError;

use super::vars::{RelayVars, SharedAuditVars};

/// Floor on `FKST_AUDIT_RELAY_MAX_BODY_BYTES`. Below this even a minimal
/// completion body would be refused, which is a silent, total ingress outage.
const MIN_BODY_BYTES: u64 = 4_096;
/// Ceiling on the same knob: one body is buffered in memory per in-flight
/// request, and the relay is deliberately a small, bounded process.
const MAX_BODY_BYTES_CEILING: u64 = 4 * 1024 * 1024;
/// Ceiling on `FKST_AUDIT_RELAY_MAX_RECORDS`. The capacity guard exists to make
/// a full disk a refusal with a metric instead of an `SQLITE_FULL` surprise.
const MAX_RECORDS_CEILING: u64 = 50_000_000;

/// Validate every bound and ordering rule across the two numeric groups.
pub(super) fn validate(relay: &RelayVars, shared: &SharedAuditVars) -> Result<(), AppError> {
    between(
        "FKST_AUDIT_RELAY_MAX_BODY_BYTES",
        relay.max_body_bytes as u64,
        MIN_BODY_BYTES,
        MAX_BODY_BYTES_CEILING,
    )?;
    between(
        "FKST_AUDIT_RELAY_MAX_RECORDS",
        relay.max_records,
        1,
        MAX_RECORDS_CEILING,
    )?;
    at_least("FKST_AUDIT_RELAY_BUSY_TIMEOUT_MS", relay.busy_timeout_ms, 1)?;
    at_least(
        "FKST_AUDIT_RELAY_WRITER_QUEUE_CAPACITY",
        relay.writer_queue_capacity as u64,
        1,
    )?;
    between(
        "FKST_AUDIT_RELAY_MAX_READ_CONCURRENCY",
        relay.max_read_concurrency as u64,
        1,
        256,
    )?;
    between(
        "FKST_AUDIT_RELAY_MAX_READ_ROWS",
        u64::from(relay.max_read_rows),
        1,
        5_000,
    )?;
    between(
        "FKST_AUDIT_RELAY_MAX_RANGE_DAYS",
        relay.max_range_days,
        1,
        400,
    )?;
    between(
        "FKST_AUDIT_RELAY_CAPTURE_BATCH_SIZE",
        relay.capture_batch_size as u64,
        1,
        1_000,
    )?;
    between(
        "FKST_AUDIT_RELAY_MAX_CAPTURE_ATTEMPTS",
        u64::from(relay.max_capture_attempts),
        1,
        100,
    )?;
    at_least(
        "FKST_AUDIT_RELAY_RETRY_INITIAL_SECS",
        relay.retry_initial_secs,
        1,
    )?;
    at_least(
        "FKST_AUDIT_RELAY_WORKER_INTERVAL_SECS",
        relay.worker_interval_secs,
        1,
    )?;
    between(
        "FKST_AUDIT_RELAY_VERIFICATION_BATCH_SIZE",
        relay.verification_batch_size as u64,
        1,
        1_000,
    )?;
    at_least(
        "FKST_AUDIT_INCOMPLETE_GRACE_SECS",
        shared.incomplete_grace_secs,
        1,
    )?;
    at_least(
        "FKST_AUDIT_RELAY_VERIFIED_RETENTION_DAYS",
        relay.verified_retention_days,
        1,
    )?;
    ordered(
        "FKST_AUDIT_RELAY_RETRY_MAX_SECS",
        relay.retry_max_secs,
        "FKST_AUDIT_RELAY_RETRY_INITIAL_SECS",
        relay.retry_initial_secs,
    )?;
    ordered(
        "FKST_AUDIT_RELAY_VERIFICATION_MAX_AGE_SECS",
        relay.verification_max_age_secs,
        "FKST_AUDIT_RELAY_VERIFICATION_DELAY_SECS",
        relay.verification_delay_secs,
    )?;
    // Retention inside out would purge proven rows AFTER the audit floor, which
    // is the one ordering an audit trail may not get wrong.
    ordered(
        "FKST_AUDIT_RELAY_AUDIT_RETENTION_DAYS",
        relay.audit_retention_days,
        "FKST_AUDIT_RELAY_VERIFIED_RETENTION_DAYS",
        relay.verified_retention_days,
    )
}

pub(super) fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub(super) fn required_secret(var: &str, value: Option<String>) -> Result<String, AppError> {
    non_blank(value).ok_or_else(|| AppError::Config(format!("{var} must be set")))
}

fn at_least(var: &str, value: u64, floor: u64) -> Result<(), AppError> {
    if value < floor {
        return Err(AppError::Config(format!(
            "{var} must be at least {floor} (got {value})"
        )));
    }
    Ok(())
}

fn between(var: &str, value: u64, floor: u64, ceiling: u64) -> Result<(), AppError> {
    if value < floor || value > ceiling {
        return Err(AppError::Config(format!(
            "{var} must be between {floor} and {ceiling} (got {value})"
        )));
    }
    Ok(())
}

/// `high` must not be smaller than `low`.
fn ordered(high_var: &str, high: u64, low_var: &str, low: u64) -> Result<(), AppError> {
    if high < low {
        return Err(AppError::Config(format!(
            "{high_var} ({high}) must be >= {low_var} ({low})"
        )));
    }
    Ok(())
}
