//! Unit tests for the [`LabelCountPending`] spawn/idle gate, driven against a fake
//! [`GithubListing`] so no network is touched: a positive count is pending, a zero
//! count is not, and a transport error propagates.

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::listing::{InstallationSummary, IssueSummary};
use crate::github_app::GithubAppError;
use crate::goals::trigger_parse::PackageRef;
use crate::models::GithubActor;
use crate::reconcile::desired::{SessionDef, SessionRegistration};

/// A fake listing whose open-issue count AND enumerated issues (or error) are fixed
/// per construction. The count feeds the blind [`has_pending`]; the issue list feeds
/// the author-filtered [`has_pending_authorized`].
struct FakeListing {
    count: Result<u64, GithubAppError>,
    issues: Result<Vec<IssueSummary>, GithubAppError>,
}

impl FakeListing {
    fn ok(count: u64) -> Self {
        Self {
            count: Ok(count),
            issues: Ok(Vec::new()),
        }
    }
    fn err() -> Self {
        Self {
            count: Err(GithubAppError::RateLimited(30)),
            issues: Err(GithubAppError::RateLimited(30)),
        }
    }
    /// A listing that enumerates the given open work issues (used by the
    /// author-filtered gate); the blind count is irrelevant here so it stays 0.
    fn with_issues(issues: Vec<IssueSummary>) -> Self {
        Self {
            count: Ok(0),
            issues: Ok(issues),
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

/// A work issue authored by `(user_id, user_login)` carrying `label`.
fn issue(number: i64, user_id: i64, user_login: &str, label: &str) -> IssueSummary {
    IssueSummary {
        number,
        title: "work item".to_string(),
        body: String::new(),
        labels: vec![label.to_string()],
        state: "open".to_string(),
        assignees: Vec::new(),
        user_login: user_login.to_string(),
        user_id,
    }
}

/// A registration whose trigger author is `trigger_author_id` and whose Session
/// Collaborators are `collaborators`.
fn registration(trigger_author_id: i64, collaborators: &[&str]) -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: repo(),
        trigger_issue: 1,
        trigger_author_id,
        trigger_author_login: "author-login".to_string(),
        def: SessionDef {
            name: "demo".to_string(),
            packages: Vec::<PackageRef>::new(),
            work_label: Some("fkst-run".to_string()),
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
        },
        session_id: "sess-1".to_string(),
        config_hash: "hash".to_string(),
        auto_merge: false,
        log_access: vec![],
        collaborators: collaborators.iter().map(|s| s.to_string()).collect(),
    }
}

fn admin(id: i64, login: &str) -> GithubActor {
    GithubActor {
        id,
        login: login.to_string(),
    }
}

#[tokio::test]
async fn positive_count_is_pending() {
    let listing = FakeListing::ok(3);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    assert!(gate
        .has_pending(42, &repo(), &["fkst-run".to_string()])
        .await
        .expect("ok"));
}

#[tokio::test]
async fn zero_count_is_not_pending() {
    let listing = FakeListing::ok(0);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    assert!(!gate
        .has_pending(42, &repo(), &["fkst-run".to_string()])
        .await
        .expect("ok"));
}

#[tokio::test]
async fn transport_error_propagates() {
    let listing = FakeListing::err();
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let err = gate
        .has_pending(42, &repo(), &["fkst-run".to_string()])
        .await
        .expect_err("must propagate");
    // The rate-limit GithubAppError maps onto AppError::Unavailable (503).
    assert!(matches!(err, AppError::Unavailable(_)));
}

#[tokio::test]
async fn empty_label_set_is_never_pending() {
    let listing = FakeListing::ok(5);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    assert!(!gate.has_pending(42, &repo(), &[]).await.expect("ok"));
}

#[tokio::test]
async fn or_across_labels_short_circuits_on_first_hit() {
    // FakeListing::ok(n) returns n for every label, so any non-empty set with a
    // positive count is pending; the empty-set case is covered above.
    let listing = FakeListing::ok(1);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let labels = vec!["fkst-a".to_string(), "fkst-b".to_string()];
    assert!(gate.has_pending(42, &repo(), &labels).await.expect("ok"));
}

// ---- author-filtered gate (R3 authority) ------------------------------------

#[tokio::test]
async fn authorized_author_is_pending() {
    // One open work issue raised by the session's own trigger author (id 7).
    let listing = FakeListing::with_issues(vec![issue(5, 7, "author-login", "fkst-run")]);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let reg = registration(7, &[]);
    assert!(gate
        .has_pending_authorized(42, &repo(), &["fkst-run".to_string()], &reg, &[])
        .await
        .expect("ok"));
}

#[tokio::test]
async fn admin_author_is_pending() {
    // Issue raised by a repo admin (id 500), not the session author.
    let listing = FakeListing::with_issues(vec![issue(5, 500, "octo-admin", "fkst-run")]);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let reg = registration(7, &[]);
    let admins = [admin(500, "octo-admin")];
    assert!(gate
        .has_pending_authorized(42, &repo(), &["fkst-run".to_string()], &reg, &admins)
        .await
        .expect("ok"));
}

#[tokio::test]
async fn collaborator_author_is_pending() {
    // Issue raised by a listed Session Collaborator (by login).
    let listing = FakeListing::with_issues(vec![issue(5, 999, "bob", "fkst-run")]);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let reg = registration(7, &["bob"]);
    assert!(gate
        .has_pending_authorized(42, &repo(), &["fkst-run".to_string()], &reg, &[])
        .await
        .expect("ok"));
}

#[tokio::test]
async fn only_unauthorized_authors_is_not_pending() {
    // The single open issue is raised by a stranger: not the author, not an admin,
    // not a collaborator — so the session has NO authorized pending work.
    let listing = FakeListing::with_issues(vec![issue(5, 999, "mallory", "fkst-run")]);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let reg = registration(7, &["bob"]);
    let admins = [admin(500, "octo-admin")];
    assert!(!gate
        .has_pending_authorized(42, &repo(), &["fkst-run".to_string()], &reg, &admins)
        .await
        .expect("ok"));
}

#[tokio::test]
async fn mixed_authors_counts_only_the_authorized_one() {
    // Two issues: one from a stranger, one from the author. Pending because ≥1 is
    // authorized (order-independent — `any` short-circuits on the author).
    let listing = FakeListing::with_issues(vec![
        issue(5, 999, "mallory", "fkst-run"),
        issue(6, 7, "author-login", "fkst-run"),
    ]);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let reg = registration(7, &[]);
    assert!(gate
        .has_pending_authorized(42, &repo(), &["fkst-run".to_string()], &reg, &[])
        .await
        .expect("ok"));
}

#[tokio::test]
async fn filtered_empty_label_set_is_never_pending() {
    let listing = FakeListing::with_issues(vec![issue(5, 7, "author-login", "fkst-run")]);
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let reg = registration(7, &[]);
    assert!(!gate
        .has_pending_authorized(42, &repo(), &[], &reg, &[])
        .await
        .expect("ok"));
}

#[tokio::test]
async fn filtered_transport_error_propagates() {
    let listing = FakeListing::err();
    let token = SecretString::from("ghs_x".to_string());
    let gate = LabelCountPending::new(&listing, &token);
    let reg = registration(7, &[]);
    let err = gate
        .has_pending_authorized(42, &repo(), &["fkst-run".to_string()], &reg, &[])
        .await
        .expect_err("must propagate");
    assert!(matches!(err, AppError::Unavailable(_)));
}
