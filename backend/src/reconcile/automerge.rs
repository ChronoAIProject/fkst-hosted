//! Best-effort, per-repo auto-merge of the fkst App bot's mergeable pull requests
//! (opt-in via a session's `### Auto-merge`). Mirrors the `ensure_issue_templates`
//! hook: called from the per-repo driver, NEVER fails the reconcile, fully logged,
//! token never logged. v1 is a REPO-LEVEL gate — if ANY registered session on the
//! repo opted in, the bot's mergeable PRs are merged; per-PR→session scoping is a
//! documented follow-up. The final pre-merge gate is intentionally stricter than
//! the merge endpoint: requested-changes reviews and non-clean merge states hold
//! even if the App token could perform an administrative merge.

use std::collections::HashMap;

use crate::github_app::{
    GithubAppTokens, PullRequestMergeStatus, PullRequestReviewSummary, PullRequestSummary,
};

/// Auto-merge the App bot's mergeable open PRs on `owner_repo`, one at a time.
/// No-op unless `any_auto_merge` (some session opted in) AND a `bot_login` is
/// configured (needed to filter to the bot's PRs). Every GitHub call is
/// best-effort: a failure is logged and skipped, never propagated.
pub async fn auto_merge_bot_pull_requests(
    github: &GithubAppTokens,
    owner_repo: &str,
    bot_login: Option<&str>,
    any_auto_merge: bool,
) {
    if !any_auto_merge {
        return;
    }
    let bot_login = match bot_login {
        Some(l) => l,
        None => {
            tracing::warn!(
                owner_repo = %owner_repo,
                "auto-merge: a session opted in but FKST_GITHUB_BOT_LOGIN is unset; skipping"
            );
            return;
        }
    };

    let pulls = match github.list_open_pull_requests(owner_repo).await {
        Ok(p) => p,
        Err(error) => {
            tracing::warn!(
                owner_repo = %owner_repo,
                error = %error,
                "auto-merge: listing open PRs failed; will retry next reconcile"
            );
            return;
        }
    };

    for pr in pulls.iter().filter(|p| p.author_login == bot_login) {
        let status = match github
            .pull_request_merge_status(owner_repo, pr.number)
            .await
        {
            Ok(status) => status,
            Err(error) => {
                tracing::warn!(
                    owner_repo = %owner_repo,
                    pr = pr.number,
                    error = %error,
                    "auto-merge: fresh PR status read failed; skipping"
                );
                continue;
            }
        };
        let reviews = match github
            .list_pull_request_reviews(owner_repo, pr.number)
            .await
        {
            Ok(reviews) => reviews,
            Err(error) => {
                tracing::warn!(
                    owner_repo = %owner_repo,
                    pr = pr.number,
                    error = %error,
                    "auto-merge: PR review state read failed; skipping"
                );
                continue;
            }
        };
        match auto_merge_hold_reason(&status, &reviews) {
            None => {
                let title = format!("Merge pull request #{} (fkst auto-merge)", pr.number);
                match github
                    .merge_pull_request_if_head(owner_repo, pr.number, &title, &status.head_sha)
                    .await
                {
                    Ok(()) => {
                        tracing::info!(
                            owner_repo = %owner_repo,
                            pr = pr.number,
                            "auto-merge: merged bot PR"
                        );
                        // A merge alone leaves the devloop work issue OPEN (the PR
                        // body carries no `Closes #N` and the engine's own post-merge
                        // close flow is bypassed). Complete the operation by closing
                        // the linked issue — best-effort: never guess, never fail.
                        close_linked_issue(github, owner_repo, pr).await;
                    }
                    Err(error) => tracing::warn!(
                        owner_repo = %owner_repo,
                        pr = pr.number,
                        error = %error,
                        "auto-merge: merge failed; will retry next reconcile"
                    ),
                }
            }
            Some(reason) if reason.is_retry_noise() => tracing::debug!(
                owner_repo = %owner_repo,
                pr = pr.number,
                hold_reason = reason.as_str(),
                "auto-merge: PR held by merge gate; retry next reconcile"
            ),
            Some(reason) => tracing::info!(
                owner_repo = %owner_repo,
                pr = pr.number,
                hold_reason = reason.as_str(),
                "auto-merge: PR held by merge gate"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoMergeHoldReason {
    NotOpen,
    MergeablePending,
    MergeConflict,
    UnknownMergeState,
    BlockedMergeState,
    NonCleanMergeState,
    ChangesRequested,
    StaleApprovalAfterChangesRequested,
    UnknownReviewState,
}

impl AutoMergeHoldReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotOpen => "pr-not-open",
            Self::MergeablePending => "mergeable-pending",
            Self::MergeConflict => "merge-conflict",
            Self::UnknownMergeState => "unknown-merge-state",
            Self::BlockedMergeState => "blocked-merge-state",
            Self::NonCleanMergeState => "non-clean-merge-state",
            Self::ChangesRequested => "changes-requested",
            Self::StaleApprovalAfterChangesRequested => "stale-approval-after-changes-requested",
            Self::UnknownReviewState => "unknown-review-state",
        }
    }

    fn is_retry_noise(self) -> bool {
        matches!(self, Self::MergeablePending)
    }
}

fn auto_merge_hold_reason(
    status: &PullRequestMergeStatus,
    reviews: &[PullRequestReviewSummary],
) -> Option<AutoMergeHoldReason> {
    if !status.state.eq_ignore_ascii_case("open") {
        return Some(AutoMergeHoldReason::NotOpen);
    }
    match status.mergeable {
        Some(true) => {}
        Some(false) => return Some(AutoMergeHoldReason::MergeConflict),
        None => return Some(AutoMergeHoldReason::MergeablePending),
    }
    match normalized_optional(&status.mergeable_state).as_deref() {
        Some("clean") => {}
        Some("blocked") => return Some(AutoMergeHoldReason::BlockedMergeState),
        Some("unknown") => return Some(AutoMergeHoldReason::MergeablePending),
        Some(_) => return Some(AutoMergeHoldReason::NonCleanMergeState),
        None => return Some(AutoMergeHoldReason::UnknownMergeState),
    }
    review_hold_reason(&status.head_sha, reviews)
}

fn review_hold_reason(
    head_sha: &str,
    reviews: &[PullRequestReviewSummary],
) -> Option<AutoMergeHoldReason> {
    let mut latest_decision_by_author: HashMap<&str, &str> = HashMap::new();
    let mut latest_changes_requested_index: Option<usize> = None;

    for (index, review) in reviews.iter().enumerate() {
        match normalized_review_state(&review.state).as_deref() {
            Some("APPROVED") => {
                latest_decision_by_author.insert(review.author_login.as_str(), "APPROVED");
            }
            Some("CHANGES_REQUESTED") => {
                latest_decision_by_author.insert(review.author_login.as_str(), "CHANGES_REQUESTED");
                latest_changes_requested_index = Some(index);
            }
            Some("COMMENTED" | "DISMISSED" | "PENDING") => {}
            Some(_) | None => return Some(AutoMergeHoldReason::UnknownReviewState),
        }
    }

    if latest_decision_by_author
        .values()
        .any(|state| *state == "CHANGES_REQUESTED")
    {
        return Some(AutoMergeHoldReason::ChangesRequested);
    }

    if let Some(changes_requested_index) = latest_changes_requested_index {
        let has_later_exact_head_approval = reviews
            .iter()
            .enumerate()
            .skip(changes_requested_index + 1)
            .any(|(_, review)| {
                normalized_review_state(&review.state).as_deref() == Some("APPROVED")
                    && review.commit_id.as_deref() == Some(head_sha)
            });
        if !has_later_exact_head_approval {
            return Some(AutoMergeHoldReason::StaleApprovalAfterChangesRequested);
        }
    }

    None
}

fn normalized_optional(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
}

fn normalized_review_state(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_uppercase())
}

/// Close the merged PR's linked work issue (best-effort). Parses the issue number
/// from the PR (branch preferred, title fallback); if neither yields a number the
/// close is skipped rather than guessed. A close failure is logged, never fatal.
async fn close_linked_issue(github: &GithubAppTokens, owner_repo: &str, pr: &PullRequestSummary) {
    let issue = match linked_issue_number(&pr.head_ref, &pr.title) {
        Some(n) => n,
        None => {
            tracing::warn!(
                owner_repo = %owner_repo,
                pr = pr.number,
                branch = %pr.head_ref,
                "auto-merge: could not parse a work-issue number from the merged PR; \
                 leaving any linked issue open"
            );
            return;
        }
    };
    match github.close_issue(owner_repo, issue).await {
        Ok(()) => tracing::info!(
            owner_repo = %owner_repo,
            pr = pr.number,
            issue = issue,
            "auto-merge: closed linked work issue"
        ),
        Err(error) => tracing::warn!(
            owner_repo = %owner_repo,
            pr = pr.number,
            issue = issue,
            error = %error,
            "auto-merge: closing linked work issue failed; leaving it open"
        ),
    }
}

/// Parse the devloop work-issue number from a bot PR, preferring the head branch
/// ref and falling back to the title. Returns `None` when neither carries a number
/// (the caller then skips the close rather than guessing the wrong issue).
///
/// - Branch: `devloop/issue/<owner>/<repo>/<N>/ready-…` — the number is the segment
///   exactly three positions after the `issue` marker (owner, repo, then `<N>`), so
///   a numeric owner/repo cannot be mistaken for it.
/// - Title: `… implementation for #<N>` / `… implementation PR for issue #<N>` — the
///   first `#<digits>` run.
///
/// `pub(crate)`: this is THE devloop PR→work-issue parse — the canvas dashboard's
/// PR listing (`crate::routes::canvas`) reuses it to link a bot PR back to its
/// work issue, so the grammar lives in exactly one place.
pub(crate) fn linked_issue_number(branch: &str, title: &str) -> Option<u64> {
    issue_number_from_branch(branch).or_else(|| issue_number_from_title(title))
}

/// Positional parse of the `<N>` in `devloop/issue/<owner>/<repo>/<N>/…`.
fn issue_number_from_branch(branch: &str) -> Option<u64> {
    let segments: Vec<&str> = branch.split('/').collect();
    let issue_idx = segments.iter().position(|s| *s == "issue")?;
    segments.get(issue_idx + 3)?.parse::<u64>().ok()
}

/// Parse the first `#<digits>` run from a devloop PR title.
fn issue_number_from_title(title: &str) -> Option<u64> {
    let after_hash = title.split('#').nth(1)?;
    let digits: String = after_hash
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse::<u64>().ok()
}

#[cfg(test)]
#[path = "automerge_tests.rs"]
mod tests;
