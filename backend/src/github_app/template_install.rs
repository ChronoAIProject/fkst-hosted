//! Effectful install/update orchestration for the bundled issue templates:
//! the branch + PR + immediate-merge flow behind
//! [`IssueTemplateGithub::install_templates`], decoupled from token minting so
//! it is unit-testable against a fake [`GithubApi`].
//!
//! Merge-blocked repos are a supported steady state, not a failure. On a repo
//! whose default branch enforces required checks/reviews, the immediate merge
//! is rejected — the PR is then LEFT OPEN ([`TemplateInstallOutcome::PendingPr`])
//! for the repo's own CI/review flow, and the next ensure pass (TTL-gated by
//! the caller) finds and reuses that same PR instead of opening a replacement.
//! The head branch of an open install PR is never deleted: deleting it
//! force-closes the PR, which is exactly the churn loop this module replaces
//! (one closed PR per reconcile, forever — see issue #5578).
//!
//! [`IssueTemplateGithub::install_templates`]: super::templates::IssueTemplateGithub::install_templates

use secrecy::SecretString;

use super::api::GithubApi;
use super::templates::{bundled_templates, encode_content, TemplateInstallOutcome};
use super::GithubAppError;

/// Head-branch name of the install PR for `target_version`.
fn template_branch(target_version: u32) -> String {
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

/// Install/update all bundled templates in `owner/repo` to `target_version`
/// via a PR onto the default branch, merged immediately when the repo allows
/// it. See the module docs for the pending-PR contract on protected branches.
pub(super) async fn install_templates_with_api(
    api: &dyn GithubApi,
    token: &SecretString,
    owner: &str,
    repo: &str,
    target_version: u32,
) -> Result<TemplateInstallOutcome, GithubAppError> {
    let branch = template_branch(target_version);
    let title = pr_title(target_version);

    // One open-PR listing serves two purposes: find a still-open install PR
    // from a prior merge-blocked run (reuse it instead of churning a
    // replacement), and supersede install PRs left open for an OLDER bundled
    // version (delete their head branches, which closes them) so two template
    // PRs are never open at once after a version bump.
    let mut pending: Option<u64> = None;
    for pr in api.list_open_pulls(token, owner, repo).await? {
        match template_branch_version(&pr.head_ref) {
            Some(version) if version == target_version => pending = Some(pr.number),
            Some(version) => {
                tracing::info!(
                    owner = %owner,
                    repo = %repo,
                    pull = pr.number,
                    superseded = version,
                    target = target_version,
                    "issue-templates: closing stale-version install PR"
                );
                // Best-effort: a failure leaves the stale PR open and the
                // next ensure pass retries the supersede.
                api.delete_ref(token, owner, repo, &pr.head_ref).await.ok();
            }
            None => {}
        }
    }

    if let Some(number) = pending {
        // A previous run opened this exact PR but could not merge it. Retry
        // the merge — the repo's required checks may have passed since — and
        // when still blocked leave the PR (and its branch!) untouched.
        return match api
            .merge_pull_request(token, owner, repo, number, &title)
            .await
        {
            Ok(()) => {
                api.delete_ref(token, owner, repo, &branch).await.ok();
                Ok(TemplateInstallOutcome::Merged)
            }
            Err(error) => {
                tracing::info!(
                    owner = %owner,
                    repo = %repo,
                    pull = number,
                    error = %error,
                    "issue-templates: pending install PR still unmergeable; leaving it open"
                );
                Ok(TemplateInstallOutcome::PendingPr { number })
            }
        };
    }

    let base = api.repo_default_branch(token, owner, repo).await?;
    let head_sha = api
        .branch_head_sha(token, owner, repo, &base)
        .await?
        .ok_or_else(|| {
            GithubAppError::Http(format!("repository default branch {base:?} has no Git ref"))
        })?;

    // Create the working branch. A surviving branch WITHOUT an open PR (none
    // was found above) is stale — a crashed run or a manually closed PR — so
    // drop and recreate it to start from the current head.
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
            // Expected on protected default branches (required checks /
            // reviews). NOT an error: the PR stays open for the repo's own
            // merge flow and the caller gates the next attempt to its TTL.
            tracing::info!(
                owner = %owner,
                repo = %repo,
                pull = number,
                error = %error,
                "issue-templates: immediate merge blocked; leaving install PR open"
            );
            Ok(TemplateInstallOutcome::PendingPr { number })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use crate::github_app::api::{PullRequestSummary, RemoteFile};

    use super::*;

    fn token() -> SecretString {
        SecretString::from("test-token".to_string())
    }

    fn summary(number: u64, head_ref: &str) -> PullRequestSummary {
        PullRequestSummary {
            number,
            author_login: "fkst-app[bot]".to_string(),
            head_sha: "abc123".to_string(),
            head_ref: head_ref.to_string(),
            title: "Install/Update fkst issue templates".to_string(),
        }
    }

    /// Fake [`GithubApi`] recording every call as `"op:detail"`, with
    /// configurable open-PR listing, merge outcome, and a first-`create_ref`
    /// [`GithubAppError::RefExists`]. Unimplemented trait methods keep their
    /// panicking defaults, so an unexpected call fails the test loudly.
    struct FakeApi {
        open_pulls: Vec<PullRequestSummary>,
        merge_ok: bool,
        ref_exists_once: bool,
        calls: Mutex<Vec<String>>,
    }

    impl FakeApi {
        fn new(open_pulls: Vec<PullRequestSummary>, merge_ok: bool) -> Self {
            Self {
                open_pulls,
                merge_ok,
                ref_exists_once: false,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }

        fn count(&self, prefix: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.starts_with(prefix))
                .count()
        }
    }

    #[async_trait]
    impl GithubApi for FakeApi {
        // The two trait methods without panicking defaults; the install
        // orchestration never mints tokens, so a call here is a test failure.
        async fn installation_for_repo(
            &self,
            _app_jwt: &SecretString,
            _owner: &str,
            _repo: &str,
        ) -> Result<crate::github_app::api::InstallationId, GithubAppError> {
            unimplemented!("install orchestration must not resolve installations")
        }

        async fn create_installation_token(
            &self,
            _app_jwt: &SecretString,
            _id: crate::github_app::api::InstallationId,
            _req: &crate::github_app::api::InstallationTokenRequest,
        ) -> Result<crate::github_app::api::InstallationToken, GithubAppError> {
            unimplemented!("install orchestration must not mint tokens")
        }

        async fn list_open_pulls(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
        ) -> Result<Vec<PullRequestSummary>, GithubAppError> {
            self.record("list_open_pulls".to_string());
            Ok(self.open_pulls.clone())
        }

        async fn repo_default_branch(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
        ) -> Result<String, GithubAppError> {
            self.record("repo_default_branch".to_string());
            Ok("main".to_string())
        }

        async fn branch_head_sha(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
            branch: &str,
        ) -> Result<Option<String>, GithubAppError> {
            self.record(format!("branch_head_sha:{branch}"));
            Ok(Some("headsha".to_string()))
        }

        async fn create_ref(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
            branch: &str,
            _sha: &str,
        ) -> Result<(), GithubAppError> {
            self.record(format!("create_ref:{branch}"));
            if self.ref_exists_once && self.count("create_ref:") == 1 {
                return Err(GithubAppError::RefExists);
            }
            Ok(())
        }

        async fn content_file(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
            path: &str,
            _git_ref: Option<&str>,
        ) -> Result<Option<RemoteFile>, GithubAppError> {
            self.record(format!("content_file:{path}"));
            Ok(Some(RemoteFile {
                sha: "blobsha".to_string(),
                content_base64: String::new(),
            }))
        }

        #[allow(clippy::too_many_arguments)]
        async fn put_file(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
            path: &str,
            _message: &str,
            _content_base64: &str,
            _branch: &str,
            _sha: Option<&str>,
        ) -> Result<(), GithubAppError> {
            self.record(format!("put_file:{path}"));
            Ok(())
        }

        #[allow(clippy::too_many_arguments)]
        async fn create_pull_request(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
            _title: &str,
            head: &str,
            _base: &str,
            _body: &str,
        ) -> Result<u64, GithubAppError> {
            self.record(format!("create_pull_request:{head}"));
            Ok(77)
        }

        async fn merge_pull_request(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
            number: u64,
            _commit_title: &str,
        ) -> Result<(), GithubAppError> {
            self.record(format!("merge_pull_request:{number}"));
            if self.merge_ok {
                Ok(())
            } else {
                Err(GithubAppError::Http(
                    "405: required status checks have not succeeded".to_string(),
                ))
            }
        }

        async fn delete_ref(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
            branch: &str,
        ) -> Result<(), GithubAppError> {
            self.record(format!("delete_ref:{branch}"));
            Ok(())
        }
    }

    async fn install(api: &FakeApi, target: u32) -> TemplateInstallOutcome {
        install_templates_with_api(api, &token(), "acme", "site", target)
            .await
            .expect("install must not error")
    }

    #[tokio::test]
    async fn fresh_install_merges_and_cleans_up() {
        let api = FakeApi::new(vec![], true);
        assert_eq!(install(&api, 10).await, TemplateInstallOutcome::Merged);
        assert_eq!(api.count("put_file:"), bundled_templates().len());
        assert_eq!(api.count("create_pull_request:"), 1);
        assert_eq!(api.count("merge_pull_request:"), 1);
        assert_eq!(
            api.count("delete_ref:fkst/issue-templates-v10"),
            1,
            "merged branch is cleaned up"
        );
    }

    #[tokio::test]
    async fn merge_blocked_leaves_pr_and_branch_open() {
        let api = FakeApi::new(vec![], false);
        assert_eq!(
            install(&api, 10).await,
            TemplateInstallOutcome::PendingPr { number: 77 }
        );
        assert_eq!(
            api.count("delete_ref:"),
            0,
            "a blocked PR's head branch must never be deleted (deleting it \
             force-closes the PR — the churn loop)"
        );
    }

    #[tokio::test]
    async fn pending_pr_is_reused_not_churned() {
        let api = FakeApi::new(vec![summary(55, "fkst/issue-templates-v10")], false);
        assert_eq!(
            install(&api, 10).await,
            TemplateInstallOutcome::PendingPr { number: 55 }
        );
        assert_eq!(api.count("create_ref:"), 0, "no new branch");
        assert_eq!(api.count("put_file:"), 0, "no rewritten files");
        assert_eq!(api.count("create_pull_request:"), 0, "no replacement PR");
        assert_eq!(api.count("delete_ref:"), 0, "the open PR's branch survives");
        assert_eq!(api.count("merge_pull_request:55"), 1, "merge is retried");
    }

    #[tokio::test]
    async fn pending_pr_merge_retry_converges() {
        let api = FakeApi::new(vec![summary(55, "fkst/issue-templates-v10")], true);
        assert_eq!(install(&api, 10).await, TemplateInstallOutcome::Merged);
        assert_eq!(
            api.count("delete_ref:fkst/issue-templates-v10"),
            1,
            "merged branch is cleaned up"
        );
        assert_eq!(api.count("create_pull_request:"), 0, "no replacement PR");
    }

    #[tokio::test]
    async fn stale_version_pr_is_superseded() {
        let api = FakeApi::new(vec![summary(41, "fkst/issue-templates-v9")], true);
        assert_eq!(install(&api, 10).await, TemplateInstallOutcome::Merged);
        assert_eq!(
            api.count("delete_ref:fkst/issue-templates-v9"),
            1,
            "the older-version PR is closed via its head branch"
        );
        assert_eq!(api.count("create_pull_request:"), 1, "v10 PR still opens");
    }

    #[tokio::test]
    async fn non_template_prs_are_untouched() {
        let api = FakeApi::new(vec![summary(3, "devloop/issue/acme/site/12/fix")], true);
        assert_eq!(install(&api, 10).await, TemplateInstallOutcome::Merged);
        assert_eq!(
            api.count("delete_ref:devloop"),
            0,
            "unrelated PR branches are never deleted"
        );
    }

    #[tokio::test]
    async fn stale_branch_without_pr_is_recreated() {
        let mut api = FakeApi::new(vec![], true);
        api.ref_exists_once = true;
        assert_eq!(install(&api, 10).await, TemplateInstallOutcome::Merged);
        assert_eq!(
            api.count("create_ref:"),
            2,
            "delete + recreate on RefExists"
        );
        // One delete for the stale branch, one cleanup after the merge.
        assert_eq!(api.count("delete_ref:fkst/issue-templates-v10"), 2);
    }

    #[test]
    fn template_branch_version_parses_only_install_branches() {
        assert_eq!(
            template_branch_version("fkst/issue-templates-v10"),
            Some(10)
        );
        assert_eq!(template_branch_version("fkst/issue-templates-v9"), Some(9));
        assert_eq!(template_branch_version("fkst/issue-templates-vX"), None);
        assert_eq!(template_branch_version("devloop/issue/a/b/1/x"), None);
        assert_eq!(template_branch_version("main"), None);
    }
}
