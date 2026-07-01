//! Unit tests for the [`LabelCountPending`] spawn/idle gate, driven against a fake
//! [`GithubListing`] so no network is touched: a positive count is pending, a zero
//! count is not, and a transport error propagates.

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::listing::{InstallationSummary, IssueSummary};
use crate::github_app::GithubAppError;

/// A fake listing whose open-issue count (or error) is fixed per construction.
struct FakeListing {
    count: Result<u64, GithubAppError>,
}

impl FakeListing {
    fn ok(count: u64) -> Self {
        Self { count: Ok(count) }
    }
    fn err() -> Self {
        Self {
            count: Err(GithubAppError::RateLimited(30)),
        }
    }
}

#[async_trait]
impl GithubListing for FakeListing {
    async fn list_issues_by_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError> {
        Ok(Vec::new())
    }

    async fn count_open_issues_with_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
    ) -> Result<u64, GithubAppError> {
        self.count.clone()
    }

    async fn list_installations(
        &self,
        _app_jwt: &SecretString,
    ) -> Result<Vec<InstallationSummary>, GithubAppError> {
        Ok(Vec::new())
    }

    async fn list_installation_repos(
        &self,
        _token: &SecretString,
    ) -> Result<Vec<RepoRef>, GithubAppError> {
        Ok(Vec::new())
    }
}

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

#[tokio::test]
async fn positive_count_is_pending() {
    let listing = FakeListing::ok(3);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    assert!(gate.has_pending(42, &repo(), "fkst-run").await.expect("ok"));
}

#[tokio::test]
async fn zero_count_is_not_pending() {
    let listing = FakeListing::ok(0);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    assert!(!gate.has_pending(42, &repo(), "fkst-run").await.expect("ok"));
}

#[tokio::test]
async fn transport_error_propagates() {
    let listing = FakeListing::err();
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let err = gate
        .has_pending(42, &repo(), "fkst-run")
        .await
        .expect_err("must propagate");
    // The rate-limit GithubAppError maps onto AppError::Unavailable (503).
    assert!(matches!(err, AppError::Unavailable(_)));
}
