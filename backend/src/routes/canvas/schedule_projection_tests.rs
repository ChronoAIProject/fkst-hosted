//! Pure projection coverage: what the dashboard shows, derived only from the
//! facts the clock also reads.

use std::sync::OnceLock;

use k8s_openapi::chrono::TimeZone;

use crate::goals::scheduled_workflow_parse::parse_scheduled_workflow;
use crate::reconcile::reserved_labels::{CRON_PAUSED_LABEL, CRON_RUNNING_LABEL};
use crate::schedule::{RunRecord, RunStep, StepStatus};

use super::*;

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, day, hour, 0, 0)
        .single()
        .expect("valid timestamp")
}

fn spec(run_mode: &str) -> ScheduledWorkflowSpec {
    parse_scheduled_workflow(&format!(
        "### Workflow\nsourcing\n\n### Run Mode\n{run_mode}\n\n### Arguments\nrole: engineer\n"
    ))
    .expect("valid definition")
}

/// The default assignment: one login, which is the only routable shape.
fn sole_assignee() -> &'static [String] {
    static ASSIGNEES: OnceLock<Vec<String>> = OnceLock::new();
    ASSIGNEES.get_or_init(|| vec!["shining".to_string()])
}

/// The routable shape: exactly one assignee, so the definition belongs to a
/// session. [`assigned_facts`] varies that when the assignment itself is the
/// subject.
fn facts<'a>(
    spec: &'a ScheduledWorkflowSpec,
    labels: &'a [String],
    records: &'a [RunRecord],
) -> ScheduleFacts<'a> {
    assigned_facts(spec, labels, records, sole_assignee())
}

fn assigned_facts<'a>(
    spec: &'a ScheduledWorkflowSpec,
    labels: &'a [String],
    records: &'a [RunRecord],
    assignees: &'a [String],
) -> ScheduleFacts<'a> {
    ScheduleFacts {
        schedule_issue: 50,
        title: "nightly sourcing",
        html_url: "https://github.com/acme/site/issues/50",
        labels,
        assignees,
        created_at: at(27, 0),
        spec: Ok(spec),
        records,
    }
}

fn labels(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

#[test]
fn an_idle_schedule_shows_its_cadence_and_next_firing() {
    let spec = spec("cron: 0 1 * * 1-5");
    let summary = summarize(&facts(&spec, &[], &[]), at(30, 12));
    assert_eq!(summary.cadence, "weekdays at 01:00 UTC");
    assert_eq!(summary.run_mode, "cron: 0 1 * * 1-5");
    assert_eq!(summary.state, ScheduleLifecycle::Idle);
    // 2026-07-30 is a Thursday, so the next weekday firing is Friday the 31st.
    assert_eq!(summary.next_due.as_deref(), Some("2026-07-31T01:00:00Z"));
    assert_eq!(summary.last_run, None);
    assert_eq!(summary.success_rate_30d, None);
}

#[test]
fn the_next_firing_is_computed_with_the_clocks_own_arithmetic() {
    // Not a second implementation: a dashboard that disagreed with the clock about
    // when a schedule fires would be worse than one that showed nothing.
    let spec = spec("cron: 0 1 * * 1-5");
    let detail = detail(&facts(&spec, &[], &[]), at(30, 12));
    assert_eq!(
        detail.upcoming,
        vec![
            "2026-07-31T01:00:00Z",
            "2026-08-03T01:00:00Z",
            "2026-08-04T01:00:00Z",
            "2026-08-05T01:00:00Z",
            "2026-08-06T01:00:00Z",
        ],
        "five real firings, weekend skipped"
    );
    assert_eq!(detail.arguments["role"], "engineer");
}

#[test]
fn a_paused_schedule_reads_paused_even_while_a_run_finishes() {
    // To its operator, a paused schedule is paused; the run still finishing is a
    // detail of the previous firing, not the schedule's current intent.
    let spec = spec("cron: 0 1 * * *");
    let names = labels(&[CRON_PAUSED_LABEL, CRON_RUNNING_LABEL]);
    assert_eq!(
        summarize(&facts(&spec, &names, &[]), at(27, 2)).state,
        ScheduleLifecycle::Paused
    );
}

#[test]
fn a_running_schedule_reads_running() {
    let spec = spec("cron: 0 1 * * *");
    let names = labels(&[CRON_RUNNING_LABEL]);
    assert_eq!(
        summarize(&facts(&spec, &names, &[]), at(27, 2)).state,
        ScheduleLifecycle::Running
    );
}

#[test]
fn an_unparseable_definition_surfaces_its_reason_rather_than_vanishing() {
    // A broken schedule the dashboard silently omitted would be invisible until
    // someone noticed it had stopped running.
    let facts = ScheduleFacts {
        schedule_issue: 50,
        title: "broken",
        html_url: "https://github.com/acme/site/issues/50",
        labels: &[],
        assignees: sole_assignee(),
        created_at: at(27, 0),
        spec: Err("missing required section `### Run Mode`".to_string()),
        records: &[],
    };
    let summary = summarize(&facts, at(27, 2));
    assert_eq!(summary.state, ScheduleLifecycle::Invalid);
    assert_eq!(
        summary.invalid_detail.as_deref(),
        Some("missing required section `### Run Mode`")
    );
    assert_eq!(summary.next_due, None);
}

#[test]
fn a_one_shot_definition_stops_showing_a_next_firing_once_it_has_run() {
    let spec = spec("once");
    let before = summarize(&facts(&spec, &[], &[]), at(27, 2));
    assert_eq!(before.cadence, "once");
    assert_eq!(before.next_due.as_deref(), Some("2026-07-27T00:00:00Z"));

    let records = vec![RunRecord::new(at(27, 0), RunStatus::Ok, at(27, 0))];
    let after = summarize(&facts(&spec, &[], &records), at(27, 2));
    assert_eq!(after.next_due, None);
    assert!(detail(&facts(&spec, &[], &records), at(27, 2))
        .upcoming
        .is_empty());
}

#[test]
fn the_run_history_is_newest_first_with_each_slot_collapsed_to_its_latest_record() {
    // A slot has both a `dispatched` record from the clock and a terminal record
    // from the pod; the history must show the outcome, not the dispatch.
    let spec = spec("cron: 0 * * * *");
    let records = vec![
        RunRecord::new(at(27, 1), RunStatus::Dispatched, at(27, 1)),
        RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1)),
        RunRecord::new(at(27, 2), RunStatus::Dispatched, at(27, 2)),
        RunRecord::new(at(27, 2), RunStatus::Failed, at(27, 2)),
        RunRecord::new(at(27, 3), RunStatus::Dispatched, at(27, 3)),
    ];
    let runs = detail(&facts(&spec, &[], &records), at(27, 3)).runs;
    let seen: Vec<(&str, &str)> = runs
        .iter()
        .map(|run| (run.slot.as_str(), run.status.as_str()))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("2026-07-27T03:00:00Z", "dispatched"),
            ("2026-07-27T02:00:00Z", "failed"),
            ("2026-07-27T01:00:00Z", "ok"),
        ]
    );
}

#[test]
fn a_runs_duration_is_derived_from_its_timestamps() {
    let spec = spec("cron: 0 * * * *");
    let records = vec![RunRecord {
        ended: Some(at(27, 1) + Duration::seconds(742)),
        ..RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1))
    }];
    let last = summarize(&facts(&spec, &[], &records), at(27, 2))
        .last_run
        .expect("a run");
    assert_eq!(last.duration_s, Some(742));
    assert_eq!(last.ended_at.as_deref(), Some("2026-07-27T01:12:22Z"));
}

#[test]
fn the_success_rate_ignores_overlap_skips_and_in_flight_runs() {
    // A busy schedule that correctly skips overlapping slots must not read as
    // unhealthy for doing exactly the right thing.
    let spec = spec("cron: 0 * * * *");
    let records = vec![
        RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1)),
        RunRecord::new(at(27, 2), RunStatus::Failed, at(27, 2)),
        RunRecord::new(at(27, 3), RunStatus::SkippedOverlap, at(27, 3)),
        RunRecord::new(at(27, 4), RunStatus::Dispatched, at(27, 4)),
    ];
    let rate = summarize(&facts(&spec, &[], &records), at(27, 5))
        .success_rate_30d
        .expect("two attempts");
    assert!((rate - 0.5).abs() < f32::EPSILON, "{rate}");
}

#[test]
fn the_success_rate_is_absent_when_nothing_has_been_attempted_in_the_window() {
    let spec = spec("cron: 0 * * * *");
    // A run 40 days before "now" is outside the 30-day window.
    let records = vec![RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1))];
    let summary = summarize(&facts(&spec, &[], &records), at(27, 1) + Duration::days(40));
    assert_eq!(summary.success_rate_30d, None);
}

#[test]
fn a_runs_step_outcomes_project_including_the_step_that_never_ran() {
    let records = vec![
        RunRecord::new(at(27, 1), RunStatus::Dispatched, at(27, 1)).with_issue(4242),
        RunRecord {
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
            ..RunRecord::new(at(27, 1), RunStatus::Failed, at(27, 1))
        },
    ];
    let run = run_detail(&records, at(27, 1), at(27, 2)).expect("the slot has a run");
    assert_eq!(run.run.status, "failed");
    assert_eq!(
        run.steps
            .iter()
            .map(|step| (step.index, step.status.as_str(), step.duration_s))
            .collect::<Vec<_>>(),
        vec![
            (1, "ok", Some(41)),
            (2, "failed", Some(9)),
            (3, "skipped", None),
        ]
    );
    assert_eq!(
        run.run_issue,
        Some(4242),
        "the run issue is recorded on the DISPATCH, so it is recovered across the \
         whole slot rather than from the terminal record alone"
    );
}

#[test]
fn an_unknown_slot_has_no_run_detail() {
    let records = vec![RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1))];
    assert!(run_detail(&records, at(27, 9), at(27, 10)).is_none());
}

#[test]
fn a_definitions_sole_assignee_is_the_session_it_belongs_to() {
    let spec = spec("cron: 0 * * * *");
    let summary = summarize(&facts(&spec, &[], &[]), at(27, 2));
    assert_eq!(
        summary.creator.as_deref(),
        Some("shining"),
        "a schedule is grouped under the session whose creator its runs route to"
    );
}

#[test]
fn a_definition_with_no_single_assignee_belongs_to_no_session() {
    // Zero or several assignees is not a display gap — it is the unroutable case
    // the reconciler refuses, and picking one of two would show the schedule under
    // a session that will never work it.
    let spec = spec("cron: 0 * * * *");
    let none: Vec<String> = Vec::new();
    assert_eq!(
        summarize(&assigned_facts(&spec, &[], &[], &none), at(27, 2)).creator,
        None
    );
    let two = labels(&["shining", "someone-else"]);
    assert_eq!(
        summarize(&assigned_facts(&spec, &[], &[], &two), at(27, 2)).creator,
        None
    );
}

#[test]
fn the_newest_runs_steps_ride_along_on_the_detail() {
    // The stepper must be reachable without a second request and a second click,
    // so the detail carries the newest slot already collapsed and projected.
    let spec = spec("cron: 0 * * * *");
    let records = vec![
        RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1)),
        RunRecord::new(at(27, 2), RunStatus::Dispatched, at(27, 2)).with_issue(4242),
        RunRecord {
            steps: vec![RunStep {
                index: 1,
                id: "scrape".to_string(),
                status: StepStatus::Ok,
                duration_s: Some(41),
            }],
            ..RunRecord::new(at(27, 2), RunStatus::Ok, at(27, 2))
        },
    ];
    let latest = detail(&facts(&spec, &[], &records), at(27, 3))
        .latest_run
        .expect("the newest slot has a run");
    assert_eq!(latest.run.slot, "2026-07-27T02:00:00Z");
    assert_eq!(latest.run.status, "ok", "the outcome, not the dispatch");
    assert_eq!(latest.steps.len(), 1);
    assert_eq!(
        latest.run_issue,
        Some(4242),
        "recovered across the whole slot, exactly as the run endpoint does"
    );
}

#[test]
fn a_schedule_that_has_never_run_has_no_latest_run() {
    let spec = spec("cron: 0 * * * *");
    assert!(detail(&facts(&spec, &[], &[]), at(27, 2))
        .latest_run
        .is_none());
}

#[test]
fn an_in_flight_run_reports_its_age_and_its_run_issue() {
    // While a run is in flight the runner has posted nothing yet — it writes ONE
    // record at the end — so the only honest live facts are how long it has been
    // going and which issue it is running as. Both come from the dispatch.
    let spec = spec("cron: 0 * * * *");
    let records =
        vec![RunRecord::new(at(27, 2), RunStatus::Dispatched, at(27, 2)).with_issue(4242)];
    let latest = detail(
        &facts(&spec, &[], &records),
        at(27, 2) + Duration::seconds(95),
    )
    .latest_run
    .expect("a run is in flight");
    assert_eq!(latest.run.status, "dispatched");
    assert_eq!(latest.run.elapsed_s, Some(95));
    assert_eq!(
        latest.run.duration_s, None,
        "an unfinished run has no duration; elapsed is the live fact"
    );
    assert!(
        latest.steps.is_empty(),
        "no per-step record exists mid-run, and inventing one would be a lie"
    );
    assert_eq!(latest.run_issue, Some(4242));
}

#[test]
fn a_finished_run_reports_a_duration_and_no_elapsed() {
    // The two must never both be set: a growing "elapsed" on a run that ended
    // yesterday is exactly the confusion this split avoids.
    let spec = spec("cron: 0 * * * *");
    let records = vec![RunRecord {
        ended: Some(at(27, 1) + Duration::seconds(30)),
        ..RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1))
    }];
    let last = summarize(&facts(&spec, &[], &records), at(27, 5))
        .last_run
        .expect("a run");
    assert_eq!(last.duration_s, Some(30));
    assert_eq!(last.elapsed_s, None);
}

#[test]
fn a_run_started_a_moment_in_the_future_reads_as_just_started() {
    // A writer whose clock is a second ahead must not produce a wrapped elapsed.
    let spec = spec("cron: 0 * * * *");
    let records = vec![RunRecord::new(at(27, 2), RunStatus::Dispatched, at(27, 2))];
    let latest = detail(
        &facts(&spec, &[], &records),
        at(27, 2) - Duration::seconds(3),
    )
    .latest_run
    .expect("a run is in flight");
    assert_eq!(latest.run.elapsed_s, Some(0));
}
