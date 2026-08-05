//! The `fkst-cron-run:v1` wire contract.
//!
//! [`the_rendered_marker_matches_the_pinned_wire_format`] is the one test that
//! matters most here: the pod-side workflow runner writes this exact string from a
//! different repository, so a change that looks harmless in Rust would silently
//! break completion detection in production. Pinning the literal makes such a
//! change impossible to land by accident.

use k8s_openapi::chrono::TimeZone;

use super::*;

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 27, hour, minute, 0)
        .single()
        .expect("valid timestamp")
}

#[test]
fn the_rendered_marker_matches_the_pinned_wire_format() {
    let record = RunRecord {
        slot: at(3, 0),
        manual: false,
        status: RunStatus::Ok,
        started: at(3, 0),
        ended: Some(at(3, 12)),
        issue: Some(4242),
        detail: Some("all steps completed".to_string()),
        steps: vec![
            RunStep {
                index: 1,
                id: "scrape".to_string(),
                status: StepStatus::Ok,
                duration_s: Some(41),
            },
            RunStep {
                index: 2,
                id: "score".to_string(),
                status: StepStatus::Ok,
                duration_s: Some(680),
            },
        ],
    };
    assert_eq!(
        render_marker(&record),
        "<!-- fkst-cron-run:v1 slot=\"2026-07-27T03:00:00Z\" manual=\"false\" status=\"ok\" \
         started=\"2026-07-27T03:00:00Z\" ended=\"2026-07-27T03:12:00Z\" issue=\"4242\" \
         detail=\"all steps completed\" steps=\"1:scrape:ok:41;2:score:ok:680\" -->"
    );
}

#[test]
fn every_status_round_trips() {
    for status in [
        RunStatus::Dispatched,
        RunStatus::Ok,
        RunStatus::Failed,
        RunStatus::Timeout,
        RunStatus::SkippedOverlap,
    ] {
        let record = RunRecord::new(at(3, 0), status, at(3, 0));
        let parsed = parse_marker(&render_marker(&record)).expect("round trips");
        assert_eq!(parsed.status, status);
        assert_eq!(parsed, record);
    }
}

#[test]
fn only_dispatched_is_non_terminal() {
    assert!(!RunStatus::Dispatched.is_terminal());
    for status in [
        RunStatus::Ok,
        RunStatus::Failed,
        RunStatus::Timeout,
        RunStatus::SkippedOverlap,
    ] {
        assert!(status.is_terminal(), "{status:?} ends its slot");
    }
}

#[test]
fn a_dispatched_record_carries_no_end_timestamp() {
    let record = RunRecord::new(at(3, 0), RunStatus::Dispatched, at(3, 0));
    assert_eq!(record.ended, None, "an in-flight run has not ended");
    assert_eq!(record.duration_s(), None);
    assert!(!render_marker(&record).contains("ended="));
}

#[test]
fn absent_optional_attributes_are_omitted_rather_than_emptied() {
    let record = RunRecord::new(at(3, 0), RunStatus::Ok, at(3, 0));
    let marker = render_marker(&record);
    for absent in ["issue=", "detail=", "steps="] {
        assert!(
            !marker.contains(absent),
            "{absent} must be omitted: {marker}"
        );
    }
    assert_eq!(parse_marker(&marker).expect("round trips"), record);
}

#[test]
fn parsing_tolerates_field_order_and_unknown_attributes() {
    // Forward compatibility: a newer writer may add attributes an older control
    // plane has never heard of, and that must not strand the schedule.
    let marker = "<!-- fkst-cron-run:v1 unknown=\"future\" status=\"failed\" issue=\"42\" \
                  started=\"2026-07-27T03:00:00Z\" manual=\"true\" \
                  slot=\"2026-07-27T03:00:00Z\" -->";
    let record = parse_marker(marker).expect("parses");
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.issue, Some(42));
    assert!(record.manual);
}

#[test]
fn an_unknown_status_is_rejected_rather_than_guessed() {
    let marker = "<!-- fkst-cron-run:v1 slot=\"2026-07-27T03:00:00Z\" manual=\"false\" \
                  status=\"maybe\" started=\"2026-07-27T03:00:00Z\" -->";
    let message = match parse_marker(marker) {
        Err(AppError::Unprocessable(message)) => message,
        other => panic!("expected a rejection, got {other:?}"),
    };
    assert!(message.contains("status"), "{message}");
    assert!(
        message.contains("maybe"),
        "names the offending value: {message}"
    );
}

#[test]
fn a_detail_cannot_break_out_of_the_attribute_or_the_comment() {
    // The detail is free text from a failing step: treated as hostile to the
    // enclosing format, not trusted to be well-behaved.
    let record = RunRecord::new(at(3, 0), RunStatus::Failed, at(3, 5))
        .with_detail("step \"scrape\" failed --> <script>\nsecond line");
    let marker = render_marker(&record);
    assert!(marker.ends_with("-->"), "{marker}");
    assert_eq!(marker.matches("-->").count(), 1, "one terminator: {marker}");
    assert!(!marker.contains("<script>"), "{marker}");
    assert!(
        parse_marker(&marker).is_ok(),
        "the sanitized marker re-parses"
    );
}

#[test]
fn an_overlong_detail_is_truncated() {
    let record =
        RunRecord::new(at(3, 0), RunStatus::Failed, at(3, 5)).with_detail("x".repeat(1000));
    let parsed = parse_marker(&render_marker(&record)).expect("round trips");
    assert_eq!(parsed.detail.expect("a detail survives").len(), 200);
}

#[test]
fn step_outcomes_round_trip_including_a_step_that_never_ran() {
    let record = RunRecord {
        steps: vec![
            RunStep {
                index: 1,
                id: "scrape".to_string(),
                status: StepStatus::Ok,
                duration_s: Some(41),
            },
            RunStep {
                index: 2,
                id: "score".to_string(),
                status: StepStatus::Failed,
                duration_s: Some(9),
            },
            RunStep {
                index: 3,
                id: "publish".to_string(),
                status: StepStatus::Skipped,
                duration_s: None,
            },
        ],
        ..RunRecord::new(at(3, 0), RunStatus::Failed, at(3, 1))
    };
    assert_eq!(
        parse_marker(&render_marker(&record))
            .expect("round trips")
            .steps,
        record.steps
    );
}

#[test]
fn a_malformed_step_tuple_is_skipped_without_losing_the_record() {
    // Losing the record — including its authoritative status — because one step
    // tuple was malformed would turn a display glitch into a stuck schedule.
    let marker = "<!-- fkst-cron-run:v1 slot=\"2026-07-27T03:00:00Z\" manual=\"false\" \
                  status=\"ok\" started=\"2026-07-27T03:00:00Z\" \
                  steps=\"1:scrape:ok:41;garbage;2:score:unknown-status:1;3:publish:ok:\" -->";
    let record = parse_marker(marker).expect("the record survives");
    assert_eq!(record.status, RunStatus::Ok);
    assert_eq!(record.steps.len(), 2, "{:?}", record.steps);
    assert_eq!(record.steps[1].id, "publish");
    assert_eq!(record.steps[1].duration_s, None);
}

#[test]
fn duration_is_derived_from_the_timestamps() {
    let record = RunRecord {
        ended: Some(at(3, 12)),
        ..RunRecord::new(at(3, 0), RunStatus::Ok, at(3, 0))
    };
    assert_eq!(record.duration_s(), Some(720));
}

#[test]
fn collect_records_reads_markers_out_of_whole_comment_bodies() {
    let human = format!(
        "⏱ Scheduled run started — slot 2026-07-27T03:00:00Z\n\n{}",
        render_marker(&RunRecord::new(at(3, 0), RunStatus::Dispatched, at(3, 0)))
    );
    let terminal = format!(
        "{}\n\n✅ Completed in 12m.",
        render_marker(&RunRecord {
            ended: Some(at(3, 12)),
            ..RunRecord::new(at(3, 0), RunStatus::Ok, at(3, 0))
        })
    );
    let records = collect_records(&[
        "an ordinary human comment".to_string(),
        human,
        "<!-- fkst-cron-run:v1 malformed".to_string(),
        terminal,
    ]);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].status, RunStatus::Dispatched);
    assert_eq!(records[1].status, RunStatus::Ok);
}
