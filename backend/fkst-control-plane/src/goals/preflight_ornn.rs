//! Ornn-availability half of submit-time pre-flight (#179), split from
//! [`super::preflight`] for the 500-line budget.
//!
//! After the cheap pin-FORMAT check has run upstream (`validate_pins`), this
//! confirms each pin is actually AVAILABLE in the Ornn catalog — the skill /
//! skillset exists, the requested version is present and not deprecated-only, and
//! the expanded closure has no version conflict — accumulating one [`PinError`]
//! per fault (never first-fail). All calls forward the caller's NyxID token so
//! Ornn enforces visibility; the token is SECRET and never logged, and only
//! fixed, secret-free reasons (never the upstream body) reach the 422.

use secrecy::SecretString;

use crate::error::AppError;
use crate::ornn::types::{OrnnPinKind, OrnnSkillPin};
use crate::ornn::OrnnClient;

use super::preflight::PinError;

/// Check Ornn availability for every pin. Accumulates one [`PinError`] per fault
/// (missing skill/skillset, version absent, version deprecated-only, or closure
/// conflict); returns an empty vec when all pins resolve.
pub(super) async fn check_ornn(
    ornn: &OrnnClient,
    token: Option<&SecretString>,
    pins: &[OrnnSkillPin],
) -> Vec<PinError> {
    let mut errors: Vec<PinError> = Vec::new();
    if pins.is_empty() {
        return errors;
    }

    // Availability requires the user's NyxID token (Ornn enforces visibility).
    let Some(token) = token else {
        // No forwarded token: every pin is unverifiable. Surface one entry per
        // pin so the caller still sees the full, aggregated picture.
        for pin in pins {
            errors.push(pin_error(
                pin,
                "cannot verify availability without a user token",
            ));
        }
        return errors;
    };

    // Per-pin existence + version availability.
    for pin in pins {
        if let Some(reason) = check_pin_availability(ornn, token, pin).await {
            errors.push(pin_error(pin, &reason));
        }
    }

    // Closure expansion + conflict surfacing across the whole selection. Only run
    // when no per-pin availability error already fired — a missing pin would 404
    // here too, which we already reported, so re-running adds no signal.
    if errors.is_empty() {
        if let Err(conflict_reason) = ornn.resolve_pins(token, pins).await {
            // The conflict is a selection-wide fault; attribute it to every pin so
            // the aggregated report names them all (the reason names the
            // conflicting versions).
            for pin in pins {
                errors.push(pin_error(pin, &conflict_reason_text(&conflict_reason)));
            }
        }
    }

    errors
}

/// Verify one pin's existence + version availability. Returns `Some(reason)` on a
/// fault (missing skill/skillset, version absent, or version deprecated-only),
/// `None` when the pin is available.
async fn check_pin_availability(
    ornn: &OrnnClient,
    token: &SecretString,
    pin: &OrnnSkillPin,
) -> Option<String> {
    let versions = match pin.kind {
        OrnnPinKind::Skill => ornn.skill_versions(token, &pin.name).await,
        OrnnPinKind::Skillset => ornn.skillset_versions(token, &pin.name).await,
    };

    let versions = match versions {
        Ok(rows) => rows,
        Err(AppError::NotFound(_)) => {
            return Some(format!(
                "{} {:?} not found in the Ornn catalog",
                kind_label(pin.kind),
                pin.name
            ));
        }
        Err(other) => {
            // Auth/forbidden/unavailable: cannot confirm availability. Report a
            // fixed, secret-free reason (never the upstream body).
            return Some(format!(
                "could not verify availability ({})",
                ornn_error_kind(&other)
            ));
        }
    };

    // Find the requested version among the available rows.
    let row = versions.iter().find(|row| row.version == pin.version);
    match row {
        None => Some(format!(
            "version {} is not available for {} {:?}",
            pin.version,
            kind_label(pin.kind),
            pin.name
        )),
        Some(row) if row.is_deprecated => Some(format!(
            "version {} of {} {:?} is deprecated",
            pin.version,
            kind_label(pin.kind),
            pin.name
        )),
        Some(_) => None,
    }
}

/// A short, fixed, secret-free label for an Ornn [`AppError`] (never the detail).
fn ornn_error_kind(err: &AppError) -> &'static str {
    match err {
        AppError::Unauthorized(_) => "ornn rejected the token",
        AppError::Forbidden(_) => "ornn denied access",
        AppError::RateLimited { .. } => "ornn rate limited",
        AppError::Unavailable(_) => "ornn unavailable",
        AppError::Upstream(_) => "ornn upstream error",
        _ => "ornn error",
    }
}

/// Lowercase pin-kind label for messages / [`PinError::kind`].
fn kind_label(kind: OrnnPinKind) -> &'static str {
    match kind {
        OrnnPinKind::Skill => "skill",
        OrnnPinKind::Skillset => "skillset",
    }
}

/// Render a resolve-time conflict [`AppError`] into a fixed, secret-free reason.
/// `resolve_pins` maps a [`crate::ornn::ConflictError`] into an
/// `AppError::Unprocessable` whose Display names the conflicting versions; a
/// missing-pin 404 is a `NotFound`. Either way the text is non-sensitive.
fn conflict_reason_text(err: &AppError) -> String {
    match err {
        AppError::Unprocessable(msg) => msg.clone(),
        AppError::NotFound(_) => "an Ornn pin in the closure could not be resolved".to_string(),
        other => format!(
            "Ornn closure resolution failed ({})",
            ornn_error_kind(other)
        ),
    }
}

/// Build a [`PinError`] from a pin + a reason.
fn pin_error(pin: &OrnnSkillPin, reason: &str) -> PinError {
    PinError {
        kind: kind_label(pin.kind).to_string(),
        name: pin.name.clone(),
        version: pin.version.clone(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
#[path = "preflight_ornn_tests.rs"]
mod tests;
