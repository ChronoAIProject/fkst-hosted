//! Expression-level coverage: the five-field shape, the day-of-month /
//! day-of-week OR rule, leap days, and the unsatisfiable-expression bound.

use k8s_openapi::chrono::TimeZone;

use super::*;

fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("valid UTC timestamp")
}

/// The first `count` firings strictly after `from`.
fn firings(expression: &str, from: DateTime<Utc>, count: usize) -> Vec<DateTime<Utc>> {
    let cron = CronExpr::parse(expression).expect("expression parses");
    let mut out = Vec::with_capacity(count);
    let mut cursor = from;
    for _ in 0..count {
        cursor = cron.next_after(cursor).expect("a firing exists");
        out.push(cursor);
    }
    out
}

fn message(expression: &str) -> String {
    match CronExpr::parse(expression) {
        Err(AppError::Unprocessable(message)) => message,
        other => panic!("expected an unprocessable cron error, got {other:?}"),
    }
}

#[test]
fn the_expression_must_have_exactly_five_fields() {
    assert!(message("0 3 * *").contains("exactly five"));
    assert!(message("0 3 * * * *").contains("exactly five"));
    // Irregular whitespace between fields is normalized, not rejected.
    let cron = CronExpr::parse("  0\t3   *  * *  ").expect("normalizes");
    assert_eq!(cron.expression(), "0 3 * * *");
}

#[test]
fn the_daily_subset_still_behaves_as_before() {
    // The pre-existing `M H * * *` contract must not regress.
    assert_eq!(
        firings("0 3 * * *", at(2026, 7, 27, 2, 0), 2),
        vec![at(2026, 7, 27, 3, 0), at(2026, 7, 28, 3, 0)]
    );
    // Strictly after: standing exactly on a slot yields the NEXT one.
    assert_eq!(
        firings("0 3 * * *", at(2026, 7, 27, 3, 0), 1),
        vec![at(2026, 7, 28, 3, 0)]
    );
}

#[test]
fn weekday_ranges_skip_the_weekend_across_a_month_boundary() {
    // 2026-07-31 is a Friday; 08-01 Saturday, 08-02 Sunday, 08-03 Monday.
    assert_eq!(
        firings("0 1 * * 1-5", at(2026, 7, 30, 12, 0), 3),
        vec![
            at(2026, 7, 31, 1, 0),
            at(2026, 8, 3, 1, 0),
            at(2026, 8, 4, 1, 0),
        ]
    );
}

#[test]
fn a_minute_step_yields_four_instants_per_hour() {
    assert_eq!(
        firings("*/15 * * * *", at(2026, 7, 27, 9, 0), 5),
        vec![
            at(2026, 7, 27, 9, 15),
            at(2026, 7, 27, 9, 30),
            at(2026, 7, 27, 9, 45),
            at(2026, 7, 27, 10, 0),
            at(2026, 7, 27, 10, 15),
        ]
    );
}

#[test]
fn day_of_month_only_restricts_by_date_alone() {
    assert_eq!(
        firings("0 0 1 * *", at(2026, 7, 27, 0, 0), 2),
        vec![at(2026, 8, 1, 0, 0), at(2026, 9, 1, 0, 0)]
    );
}

#[test]
fn day_of_week_only_restricts_by_weekday_alone() {
    // 2026-07-27 is a Monday, so the next Monday firing is a week later.
    assert_eq!(
        firings("0 0 * * 1", at(2026, 7, 27, 0, 0), 2),
        vec![at(2026, 8, 3, 0, 0), at(2026, 8, 10, 0, 0)]
    );
}

#[test]
fn restricting_both_day_fields_matches_either_one() {
    // `0 0 1 * 1` = the 1st of the month OR any Monday. Through early August 2026:
    // Aug 1 is a Saturday (matches via day-of-month), Aug 3/10 are Mondays.
    assert_eq!(
        firings("0 0 1 * 1", at(2026, 7, 27, 0, 0), 3),
        vec![
            at(2026, 8, 1, 0, 0),
            at(2026, 8, 3, 0, 0),
            at(2026, 8, 10, 0, 0),
        ]
    );
}

#[test]
fn an_unrestricted_month_field_does_not_trigger_the_or_rule() {
    // Only day-of-month is restricted, so Mondays that are not the 1st must NOT
    // fire — this is the asymmetry the OR rule is famous for getting wrong.
    let cron = CronExpr::parse("0 0 1 * *").expect("parses");
    let monday_not_the_first = at(2026, 8, 3, 0, 0);
    assert_ne!(
        cron.next_after(at(2026, 8, 2, 0, 0)).expect("a firing"),
        monday_not_the_first
    );
}

#[test]
fn month_restriction_is_always_an_and() {
    // February only, on the 1st — a Monday in March must never match.
    assert_eq!(
        firings("0 0 1 2 1", at(2026, 3, 1, 0, 0), 1),
        vec![at(2027, 2, 1, 0, 0)]
    );
}

#[test]
fn the_leap_day_is_reachable_and_skips_non_leap_years() {
    // 2028 is the next leap year after 2026.
    assert_eq!(
        firings("0 0 29 2 *", at(2026, 3, 1, 0, 0), 2),
        vec![at(2028, 2, 29, 0, 0), at(2032, 2, 29, 0, 0)]
    );
}

#[test]
fn the_leap_day_survives_a_skipped_century_year() {
    // 2100 is NOT a leap year, so the gap 2096 -> 2104 is eight years. A shorter
    // search horizon would wrongly report this schedule as unsatisfiable.
    assert_eq!(
        firings("0 0 29 2 *", at(2096, 3, 1, 0, 0), 1),
        vec![at(2104, 2, 29, 0, 0)]
    );
}

#[test]
fn an_unsatisfiable_expression_errors_rather_than_hangs() {
    let cron = CronExpr::parse("0 0 30 2 *").expect("30 February parses; it just never occurs");
    let message = match cron.next_after(at(2026, 1, 1, 0, 0)) {
        Err(AppError::Unprocessable(message)) => message,
        other => panic!("expected an unprocessable error, got {other:?}"),
    };
    assert!(message.contains("never fire"), "{message}");
    assert!(
        message.contains("0 0 30 2 *"),
        "names the expression: {message}"
    );
}

#[test]
fn field_rejections_are_surfaced_from_the_expression_parser() {
    assert!(message("60 3 * * *").contains("minute"));
    assert!(message("0 24 * * *").contains("hour"));
    assert!(message("0 3 32 * *").contains("day-of-month"));
    assert!(message("0 3 * 13 *").contains("month"));
    assert!(message("0 3 * * 7").contains("day-of-week"));
}

#[test]
fn parsing_reads_no_clock_and_performs_no_io() {
    // A compile-time-ish guard expressed as behaviour: two parses of the same
    // expression are equal, and equality includes the normalized text, so nothing
    // ambient (time, environment) can leak into the value.
    assert_eq!(
        CronExpr::parse("0 1 * * 1-5").expect("parses"),
        CronExpr::parse("0  1 *  * 1-5").expect("parses")
    );
    assert_ne!(
        CronExpr::parse("0 1 * * 1-5").expect("parses"),
        CronExpr::parse("0 1 * * 1,2,3,4,5").expect("parses"),
        "the normalized expression is part of the value, so equivalent spellings \
         stay distinguishable for round-tripping"
    );
}

#[test]
fn common_cadences_describe_readably() {
    let describe = |expression: &str| CronExpr::parse(expression).expect("parses").describe();
    assert_eq!(describe("0 3 * * *"), "daily at 03:00 UTC");
    assert_eq!(describe("30 14 * * *"), "daily at 14:30 UTC");
    assert_eq!(describe("0 1 * * 1-5"), "weekdays at 01:00 UTC");
    assert_eq!(describe("0 9 * * 0,6"), "weekends at 09:00 UTC");
    assert_eq!(describe("0 9 * * 1"), "every Monday at 09:00 UTC");
    assert_eq!(describe("*/15 * * * *"), "every 15 minutes");
    assert_eq!(describe("* * * * *"), "every minute");
}

#[test]
fn an_expression_with_no_simple_reading_falls_back_to_itself() {
    // A description that paraphrases a complex expression approximately is worse
    // than none: an operator would trust it, and the clock would do something else.
    let describe = |expression: &str| CronExpr::parse(expression).expect("parses").describe();
    assert_eq!(describe("0,1,30 * * * *"), "cron `0,1,30 * * * *` (UTC)");
    assert_eq!(describe("0 0 1 * 1"), "cron `0 0 1 * 1` (UTC)");
    assert_eq!(describe("0 3 1 2 *"), "cron `0 3 1 2 *` (UTC)");
    assert_eq!(describe("0 3,15 * * *"), "cron `0 3,15 * * *` (UTC)");
}
