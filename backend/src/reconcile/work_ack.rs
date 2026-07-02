//! One-time acknowledgment of every open WORK issue (visibility follow-up).
//!
//! A user files a WORK issue (one carrying a session's `work_label`) and the pod
//! picks it up and works it — but from GitHub the issue often shows NO labels or
//! comments (a session's output, e.g. the codex-triage package, lands elsewhere),
//! so the author has no signal it was even claimed. This step gives the WORK issue
//! a visible, fkst-hosted-owned acknowledgment: on each reconcile, for every VALID
//! registration in the repo it LISTs the open issues carrying that session's work
//! label and, for each one NOT yet acknowledged, posts a friendly "picked up"
//! comment and latches [`crate::reconcile::WORK_PICKED_UP_LABEL`].
//!
//! It exactly mirrors the session-announce pattern (comment + durable latch read
//! back each reconcile) so a control-plane restart never re-posts, and mirrors the
//! `ensure_issue_templates`/`auto_merge_bot_pull_requests` hooks: called
//! best-effort from the per-repo driver, it NEVER fails the reconcile (a list/post
//! failure is logged and skipped), and it reuses the repo-scoped installation token
//! the driver already minted. The comment + label carry only PUBLIC metadata (the
//! session name + work label) — never the minted token or any environment secret.

use secrecy::SecretString;

use crate::github_app::listing::GithubListing;
use crate::github_app::GithubAppTokens;
use crate::models::RepoRef;
use crate::reconcile::desired::SessionRegistration;

use super::WORK_PICKED_UP_LABEL;

/// Render the "picked up" acknowledgment comment for a work issue (pure;
/// unit-tested). Both `session_name` and `work_label` are public metadata safe to
/// display verbatim in backticks.
pub fn work_ack_comment(session_name: &str, work_label: &str) -> String {
    format!(
        "👀 **Picked up by fkst session `{session_name}`.**\n\n\
         A fkst pod is working this repo's `{work_label}` issues, including this one. \
         The session posts its progress on this issue as it works, and the outcome \
         will be a pull request (or, for issue-producing sessions, linked issues)."
    )
}

/// Best-effort, NON-failing: acknowledge every open work issue exactly once.
///
/// For each registration in `regs`, LIST the open issues carrying its `work_label`
/// and, for each returned issue whose labels do NOT already include
/// [`WORK_PICKED_UP_LABEL`], post the ack comment then latch the label. No-op when
/// there are no registrations (mirrors the other driver hooks' ≥1-registration
/// gate). Reuses the repo-scoped installation `token` the driver already minted.
/// Every GitHub call is best-effort: a failure is logged and skipped, never
/// propagated, so one bad issue never stalls the rest of the reconcile.
pub async fn ack_open_work_issues(
    github: &GithubAppTokens,
    listing: &dyn GithubListing,
    token: &SecretString,
    repo: &RepoRef,
    regs: &[SessionRegistration],
) {
    if regs.is_empty() {
        return;
    }
    for reg in regs {
        ack_label(
            github,
            listing,
            token,
            repo,
            &reg.def.name,
            &reg.def.work_label,
        )
        .await;
    }
}

/// Acknowledge every un-acked open issue carrying `work_label` for one session.
/// A listing failure is logged and skipped (the next reconcile retries).
async fn ack_label(
    github: &GithubAppTokens,
    listing: &dyn GithubListing,
    token: &SecretString,
    repo: &RepoRef,
    session_name: &str,
    work_label: &str,
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
                "work-ack: listing open work issues failed; will retry next reconcile"
            );
            return;
        }
    };

    for issue in issues {
        // The durable latch (read back from GitHub each reconcile) makes this
        // idempotent across restarts: an already-acked issue is skipped.
        if issue.labels.iter().any(|l| l == WORK_PICKED_UP_LABEL) {
            continue;
        }
        ack_issue(github, repo, issue.number, session_name, work_label).await;
    }
}

/// Acknowledge ONE work issue: post the comment, then latch the durable label.
/// Mirrors the executor's announce arm — the comment is best-effort (a failure is
/// logged, never propagated) and the label add is additive/idempotent.
async fn ack_issue(
    github: &GithubAppTokens,
    repo: &RepoRef,
    number: i64,
    session_name: &str,
    work_label: &str,
) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let comment = work_ack_comment(session_name, work_label);

    if let Err(error) = github
        .post_issue_comment(&owner_repo, number as u64, &comment)
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-ack: issue comment failed");
    }
    if let Err(error) = github
        .add_issue_labels(
            &owner_repo,
            number as u64,
            &[WORK_PICKED_UP_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-ack: latch picked-up label failed");
    }
}

#[cfg(test)]
#[path = "work_ack_tests.rs"]
mod tests;
