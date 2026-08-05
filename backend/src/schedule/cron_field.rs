//! One field of a five-field cron expression, parsed into a value set.
//!
//! Split out of [`super::cron`] so each file stays inside the 500-line budget and
//! so the grammar (which is where every author-facing rejection message is
//! produced) is unit-testable on its own.
//!
//! Purity: no clock reads, no I/O. A [`CronField`] is a value set plus the single
//! bit [`CronField::is_restricted`] that the day-of-month / day-of-week OR rule in
//! [`super::CronExpr::next_after`] needs.

use crate::error::AppError;

/// The parsed value set of ONE cron field.
///
/// Values are held as a 64-bit mask because every field's domain fits in 0..=59.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CronField {
    mask: u64,
    restricted: bool,
}

impl CronField {
    /// True when `value` is in this field's set.
    pub(super) fn matches(&self, value: u32) -> bool {
        value < 64 && self.mask & (1u64 << value) != 0
    }

    /// Every matching value, ascending. Used to walk candidate hours/minutes in
    /// order without scanning the whole domain.
    pub(super) fn values(&self) -> impl Iterator<Item = u32> + '_ {
        (0..64u32).filter(|value| self.matches(*value))
    }

    /// True when the author narrowed the field (anything other than a bare `*`).
    ///
    /// Only the day-of-month and day-of-week fields consult this: the standard
    /// cron rule switches between AND and OR depending on which of the two the
    /// author restricted.
    pub(super) fn is_restricted(&self) -> bool {
        self.restricted
    }
}

/// The inclusive domain and author-facing name of one field.
pub(super) struct FieldSpec {
    pub(super) name: &'static str,
    pub(super) min: u32,
    pub(super) max: u32,
}

/// The five field domains, in cron order.
pub(super) const MINUTE: FieldSpec = FieldSpec {
    name: "minute",
    min: 0,
    max: 59,
};
pub(super) const HOUR: FieldSpec = FieldSpec {
    name: "hour",
    min: 0,
    max: 23,
};
pub(super) const DAY_OF_MONTH: FieldSpec = FieldSpec {
    name: "day-of-month",
    min: 1,
    max: 31,
};
pub(super) const MONTH: FieldSpec = FieldSpec {
    name: "month",
    min: 1,
    max: 12,
};
/// Day-of-week is `0..=6` with `0` = Sunday. `7` is REJECTED rather than aliased
/// to Sunday: silently accepting both spellings hides a typo in the neighbouring
/// month field (`* * * 7 *` vs `* * * * 7`) that would otherwise be caught here.
pub(super) const DAY_OF_WEEK: FieldSpec = FieldSpec {
    name: "day-of-week",
    min: 0,
    max: 6,
};

/// Parse one field of the standard grammar: `*`, a single value, a range `a-b`, a
/// comma list of those, and a step suffix `/n` on either `*` or a range.
///
/// Every rejection names both the field and the offending token, because the
/// message is surfaced verbatim to the issue author who has to self-correct.
pub(super) fn parse_field(token: &str, spec: &FieldSpec) -> Result<CronField, AppError> {
    if token.is_empty() {
        return Err(invalid(spec, token, "must not be empty"));
    }

    let mut mask = 0u64;
    let mut restricted = false;
    for item in token.split(',') {
        let (range, step) = split_step(item, spec, token)?;
        let (start, end, item_restricted) = parse_range(range, spec, token)?;
        restricted = restricted || item_restricted || step > 1;
        let mut value = start;
        while value <= end {
            mask |= 1u64 << value;
            value += step;
        }
    }

    // Unreachable through the grammar above (every branch sets at least one bit),
    // but asserted rather than assumed: an empty set would make `next_after` scan
    // its whole horizon and then report an unsatisfiable schedule, which is a far
    // more confusing message than naming the field here.
    if mask == 0 {
        return Err(invalid(spec, token, "matches no value"));
    }
    Ok(CronField { mask, restricted })
}

/// Split `a-b/n` into `("a-b", n)`. A missing suffix yields a step of 1.
fn split_step<'a>(
    item: &'a str,
    spec: &FieldSpec,
    token: &str,
) -> Result<(&'a str, u32), AppError> {
    match item.split_once('/') {
        None => Ok((item, 1)),
        Some((range, step)) => {
            let step = parse_u32(step, spec, token)?;
            if step == 0 {
                return Err(invalid(spec, token, "step must be at least 1"));
            }
            if step > spec.max.saturating_sub(spec.min) + 1 {
                return Err(invalid(
                    spec,
                    token,
                    &format!("step {step} exceeds the {}..={} domain", spec.min, spec.max),
                ));
            }
            Ok((range, step))
        }
    }
}

/// Resolve one range item to its inclusive `(start, end)` plus whether it narrows
/// the field. `*` spans the whole domain and does NOT narrow it.
fn parse_range(range: &str, spec: &FieldSpec, token: &str) -> Result<(u32, u32, bool), AppError> {
    if range == "*" {
        return Ok((spec.min, spec.max, false));
    }
    match range.split_once('-') {
        Some((start, end)) => {
            let start = parse_bounded(start, spec, token)?;
            let end = parse_bounded(end, spec, token)?;
            if start > end {
                return Err(invalid(
                    spec,
                    token,
                    &format!("range {start}-{end} is descending"),
                ));
            }
            Ok((start, end, true))
        }
        None => {
            let value = parse_bounded(range, spec, token)?;
            Ok((value, value, true))
        }
    }
}

/// Parse a single value and check it against the field's inclusive domain.
fn parse_bounded(value: &str, spec: &FieldSpec, token: &str) -> Result<u32, AppError> {
    let parsed = parse_u32(value, spec, token)?;
    if parsed < spec.min || parsed > spec.max {
        // Day-of-week gets the extra sentence because `7` is the one out-of-range
        // value users write on purpose, having learned it from a cron dialect that
        // aliases it to Sunday.
        let hint = if spec.name == DAY_OF_WEEK.name && parsed == 7 {
            " (7 is not accepted as an alias for Sunday; use 0)"
        } else {
            ""
        };
        return Err(invalid(
            spec,
            token,
            &format!(
                "value {parsed} is outside the {}..={} range{hint}",
                spec.min, spec.max
            ),
        ));
    }
    Ok(parsed)
}

fn parse_u32(value: &str, spec: &FieldSpec, token: &str) -> Result<u32, AppError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(
            spec,
            token,
            &format!("{value:?} is not a decimal number"),
        ));
    }
    value
        .parse::<u32>()
        .map_err(|_| invalid(spec, token, &format!("{value:?} is not a decimal number")))
}

/// The single rejection shape: names the field AND the offending token.
pub(super) fn invalid(spec: &FieldSpec, token: &str, detail: &str) -> AppError {
    AppError::Unprocessable(format!(
        "invalid cron {} field {token:?}: {detail}",
        spec.name
    ))
}

#[cfg(test)]
#[path = "cron_field_tests.rs"]
mod tests;
