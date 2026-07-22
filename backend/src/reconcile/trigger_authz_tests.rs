use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::listing::{InstallationSummary, IssueSummary};
use crate::github_app::GithubAppError;
use crate::models::GithubActor;

enum Reply {
    Role(Option<String>),
    Error,
}

struct FakeListing {
    reply: Reply,
    calls: AtomicUsize,
}

impl FakeListing {
    fn role(role: Option<&str>) -> Self {
        Self {
            reply: Reply::Role(role.map(str::to_string)),
            calls: AtomicUsize::new(0),
        }
    }

    fn error() -> Self {
        Self {
            reply: Reply::Error,
            calls: AtomicUsize::new(0),
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
        Ok(0)
    }

    async fn get_collaborator_role(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _username: &str,
    ) -> Result<Option<String>, GithubAppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.reply {
            Reply::Role(role) => Ok(role.clone()),
            Reply::Error => Err(GithubAppError::Http("role lookup failed".to_string())),
        }
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

    async fn list_repo_admins(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<Vec<GithubActor>, GithubAppError> {
        Ok(Vec::new())
    }
}

fn token() -> SecretString {
    SecretString::from("token".to_string())
}

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

fn creator(login: &str, id: Option<i64>) -> SessionCreator {
    SessionCreator {
        login: login.to_string(),
        id,
    }
}

fn access(vars: &[(&str, &str)]) -> AccessPolicy {
    AccessPolicy::from_vars(
        &vars
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<Vec<_>>(),
    )
    .expect("access policy parses")
}

async fn decision(listing: &FakeListing, creator: &SessionCreator) -> TriggerGateDecision {
    check_trigger_creator(
        listing,
        &token(),
        &repo(),
        &access(&[]),
        creator,
        &mut TriggerAuthzCache::default(),
    )
    .await
}

#[tokio::test]
async fn global_admin_short_circuits_without_api_call() {
    let listing = FakeListing::error();
    let result = check_trigger_creator(
        &listing,
        &token(),
        &repo(),
        &access(&[("FKST_GLOBAL_ADMINS", "@Alice")]),
        &creator("alice", Some(42)),
        &mut TriggerAuthzCache::default(),
    )
    .await;
    assert_eq!(result, TriggerGateDecision::Authorized);
    assert_eq!(listing.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn admin_and_maintain_roles_are_authorized() {
    for role in ["admin", "maintain"] {
        let listing = FakeListing::role(Some(role));
        assert_eq!(
            decision(&listing, &creator("alice", Some(42))).await,
            TriggerGateDecision::Authorized,
            "role={role}"
        );
    }
}

#[tokio::test]
async fn lesser_or_missing_roles_are_unauthorized() {
    for role in [Some("write"), Some("triage"), Some("read"), None] {
        let listing = FakeListing::role(role);
        assert_eq!(
            decision(&listing, &creator("alice", Some(42))).await,
            TriggerGateDecision::Unauthorized {
                reason: "@alice does not have admin or maintain permission on acme/site"
                    .to_string(),
            },
            "role={role:?}"
        );
    }
}

#[tokio::test]
async fn transport_error_defers_the_decision() {
    let listing = FakeListing::error();
    assert_eq!(
        decision(&listing, &creator("alice", Some(42))).await,
        TriggerGateDecision::Deferred
    );
}

#[tokio::test]
async fn role_lookup_is_cached_case_insensitively_per_pass() {
    let listing = FakeListing::role(Some("maintain"));
    let mut cache = TriggerAuthzCache::default();
    for login in ["Alice", "alice"] {
        assert_eq!(
            check_trigger_creator(
                &listing,
                &token(),
                &repo(),
                &access(&[]),
                &creator(login, Some(42)),
                &mut cache,
            )
            .await,
            TriggerGateDecision::Authorized
        );
    }
    assert_eq!(listing.calls.load(Ordering::SeqCst), 1);
}
