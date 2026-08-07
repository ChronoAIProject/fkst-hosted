//! Pure planning for ONE scheduled workflow: recover its state from GitHub facts,
//! ask the clock what to do, and express the answer as effects.
//!
//! Split from [`super::schedule_pass`] (which does the enumeration, authorization
//! and reads) so the whole matrix — including the interrupted-write repairs — is
//! unit-testable without a GitHub transport.

use k8s_openapi::chrono::{DateTime, Duration, Utc};

use crate::goals::scheduled_workflow_parse::{RunMode, ScheduledWorkflowSpec};
use crate::reconcile::reserved_labels::{
    CRON_PAUSED_LABEL, CRON_RUNNING_LABEL, SCHEDULE_INVALID_LABEL,
};
use crate::reconcile::schedule_run_issue::RunIssueRequest;
use crate::reconcile_config::ReconcileConfig;
use crate::schedule::{decide, OpenDispatch, RunRecord, RunStatus, ScheduleAction, ScheduleState};

/// One effect the schedule pass wants applied to a definition issue.
///
/// Deliberately coarse — one variant per decision, not one per API call — so the
/// executor owns the write ORDER and the planner stays a pure function of GitHub
/// state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleEffect {
    /// Fire: create the run issue, latch running, record the dispatch.
    Dispatch {
        schedule_issue: i64,
        request: Box<RunIssueRequest>,
        /// Slots passed over by the misfire policy, reported in the human comment
        /// so a gap in the history is explained rather than mysterious.
        skipped: u32,
    },
    /// A slot came due mid-run: record it, touch no labels.
    RecordSkip {
        schedule_issue: i64,
        slot: DateTime<Utc>,
    },
    /// The watchdog budget elapsed: record a timeout and release the schedule.
    Expire {
        schedule_issue: i64,
        slot: DateTime<Utc>,
        started: DateTime<Utc>,
    },
    /// A terminal record arrived: drop the running latch and reflect the outcome.
    Complete {
        schedule_issue: i64,
        slot: DateTime<Utc>,
        status: RunStatus,
    },
    /// Repair: a running latch with no dispatch behind it.
    ReleaseRunning { schedule_issue: i64 },
    /// Repair: a dispatch record whose latch write did not land.
    AdoptRunning {
        schedule_issue: i64,
        slot: DateTime<Utc>,
    },
    /// The definition cannot be accepted. Clearable latch + one comment.
    FlagInvalid { schedule_issue: i64, detail: String },
    /// The definition is acceptable again.
    ClearInvalid { schedule_issue: i64 },
}

impl ScheduleEffect {
    /// The definition issue this effect targets, for logging and test assertions.
    pub fn schedule_issue(&self) -> i64 {
        match self {
            ScheduleEffect::Dispatch { schedule_issue, .. }
            | ScheduleEffect::RecordSkip { schedule_issue, .. }
            | ScheduleEffect::Expire { schedule_issue, .. }
            | ScheduleEffect::Complete { schedule_issue, .. }
            | ScheduleEffect::ReleaseRunning { schedule_issue }
            | ScheduleEffect::AdoptRunning { schedule_issue, .. }
            | ScheduleEffect::FlagInvalid { schedule_issue, .. }
            | ScheduleEffect::ClearInvalid { schedule_issue } => *schedule_issue,
        }
    }
}

/// One accepted definition, with everything read from GitHub for it.
pub struct ScheduleObservation<'a> {
    pub schedule_issue: i64,
    /// The definition issue's current labels — its own durable latches.
    pub labels: &'a [String],
    /// The recurrence anchor.
    pub created_at: DateTime<Utc>,
    pub spec: &'a ScheduledWorkflowSpec,
    /// Run records recovered from BOT-AUTHORED comments only, in comment order.
    pub records: &'a [RunRecord],
    /// The single effective work label that routes this schedule's run issues.
    pub work_label: &'a str,
    pub creator_login: &'a str,
}

/// Recover the clock's view of a definition from its labels and run records.
///
/// Both halves of the "is it running?" question are carried rather than reconciled
/// here: [`crate::schedule::decide`] owns the convergence rules for the states an
/// interrupted dispatch can leave behind.
pub fn build_state(observation: &ScheduleObservation<'_>) -> ScheduleState {
    let has_label = |name: &str| observation.labels.iter().any(|label| label == name);

    ScheduleState {
        anchor: observation.created_at,
        cursor: observation.records.iter().map(|record| record.slot).max(),
        running_label: has_label(CRON_RUNNING_LABEL),
        // Shared with the dashboard projection so the clock and the UI can never
        // disagree about which run is in flight.
        open_dispatch: crate::schedule::open_dispatch(observation.records).map(|record| {
            OpenDispatch {
                slot: record.slot,
                started: record.started,
            }
        }),
        latest_terminal: observation
            .records
            .iter()
            .filter(|record| record.status.is_terminal())
            .max_by_key(|record| record.slot)
            .map(|record| (record.slot, record.status)),
        paused: has_label(CRON_PAUSED_LABEL),
    }
}

/// Plan the effects for one accepted definition.
///
/// An accepted definition also CLEARS a stale invalid latch: the clearable-latch
/// convention is what lets an author fix a typo without recreating the issue.
pub fn plan_schedule(
    observation: &ScheduleObservation<'_>,
    now: DateTime<Utc>,
    cfg: &ReconcileConfig,
) -> Vec<ScheduleEffect> {
    let mut effects = Vec::new();
    if observation
        .labels
        .iter()
        .any(|label| label == SCHEDULE_INVALID_LABEL)
    {
        effects.push(ScheduleEffect::ClearInvalid {
            schedule_issue: observation.schedule_issue,
        });
    }

    let state = build_state(observation);
    let timeout = Duration::seconds(cfg.cron_max_runtime_secs as i64);
    let schedule_issue = observation.schedule_issue;

    match decide(&observation.spec.run_mode, &state, now, timeout) {
        ScheduleAction::Nothing => {}
        ScheduleAction::Dispatch { slot, skipped } => effects.push(ScheduleEffect::Dispatch {
            schedule_issue,
            request: Box::new(RunIssueRequest {
                schedule_issue,
                workflow_id: observation.spec.workflow_id.clone(),
                slot,
                arguments: observation.spec.arguments.clone(),
                work_label: observation.work_label.to_string(),
                creator_login: observation.creator_login.to_string(),
                manual: false,
            }),
            skipped,
        }),
        ScheduleAction::SkipOverlap { slot } => effects.push(ScheduleEffect::RecordSkip {
            schedule_issue,
            slot,
        }),
        ScheduleAction::Expire { slot, started } => effects.push(ScheduleEffect::Expire {
            schedule_issue,
            slot,
            started,
        }),
        ScheduleAction::Complete { slot, status } => effects.push(ScheduleEffect::Complete {
            schedule_issue,
            slot,
            status,
        }),
        ScheduleAction::ReleaseRunning => {
            effects.push(ScheduleEffect::ReleaseRunning { schedule_issue })
        }
        ScheduleAction::AdoptRunning { slot } => effects.push(ScheduleEffect::AdoptRunning {
            schedule_issue,
            slot,
        }),
    }
    effects
}

/// Plan the clearable invalid latch for a definition that cannot be accepted.
///
/// Deduped by the latch already on the issue, mirroring `fkst-substrate-invalid`:
/// the comment is posted once per transition, never once per sweep.
pub fn plan_invalid(schedule_issue: i64, labels: &[String], detail: String) -> Vec<ScheduleEffect> {
    if labels.iter().any(|label| label == SCHEDULE_INVALID_LABEL) {
        return Vec::new();
    }
    vec![ScheduleEffect::FlagInvalid {
        schedule_issue,
        detail,
    }]
}

/// Reject a cadence tighter than the deployment's minimum.
///
/// Returned as a rejection rather than silently slowed: an author who wrote
/// `*/1 * * * *` needs to know the deployment will not run it, not to discover
/// later that it quietly runs every fifteen minutes.
pub fn check_min_interval(mode: &RunMode, cfg: &ReconcileConfig) -> Result<(), String> {
    let RunMode::Cron(cron) = mode else {
        return Ok(());
    };
    let Some(interval) = cron.min_interval_secs() else {
        return Ok(());
    };
    if interval < cfg.cron_min_interval_secs {
        return Err(format!(
            "the cadence `{}` can fire as often as every {interval}s, but this deployment's \
             minimum is {}s. Every firing creates a run issue and boots a session pod, so \
             choose a wider cadence.",
            cron.expression(),
            cfg.cron_min_interval_secs
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "schedule_plan_tests.rs"]
mod tests;
