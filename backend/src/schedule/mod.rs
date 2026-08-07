//! The pure half of scheduled workflows: cron arithmetic, the durable run-record
//! wire format, and the clock that turns both into one decision.
//!
//! Everything here is I/O-free and clock-free (`now` is always injected), so the
//! whole decision matrix is unit-testable without a cluster. The impure half — the
//! per-repository enumeration, authorization, and effects — lives in
//! [`crate::reconcile::schedule_pass`].
//!
//! The `### Schedule` / `### Job Type` TRIGGER grammar this module first shipped
//! with is gone: a schedule is now declared on a work issue
//! ([`crate::goals::scheduled_workflow_parse`]), because trigger configuration is
//! frozen at registration and a cadence nobody can edit is not a feature.

mod cron;
mod cron_field;
mod decision;
mod marker;

pub use cron::CronExpr;
pub use decision::{decide, open_dispatch, OpenDispatch, ScheduleAction, ScheduleState};
pub use marker::{
    collect_records, parse_marker, render_marker, RunRecord, RunStatus, RunStep, StepStatus,
};
