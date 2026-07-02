//! Best-effort, per-repo auto-merge of the fkst App bot's mergeable pull requests
//! (opt-in via a session's `### Auto-merge`). Mirrors the `ensure_issue_templates`
//! hook: called from the per-repo driver, NEVER fails the reconcile, fully logged,
//! token never logged. v1 is a REPO-LEVEL gate — if ANY registered session on the
//! repo opted in, the bot's mergeable PRs are merged; per-PR→session scoping is a
//! documented follow-up.

use crate::github_app::{GithubAppTokens, PullRequestSummary};

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
        match github.pull_request_mergeable(owner_repo, pr.number).await {
            Ok(Some(true)) => {
                let title = format!("Merge pull request #{} (fkst auto-merge)", pr.number);
                match github
                    .merge_pull_request(owner_repo, pr.number, &title)
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
            Ok(Some(false)) => tracing::info!(
                owner_repo = %owner_repo,
                pr = pr.number,
                "auto-merge: PR not mergeable (conflict); skipping"
            ),
            Ok(None) => tracing::debug!(
                owner_repo = %owner_repo,
                pr = pr.number,
                "auto-merge: PR mergeable not yet computed; retry next reconcile"
            ),
            Err(error) => tracing::warn!(
                owner_repo = %owner_repo,
                pr = pr.number,
                error = %error,
                "auto-merge: mergeable read failed; skipping"
            ),
        }
    }
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
fn linked_issue_number(branch: &str, title: &str) -> Option<u64> {
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
