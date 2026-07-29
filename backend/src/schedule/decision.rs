use k8s_openapi::chrono::{DateTime, Utc};

use crate::error::AppError;

use super::CronJobSpec;

/// Durable scheduling state needed to decide the first run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleState {
    pub trigger_created_at: DateTime<Utc>,
}

/// The pure action selected for the current clock instant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleAction {
    Nothing,
    ExecuteRaise { slot: DateTime<Utc> },
}

/// Decide whether the first daily slot is due.
pub fn decide(
    spec: &CronJobSpec,
    state: &ScheduleState,
    now: DateTime<Utc>,
) -> Result<ScheduleAction, AppError> {
    let slot = spec.schedule.cron.next_after(state.trigger_created_at)?;
    if now < slot {
        Ok(ScheduleAction::Nothing)
    } else {
        Ok(ScheduleAction::ExecuteRaise { slot })
    }
}
