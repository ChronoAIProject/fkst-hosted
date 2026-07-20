//! One-time retire-notify of every still-OPEN WORK issue when a session is retired.
//!
//! When a session's trigger issue is CLOSED the session is retired and its pod is
//! cleaned up (the orphan-pod `Kill { TriggerClosed }`). But the WORK issues that
//! session was working stay OPEN and otherwise keep a now-stale
//! [`crate::reconcile::WORK_PICKED_UP_LABEL`] + "picked up" comment, with no signal
//! that nobody is working them anymore. This step closes that gap: for EACH of the
//! retired session's effective work labels (epic #594 I4 — a session may claim more than
//! one) it LISTs the open issues carrying that label and, for each one NOT yet retired,
//! posts a "session retired, no longer worked" comment, latches the durable
//! [`crate::reconcile::SUBSTRATE_RETIRED_LABEL`], and removes the stale picked-up label.
//! Each issue is LEFT OPEN. An issue shared by two of the session's labels is retired
//! ONCE (an in-pass `retired` set dedups across labels).
//!
//! It exactly mirrors [`crate::reconcile::work_ack::ack_open_work_issues`]: emitted
//! from the pure planner (the orphan-pod branch, alongside the kill) and executed
//! best-effort so a list/post/label failure is logged and skipped — NEVER propagated,
//! so one bad issue never stalls the rest of the reconcile. The comment carries only
//! PUBLIC metadata (the work label) — never the minted token or any environment
//! secret. The retired latch, read back from GitHub each reconcile, makes it
//! idempotent: an already-retired issue is skipped, so the ~60s the orphan pod lingers
//! before deletion never re-notifies.

use std::collections::HashSet;

use secrecy::SecretString;

use crate::github_app::listing::GithubListing;
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::models::RepoRef;

use super::{SUBSTRATE_RETIRED_LABEL, WORK_PICKED_UP_LABEL};

/// Render the "session retired" notice for a work issue (pure; unit-tested).
/// `work_label` is public metadata safe to display verbatim in backticks.
pub fn retire_notice_comment(work_label: &str) -> String {
    format!(
        "⚠️ **Session retired.** The trigger issue for work label `{work_label}` was \
         closed, so this session was retired and its pod cleaned up. This issue is left \
         OPEN but is no longer being worked. To resume, open a new trigger issue (label \
         `fkst-substrate-trigger`) with work label `{work_label}`."
    )
}

/// Executor entry point (the [`crate::reconcile::desired::ReconcileAction::RetireWorkIssues`]
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
) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let token = match github.token_for_repo(&owner_repo, None).await {
        Ok(token) => token,
        Err(error) => {
            tracing::warn!(owner_repo = %owner_repo, labels = work_labels.len(), error = %error, "retire: token mint failed; skipping (retry next reconcile)");
            return;
        }
    };
    // Dedup an issue carrying more than one of the session's labels so it is retired
    // once per pass, independent of GitHub's list read-after-write timing.
    let mut retired: HashSet<i64> = HashSet::new();
    for work_label in work_labels {
        retire_open_work_issues(github, listing, &token, repo, work_label, &mut retired).await;
    }
}

/// Best-effort, NON-failing: retire-notify every still-open work issue carrying
/// `work_label` exactly once.
///
/// LIST the open issues carrying `work_label` and, for each returned issue whose labels
/// do NOT already include [`SUBSTRATE_RETIRED_LABEL`] and that `retired` has not already
/// handled this pass, post the retire notice, latch the retired label, then remove the
/// now-stale [`WORK_PICKED_UP_LABEL`]. The issue number is inserted into `retired` so a
/// sibling label in the same pass never re-notifies it (epic #594 I4). The issue is LEFT
/// OPEN. Reuses the repo-scoped installation `token` the executor minted. Every GitHub
/// call is best-effort: a failure is logged and skipped, never propagated, so one bad
/// issue never stalls the rest of the reconcile.
pub async fn retire_open_work_issues(
    github: &GithubAppTokens,
    listing: &dyn GithubListing,
    token: &SecretString,
    repo: &RepoRef,
    work_label: &str,
    retired: &mut HashSet<i64>,
) {
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
            return;
        }
    };

    for issue in issues {
        // Already handled under a sibling label THIS pass (a multi-label session's issue
        // that carries two of its labels) — retire it once.
        if retired.contains(&issue.number) {
            continue;
        }
        // The durable latch (read back from GitHub each reconcile) makes this
        // idempotent across the ~60s the orphan pod lingers before deletion: an
        // already-retired issue is skipped, so it is never re-notified.
        if issue.labels.iter().any(|l| l == SUBSTRATE_RETIRED_LABEL) {
            continue;
        }
        retire_issue(github, repo, issue.number, work_label).await;
        retired.insert(issue.number);
    }
}

/// Retire-notify ONE work issue: post the notice, latch the retired label, then drop
/// the now-stale picked-up label. Mirrors the executor's announce/ack arms — the
/// comment is best-effort (a failure is logged, never propagated), the label add is
/// additive/idempotent, and the label remove is 404-tolerant (the label may already
/// be gone), reusing the same tolerance the invalid-flag clear uses.
async fn retire_issue(github: &GithubAppTokens, repo: &RepoRef, number: i64, work_label: &str) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let comment = retire_notice_comment(work_label);

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
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "retire: latch retired label failed");
    }
    // Drop the now-stale picked-up latch so the issue no longer looks claimed. A
    // missing label (already gone) is tolerated, exactly like the invalid-flag clear.
    match github
        .remove_issue_label(&owner_repo, number as u64, WORK_PICKED_UP_LABEL)
        .await
    {
        Ok(()) => {}
        Err(GithubAppError::NotFound { .. }) => {}
        Err(error) => {
            tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "retire: remove picked-up label failed");
        }
    }
}

#[cfg(test)]
#[path = "retire_tests.rs"]
mod tests;
