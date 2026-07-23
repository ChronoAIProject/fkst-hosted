use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::listing::{InstallationSummary, IssueSummary};
use crate::github_app::GithubAppError;
use crate::goals::trigger_parse::PackageRef;
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::desired::{SessionDef, SessionRegistration};

struct FakeListing {
    count: Result<u64, GithubAppError>,
    issues: Result<Vec<IssueSummary>, GithubAppError>,
    count_calls: AtomicUsize,
    list_calls: AtomicUsize,
}

impl FakeListing {
    fn new(
        count: Result<u64, GithubAppError>,
        issues: Result<Vec<IssueSummary>, GithubAppError>,
    ) -> Self {
        Self {
            count,
            issues,
            count_calls: AtomicUsize::new(0),
            list_calls: AtomicUsize::new(0),
        }
    }

    fn ok(count: u64, issues: Vec<IssueSummary>) -> Self {
        Self::new(Ok(count), Ok(issues))
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
        self.issues.clone()
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

    async fn list_issues_by_label_assignee(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
        _assignee: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        self.issues.clone()
    }

    async fn count_open_issues_with_label_assignee(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
        _assignee: &str,
    ) -> Result<u64, GithubAppError> {
        self.count_calls.fetch_add(1, Ordering::SeqCst);
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

    async fn list_repo_admins(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<Vec<GithubActor>, GithubAppError> {
        Ok(Vec::new())
    }
}

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

fn issue(author_id: i64, author_login: &str, assignees: &[&str]) -> IssueSummary {
    IssueSummary {
        number: 5,
        title: "work item".to_string(),
        body: "content must never be consulted".to_string(),
        labels: vec!["fkst-run".to_string()],
        state: "open".to_string(),
        assignees: assignees.iter().map(|value| value.to_string()).collect(),
        user_login: author_login.to_string(),
        user_id: author_id,
    }
}

fn registration() -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: repo(),
        trigger_issue: 1,
        trigger_author_id: 7,
        trigger_author_login: "alice".to_string(),
        creator_login: "alice".to_string(),
        creator_id: Some(7),
        def: SessionDef {
            name: "demo".to_string(),
            packages: Vec::<PackageRef>::new(),
            manifest_refs: Vec::<PackageRef>::new(),
            work_label: Some("fkst-run".to_string()),
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
            source_branch: None,
            target_branch: None,
        },
        effective_packages: Vec::new(),
        session_id: "sess-1".to_string(),
        config_hash: "hash".to_string(),
        auto_merge: false,
        log_access: vec![],
        collaborators: vec!["bob".to_string()],
    }
}

fn access(global_admins: &str) -> AccessPolicy {
    AccessPolicy::from_vars(&[("FKST_GLOBAL_ADMINS".to_string(), global_admins.to_string())])
        .expect("access")
}

async fn pending(
    listing: &FakeListing,
    reg: &SessionRegistration,
    policy: &AccessPolicy,
) -> Result<bool, AppError> {
    let token = SecretString::from("ghs_x".to_string());
    LabelCountPending::new(listing, &token)
        .has_pending(42, &repo(), &["fkst-run".to_string()], reg, policy)
        .await
}

#[tokio::test]
async fn zero_count_short_circuits_without_listing() {
    let listing = FakeListing::ok(0, vec![issue(7, "alice", &["alice"])]);
    assert!(!pending(&listing, &registration(), &access(""))
        .await
        .unwrap());
    assert_eq!(listing.count_calls.load(Ordering::SeqCst), 1);
    assert_eq!(listing.list_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn positive_count_with_only_wrong_or_ambiguous_assignees_is_not_pending() {
    let listing = FakeListing::ok(
        2,
        vec![
            issue(7, "alice", &["bob"]),
            issue(7, "alice", &["alice", "bob"]),
        ],
    );
    assert!(!pending(&listing, &registration(), &access(""))
        .await
        .unwrap());
    assert_eq!(listing.list_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn routed_authorized_issue_is_pending() {
    let listing = FakeListing::ok(1, vec![issue(7, "alice", &["ALICE"])]);
    assert!(pending(&listing, &registration(), &access(""))
        .await
        .unwrap());
}

#[tokio::test]
async fn routed_collaborator_or_global_admin_issue_is_pending() {
    let collaborator = FakeListing::ok(1, vec![issue(88, "BOB", &["alice"])]);
    assert!(pending(&collaborator, &registration(), &access(""))
        .await
        .unwrap());

    let admin = FakeListing::ok(1, vec![issue(99, "deploy-admin", &["alice"])]);
    assert!(pending(&admin, &registration(), &access("Deploy-Admin"))
        .await
        .unwrap());
}

#[tokio::test]
async fn routed_unauthorized_issue_is_not_pending() {
    let listing = FakeListing::ok(1, vec![issue(99, "mallory", &["alice"])]);
    assert!(!pending(&listing, &registration(), &access(""))
        .await
        .unwrap());
}

#[tokio::test]
async fn configured_app_child_is_pending_but_an_unconfigured_bot_is_not() {
    let listing = FakeListing::ok(1, vec![issue(9000, "fkst-app[bot]", &["alice"])]);
    let token = SecretString::from("ghs_x".to_string());
    let reg = registration();
    let policy = access("");

    let configured = LabelCountPending::new_with_bot_login(&listing, &token, Some("app/FKST-App"))
        .has_pending(42, &repo(), &["fkst-run".to_string()], &reg, &policy)
        .await
        .unwrap();
    assert!(configured);

    let mismatched =
        LabelCountPending::new_with_bot_login(&listing, &token, Some("other-app[bot]"))
            .has_pending(42, &repo(), &["fkst-run".to_string()], &reg, &policy)
            .await
            .unwrap();
    assert!(!mismatched);
}

#[tokio::test]
async fn empty_label_set_never_calls_github() {
    let listing = FakeListing::ok(1, vec![issue(7, "alice", &["alice"])]);
    let token = SecretString::from("ghs_x".to_string());
    let result = LabelCountPending::new(&listing, &token)
        .has_pending(42, &repo(), &[], &registration(), &access(""))
        .await
        .unwrap();
    assert!(!result);
    assert_eq!(listing.count_calls.load(Ordering::SeqCst), 0);
    assert_eq!(listing.list_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn count_and_listing_transport_errors_propagate() {
    let count_error = FakeListing::new(Err(GithubAppError::RateLimited(30)), Ok(Vec::new()));
    assert!(matches!(
        pending(&count_error, &registration(), &access("")).await,
        Err(AppError::Unavailable(_))
    ));

    let list_error = FakeListing::new(Ok(1), Err(GithubAppError::RateLimited(30)));
    assert!(matches!(
        pending(&list_error, &registration(), &access("")).await,
        Err(AppError::Unavailable(_))
    ));
}
