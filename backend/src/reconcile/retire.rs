//! One-time retire-notify of every still-OPEN WORK issue when a session is retired.
//!
//! When a session's trigger issue is CLOSED the session is retired and its pod is
//! cleaned up (the orphan-pod `Kill { TriggerClosed }`). But the WORK issues that
//! session was working stay OPEN and otherwise keep a now-stale
//! [`crate::reconcile::WORK_PICKED_UP_LABEL`] + "picked up" comment, with no signal
//! that nobody is working them anymore. This step closes that gap: for EACH of the
//! retired session's effective work labels (epic #594 I4 — a session may claim more than
//! one) it LISTs the open issues carrying that label. New retirements post a "session
//! retired, no longer worked" comment and latch the durable
//! [`crate::reconcile::SUBSTRATE_RETIRED_LABEL`] before removing the stale picked-up label;
//! partial retirements that already carry both labels only repair the stale picked-up latch.
//! Each issue is LEFT OPEN. An issue shared by two of the session's labels is retired
//! ONCE (an in-pass `retired` set dedups across labels).
//!
//! It exactly mirrors [`crate::reconcile::work_ack::ack_open_work_issues`]: emitted
//! from the pure planner's orphan branch. Comment failures remain best-effort, while
//! listing or durable-label failures report an incomplete transaction so the executor
//! keeps the orphan runtime as the next reconcile's retry owner. The comment carries only
//! PUBLIC metadata (the effective work-label set) — never the minted token or any environment
//! secret. The retired latch, read back from GitHub each reconcile, makes notification
//! idempotent while still allowing a later pass to finish removing a stale picked-up latch.

use std::collections::HashSet;

use secrecy::SecretString;

use crate::github_app::listing::GithubListing;
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::k8s::work_label_wire::join_work_labels;
use crate::models::RepoRef;

use super::{SUBSTRATE_RETIRED_LABEL, WORK_PICKED_UP_LABEL};

/// Render the "session retired" notice for a work issue (pure; unit-tested).
/// `work_labels` is the retired session's full effective label set and is public
/// metadata safe to display verbatim in backticks.
pub fn retire_notice_comment(work_labels: &[String]) -> String {
    let joined_labels = join_work_labels(work_labels);
    format!(
        "⚠️ **Session retired.** The trigger issue for effective work labels \
         `{joined_labels}` was closed, so this session was retired and its pod cleaned up. \
         This issue is left OPEN but is no longer being worked. To resume, open a new trigger \
         issue (label `fkst-substrate-trigger`) whose effective labels cover this issue, then \
         give this issue exactly one assignee: that new session's creator."
    )
}

/// Executor entry point (the [`crate::reconcile::desired::ReconcileAction::RetireSession`]
/// arm): mint the repo-scoped installation token ONCE, then retire across EACH of the
/// session's effective `work_labels` (epic #594 I4) via [`retire_open_work_issues`]. A
/// token-mint failure is logged and skipped — the next reconcile retries while the orphan
/// pod still lingers, and the retired latch keeps that retry from re-notifying an
/// already-handled issue. An in-pass `retired` set dedups an issue shared by two of the
/// session's labels so it is notified once. The minted token is passed straight through
/// and NEVER logged.
pub async fn retire_work_issues(
    github: &GithubAppTokens,
    listing: &dyn GithubListing,
    repo: &RepoRef,
    work_labels: &[String],
) -> bool {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let token = match github.token_for_repo(&owner_repo, None).await {
        Ok(token) => token,
        Err(error) => {
            tracing::warn!(owner_repo = %owner_repo, labels = work_labels.len(), error = %error, "retire: token mint failed; skipping (retry next reconcile)");
            return false;
        }
    };
    // Dedup an issue carrying more than one of the session's labels so it is retired
    // once per pass, independent of GitHub's list read-after-write timing.
    let mut retired: HashSet<i64> = HashSet::new();
    let mut complete = true;
    for work_label in work_labels {
        complete &= retire_open_work_issues(
            github,
            listing,
            &token,
            repo,
            work_label,
            work_labels,
            &mut retired,
        )
        .await;
    }
    complete
}

/// Best-effort, NON-failing: retire-notify every still-open work issue carrying
/// `work_label` exactly once.
///
/// LIST the open issues carrying `work_label` and, for each returned issue that `retired`
/// has not already handled this pass, either complete a new retirement or converge an
/// existing retired + picked-up contradiction by removing the stale
/// [`WORK_PICKED_UP_LABEL`]. A new retirement posts the notice and must successfully latch
/// [`SUBSTRATE_RETIRED_LABEL`] before removing picked-up. The issue number is inserted into
/// `retired` so a sibling label in the same pass never re-notifies it (epic #594 I4). The issue is LEFT
/// OPEN. The notice receives the full `effective_work_labels` set even though this step
/// queries one `work_label` at a time, so a shared issue never gets misleading single-label
/// restart guidance. Reuses the repo-scoped installation `token` the executor minted.
/// Comment failures are best-effort. Listing and durable-label failures return `false`
/// so the caller retains the runtime and retries the transaction next reconcile.
pub async fn retire_open_work_issues(
    github: &GithubAppTokens,
    listing: &dyn GithubListing,
    token: &SecretString,
    repo: &RepoRef,
    work_label: &str,
    effective_work_labels: &[String],
    retired: &mut HashSet<i64>,
) -> bool {
    let issues = match listing
        .list_issues_by_label(token, &repo.owner, &repo.name, work_label)
        .await
    {
        Ok(issues) => issues,
        Err(error) => {
            tracing::warn!(
                owner = %repo.owner,
                name = %repo.name,
                work_label = %work_label,
                error = %error,
                "retire: listing open work issues failed; will retry next reconcile"
            );
            return false;
        }
    };

    let mut complete = true;
    for issue in issues {
        // Already handled under a sibling label THIS pass (a multi-label session's issue
        // that carries two of its labels) — retire it once.
        if retired.contains(&issue.number) {
            continue;
        }
        let carries_retired = issue.labels.iter().any(|l| l == SUBSTRATE_RETIRED_LABEL);
        let carries_picked_up = issue.labels.iter().any(|l| l == WORK_PICKED_UP_LABEL);
        let issue_complete = if carries_retired {
            !carries_picked_up || remove_picked_up(github, repo, issue.number).await
        } else {
            retire_issue(github, repo, issue.number, effective_work_labels).await
        };
        complete &= issue_complete;
        retired.insert(issue.number);
    }
    complete
}

/// Retire-notify ONE work issue: post the notice, latch the retired label, then drop
/// the now-stale picked-up label. Mirrors the executor's announce/ack arms — the
/// comment is best-effort (a failure is logged, never propagated), the label add is
/// additive/idempotent, and the label remove is 404-tolerant (the label may already
/// be gone), reusing the same tolerance the invalid-flag clear uses.
async fn retire_issue(
    github: &GithubAppTokens,
    repo: &RepoRef,
    number: i64,
    effective_work_labels: &[String],
) -> bool {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let comment = retire_notice_comment(effective_work_labels);

    if let Err(error) = github
        .post_issue_comment(&owner_repo, number as u64, &comment)
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "retire: issue comment failed");
    }
    if let Err(error) = github
        .add_issue_labels(
            &owner_repo,
            number as u64,
            &[SUBSTRATE_RETIRED_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "retire: latch retired label failed; keeping picked-up until retirement is authoritative");
        return false;
    }
    remove_picked_up(github, repo, number).await
}

async fn remove_picked_up(github: &GithubAppTokens, repo: &RepoRef, number: i64) -> bool {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    match github
        .remove_issue_label(&owner_repo, number as u64, WORK_PICKED_UP_LABEL)
        .await
    {
        Ok(()) | Err(GithubAppError::NotFound { .. }) => true,
        Err(error) => {
            tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "retire: remove picked-up label failed; will retry while retired remains authoritative");
            false
        }
    }
}

#[cfg(test)]
#[path = "retire_tests.rs"]
mod tests;
