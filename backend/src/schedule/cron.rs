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

    /// The latest matching UTC instant at or before `at`, or `None` when this
    /// expression has never fired within the search horizon.
    ///
    /// This is what makes the schedule pass stateless. Walking FORWARD from a
    /// definition's anchor to find the slot that has just come due would cost one
    /// iteration per elapsed slot — unbounded for a `*/15` cadence on an issue that
    /// has been open for months — so the clock instead asks "what is the most recent
    /// slot?" directly and compares it with the cursor recovered from the run
    /// history.
    pub fn previous_or_equal(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let start = at.date_naive();
        for offset in 0..MAX_SEARCH_DAYS {
            let date = start.checked_sub_days(Days::new(offset))?;
            if !self.day_matches(date) {
                continue;
            }
            // Descending, so the first candidate at or before `at` is the latest one.
            for hour in self.hour.values().rev() {
                for minute in self.minute.values().rev() {
                    let Some(naive) = date.and_hms_opt(hour, minute, 0) else {
                        continue;
                    };
                    let candidate = Utc.from_utc_datetime(&naive);
                    if candidate <= at {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    /// The shortest gap this expression can produce between consecutive firings,
    /// in seconds, used to enforce the deployment's minimum-cadence guard.
    ///
    /// Sampled from a FIXED reference instant rather than from the caller's clock so
    /// the verdict is a property of the expression alone: the same schedule must not
    /// be accepted on Monday and rejected on Tuesday. The reference is a Monday so
    /// that a weekday-restricted expression starts inside its own active window and
    /// the sample is not dominated by the weekend gap.
    ///
    /// `None` when the expression fires fewer than twice inside the sample, which
    /// means its cadence is far coarser than any plausible minimum.
    pub fn min_interval_secs(&self) -> Option<u64> {
        /// Enough firings to see the tight gaps inside an irregular list such as
        /// `0,1,30 * * * *`, and cheap: each step is one `next_after` walk.
        const SAMPLE: usize = 32;
        // 2001-01-01 was a Monday. The first firing is only the starting point:
        // measuring from the reference itself would report the distance to the
        // first slot (three hours for `0 3 * * *`) as if it were the cadence.
        let reference = Utc.with_ymd_and_hms(2001, 1, 1, 0, 0, 0).single()?;
        let mut cursor = self.next_after(reference).ok()?;
        let mut smallest: Option<u64> = None;
        for _ in 0..SAMPLE {
            let Ok(next) = self.next_after(cursor) else {
                break;
            };
            let gap = (next - cursor).num_seconds().max(0) as u64;
            smallest = Some(smallest.map_or(gap, |current: u64| current.min(gap)));
            cursor = next;
        }
        smallest
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
