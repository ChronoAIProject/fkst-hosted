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
/// of open PRs, a per-PR `mergeable` verdict, and records the numbers merged.
#[derive(Default)]
struct FakePrApi {
    pulls: Vec<PullRequestSummary>,
    mergeable: HashMap<u64, Option<bool>>,
    merged: Mutex<Vec<u64>>,
    list_calls: AtomicUsize,
    mergeable_queried: Mutex<Vec<u64>>,
    fail_list: bool,
    fail_merge: bool,
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

fn pr(number: u64, author: &str) -> PullRequestSummary {
    PullRequestSummary {
        number,
        author_login: author.to_string(),
        head_sha: format!("sha{number}"),
    }
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
