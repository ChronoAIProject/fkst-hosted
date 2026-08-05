//! Field-grammar coverage: every accepted form, and every rejection naming both
//! the field and the offending token.

use super::*;

fn values(token: &str, spec: &FieldSpec) -> Vec<u32> {
    parse_field(token, spec)
        .expect("field parses")
        .values()
        .collect()
}

fn reject(token: &str, spec: &FieldSpec) -> String {
    match parse_field(token, spec) {
        Err(AppError::Unprocessable(message)) => message,
        other => panic!("expected an unprocessable field error, got {other:?}"),
    }
}

#[test]
fn star_spans_the_whole_domain_without_restricting_it() {
    let field = parse_field("*", &HOUR).expect("star parses");
    assert_eq!(
        field.values().collect::<Vec<_>>(),
        (0..=23).collect::<Vec<_>>()
    );
    assert!(!field.is_restricted(), "`*` must not narrow the field");
}

#[test]
fn a_single_value_restricts_the_field() {
    let field = parse_field("3", &HOUR).expect("value parses");
    assert_eq!(field.values().collect::<Vec<_>>(), vec![3]);
    assert!(field.is_restricted());
}

#[test]
fn ranges_lists_and_steps_compose() {
    assert_eq!(values("1-5", &DAY_OF_WEEK), vec![1, 2, 3, 4, 5]);
    assert_eq!(values("1,3,5", &HOUR), vec![1, 3, 5]);
    assert_eq!(values("1-5,0", &DAY_OF_WEEK), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(values("*/15", &MINUTE), vec![0, 15, 30, 45]);
    assert_eq!(values("0-30/10", &MINUTE), vec![0, 10, 20, 30]);
    // A step over a list item applies to that item alone.
    assert_eq!(values("0-6/3,20", &HOUR), vec![0, 3, 6, 20]);
}

#[test]
fn a_step_over_star_restricts_the_field() {
    // `*/15` is not "every minute": the day-of-month / day-of-week OR rule must
    // see it as a narrowing, or `0 0 */2 * 1` would silently become Mondays-only.
    assert!(parse_field("*/2", &DAY_OF_MONTH)
        .expect("step parses")
        .is_restricted());
}

#[test]
fn day_of_month_steps_start_at_the_domain_minimum() {
    // The domain starts at 1, so `*/2` is the odd days — not 0,2,4.
    assert_eq!(
        values("*/2", &DAY_OF_MONTH),
        vec![1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31]
    );
}

#[test]
fn out_of_range_values_name_the_field_and_the_token() {
    let detail = reject("60", &MINUTE);
    assert!(detail.contains("minute"), "names the field: {detail}");
    assert!(detail.contains("\"60\""), "names the token: {detail}");
    assert!(detail.contains("0..=59"), "states the domain: {detail}");

    assert!(reject("24", &HOUR).contains("hour"));
    assert!(reject("0", &DAY_OF_MONTH).contains("day-of-month"));
    assert!(reject("13", &MONTH).contains("month"));
}

#[test]
fn day_of_week_seven_is_rejected_rather_than_aliased_to_sunday() {
    let detail = reject("7", &DAY_OF_WEEK);
    assert!(detail.contains("day-of-week"), "names the field: {detail}");
    assert!(
        detail.contains("alias for Sunday"),
        "explains the rejection: {detail}"
    );
    assert!(detail.contains("use 0"), "states the fix: {detail}");
    // The rejection must survive inside a list, not only when written alone.
    assert!(reject("1-5,7", &DAY_OF_WEEK).contains("day-of-week"));
}

#[test]
fn malformed_tokens_are_rejected_with_the_offending_text() {
    for token in ["", "x", "1-", "-5", "1..5", "1-5/", "*/x", " "] {
        let detail = reject(token, &MINUTE);
        assert!(
            detail.contains("minute"),
            "token {token:?} must name the field: {detail}"
        );
    }
}

#[test]
fn a_descending_range_is_rejected() {
    assert!(reject("5-1", &HOUR).contains("descending"));
}

#[test]
fn a_zero_or_oversized_step_is_rejected() {
    assert!(reject("*/0", &MINUTE).contains("at least 1"));
    assert!(reject("*/61", &MINUTE).contains("exceeds"));
}

#[test]
fn matches_answers_only_for_members_of_the_set() {
    let field = parse_field("1-5", &DAY_OF_WEEK).expect("range parses");
    assert!(field.matches(1) && field.matches(5));
    assert!(!field.matches(0) && !field.matches(6));
    // Out-of-domain probes must answer false rather than shift the mask.
    assert!(!field.matches(64) && !field.matches(1000));
}
