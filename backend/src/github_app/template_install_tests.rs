//! Tests for the issue-template install orchestration (extracted from
//! `template_install.rs` to keep it under the 500-line budget; sibling `#[path]`
//! module, mirroring `automerge_tests.rs` / `token_rotation_tests.rs`).
//!
//! The fake transport records every call as `"op:detail"`, so each test asserts
//! on what the orchestration DID — in particular that it never deletes a branch
//! backing an open pull request, which is the churn loop of issue #5578.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::github_app::api::{
    InstallationId, InstallationToken, InstallationTokenRequest, PullFileMeta, PullRequestSummary,
    RemoteFile,
};

use super::*;

const OWNER: &str = "acme";
const REPO: &str = "site";
const TARGET: u32 = 10;

fn token() -> SecretString {
    SecretString::from("test-token".to_string())
}

/// An open PR on `head_ref` whose head repository is `head_repo` (`None` models
/// a deleted head repository).
fn pull(number: u64, head_ref: &str, head_repo: Option<&str>) -> PullRequestSummary {
    PullRequestSummary {
        number,
        author_login: "fkst-app[bot]".to_string(),
        head_sha: "abc123".to_string(),
        head_ref: head_ref.to_string(),
        head_repo_full_name: head_repo.map(str::to_string),
        title: "Install/Update fkst issue templates".to_string(),
    }
}

/// An open install PR for [`TARGET`], from this repository (the normal shape a
/// merge-blocked previous pass leaves behind).
fn ours(number: u64) -> PullRequestSummary {
    pull(number, &template_branch(TARGET), Some("acme/site"))
}

fn changed(path: &str) -> PullFileMeta {
    PullFileMeta {
        filename: path.to_string(),
        status: "modified".to_string(),
        additions: 1,
        deletions: 1,
        changes: 2,
        sha: "blobsha".to_string(),
        previous_filename: None,
    }
}

/// Exactly the diff the App's own install PR produces.
fn template_files() -> Vec<PullFileMeta> {
    bundled_templates()
        .iter()
        .map(|tf| changed(tf.path))
        .collect()
}

/// Fake [`GithubApi`] recording every call as `"op:detail"`. Every knob models
/// one GitHub state the orchestration must handle; unset knobs give the happy
/// path. Trait methods the orchestration must never call keep their panicking
/// defaults, so an unexpected call fails the test loudly.
#[derive(Default)]
struct FakeApi {
    /// Result of the exact head-scoped pending lookup.
    pending: Option<PullRequestSummary>,
    /// Result of the broad listing used only for stale-version cleanup.
    open_pulls: Vec<PullRequestSummary>,
    /// Files the pending PR changes (defaults to the bundled template paths).
    pull_files: Option<Vec<PullFileMeta>>,
    merge_ok: bool,
    /// GitHub's `mergeable` tri-state for the pending PR.
    mergeable: Option<bool>,
    ref_exists_once: bool,
    fail_list_open_pulls: bool,
    fail_delete_ref: bool,
    fail_pull_files: bool,
    calls: Mutex<Vec<String>>,
}

impl FakeApi {
    /// A repo with no install PR yet, whose merges succeed.
    fn fresh() -> Self {
        Self {
            merge_ok: true,
            ..Self::default()
        }
    }

    /// A repo whose default branch rejects the App's merge.
    fn merge_blocked() -> Self {
        Self::default()
    }

    fn with_pending(mut self, pending: PullRequestSummary) -> Self {
        self.pending = Some(pending);
        self
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
    // The two methods without panicking defaults; the install orchestration
    // never mints tokens, so a call here is a test failure.
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        unimplemented!("install orchestration must not resolve installations")
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        _id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        unimplemented!("install orchestration must not mint tokens")
    }

    async fn open_pull_for_head(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        branch: &str,
    ) -> Result<Option<PullRequestSummary>, GithubAppError> {
        self.record(format!("open_pull_for_head:{branch}"));
        Ok(self.pending.clone())
    }

    async fn list_open_pulls(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<Vec<PullRequestSummary>, GithubAppError> {
        self.record("list_open_pulls".to_string());
        if self.fail_list_open_pulls {
            return Err(GithubAppError::Http("listing boom".to_string()));
        }
        Ok(self.open_pulls.clone())
    }

    async fn list_pull_files(
        &self,
        _installation_token: &str,
        _owner: &str,
        _repo: &str,
        pull_number: i64,
    ) -> Result<Vec<PullFileMeta>, GithubAppError> {
        self.record(format!("list_pull_files:{pull_number}"));
        if self.fail_pull_files {
            return Err(GithubAppError::Http("files boom".to_string()));
        }
        Ok(self.pull_files.clone().unwrap_or_else(template_files))
    }

    async fn pull_request_mergeable(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        number: u64,
    ) -> Result<Option<bool>, GithubAppError> {
        self.record(format!("pull_request_mergeable:{number}"));
        Ok(self.mergeable)
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
        if self.fail_delete_ref {
            return Err(GithubAppError::Http("delete boom".to_string()));
        }
        Ok(())
    }
}

async fn install(api: &FakeApi) -> TemplateInstallOutcome {
    install_templates_with_api(api, &token(), OWNER, REPO, TARGET)
        .await
        .expect("install must not error")
}

/// The invariant the whole module exists to protect: no branch backing an open
/// pull request is ever deleted, because deleting it force-closes that PR.
fn assert_no_branch_deleted(api: &FakeApi) {
    assert_eq!(
        api.count("delete_ref:"),
        0,
        "deleting an open PR's head branch force-closes it — the churn loop"
    );
}

// ---- fresh install -------------------------------------------------------

#[tokio::test]
async fn fresh_install_merges_and_cleans_up() {
    let api = FakeApi::fresh();
    assert_eq!(install(&api).await, TemplateInstallOutcome::Merged);
    assert_eq!(api.count("put_file:"), bundled_templates().len());
    assert_eq!(api.count("create_pull_request:"), 1);
    assert_eq!(api.count("merge_pull_request:"), 1);
    assert_eq!(
        api.count("delete_ref:fkst/issue-templates-v10"),
        1,
        "the merged branch is cleaned up"
    );
}

#[tokio::test]
async fn merge_blocked_leaves_pr_and_branch_open() {
    let api = FakeApi::merge_blocked();
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 77 }
    );
    assert_no_branch_deleted(&api);
}

#[tokio::test]
async fn stale_branch_without_pr_is_recreated() {
    // No open PR holds the branch, so a surviving ref is debris from a crashed
    // run: recreate it from the current head.
    let api = FakeApi {
        ref_exists_once: true,
        ..FakeApi::fresh()
    };
    assert_eq!(install(&api).await, TemplateInstallOutcome::Merged);
    assert_eq!(
        api.count("create_ref:"),
        2,
        "delete + recreate on RefExists"
    );
    assert_eq!(api.count("delete_ref:fkst/issue-templates-v10"), 2);
}

// ---- reusing a pending install PR ----------------------------------------

#[tokio::test]
async fn pending_pr_is_reused_not_churned() {
    let api = FakeApi::merge_blocked().with_pending(ours(55));
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 55 }
    );
    assert_eq!(api.count("create_ref:"), 0, "no new branch");
    assert_eq!(api.count("put_file:"), 0, "no rewritten files");
    assert_eq!(api.count("create_pull_request:"), 0, "no replacement PR");
    assert_eq!(
        api.count("merge_pull_request:55"),
        1,
        "the merge is retried"
    );
    assert_no_branch_deleted(&api);
}

#[tokio::test]
async fn pending_pr_merge_retry_converges() {
    let api = FakeApi {
        merge_ok: true,
        ..FakeApi::default()
    }
    .with_pending(ours(55));
    assert_eq!(install(&api).await, TemplateInstallOutcome::Merged);
    assert_eq!(api.count("create_pull_request:"), 0, "no replacement PR");
    assert_eq!(
        api.count("delete_ref:fkst/issue-templates-v10"),
        1,
        "the merged branch is cleaned up"
    );
}

#[tokio::test]
async fn pending_lookup_is_scoped_to_the_install_branch() {
    let api = FakeApi::fresh();
    install(&api).await;
    assert_eq!(
        api.count("open_pull_for_head:fkst/issue-templates-v10"),
        1,
        "the pending lookup must be the exact, owner-qualified head query — a \
         broad listing can page past the PR on a busy repo"
    );
}

// ---- refusing to merge pull requests this app did not write --------------

#[tokio::test]
async fn fork_pull_request_on_the_install_branch_is_never_merged() {
    // A fork's head.ref is an unqualified branch name, so it can claim any
    // App-owned name. Merging it would hand an outside contributor the App's
    // token and a bypass of the base branch's protection.
    let api = FakeApi::merge_blocked().with_pending(pull(
        91,
        &template_branch(TARGET),
        Some("mallory/site"),
    ));
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 91 }
    );
    assert_eq!(api.count("merge_pull_request:"), 0, "never merged");
    assert_no_branch_deleted(&api);
    assert_eq!(api.count("create_pull_request:"), 0, "and nothing churned");
}

#[tokio::test]
async fn pull_request_with_a_deleted_head_repo_is_never_merged() {
    let api = FakeApi::merge_blocked().with_pending(pull(92, &template_branch(TARGET), None));
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 92 }
    );
    assert_eq!(api.count("merge_pull_request:"), 0);
}

#[tokio::test]
async fn pending_pr_touching_a_foreign_file_is_never_merged() {
    // Same-repo, right branch name — but someone staged an extra file on it.
    let mut files = template_files();
    files.push(changed(".github/workflows/release.yml"));
    let api = FakeApi {
        pull_files: Some(files),
        ..FakeApi::merge_blocked()
    }
    .with_pending(ours(55));
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 55 }
    );
    assert_eq!(
        api.count("merge_pull_request:"),
        0,
        "a diff beyond the bundled template paths is never merged"
    );
    assert_no_branch_deleted(&api);
}

#[tokio::test]
async fn pending_pr_renaming_a_template_away_is_never_merged() {
    let api = FakeApi {
        pull_files: Some(vec![PullFileMeta {
            previous_filename: Some(".github/workflows/release.yml".to_string()),
            status: "renamed".to_string(),
            ..changed(".github/ISSUE_TEMPLATE/config.yml")
        }]),
        ..FakeApi::merge_blocked()
    }
    .with_pending(ours(55));
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 55 }
    );
    assert_eq!(
        api.count("merge_pull_request:"),
        0,
        "a rename also REMOVES its source path, so both ends must be ours"
    );
}

#[tokio::test]
async fn unreadable_pending_pr_files_fail_closed() {
    let api = FakeApi {
        fail_pull_files: true,
        ..FakeApi::merge_blocked()
    }
    .with_pending(ours(55));
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 55 }
    );
    assert_eq!(
        api.count("merge_pull_request:"),
        0,
        "an unverifiable diff is never merged"
    );
}

// ---- conflict recovery ---------------------------------------------------

#[tokio::test]
async fn conflicted_pending_pr_is_rebuilt_from_the_base_head() {
    // GitHub reports the PR as definitively unmergeable: retrying its merge can
    // never work, so the branch is rebuilt (which closes the stale PR).
    let api = FakeApi {
        mergeable: Some(false),
        ..FakeApi::merge_blocked()
    }
    .with_pending(ours(55));
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 77 },
        "a fresh PR replaces the conflicted one"
    );
    assert_eq!(api.count("delete_ref:fkst/issue-templates-v10"), 1);
    assert_eq!(api.count("create_pull_request:"), 1);
    assert_eq!(api.count("put_file:"), bundled_templates().len());
}

#[tokio::test]
async fn uncomputed_mergeable_leaves_the_pending_pr_alone() {
    // `None` means GitHub has not computed mergeability yet — never a licence
    // to delete the branch.
    let api = FakeApi {
        mergeable: None,
        ..FakeApi::merge_blocked()
    }
    .with_pending(ours(55));
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 55 }
    );
    assert_no_branch_deleted(&api);
}

#[tokio::test]
async fn mergeable_pending_pr_is_left_alone() {
    let api = FakeApi {
        mergeable: Some(true),
        ..FakeApi::merge_blocked()
    }
    .with_pending(ours(55));
    assert_eq!(
        install(&api).await,
        TemplateInstallOutcome::Deferred { pull: 55 }
    );
    assert_no_branch_deleted(&api);
}

// ---- superseding older-version install PRs -------------------------------

#[tokio::test]
async fn stale_version_pr_is_superseded() {
    let api = FakeApi {
        open_pulls: vec![pull(41, "fkst/issue-templates-v9", Some("acme/site"))],
        ..FakeApi::fresh()
    };
    assert_eq!(install(&api).await, TemplateInstallOutcome::Merged);
    assert_eq!(
        api.count("delete_ref:fkst/issue-templates-v9"),
        1,
        "the older-version PR is closed via its head branch"
    );
    assert_eq!(
        api.count("create_pull_request:"),
        1,
        "the v10 PR still opens"
    );
}

#[tokio::test]
async fn newer_and_foreign_pulls_are_not_superseded() {
    let api = FakeApi {
        open_pulls: vec![
            // A fork PR named like an install branch says nothing about this repo.
            pull(41, "fkst/issue-templates-v9", Some("mallory/site")),
            // A future version is not stale.
            pull(42, "fkst/issue-templates-v11", Some("acme/site")),
            // An unrelated branch.
            pull(43, "devloop/issue/acme/site/12/fix", Some("acme/site")),
        ],
        ..FakeApi::fresh()
    };
    assert_eq!(install(&api).await, TemplateInstallOutcome::Merged);
    assert_eq!(api.count("delete_ref:fkst/issue-templates-v9"), 0);
    assert_eq!(api.count("delete_ref:fkst/issue-templates-v11"), 0);
    assert_eq!(api.count("delete_ref:devloop"), 0);
}

#[tokio::test]
async fn supersede_failures_never_block_the_install() {
    // Cleanup is cosmetic: neither a failed listing nor a failed branch delete
    // may stop the repo from getting its templates.
    let listing_failed = FakeApi {
        fail_list_open_pulls: true,
        ..FakeApi::fresh()
    };
    assert_eq!(
        install(&listing_failed).await,
        TemplateInstallOutcome::Merged
    );
    assert_eq!(listing_failed.count("create_pull_request:"), 1);

    let delete_failed = FakeApi {
        open_pulls: vec![pull(41, "fkst/issue-templates-v9", Some("acme/site"))],
        fail_delete_ref: true,
        ..FakeApi::fresh()
    };
    assert_eq!(
        install(&delete_failed).await,
        TemplateInstallOutcome::Merged
    );
    assert_eq!(delete_failed.count("create_pull_request:"), 1);
}

// ---- error propagation ---------------------------------------------------

#[tokio::test]
async fn pending_lookup_error_propagates_before_any_mutation() {
    struct FailingLookup;

    #[async_trait]
    impl GithubApi for FailingLookup {
        async fn installation_for_repo(
            &self,
            _app_jwt: &SecretString,
            _owner: &str,
            _repo: &str,
        ) -> Result<InstallationId, GithubAppError> {
            unimplemented!()
        }

        async fn create_installation_token(
            &self,
            _app_jwt: &SecretString,
            _id: InstallationId,
            _req: &InstallationTokenRequest,
        ) -> Result<InstallationToken, GithubAppError> {
            unimplemented!()
        }

        async fn open_pull_for_head(
            &self,
            _token: &SecretString,
            _owner: &str,
            _repo: &str,
            _branch: &str,
        ) -> Result<Option<PullRequestSummary>, GithubAppError> {
            Err(GithubAppError::Http("lookup boom".to_string()))
        }
        // Every other method keeps its panicking default: reaching one would
        // mean the orchestration mutated the repo despite an unknown state.
    }

    let error = install_templates_with_api(&FailingLookup, &token(), OWNER, REPO, TARGET)
        .await
        .expect_err("an unresolvable pending state must not be papered over");
    assert!(matches!(error, GithubAppError::Http(_)));
}

// ---- pure helpers --------------------------------------------------------

#[test]
fn template_branch_version_parses_only_install_branches() {
    assert_eq!(
        template_branch_version("fkst/issue-templates-v11"),
        Some(11)
    );
    assert_eq!(
        template_branch_version("fkst/issue-templates-v10"),
        Some(10)
    );
    assert_eq!(template_branch_version("fkst/issue-templates-vX"), None);
    assert_eq!(template_branch_version("devloop/issue/a/b/1/x"), None);
    assert_eq!(template_branch_version("main"), None);
}

#[test]
fn is_same_repo_compares_the_head_repository_not_the_branch_name() {
    assert!(is_same_repo(&ours(1), OWNER, REPO));
    // GitHub treats owner/name case-insensitively.
    assert!(is_same_repo(
        &pull(1, "any", Some("ACME/Site")),
        OWNER,
        REPO
    ));
    assert!(!is_same_repo(
        &pull(1, &template_branch(TARGET), Some("mallory/site")),
        OWNER,
        REPO
    ));
    assert!(!is_same_repo(&pull(1, "any", None), OWNER, REPO));
}
