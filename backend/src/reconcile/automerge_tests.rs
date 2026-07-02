//! Unit tests for the best-effort bot-PR auto-merge step ([`super`]). A recording
//! fake [`GithubApi`] backs a real [`GithubAppTokens`] (via `with_api`) so the
//! list → mergeable → merge flow is exercised end-to-end without a live GitHub.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest, PullRequestSummary,
};
use crate::github_app::config::GithubAppConfig;
use crate::github_app::{GithubAppError, GithubAppTokens};

/// A recording fake GitHub transport for the auto-merge flow. Returns a fixed set
/// of open PRs, a per-PR `mergeable` verdict, and records the numbers merged and
/// the issue numbers closed.
#[derive(Default)]
struct FakePrApi {
    pulls: Vec<PullRequestSummary>,
    mergeable: HashMap<u64, Option<bool>>,
    merged: Mutex<Vec<u64>>,
    closed_issues: Mutex<Vec<u64>>,
    list_calls: AtomicUsize,
    mergeable_queried: Mutex<Vec<u64>>,
    fail_list: bool,
    fail_merge: bool,
    fail_close: bool,
}

#[async_trait]
impl GithubApi for FakePrApi {
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        Ok(InstallationId(1))
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        _id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        Ok(InstallationToken {
            token: SecretString::from("ghs_fake".to_string()),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        })
    }

    async fn list_open_pulls(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<Vec<PullRequestSummary>, GithubAppError> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_list {
            return Err(GithubAppError::Http("list boom".to_string()));
        }
        Ok(self.pulls.clone())
    }

    async fn pull_request_mergeable(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        number: u64,
    ) -> Result<Option<bool>, GithubAppError> {
        self.mergeable_queried.lock().unwrap().push(number);
        Ok(self.mergeable.get(&number).copied().unwrap_or(None))
    }

    async fn merge_pull_request(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        number: u64,
        _commit_title: &str,
    ) -> Result<(), GithubAppError> {
        if self.fail_merge {
            return Err(GithubAppError::Http("merge boom".to_string()));
        }
        self.merged.lock().unwrap().push(number);
        Ok(())
    }

    async fn close_issue(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        number: u64,
    ) -> Result<(), GithubAppError> {
        if self.fail_close {
            return Err(GithubAppError::Http("close boom".to_string()));
        }
        self.closed_issues.lock().unwrap().push(number);
        Ok(())
    }
}

fn test_config() -> GithubAppConfig {
    use rand::rngs::OsRng;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;
    let mut rng = OsRng;
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("key");
    let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");
    GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(pem.to_string()),
        app_slug: Some("fkst-test".to_string()),
        webhook_secret: None,
        api_base: "https://api.github.com".to_string(),
    }
}

fn tokens(api: Arc<FakePrApi>) -> GithubAppTokens {
    GithubAppTokens::with_api(&test_config(), api).expect("tokens")
}

fn pr_with(number: u64, author: &str, head_ref: &str, title: &str) -> PullRequestSummary {
    PullRequestSummary {
        number,
        author_login: author.to_string(),
        head_sha: format!("sha{number}"),
        head_ref: head_ref.to_string(),
        title: title.to_string(),
    }
}

/// A realistic bot PR whose branch encodes issue `number` (so the default merge
/// path also exercises the post-merge close).
fn pr(number: u64, author: &str) -> PullRequestSummary {
    pr_with(
        number,
        author,
        &format!("devloop/issue/acme/site/{number}/ready-1720000000"),
        &format!("github-devloop implementation for #{number}"),
    )
}

#[tokio::test]
async fn gated_off_when_no_session_opted_in() {
    let api = Arc::new(FakePrApi {
        pulls: vec![pr(1, "fkst-bot")],
        ..Default::default()
    });
    let github = tokens(api.clone());

    auto_merge_bot_pull_requests(&github, "acme/site", Some("fkst-bot"), false).await;

    assert_eq!(
        api.list_calls.load(Ordering::SeqCst),
        0,
        "gated off: no list"
    );
    assert!(api.merged.lock().unwrap().is_empty(), "no merges");
}

#[tokio::test]
async fn skips_when_bot_login_unset() {
    let api = Arc::new(FakePrApi {
        pulls: vec![pr(1, "fkst-bot")],
        ..Default::default()
    });
    let github = tokens(api.clone());

    auto_merge_bot_pull_requests(&github, "acme/site", None, true).await;

    assert_eq!(
        api.list_calls.load(Ordering::SeqCst),
        0,
        "no bot login: no list"
    );
    assert!(api.merged.lock().unwrap().is_empty(), "no merges");
}

#[tokio::test]
async fn merges_only_mergeable_bot_prs() {
    let api = Arc::new(FakePrApi {
        pulls: vec![pr(1, "fkst-bot"), pr(2, "fkst-bot"), pr(3, "fkst-bot")],
        mergeable: HashMap::from([(1, Some(true)), (2, Some(false)), (3, None)]),
        ..Default::default()
    });
    let github = tokens(api.clone());

    auto_merge_bot_pull_requests(&github, "acme/site", Some("fkst-bot"), true).await;

    assert_eq!(
        *api.merged.lock().unwrap(),
        vec![1],
        "only the mergeable=Some(true) bot PR merges"
    );
}

#[tokio::test]
async fn non_bot_prs_are_untouched() {
    let api = Arc::new(FakePrApi {
        pulls: vec![pr(4, "someone-else")],
        mergeable: HashMap::from([(4, Some(true))]),
        ..Default::default()
    });
    let github = tokens(api.clone());

    auto_merge_bot_pull_requests(&github, "acme/site", Some("fkst-bot"), true).await;

    assert!(
        api.merged.lock().unwrap().is_empty(),
        "a non-bot PR is filtered out before merge"
    );
    assert!(
        !api.mergeable_queried.lock().unwrap().contains(&4),
        "a non-bot PR is never even queried for mergeable"
    );
}

#[tokio::test]
async fn merge_failure_is_swallowed() {
    let api = Arc::new(FakePrApi {
        pulls: vec![pr(1, "fkst-bot")],
        mergeable: HashMap::from([(1, Some(true))]),
        fail_merge: true,
        ..Default::default()
    });
    let github = tokens(api.clone());

    // Must return normally despite the merge error (best-effort, non-failing).
    auto_merge_bot_pull_requests(&github, "acme/site", Some("fkst-bot"), true).await;
    assert!(
        api.merged.lock().unwrap().is_empty(),
        "merge failed, nothing recorded"
    );
}

#[tokio::test]
async fn list_failure_is_swallowed() {
    let api = Arc::new(FakePrApi {
        fail_list: true,
        ..Default::default()
    });
    let github = tokens(api.clone());

    // Must return normally despite the list error.
    auto_merge_bot_pull_requests(&github, "acme/site", Some("fkst-bot"), true).await;
    assert!(
        api.merged.lock().unwrap().is_empty(),
        "no merges after list failure"
    );
}

#[tokio::test]
async fn merged_pr_closes_the_linked_issue() {
    // The PR's branch encodes work-issue 42 (distinct from the PR number 1) so the
    // assertion proves the number came from parsing, not from the PR number.
    let api = Arc::new(FakePrApi {
        pulls: vec![pr_with(
            1,
            "fkst-bot",
            "devloop/issue/acme/site/42/ready-x",
            "github-devloop implementation for #99",
        )],
        mergeable: HashMap::from([(1, Some(true))]),
        ..Default::default()
    });
    let github = tokens(api.clone());

    auto_merge_bot_pull_requests(&github, "acme/site", Some("fkst-bot"), true).await;

    assert_eq!(*api.merged.lock().unwrap(), vec![1], "the bot PR merges");
    assert_eq!(
        *api.closed_issues.lock().unwrap(),
        vec![42],
        "branch-derived issue 42 is closed (branch beats the title's #99)"
    );
}

#[tokio::test]
async fn unparseable_pr_merges_without_closing() {
    let api = Arc::new(FakePrApi {
        pulls: vec![pr_with(
            1,
            "fkst-bot",
            "feature/no-issue-here",
            "just a title",
        )],
        mergeable: HashMap::from([(1, Some(true))]),
        ..Default::default()
    });
    let github = tokens(api.clone());

    // No panic, the PR still merges, and no close is attempted for an unknown issue.
    auto_merge_bot_pull_requests(&github, "acme/site", Some("fkst-bot"), true).await;

    assert_eq!(
        *api.merged.lock().unwrap(),
        vec![1],
        "the bot PR still merges"
    );
    assert!(
        api.closed_issues.lock().unwrap().is_empty(),
        "no issue number parsed => no close call"
    );
}

#[tokio::test]
async fn close_failure_is_swallowed() {
    let api = Arc::new(FakePrApi {
        pulls: vec![pr(1, "fkst-bot")],
        mergeable: HashMap::from([(1, Some(true))]),
        fail_close: true,
        ..Default::default()
    });
    let github = tokens(api.clone());

    // The close errors, but the merge already succeeded and nothing propagates.
    auto_merge_bot_pull_requests(&github, "acme/site", Some("fkst-bot"), true).await;

    assert_eq!(*api.merged.lock().unwrap(), vec![1], "merge still recorded");
    assert!(
        api.closed_issues.lock().unwrap().is_empty(),
        "close failed, nothing recorded"
    );
}

#[test]
fn linked_issue_number_parses_branch_ready_form() {
    assert_eq!(
        linked_issue_number("devloop/issue/acme/site/42/ready-1720000000", ""),
        Some(42),
        "the segment three after `issue` is the work-issue number"
    );
}

#[test]
fn linked_issue_number_handles_hyphenated_owner_and_dotted_repo() {
    assert_eq!(
        linked_issue_number("devloop/issue/my-org/my.repo/7/ready-abc", "irrelevant"),
        Some(7)
    );
}

#[test]
fn linked_issue_number_branch_beats_title() {
    assert_eq!(
        linked_issue_number(
            "devloop/issue/acme/site/55/ready-x",
            "github-devloop implementation for #77",
        ),
        Some(55),
        "the branch is the preferred source"
    );
}

#[test]
fn linked_issue_number_falls_back_to_title_for_hash_form() {
    assert_eq!(
        linked_issue_number("main", "github-devloop implementation for #99"),
        Some(99),
    );
}

#[test]
fn linked_issue_number_falls_back_to_title_for_issue_hash_form() {
    assert_eq!(
        linked_issue_number(
            "feature/x",
            "github-devloop implementation PR for issue #123",
        ),
        Some(123),
    );
}

#[test]
fn linked_issue_number_is_none_for_garbage() {
    assert_eq!(linked_issue_number("feature/foo", "no number at all"), None);
    assert_eq!(linked_issue_number("", ""), None);
    // A `#` with no trailing digits yields nothing rather than a wrong guess.
    assert_eq!(linked_issue_number("main", "closes #"), None);
    // The `issue` marker present but no numeric segment after owner/repo.
    assert_eq!(
        linked_issue_number("devloop/issue/acme/site", "title"),
        None
    );
}
