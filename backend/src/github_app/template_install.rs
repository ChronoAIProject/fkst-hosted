//! Effectful install/update orchestration for the bundled issue templates:
//! the branch + PR + immediate-merge flow behind
//! [`IssueTemplateGithub::install_templates`], decoupled from token minting so
//! it is unit-testable against a fake [`GithubApi`].
//!
//! [`IssueTemplateGithub::install_templates`]: super::templates::IssueTemplateGithub::install_templates

use secrecy::SecretString;

use super::api::GithubApi;
use super::templates::{bundled_templates, encode_content};
use super::GithubAppError;

/// Install/update all bundled templates in `owner/repo` to `target_version`
/// via a single merged PR onto the default branch.
pub(super) async fn install_templates_with_api(
    api: &dyn GithubApi,
    token: &SecretString,
    owner: &str,
    repo: &str,
    target_version: u32,
) -> Result<(), GithubAppError> {
    let base = api.repo_default_branch(token, owner, repo).await?;
    let head_sha = api
        .branch_head_sha(token, owner, repo, &base)
        .await?
        .ok_or_else(|| {
            GithubAppError::Http(format!("repository default branch {base:?} has no Git ref"))
        })?;
    let branch = format!("fkst/issue-templates-v{target_version}");

    // Create the working branch. If a stale one lingers from a prior failed
    // run, drop and recreate it so this run starts from the current head.
    if let Err(GithubAppError::RefExists) =
        api.create_ref(token, owner, repo, &branch, &head_sha).await
    {
        api.delete_ref(token, owner, repo, &branch).await.ok();
        api.create_ref(token, owner, repo, &branch, &head_sha)
            .await?;
    }

    for tf in bundled_templates() {
        // An existing blob on the base branch => UPDATE (PUT requires its
        // sha); None => CREATE.
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

    let title = format!("Install/Update fkst issue templates to v{target_version}");
    let number = api
        .create_pull_request(
            token,
            owner,
            repo,
            &title,
            &branch,
            &base,
            "Automated by fkst-hosted: bundled issue templates (trusted fixed content). \
             Merged without review/CI by design.",
        )
        .await?;
    api.merge_pull_request(token, owner, repo, number, &title)
        .await?;
    // Best-effort cleanup of the merged branch; a failure here never fails
    // the install (the PR is already merged).
    api.delete_ref(token, owner, repo, &branch).await.ok();
    Ok(())
}
