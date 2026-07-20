//! Unit tests for the work-issue acknowledgment step. The renderer is pure; the
//! step runs against a recording fake [`GithubApi`] (so no network is touched) plus
//! a fake [`GithubListing`] whose returned issues (or error) are fixed per
//! construction. Covers: acking an un-acked open work issue, skipping an
//! already-acked one, the no-registration no-op, and swallowing a list/post failure.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::config::GithubAppConfig;
use crate::github_app::listing::{InstallationSummary, IssueSummary};
use crate::github_app::GithubAppError;
use crate::models::GithubActor;
use crate::reconcile::desired::SessionDef;

// ---- recording fake GitHub transport (mirrors execute_tests) ----------------

/// A recorded issue call: `(owner, repo, issue_number, payload)`.
type Call = (String, String, u64, String);
/// A recorded label-add call: `(owner, repo, issue_number, labels)`.
type LabelCall = (String, String, u64, Vec<String>);

#[derive(Default)]
struct RecordingApi {
    comments: Mutex<Vec<Call>>,
    labels_added: Mutex<Vec<LabelCall>>,
    /// When set, `create_issue_comment` fails — exercising the best-effort comment
    /// arm (the latch label must still be added, mirroring the announce arm).
    fail_comment: bool,
}

impl RecordingApi {
    fn with_comment_failure() -> Self {
        Self {
            fail_comment: true,
            ..Self::default()
        }
    }
}

#[async_trait]
impl GithubApi for RecordingApi {
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

    async fn create_issue_comment(
        &self,
        _token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<(), GithubAppError> {
        if self.fail_comment {
            return Err(GithubAppError::Http("boom".to_string()));
        }
        self.comments.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number,
            body.to_string(),
        ));
        Ok(())
    }

    async fn add_issue_labels(
        &self,
        _token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        labels: &[String],
    ) -> Result<(), GithubAppError> {
        self.labels_added.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number,
            labels.to_vec(),
        ));
        Ok(())
    }

    async fn remove_issue_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _label: &str,
    ) -> Result<(), GithubAppError> {
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

fn tokens(api: std::sync::Arc<RecordingApi>) -> GithubAppTokens {
    GithubAppTokens::with_api(&test_config(), api).expect("tokens")
}

// ---- fake listing -----------------------------------------------------------

/// A fake listing whose `list_issues_by_label` result (or error) is fixed per
/// construction, recording how many times it was called. `repo_admins` is the
/// programmable hook the later R3 work-issue-authority tests inject an admin set
/// through — F2 wires no consumer, so it defaults empty.
struct FakeListing {
    issues: Result<Vec<IssueSummary>, GithubAppError>,
    list_calls: AtomicUsize,
    repo_admins: Vec<GithubActor>,
}

impl FakeListing {
    fn ok(issues: Vec<IssueSummary>) -> Self {
        Self {
            issues: Ok(issues),
            list_calls: AtomicUsize::new(0),
            repo_admins: Vec::new(),
        }
    }
    fn err() -> Self {
        Self {
            issues: Err(GithubAppError::RateLimited(30)),
            list_calls: AtomicUsize::new(0),
            repo_admins: Vec::new(),
        }
    }
    fn list_calls(&self) -> usize {
        self.list_calls.load(Ordering::SeqCst)
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
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        self.issues.clone()
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
        Ok(self.repo_admins.clone())
    }
}

// ---- fixtures ---------------------------------------------------------------

fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

fn issue(number: i64, labels: &[&str]) -> IssueSummary {
    IssueSummary {
        number,
        title: "work item".to_string(),
        body: String::new(),
        labels: labels.iter().map(|s| s.to_string()).collect(),
        state: "open".to_string(),
        assignees: Vec::new(),
        user_login: "alice".to_string(),
        user_id: 7,
    }
}

fn registration(name: &str, work_label: &str) -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: repo(),
        trigger_issue: 1,
        trigger_author_id: 7,
        trigger_author_login: "author-login".to_string(),
        def: SessionDef {
            name: name.to_string(),
            packages: Vec::new(),
            work_label: Some(work_label.to_string()),
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
        },
        session_id: "sess-1".to_string(),
        config_hash: "hash".to_string(),
        auto_merge: false,
        log_access: vec![],
    }
}

fn token() -> SecretString {
    SecretString::from("ghs_x".to_string())
}

// ---- renderer ---------------------------------------------------------------

#[test]
fn renders_session_name_work_label_and_outcome() {
    let body = work_ack_comment("mysession", "fkst-run");
    // Headline names the session verbatim in backticks.
    assert!(body.contains("Picked up by fkst session `mysession`."));
    // Names the work label the pod is working, in backticks.
    assert!(body.contains("`fkst-run` issues"));
    // Sets expectations: progress on this issue + a PR (or linked issues) outcome.
    assert!(body.contains("posts its progress on this issue"));
    assert!(body.contains("pull request"));
    assert!(body.contains("linked issues"));
}

// ---- step -------------------------------------------------------------------

#[tokio::test]
async fn acks_an_unacked_open_work_issue() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // One open work issue that has NOT been acked yet (only the work label).
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
    )
    .await;

    // Exactly one comment, carrying the rendered ack for the right issue.
    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one comment");
    assert_eq!(comments[0].2, 5);
    assert!(comments[0].3.contains("Picked up by fkst session `demo`."));

    // Exactly one label add: the durable picked-up latch on that issue.
    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 5);
    assert_eq!(added[0].3, vec![WORK_PICKED_UP_LABEL.to_string()]);
}

#[tokio::test]
async fn skips_an_already_acked_issue() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // The issue already carries the picked-up latch → must be skipped.
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run", WORK_PICKED_UP_LABEL])]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
    )
    .await;

    assert!(
        api.comments.lock().unwrap().is_empty(),
        "an already-acked issue is not re-commented"
    );
    assert!(
        api.labels_added.lock().unwrap().is_empty(),
        "an already-acked issue is not re-latched"
    );
}

#[tokio::test]
async fn no_op_without_registrations() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);

    ack_open_work_issues(&github, &listing, &token(), &repo(), &[]).await;

    assert_eq!(
        listing.list_calls(),
        0,
        "no registrations means the listing is never even queried"
    );
    assert!(api.comments.lock().unwrap().is_empty());
    assert!(api.labels_added.lock().unwrap().is_empty());
}

#[tokio::test]
async fn swallows_a_listing_failure() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::err();

    // Must not panic/propagate — the failure is logged and skipped.
    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
    )
    .await;

    assert_eq!(listing.list_calls(), 1, "the list was attempted once");
    assert!(
        api.comments.lock().unwrap().is_empty(),
        "a failed list posts nothing"
    );
    assert!(api.labels_added.lock().unwrap().is_empty());
}

#[tokio::test]
async fn swallows_a_comment_failure_but_still_latches() {
    // Mirrors the announce arm: a best-effort comment failure is swallowed, yet the
    // durable latch is still added so the issue is not endlessly re-processed.
    let api = std::sync::Arc::new(RecordingApi::with_comment_failure());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run"])]);

    ack_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        &[registration("demo", "fkst-run")],
    )
    .await;

    assert!(
        api.comments.lock().unwrap().is_empty(),
        "the comment failed (recorded nothing)"
    );
    let added = api.labels_added.lock().unwrap();
    assert_eq!(
        added.len(),
        1,
        "the latch is added despite the comment failure"
    );
    assert_eq!(added[0].3, vec![WORK_PICKED_UP_LABEL.to_string()]);
}
