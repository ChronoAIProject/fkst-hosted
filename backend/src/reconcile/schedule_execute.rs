//! Applying one [`ScheduleEffect`] to GitHub.
//!
//! Best-effort like every other reconcile effect: a failure is logged and skipped,
//! never propagated, so one bad definition cannot stall a repository's sweep. What
//! makes that safe here is that the planner converges from every partial write —
//! see the state table on [`crate::schedule::decide`] — so an interrupted effect is
//! repaired on the next pass rather than leaving a schedule wedged.
//!
//! ## Write order
//!
//! A dispatch is four writes and the order is load-bearing:
//!
//! 1. `create_issue` — the run issue, with NO labels and NO assignee yet;
//! 2. `add_issue_labels` — the work label;
//! 3. `add_issue_assignees` — the creator;
//! 4. the `Dispatched` record + running latch on the DEFINITION issue.
//!
//! Steps 2 and 3 are in that order because an issue that is briefly assigned but
//! unlabeled is invisible to the wake gate, whereas one that is briefly labeled but
//! unassigned is visible and simply unrouted — the reconciler latches
//! `fkst-unrouted` on it, which self-heals the moment step 3 lands. The reverse
//! order would instead produce a silent, unexplained non-start.
//!
//! Step 4 comes last so a failed dispatch never leaves a record claiming a run that
//! does not exist. The opposite interruption — issue created, record missing — is
//! recovered by the next pass through the adoption path, because the run issue is
//! itself an open work issue that keeps the session awake.

use k8s_openapi::chrono::Utc;

use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::models::RepoRef;
use crate::reconcile::execute::post_comment_best_effort;
use crate::reconcile::reserved_labels::{
    CRON_FAILED_LABEL, CRON_RUNNING_LABEL, CRON_TIMEOUT_LABEL, SCHEDULE_INVALID_LABEL,
};
use crate::reconcile::schedule_plan::ScheduleEffect;
use crate::reconcile::schedule_run_issue::{render_run_issue_body, RunIssueRequest};
use crate::schedule::{render_marker, RunRecord, RunStatus};

/// Apply one effect. Never propagates.
pub async fn execute_schedule_effect(
    effect: ScheduleEffect,
    repo: &RepoRef,
    github: &GithubAppTokens,
) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    match effect {
        ScheduleEffect::Dispatch {
            schedule_issue,
            request,
            skipped,
        } => dispatch(&owner_repo, schedule_issue, *request, skipped, github).await,
        ScheduleEffect::RecordSkip {
            schedule_issue,
            slot,
        } => {
            // No label changes: the previous run still owns the latch.
            let record = RunRecord::new(slot, RunStatus::SkippedOverlap, Utc::now())
                .with_detail("the previous run was still in flight");
            post_record(
                &owner_repo,
                schedule_issue,
                &record,
                "⏭ Slot skipped — the previous run was still in flight.",
                github,
            )
            .await;
        }
        ScheduleEffect::Expire {
            schedule_issue,
            slot,
            started,
        } => {
            let now = Utc::now();
            let elapsed = (now - started).num_seconds().max(0);
            let record = RunRecord {
                started,
                ended: Some(now),
                ..RunRecord::new(slot, RunStatus::Timeout, now)
            }
            .with_detail(format!("exceeded its budget after {elapsed}s"));
            post_record(
                &owner_repo,
                schedule_issue,
                &record,
                &format!(
                    "⏱ Run released by the watchdog after {elapsed}s. The next slot may proceed."
                ),
                github,
            )
            .await;
            set_labels(
                &owner_repo,
                schedule_issue,
                &[CRON_TIMEOUT_LABEL],
                &[CRON_RUNNING_LABEL],
                github,
            )
            .await;
        }
        ScheduleEffect::Complete {
            schedule_issue,
            slot,
            status,
        } => {
            // The pod (or the watchdog) already wrote the record; the control plane
            // only reflects it in the labels, which it alone owns.
            let (add, remove): (&[&str], &[&str]) = match status {
                RunStatus::Ok => (
                    &[],
                    &[CRON_RUNNING_LABEL, CRON_FAILED_LABEL, CRON_TIMEOUT_LABEL],
                ),
                RunStatus::Timeout => (&[CRON_TIMEOUT_LABEL], &[CRON_RUNNING_LABEL]),
                _ => (&[CRON_FAILED_LABEL], &[CRON_RUNNING_LABEL]),
            };
            set_labels(&owner_repo, schedule_issue, add, remove, github).await;
            tracing::info!(
                owner_repo = %owner_repo,
                schedule_issue,
                slot = %slot,
                status = status.as_str(),
                "schedule: run completed"
            );
        }
        ScheduleEffect::ReleaseRunning { schedule_issue } => {
            tracing::warn!(
                owner_repo = %owner_repo,
                schedule_issue,
                "schedule: releasing a running latch with no dispatch record behind it"
            );
            set_labels(
                &owner_repo,
                schedule_issue,
                &[],
                &[CRON_RUNNING_LABEL],
                github,
            )
            .await;
        }
        ScheduleEffect::AdoptRunning {
            schedule_issue,
            slot,
        } => {
            tracing::warn!(
                owner_repo = %owner_repo,
                schedule_issue,
                slot = %slot,
                "schedule: re-latching a dispatch whose label write did not land"
            );
            set_labels(
                &owner_repo,
                schedule_issue,
                &[CRON_RUNNING_LABEL],
                &[],
                github,
            )
            .await;
        }
        ScheduleEffect::FlagInvalid {
            schedule_issue,
            detail,
        } => {
            // Label BEFORE comment: the label is the dedupe gate, so a failed label
            // write must not risk an unlatched duplicate comment next sweep.
            set_labels(
                &owner_repo,
                schedule_issue,
                &[SCHEDULE_INVALID_LABEL],
                &[],
                github,
            )
            .await;
            post_comment_best_effort(
                github,
                &owner_repo,
                schedule_issue,
                &format!(
                    "⚠️ **This scheduled workflow is not running.**\n\n{detail}\n\n\
                     Edit this issue to fix it — the label clears automatically on the next \
                     reconcile."
                ),
            )
            .await;
        }
        ScheduleEffect::ClearInvalid { schedule_issue } => {
            set_labels(
                &owner_repo,
                schedule_issue,
                &[],
                &[SCHEDULE_INVALID_LABEL],
                github,
            )
            .await;
        }
    }
}

/// Create the run issue, then latch and record the dispatch. See the module docs
/// for why the four writes are in this order.
async fn dispatch(
    owner_repo: &str,
    schedule_issue: i64,
    request: RunIssueRequest,
    skipped: u32,
    github: &GithubAppTokens,
) {
    let body = render_run_issue_body(&request);
    let run_issue = match github
        .create_issue(owner_repo, &request.title(), &body, &[], &[])
        .await
    {
        Ok(number) => number,
        Err(error) => {
            tracing::warn!(
                owner_repo = %owner_repo,
                schedule_issue,
                error = %error,
                "schedule: run-issue creation failed; the slot retries next sweep"
            );
            return;
        }
    };

    if let Err(error) = github
        .add_issue_labels(
            owner_repo,
            run_issue,
            std::slice::from_ref(&request.work_label),
        )
        .await
    {
        // Without its label the run issue cannot wake the session. Say so loudly:
        // the next sweep will not adopt it either, because nothing recorded it.
        tracing::error!(
            owner_repo = %owner_repo,
            schedule_issue,
            run_issue,
            error = %error,
            "schedule: run issue created but labelling failed; it will not wake a session"
        );
        return;
    }
    if let Err(error) = github
        .add_issue_assignees(
            owner_repo,
            run_issue,
            std::slice::from_ref(&request.creator_login),
        )
        .await
    {
        // Recoverable and self-explaining: the reconciler latches `fkst-unrouted`
        // on the labelled-but-unassigned issue, which clears when it is assigned.
        tracing::warn!(
            owner_repo = %owner_repo,
            schedule_issue,
            run_issue,
            error = %error,
            "schedule: run issue created but assignment failed; it will latch fkst-unrouted"
        );
    }

    let now = Utc::now();
    let record = RunRecord::new(request.slot, RunStatus::Dispatched, now).with_issue(run_issue);
    let skipped_note = if skipped > 0 {
        format!(" {skipped} earlier slot(s) were missed and will not be replayed.")
    } else {
        String::new()
    };
    post_record(
        owner_repo,
        schedule_issue,
        &record,
        &format!(
            "⏱ Scheduled run started — slot `{}`, tracked in #{run_issue}.{skipped_note}",
            request
                .slot
                .to_rfc3339_opts(k8s_openapi::chrono::SecondsFormat::Secs, true)
        ),
        github,
    )
    .await;
    set_labels(
        owner_repo,
        schedule_issue,
        &[CRON_RUNNING_LABEL],
        &[],
        github,
    )
    .await;
}

/// Post one run record as a comment carrying its human line and hidden marker.
async fn post_record(
    owner_repo: &str,
    schedule_issue: i64,
    record: &RunRecord,
    human: &str,
    github: &GithubAppTokens,
) {
    post_comment_best_effort(
        github,
        owner_repo,
        schedule_issue,
        &format!("{human}\n\n{}", render_marker(record)),
    )
    .await;
}

/// Add and remove labels, tolerating a label that is already absent.
async fn set_labels(
    owner_repo: &str,
    issue: i64,
    add: &[&str],
    remove: &[&str],
    github: &GithubAppTokens,
) {
    if !add.is_empty() {
        let labels: Vec<String> = add.iter().map(|label| (*label).to_string()).collect();
        if let Err(error) = github
            .add_issue_labels(owner_repo, issue as u64, &labels)
            .await
        {
            tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "schedule: add labels failed");
        }
    }
    for label in remove {
        match github
            .remove_issue_label(owner_repo, issue as u64, label)
            .await
        {
            Ok(()) => {}
            // Already gone is the desired state, not a failure.
            Err(GithubAppError::NotFound { .. }) => {}
            Err(error) => {
                tracing::warn!(owner_repo = %owner_repo, issue, label, error = %error, "schedule: remove label failed");
            }
        }
    }
}
