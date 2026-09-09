//! The registry of PLATFORM-OWNED GitHub labels.
//!
//! Until now every label the control plane touched was either a per-session work
//! label (chosen by the trigger author or discovered from a package) or a durable
//! latch it wrote onto an issue it already owned. Nothing marked a label name as
//! reserved, and nothing stopped an author adopting one as their `### Work Label`.
//!
//! The scheduled-workflow kind changes that: [`SCHEDULED_WORKFLOW_LABEL`] is a
//! DEPLOYMENT-WIDE label whose presence selects an issue for a different parser and
//! a different lifecycle. Two consequences follow, and both are implemented here:
//!
//! 1. **An author may not claim a reserved name.** A trigger whose `### Work Label`
//!    is reserved would let any session's ordinary work queue impersonate the
//!    schedule surface, so it is refused at parse time
//!    ([`reserved_work_label_rejection`]).
//! 2. **Reserved labels are exempt from collision detection.** The collision
//!    backstop demotes two of one creator's sessions that share a label
//!    ([`crate::reconcile::collision::detect_work_label_collisions`]). A
//!    deployment-wide label is shared by construction, so without an exemption every
//!    second session a creator opens would be demoted the moment schedules exist.
//!
//! Pure: string predicates only, no I/O.

/// The reserved label that marks a WORK issue as a scheduled-workflow definition.
///
/// An issue carrying it is not ordinary work: it is never routed to a session's
/// wake gate, its body is parsed by
/// [`crate::goals::scheduled_workflow_parse`], and its lifecycle is driven by the
/// schedule pass rather than by a session pod.
pub const SCHEDULED_WORKFLOW_LABEL: &str = "fkst-scheduled-workflow";

/// The clearable DURABLE latch for a scheduled-workflow issue whose definition
/// cannot be accepted — a parse failure, an unroutable assignee, an over-cap
/// schedule, or a cadence tighter than the deployment minimum.
///
/// Clearable on purpose, and this is the one place the scheduled-workflow kind
/// deliberately differs from a trigger issue: trigger config is frozen at
/// registration, but a schedule the author cannot edit is useless. The latch is
/// re-read from GitHub each pass, so a fixed definition un-latches itself and
/// resumes without the author recreating the issue.
pub const SCHEDULE_INVALID_LABEL: &str = "fkst-schedule-invalid";

/// A scheduled run is in flight for this definition. Control-plane owned: it is
/// what makes the overlap check and the watchdog trustworthy, so no package and no
/// session pod ever writes it.
pub const CRON_RUNNING_LABEL: &str = "fkst-cron-running";

/// USER-applied pause. The control plane only ever READS this one — it is the
/// supported way to stop a schedule firing without closing its issue or editing
/// its body.
pub const CRON_PAUSED_LABEL: &str = "fkst-cron-paused";

/// The last scheduled run failed. Control-plane owned; cleared on the next
/// successful run.
pub const CRON_FAILED_LABEL: &str = "fkst-cron-failed";

/// The last scheduled run exceeded its budget and was released by the watchdog.
/// Control-plane owned; cleared on the next successful run.
pub const CRON_TIMEOUT_LABEL: &str = "fkst-cron-timeout";

/// The LOGICAL work label a one-time workflow run's issue carries, before the
/// deployment work-label namespace is applied.
///
/// A run issue is work for the workflow runner and for nothing else. It used to
/// carry the SESSION's work label, which in a deployment that mandates the devloop
/// adapters resolves to `fkst-dev` — so every run issue also looked like ordinary
/// development work and was admitted by the dev intake, which has no knowledge of
/// the run-issue marker (#5890). Giving the runner its own label family is what
/// separates the two queues, rather than teaching every other adapter to recognise
/// and decline a run.
///
/// Deliberately NOT `fkst-workflow`: that name is already `workflow-writer`'s
/// authoring queue, where an issue means "author or refine a workflow template".
/// A run issue landing there would be answered with a template pull request.
pub const WORKFLOW_RUN_LABEL: &str = "fkst-workflow-run";

/// The LOGICAL work label a CRON-scheduled run's issue carries, before the
/// deployment work-label namespace is applied.
///
/// Split from [`WORKFLOW_RUN_LABEL`] so a repeating cadence is distinguishable from
/// a one-shot at a glance — in the issue list, in a label-scoped query, and in any
/// automation an operator hangs off it. Both are the runner's own family and
/// neither is ever `fkst-dev`.
pub const WORKFLOW_SCHEDULED_RUN_LABEL: &str = "fkst-workflow-scheduled";

/// Every platform-owned label, for the repo-level label bootstrap and for the
/// reserved-name check below.
///
/// [`CRON_PAUSED_LABEL`] is in this list because it is still platform-owned
/// VOCABULARY — a session may not adopt the name — even though the human, not the
/// control plane, is the one who applies it.
/// The two run-issue labels are in this list for a second reason beyond
/// bootstrapping: [`crate::reconcile::collision`] excludes reserved labels from
/// collision detection. Every session that composes the workflow runner declares
/// the same two, so without the exclusion a second session on a repository would
/// collide with the first on the runner's own family and be demoted. Sharing them
/// is safe because a run issue is routed by SOLE ASSIGNEE, so only the creator
/// whose schedule produced it can pick it up.
pub const RESERVED_LABELS: &[&str] = &[
    SCHEDULED_WORKFLOW_LABEL,
    SCHEDULE_INVALID_LABEL,
    CRON_RUNNING_LABEL,
    CRON_PAUSED_LABEL,
    CRON_FAILED_LABEL,
    CRON_TIMEOUT_LABEL,
    WORKFLOW_RUN_LABEL,
    WORKFLOW_SCHEDULED_RUN_LABEL,
];

/// True when `label` is platform-owned. GitHub label identity is case-insensitive,
/// so the comparison is too — otherwise `FKST-Scheduled-Workflow` would pass this
/// check and then collide with the reserved label on GitHub itself.
pub fn is_reserved_label(label: &str) -> bool {
    RESERVED_LABELS
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(label.trim()))
}

/// The 422 detail for a trigger claiming a reserved name as its work label, or
/// `None` when the label is the author's to use.
///
/// Returned as a message rather than a bool so the rejection reads the same
/// wherever it surfaces — the trigger parser, the create-session API, and the
/// invalid-trigger latch comment.
pub fn reserved_work_label_rejection(label: &str) -> Option<String> {
    is_reserved_label(label).then(|| {
        format!(
            "the `### Work Label` section names {label:?}, which is reserved by the \
             deployment: platform labels ({}) select issues for platform behaviour and \
             cannot be adopted as a session's work label. Choose a different label.",
            RESERVED_LABELS.join(", ")
        )
    })
}

#[cfg(test)]
#[path = "reserved_labels_tests.rs"]
mod tests;
