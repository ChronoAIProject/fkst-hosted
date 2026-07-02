//! Best-effort, per-repo auto-merge of the fkst App bot's mergeable pull requests
//! (opt-in via a session's `### Auto-merge`). Mirrors the `ensure_issue_templates`
//! hook: called from the per-repo driver, NEVER fails the reconcile, fully logged,
//! token never logged. v1 is a REPO-LEVEL gate — if ANY registered session on the
//! repo opted in, the bot's mergeable PRs are merged; per-PR→session scoping is a
//! documented follow-up.

use crate::github_app::GithubAppTokens;

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
                    Ok(()) => tracing::info!(
                        owner_repo = %owner_repo,
                        pr = pr.number,
                        "auto-merge: merged bot PR"
                    ),
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

#[cfg(test)]
#[path = "automerge_tests.rs"]
mod tests;
