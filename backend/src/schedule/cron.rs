//! The complete standard five-field cron expression, in UTC.
//!
//! Pure: no I/O, no clock reads. [`CronExpr::next_after`] takes the reference
//! instant as an argument so every cadence in the milestone is unit-testable
//! without a running deployment.
//!
//! Timezones are deliberately out of scope: a schedule is anchored in UTC, and
//! [`super::spec`] rejects any other `timezone` value. That keeps the recurrence
//! arithmetic free of DST discontinuities (a local-time schedule has slots that
//! either never happen or happen twice), which is a separate design decision from
//! completing the grammar.

use k8s_openapi::chrono::{DateTime, Datelike, Days, NaiveDate, TimeZone, Utc};

use crate::error::AppError;

use super::cron_field::{parse_field, CronField, DAY_OF_MONTH, DAY_OF_WEEK, HOUR, MINUTE, MONTH};

/// How far [`CronExpr::next_after`] searches before declaring an expression
/// unsatisfiable.
///
/// The sparsest expression the grammar can express is a fixed 29 February
/// (`M H 29 2 *`). February 29 is skipped in century years that are not divisible
/// by 400, so the longest real gap between two consecutive matches is EIGHT years
/// (2096 → 2104, with 2100 skipped). A horizon shorter than that would reject a
/// legitimate leap-day schedule authored in the wrong decade; a longer one only
/// costs a slower rejection of an expression that can never match at all.
const MAX_SEARCH_DAYS: u64 = 366 * 8 + 1;

/// A parsed standard five-field cron expression: `minute hour day-of-month month
/// day-of-week`, evaluated in UTC.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CronExpr {
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
    /// The author's expression, whitespace-normalized. Kept so the schedule can be
    /// echoed back verbatim in run markers, API projections, and issue comments
    /// without a lossy re-render of the parsed value sets.
    expression: String,
}

impl CronExpr {
    /// Parse the standard five-field grammar. Each field accepts `*`, a value, a
    /// range `a-b`, a comma list, and a `/n` step on `*` or a range.
    ///
    /// Day-of-week is `0..=6` with `0` = Sunday; `7` is rejected rather than
    /// aliased (see [`super::cron_field::DAY_OF_WEEK`]).
    pub fn parse(expression: &str) -> Result<Self, AppError> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(AppError::Unprocessable(format!(
                "invalid cron expression {expression:?}: must contain exactly five \
                 whitespace-separated fields (minute hour day-of-month month day-of-week), \
                 found {}",
                fields.len()
            )));
        }

        Ok(Self {
            minute: parse_field(fields[0], &MINUTE)?,
            hour: parse_field(fields[1], &HOUR)?,
            day_of_month: parse_field(fields[2], &DAY_OF_MONTH)?,
            month: parse_field(fields[3], &MONTH)?,
            day_of_week: parse_field(fields[4], &DAY_OF_WEEK)?,
            expression: fields.join(" "),
        })
    }

    /// The whitespace-normalized expression the author wrote.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// The first matching UTC instant STRICTLY after `after`.
    ///
    /// Day selection follows the standard day-of-month / day-of-week rule, which is
    /// the single most commonly mis-implemented part of cron:
    ///
    /// - when BOTH fields are restricted, a day matches if EITHER matches (OR);
    /// - when only one is restricted, only that one applies;
    /// - when neither is restricted, every day matches.
    ///
    /// So `0 0 1 * 1` fires on the 1st of every month AND on every Monday, while
    /// `0 0 1 * *` fires only on the 1st and `0 0 * * 1` only on Mondays.
    ///
    /// An expression that cannot match within [`MAX_SEARCH_DAYS`] (for example
    /// `0 0 30 2 *`) returns an error rather than looping forever.
    pub fn next_after(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, AppError> {
        let start = after.date_naive();
        for offset in 0..MAX_SEARCH_DAYS {
            let Some(date) = start.checked_add_days(Days::new(offset)) else {
                break;
            };
            if !self.day_matches(date) {
                continue;
            }
            for hour in self.hour.values() {
                for minute in self.minute.values() {
                    let Some(naive) = date.and_hms_opt(hour, minute, 0) else {
                        continue;
                    };
                    let candidate = Utc.from_utc_datetime(&naive);
                    if candidate > after {
                        return Ok(candidate);
                    }
                }
            }
        }
        Err(AppError::Unprocessable(format!(
            "cron expression {:?} has no matching instant within {MAX_SEARCH_DAYS} days of \
             {}; it can never fire",
            self.expression,
            after.to_rfc3339()
        )))
    }

    /// Whether `date` is a day this expression fires on, applying the month filter
    /// and the day-of-month / day-of-week OR rule documented on [`Self::next_after`].
    fn day_matches(&self, date: NaiveDate) -> bool {
        if !self.month.matches(date.month()) {
            return false;
        }
        let dom = self.day_of_month.matches(date.day());
        let dow = self
            .day_of_week
            .matches(date.weekday().num_days_from_sunday());
        match (
            self.day_of_month.is_restricted(),
            self.day_of_week.is_restricted(),
        ) {
            (true, true) => dom || dow,
            (true, false) => dom,
            (false, true) => dow,
            (false, false) => true,
        }
    }
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
