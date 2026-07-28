//! Effectful install/update orchestration for the bundled issue templates:
//! the branch + PR + immediate-merge flow behind
//! [`IssueTemplateGithub::install_templates`], decoupled from token minting so
//! it is unit-testable against a fake [`GithubApi`].
//!
//! Two invariants drive the design, both learned from issue #5578 (a repo whose
//! protected default branch blocks the immediate merge saw a fresh install PR
//! opened and force-closed roughly once a minute, forever):
//!
//! 1. **Never churn.** A merge this App cannot complete is a normal steady state,
//!    not a failure: the PR is LEFT OPEN for the repository's own CI/review flow
//!    ([`TemplateInstallOutcome::Deferred`]) and the caller TTL-gates the retry.
//!    The head branch of an open install PR is never deleted — deleting it
//!    force-closes the PR, which is precisely how the churn loop sustained
//!    itself. The one exception is a PR GitHub reports as definitively
//!    conflicted: it can never merge, so it is rebuilt from the current base
//!    head (see [`recreate_conflicted`]), and even that is TTL-bounded.
//! 2. **Never merge what this App did not write.** The install branch lives in an
//!    App-owned namespace, but a branch NAME is not an identity: a fork PR
//!    carries a bare `head.ref` that can say anything. Reuse therefore resolves
//!    the PR through [`GithubApi::open_pull_for_head`] (owner-qualified, so forks
//!    cannot match) and then verifies the PR's diff touches ONLY bundled template
//!    paths before merging it with the App token.
//!
//! [`IssueTemplateGithub::install_templates`]: super::templates::IssueTemplateGithub::install_templates

use secrecy::{ExposeSecret, SecretString};

use super::api::{GithubApi, PullRequestSummary};
use super::templates::{bundled_templates, encode_content, TemplateInstallOutcome};
use super::GithubAppError;

#[cfg(test)]
#[path = "template_install_tests.rs"]
mod tests;

/// Head-branch name of the install PR for `target_version`.
pub(super) fn template_branch(target_version: u32) -> String {
    format!("fkst/issue-templates-v{target_version}")
}

/// The version encoded in an install-PR head branch
/// (`fkst/issue-templates-v{N}`); `None` for any other branch.
fn template_branch_version(head_ref: &str) -> Option<u32> {
    head_ref
        .strip_prefix("fkst/issue-templates-v")?
        .parse::<u32>()
        .ok()
}

/// PR title for `target_version` (also the merge-commit title).
fn pr_title(target_version: u32) -> String {
    format!("Install/Update fkst issue templates to v{target_version}")
}

/// Whether `pr` is a pull request from a branch in `owner/repo` ITSELF rather
/// than from a fork. A fork PR's `head.ref` is an unqualified branch name in the
/// fork, so it can impersonate any App-owned branch name; only the head
/// repository identity settles it. A PR whose head repository was deleted
/// (`None`) is not treated as ours.
fn is_same_repo(pr: &PullRequestSummary, owner: &str, repo: &str) -> bool {
    pr.head_repo_full_name
        .as_deref()
        .is_some_and(|full_name| full_name.eq_ignore_ascii_case(&format!("{owner}/{repo}")))
}

/// Whether merging `pr` can only touch files this module manages, i.e. every
/// changed path is one of the bundled template paths. This is the last gate
/// before the App merges a pull request it did not create in this process — it
/// bounds a reused PR's blast radius to the inert template files even if someone
/// with push access staged something else on the App's branch.
///
/// FAILS CLOSED: a listing error, or a diff larger than the transport's page cap
/// (which necessarily contains non-template paths), yields `false`.
async fn touches_only_templates(
    api: &dyn GithubApi,
    token: &SecretString,
    owner: &str,
    repo: &str,
    number: u64,
) -> bool {
    let files = match api
        .list_pull_files(token.expose_secret(), owner, repo, number as i64)
        .await
    {
        Ok(files) => files,
        Err(error) => {
            tracing::warn!(
                owner = %owner,
                repo = %repo,
                pull = number,
                error = %error,
                "issue-templates: cannot read the pending PR's files; refusing to merge it"
            );
            return false;
        }
    };
    let bundled = bundled_templates();
    let is_bundled = |path: &str| bundled.iter().any(|tf| tf.path == path);
    // A rename is two paths: merging it also REMOVES the old one, so both ends
    // must be ours.
    files
        .iter()
        .all(|f| is_bundled(&f.filename) && f.previous_filename.as_deref().is_none_or(&is_bundled))
}

/// Rebuild the install branch from the current base head and open a fresh PR,
/// after the existing one was found permanently unmergeable (conflicted). This
/// DOES force-close the old PR (deleting a head branch closes its PR) — the
/// deliberate exception to the never-churn rule, safe only because the caller
/// reaches it at most once per ensure TTL and only on a definitive conflict.
async fn recreate_conflicted(
    api: &dyn GithubApi,
    token: &SecretString,
    owner: &str,
    repo: &str,
    target_version: u32,
    stale_pull: u64,
) -> Result<TemplateInstallOutcome, GithubAppError> {
    tracing::warn!(
        owner = %owner,
        repo = %repo,
        pull = stale_pull,
        "issue-templates: pending install PR is conflicted; rebuilding it from the base head"
    );
    let branch = template_branch(target_version);
    api.delete_ref(token, owner, repo, &branch).await.ok();
    open_install_pull(api, token, owner, repo, target_version).await
}

/// Create the install branch, write every bundled template onto it, open the PR,
/// and try to merge it. Assumes no open PR already owns the branch.
async fn open_install_pull(
    api: &dyn GithubApi,
    token: &SecretString,
    owner: &str,
    repo: &str,
    target_version: u32,
) -> Result<TemplateInstallOutcome, GithubAppError> {
    let branch = template_branch(target_version);
    let title = pr_title(target_version);

    let base = api.repo_default_branch(token, owner, repo).await?;
    let head_sha = api
        .branch_head_sha(token, owner, repo, &base)
        .await?
        .ok_or_else(|| {
            GithubAppError::Http(format!("repository default branch {base:?} has no Git ref"))
        })?;

    // A surviving branch with no open PR (the caller established that) is stale
    // — a crashed run, or a PR closed by hand — so restart it from the current
    // head rather than building on whatever it holds.
    if let Err(GithubAppError::RefExists) =
        api.create_ref(token, owner, repo, &branch, &head_sha).await
    {
        api.delete_ref(token, owner, repo, &branch).await.ok();
        api.create_ref(token, owner, repo, &branch, &head_sha)
            .await?;
    }

    for tf in bundled_templates() {
        // An existing blob on the base branch => UPDATE (PUT requires its sha);
        // None => CREATE.
        let existing = api
            .content_file(token, owner, repo, tf.path, Some(&base))
            .await?;
        let sha = existing.map(|f| f.sha);
        let content_b64 = encode_content(tf.content);
        let msg = format!("chore(fkst): sync {} to v{target_version}", tf.path);
        api.put_file(
            token,
            owner,
            repo,
            tf.path,
            &msg,
            &content_b64,
            &branch,
            sha.as_deref(),
        )
        .await?;
    }

    let number = api
        .create_pull_request(
            token,
            owner,
            repo,
            &title,
            &branch,
            &base,
            "Automated by fkst-hosted: bundled issue templates (trusted fixed content). \
             Merged without review/CI where the base branch allows it; otherwise left \
             open for the repository's own merge flow.",
        )
        .await?;

    match api
        .merge_pull_request(token, owner, repo, number, &title)
        .await
    {
        Ok(()) => {
            // Best-effort cleanup of the merged branch; a failure here never
            // fails the install (the PR is already merged).
            api.delete_ref(token, owner, repo, &branch).await.ok();
            Ok(TemplateInstallOutcome::Merged)
        }
        Err(error) => {
            // Expected wherever the base branch enforces checks or reviews, and
            // on repositories that disallow merge commits. NOT an error: the PR
            // stays open for the repository's own merge flow.
            tracing::info!(
                owner = %owner,
                repo = %repo,
                pull = number,
                error = %error,
                "issue-templates: immediate merge unavailable; leaving install PR open"
            );
            Ok(TemplateInstallOutcome::Deferred { pull: number })
        }
    }
}

/// Retry the merge of an install PR a previous pass left open, recovering if it
/// has since become conflicted. Never deletes the branch of a PR that is still
/// mergeable — that is the churn loop.
async fn resume_pending_pull(
    api: &dyn GithubApi,
    token: &SecretString,
    owner: &str,
    repo: &str,
    target_version: u32,
    pending: PullRequestSummary,
) -> Result<TemplateInstallOutcome, GithubAppError> {
    let number = pending.number;
    if !is_same_repo(&pending, owner, repo)
        || !touches_only_templates(api, token, owner, repo, number).await
    {
        // Someone else's pull request occupies the App's branch name. Touch
        // nothing: not the branch (deleting it would close their PR), not the
        // merge. The repository simply stays on its current template version
        // until a human resolves the squat.
        tracing::warn!(
            owner = %owner,
            repo = %repo,
            pull = number,
            head_repo = pending.head_repo_full_name.as_deref().unwrap_or("<deleted>"),
            author = %pending.author_login,
            "issue-templates: install branch held by a pull request this app did not \
             write; refusing to merge it"
        );
        return Ok(TemplateInstallOutcome::Deferred { pull: number });
    }

    let title = pr_title(target_version);
    match api
        .merge_pull_request(token, owner, repo, number, &title)
        .await
    {
        Ok(()) => {
            api.delete_ref(token, owner, repo, &template_branch(target_version))
                .await
                .ok();
            Ok(TemplateInstallOutcome::Merged)
        }
        Err(error) => {
            // A merge can fail because the base branch gates it (retry later) or
            // because the PR is genuinely conflicted (retrying can never work).
            // Only GitHub's `mergeable` flag separates the two, and only
            // `Some(false)` is definitive — `None` means "not computed yet".
            if let Ok(Some(false)) = api.pull_request_mergeable(token, owner, repo, number).await {
                return recreate_conflicted(api, token, owner, repo, target_version, number).await;
            }
            tracing::info!(
                owner = %owner,
                repo = %repo,
                pull = number,
                error = %error,
                "issue-templates: pending install PR still unmergeable; leaving it open"
            );
            Ok(TemplateInstallOutcome::Deferred { pull: number })
        }
    }
}

/// Close install PRs left open for a version BELOW `target_version` so a version
/// bump never leaves two template PRs open at once. Best-effort and purely
/// cosmetic: every failure is logged and skipped, because a lingering stale PR
/// is untidy, not harmful. Only same-repository PRs are considered — a fork PR
/// named like an install branch describes nothing about this repository.
async fn supersede_older_pulls(
    api: &dyn GithubApi,
    token: &SecretString,
    owner: &str,
    repo: &str,
    target_version: u32,
) {
    let open = match api.list_open_pulls(token, owner, repo).await {
        Ok(open) => open,
        Err(error) => {
            tracing::warn!(
                owner = %owner,
                repo = %repo,
                error = %error,
                "issue-templates: cannot list open PRs; skipping stale-version cleanup"
            );
            return;
        }
    };
    for pr in open {
        let Some(version) = template_branch_version(&pr.head_ref) else {
            continue;
        };
        if version >= target_version || !is_same_repo(&pr, owner, repo) {
            continue;
        }
        tracing::info!(
            owner = %owner,
            repo = %repo,
            pull = pr.number,
            superseded = version,
            target = target_version,
            "issue-templates: closing stale-version install PR"
        );
        if let Err(error) = api.delete_ref(token, owner, repo, &pr.head_ref).await {
            tracing::warn!(
                owner = %owner,
                repo = %repo,
                pull = pr.number,
                error = %error,
                "issue-templates: stale-version install PR could not be closed; leaving it open"
            );
        }
    }
}

/// Install/update all bundled templates in `owner/repo` to `target_version` via
/// a PR onto the default branch, merged immediately where the repository allows
/// it. See the module docs for the never-churn and never-merge-foreign-content
/// contracts.
pub(super) async fn install_templates_with_api(
    api: &dyn GithubApi,
    token: &SecretString,
    owner: &str,
    repo: &str,
    target_version: u32,
) -> Result<TemplateInstallOutcome, GithubAppError> {
    // Exact + owner-qualified: a busy repo's 100-PR listing page cannot hide
    // this, and a fork branch of the same name cannot impersonate it.
    let pending = api
        .open_pull_for_head(token, owner, repo, &template_branch(target_version))
        .await?;

    if let Some(pending) = pending {
        return resume_pending_pull(api, token, owner, repo, target_version, pending).await;
    }

    supersede_older_pulls(api, token, owner, repo, target_version).await;
    open_install_pull(api, token, owner, repo, target_version).await
}
