//! The schedules API's DTOs and their PURE assembly from GitHub facts.
//!
//! The API holds no state of its own. Every field below is derived on read from
//! the same three things the reconciler's clock reads — the definition issue's
//! body, its labels, and its `fkst-cron-run:v1` marker comments — which is what
//! keeps the deployment stateless and what makes a dashboard reading and a clock
//! decision incapable of disagreeing.
//!
//! In particular `next_due` and `upcoming` are computed with the SAME
//! `CronExpr::next_after` the clock uses. A second implementation (in this module,
//! or in TypeScript) would eventually drift, and the symptom would be a dashboard
//! confidently showing a firing time the schedule does not honour.

use std::collections::BTreeMap;

use k8s_openapi::chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::goals::scheduled_workflow_parse::{RunMode, ScheduledWorkflowSpec};
use crate::schedule::{RunRecord, RunStatus, RunStep, StepStatus};

/// How many future firings the detail view previews.
const UPCOMING: usize = 5;

/// The window the success rate is computed over.
const SUCCESS_WINDOW_DAYS: i64 = 30;

/// A schedule's current lifecycle, as the dashboard shows it.
///
/// Named `ScheduleLifecycle` rather than `ScheduleState` on purpose:
/// [`crate::schedule::ScheduleState`] is the clock's durable-state INPUT and a
/// different type, and OpenAPI component names derive from the Rust identifier, so
/// two `ScheduleState`s would collide in the generated spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ScheduleLifecycle {
    /// Registered and waiting for its next slot.
    Idle,
    /// A run is in flight.
    Running,
    /// The user applied `fkst-cron-paused`.
    Paused,
    /// The definition was refused; `invalidDetail` says why.
    Invalid,
}

/// One past (or in-flight) run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    /// The scheduled instant this run belongs to (RFC 3339).
    pub slot: String,
    /// True for a dashboard "run now" rather than a clock firing.
    pub manual: bool,
    /// `dispatched` | `ok` | `failed` | `timeout` | `skipped-overlap`.
    pub status: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Derived from the timestamps, never stored, so it cannot disagree with them.
    pub duration_s: Option<u64>,
    /// How long an IN-FLIGHT run has been going, as of this response. Present only
    /// while the run has not ended — a terminal run reports `duration_s` instead,
    /// and the two are never both set.
    ///
    /// Computed server-side against the same clock the reconciler uses rather than
    /// left to the browser: a skewed client clock would render a run that started
    /// "in the future" as a negative age, and the one thing this field exists to
    /// answer is how long a run has really been going.
    pub elapsed_s: Option<u64>,
    /// The run issue this slot produced.
    pub issue: Option<u64>,
    pub detail: Option<String>,
}

/// One step's outcome within a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunStepView {
    pub index: u32,
    pub id: String,
    /// `ok` | `failed` | `skipped`.
    pub status: String,
    pub duration_s: Option<u64>,
}

/// A schedule as the list view shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSummary {
    /// The definition issue's number — this schedule's identity.
    pub schedule_issue: i64,
    pub title: String,
    pub html_url: String,
    pub workflow_id: String,
    /// The author's `### Run Mode` text, round-tripped.
    pub run_mode: String,
    /// A short human reading of the cadence, e.g. "weekdays at 01:00 UTC".
    pub cadence: String,
    pub state: ScheduleLifecycle,
    /// The definition's SOLE assignee: the session creator its run issues route to,
    /// and therefore the session this schedule belongs to.
    ///
    /// `None` when the definition has zero or several assignees. That is not a
    /// display detail — it is exactly the unroutable case, because a run issue is
    /// routed to a session by having exactly one assignee equal to that session's
    /// creator. A caller grouping schedules by session must treat `None` as
    /// "belongs to no session" rather than as "unknown".
    pub creator: Option<String>,
    /// The next firing, or null for a one-shot definition that already ran.
    pub next_due: Option<String>,
    pub last_run: Option<RunSummary>,
    /// Successful share of the terminal runs in the last 30 days, or null when
    /// there were none. Overlap skips are excluded: they are not attempts.
    pub success_rate_30d: Option<f32>,
    /// Why the definition was refused, when `state` is `invalid`.
    pub invalid_detail: Option<String>,
}

/// A schedule's full view: its definition, its upcoming firings, and its history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDetail {
    pub summary: ScheduleSummary,
    /// The next five firings, so the UI can show a real preview rather than one
    /// date the user has to extrapolate from.
    pub upcoming: Vec<String>,
    pub arguments: BTreeMap<String, String>,
    /// Newest first.
    pub runs: Vec<RunSummary>,
    /// The newest run projected WITH its per-step outcomes.
    ///
    /// Carried on the detail so the most recent run's stepper is reachable without
    /// a second request and a second click — including while the run is still in
    /// flight, where the record is the control plane's `dispatched` marker and the
    /// step list is legitimately empty (the runner posts one record at the end, so
    /// there is nothing finer to report yet). An in-flight run is recognised by
    /// `run.status == "dispatched"`; its age is `run.elapsed_s`.
    pub latest_run: Option<ScheduleRunDetail>,
}

/// One run with its per-step outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRunDetail {
    pub run: RunSummary,
    pub steps: Vec<RunStepView>,
    /// The run issue, for a link out to what actually happened.
    pub run_issue: Option<u64>,
}

/// The list response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoSchedulesResponse {
    pub owner: String,
    pub name: String,
    /// False when the App does not cover this repository for the caller; the list
    /// is then empty rather than an error, mirroring the sessions endpoint.
    pub installed: bool,
    pub schedules: Vec<ScheduleSummary>,
}

/// Everything read from GitHub for one definition.
pub struct ScheduleFacts<'a> {
    pub schedule_issue: i64,
    pub title: &'a str,
    pub html_url: &'a str,
    pub labels: &'a [String],
    /// The definition issue's assignee logins, in GitHub's order. Exactly one is
    /// the routable case (see [`ScheduleSummary::creator`]).
    pub assignees: &'a [String],
    pub created_at: DateTime<Utc>,
    /// `Ok` for an accepted definition, `Err(detail)` for a refused one.
    pub spec: Result<&'a ScheduledWorkflowSpec, String>,
    /// Bot-authored run records, in comment order.
    pub records: &'a [RunRecord],
}

/// Project one definition into its list-view summary.
pub fn summarize(facts: &ScheduleFacts<'_>, now: DateTime<Utc>) -> ScheduleSummary {
    let paused = facts
        .labels
        .iter()
        .any(|label| label == crate::reconcile::reserved_labels::CRON_PAUSED_LABEL);
    let running = facts
        .labels
        .iter()
        .any(|label| label == crate::reconcile::reserved_labels::CRON_RUNNING_LABEL);

    let (workflow_id, run_mode, cadence, next_due, invalid_detail, state) = match &facts.spec {
        Err(detail) => (
            String::new(),
            String::new(),
            String::new(),
            None,
            Some(detail.clone()),
            ScheduleLifecycle::Invalid,
        ),
        Ok(spec) => {
            let cursor = facts
                .records
                .iter()
                .map(|record| record.slot)
                .max()
                .unwrap_or(facts.created_at);
            let next = match &spec.run_mode {
                // A one-shot definition has a next firing only until it has run.
                RunMode::Once => facts.records.is_empty().then_some(facts.created_at),
                RunMode::Cron(cron) => cron.next_after(cursor.max(now)).ok(),
            };
            (
                spec.workflow_id.clone(),
                spec.run_mode.render(),
                describe(&spec.run_mode),
                next.map(timestamp),
                None,
                // Paused outranks running in the display: a paused schedule with a
                // run still finishing is, to its operator, paused.
                if paused {
                    ScheduleLifecycle::Paused
                } else if running {
                    ScheduleLifecycle::Running
                } else {
                    ScheduleLifecycle::Idle
                },
            )
        }
    };

    ScheduleSummary {
        schedule_issue: facts.schedule_issue,
        title: facts.title.to_string(),
        html_url: facts.html_url.to_string(),
        workflow_id,
        run_mode,
        cadence,
        state,
        creator: sole_assignee(facts.assignees),
        next_due,
        last_run: latest_run_record(facts.records).map(|record| summarize_run(record, now)),
        success_rate_30d: success_rate(facts.records, now),
        invalid_detail,
    }
}

/// The one assignee a definition routes through, or `None` for zero or several.
///
/// Deliberately not "the first of several": a definition with two assignees has no
/// session to run it — the reconciler refuses it for exactly that reason — and
/// silently picking one would show it under a session that will never work it.
fn sole_assignee(assignees: &[String]) -> Option<String> {
    match assignees {
        [only] if !only.is_empty() => Some(only.clone()),
        _ => None,
    }
}

/// Project one definition into its full detail view.
pub fn detail(facts: &ScheduleFacts<'_>, now: DateTime<Utc>) -> ScheduleDetail {
    let summary = summarize(facts, now);
    let (upcoming, arguments) = match &facts.spec {
        Err(_) => (Vec::new(), BTreeMap::new()),
        Ok(spec) => (upcoming(spec, facts, now), spec.arguments.clone()),
    };
    ScheduleDetail {
        summary,
        upcoming,
        arguments,
        runs: runs_newest_first(facts.records, now),
        // Projected through `run_detail`, which collapses a slot's dispatch and its
        // terminal record exactly as the run endpoint does — so the inlined view and
        // a later fetched one cannot disagree.
        latest_run: latest_run_record(facts.records)
            .and_then(|record| run_detail(facts.records, record.slot, now)),
    }
}

/// The next [`UPCOMING`] firings.
fn upcoming(
    spec: &ScheduledWorkflowSpec,
    facts: &ScheduleFacts<'_>,
    now: DateTime<Utc>,
) -> Vec<String> {
    let RunMode::Cron(cron) = &spec.run_mode else {
        // A one-shot definition's only firing is its first, and only until it runs.
        return if facts.records.is_empty() {
            vec![timestamp(facts.created_at)]
        } else {
            Vec::new()
        };
    };
    let mut cursor = facts
        .records
        .iter()
        .map(|record| record.slot)
        .max()
        .unwrap_or(facts.created_at)
        .max(now);
    let mut out = Vec::with_capacity(UPCOMING);
    for _ in 0..UPCOMING {
        match cron.next_after(cursor) {
            Ok(next) => {
                out.push(timestamp(next));
                cursor = next;
            }
            Err(_) => break,
        }
    }
    out
}

/// The run records that belong to one slot, projected with their step outcomes.
pub fn run_detail(
    records: &[RunRecord],
    slot: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<ScheduleRunDetail> {
    // Later records for a slot supersede earlier ones: a terminal record written
    // by the pod replaces the control plane's `dispatched`.
    let record = records.iter().rfind(|record| record.slot == slot)?;
    // The run issue is recorded on the DISPATCH, not necessarily on the terminal
    // record the pod wrote, so it is recovered across the whole slot.
    let run_issue = records
        .iter()
        .filter(|candidate| candidate.slot == slot)
        .find_map(|candidate| candidate.issue);
    Some(ScheduleRunDetail {
        run: summarize_run(record, now),
        steps: record.steps.iter().map(project_step).collect(),
        run_issue,
    })
}

fn project_step(step: &RunStep) -> RunStepView {
    RunStepView {
        index: step.index,
        id: step.id.clone(),
        status: match step.status {
            StepStatus::Ok => "ok",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
        }
        .to_string(),
        duration_s: step.duration_s,
    }
}

/// The run this schedule is doing, or last did.
///
/// The run IN FLIGHT wins over the newest record, using the clock's own
/// [`crate::schedule::open_dispatch`] rule rather than a second implementation of
/// it. On a busy schedule the newest record is routinely NOT the current run: when
/// a slot comes due while the previous run is still going, the control plane
/// records a terminal `skipped-overlap` for that LATER slot. Picking the newest
/// record would then report "Skipped" — with no elapsed time, no run issue and no
/// steps — for a workflow that is at that moment executing, which is precisely the
/// busy schedule where seeing the live run matters most.
///
/// With nothing in flight this is the newest record, which is the last run.
fn latest_run_record(records: &[RunRecord]) -> Option<&RunRecord> {
    crate::schedule::open_dispatch(records).or_else(|| {
        records
            .iter()
            .max_by_key(|record| (record.slot, record.started))
    })
}

/// Every run, newest first, with each slot collapsed to its latest record.
fn runs_newest_first(records: &[RunRecord], now: DateTime<Utc>) -> Vec<RunSummary> {
    let mut latest: BTreeMap<DateTime<Utc>, &RunRecord> = BTreeMap::new();
    for record in records {
        latest.insert(record.slot, record);
    }
    latest
        .into_values()
        .rev()
        .map(|record| summarize_run(record, now))
        .collect()
}

fn summarize_run(record: &RunRecord, now: DateTime<Utc>) -> RunSummary {
    RunSummary {
        slot: timestamp(record.slot),
        manual: record.manual,
        status: record.status.as_str().to_string(),
        started_at: timestamp(record.started),
        ended_at: record.ended.map(timestamp),
        duration_s: record.duration_s(),
        elapsed_s: elapsed(record, now),
        issue: record.issue,
        detail: record.detail.clone(),
    }
}

/// How long an unfinished run has been going, as of `now`.
///
/// `None` for a run that has ended — it reports `duration_s` instead, and emitting
/// both would invite a reader to show a growing "elapsed" for a run that finished
/// yesterday. A TERMINAL status counts as ended even without an `ended` timestamp:
/// the marker format tolerates a writer that omits it, and the honest reading of
/// "ok, no end time" is a finished run of unknown length, never one still going.
/// Clamped at zero so a record written a second into the future by a
/// slightly-ahead writer reads as "just started" rather than wrapping.
fn elapsed(record: &RunRecord, now: DateTime<Utc>) -> Option<u64> {
    if record.ended.is_some() || record.status.is_terminal() {
        return None;
    }
    Some((now - record.started).num_seconds().max(0) as u64)
}

/// The successful share of the terminal runs inside the window.
///
/// Overlap skips are excluded because they are not attempts: counting them would
/// make a busy schedule look unhealthy for doing exactly the right thing.
fn success_rate(records: &[RunRecord], now: DateTime<Utc>) -> Option<f32> {
    let since = now - Duration::days(SUCCESS_WINDOW_DAYS);
    let attempts: Vec<&RunRecord> = records
        .iter()
        .filter(|record| record.slot >= since)
        .filter(|record| {
            matches!(
                record.status,
                RunStatus::Ok | RunStatus::Failed | RunStatus::Timeout
            )
        })
        .collect();
    if attempts.is_empty() {
        return None;
    }
    let ok = attempts
        .iter()
        .filter(|record| record.status == RunStatus::Ok)
        .count();
    Some(ok as f32 / attempts.len() as f32)
}

fn describe(mode: &RunMode) -> String {
    match mode {
        RunMode::Once => "once".to_string(),
        RunMode::Cron(cron) => cron.describe(),
    }
}

pub(super) fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
#[path = "schedule_projection_tests.rs"]
mod tests;
