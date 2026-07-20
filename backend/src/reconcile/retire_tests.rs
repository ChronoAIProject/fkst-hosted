//! Unit tests for the retire-notify executor step. The renderer is pure; the step
//! runs against a recording fake [`GithubApi`] (so no network is touched) plus a fake
//! [`GithubListing`] whose returned issues (or error) are fixed per construction.
//! Covers: retiring an un-retired open work issue (comment + retired label + drop the
//! stale picked-up label), skipping an already-retired one, the empty-list no-op, and
//! swallowing a list/comment failure. Mirrors `work_ack_tests`.

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
use crate::models::GithubActor;
// `GithubAppError`, `GithubAppTokens`, `GithubListing`, `RepoRef`, and the label
// consts are re-exported into scope by the `use super::*;` glob above (they are the
// parent `retire` module's own imports), so they are not re-imported here.

// ---- recording fake GitHub transport (mirrors work_ack_tests) ----------------

/// A recorded issue call: `(owner, repo, issue_number, payload)`.
type Call = (String, String, u64, String);
/// A recorded label-add call: `(owner, repo, issue_number, labels)`.
type LabelCall = (String, String, u64, Vec<String>);
/// A recorded label-remove call: `(owner, repo, issue_number, label)`.
type RemoveCall = (String, String, u64, String);

#[derive(Default)]
struct RecordingApi {
    comments: Mutex<Vec<Call>>,
    labels_added: Mutex<Vec<LabelCall>>,
    labels_removed: Mutex<Vec<RemoveCall>>,
    /// When set, `create_issue_comment` fails — exercising the best-effort comment
    /// arm (the latch + un-latch must still happen, mirroring the announce/ack arms).
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
        owner: &str,
        repo: &str,
        number: u64,
        label: &str,
    ) -> Result<(), GithubAppError> {
        self.labels_removed.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number,
            label.to_string(),
        ));
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

// ---- fake listing (mirrors work_ack_tests) ----------------------------------

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

fn token() -> SecretString {
    SecretString::from("ghs_x".to_string())
}

// ---- renderer ---------------------------------------------------------------

#[test]
fn renders_work_label_and_retired_notice() {
    let body = retire_notice_comment("fkst-run");
    // Names the work label verbatim in backticks (twice: the notice + the resume hint).
    assert!(body.contains("work label `fkst-run`"));
    assert!(body.contains("with work label `fkst-run`"));
    // The headline + the "left OPEN, no longer worked" wording (the string-literal
    // line continuations collapse to single spaces, so this is one flat sentence).
    assert!(body.contains("Session retired."));
    assert!(body.contains("left OPEN"));
    assert!(body.contains("no longer being worked"));
    // The resume hint names the trigger label.
    assert!(body.contains("fkst-substrate-trigger"));
}

// ---- step -------------------------------------------------------------------

#[tokio::test]
async fn retires_an_unretired_open_work_issue() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // One open work issue still carrying the (now-stale) picked-up latch, not retired.
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run", WORK_PICKED_UP_LABEL])]);

    retire_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        "fkst-run",
        &mut std::collections::HashSet::new(),
    )
    .await;

    // Exactly one comment, the rendered retire notice for the right issue.
    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one comment");
    assert_eq!(comments[0].2, 5);
    assert!(comments[0].3.contains("Session retired."));

    // The durable retired latch is added to that issue.
    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 5);
    assert_eq!(added[0].3, vec![SUBSTRATE_RETIRED_LABEL.to_string()]);

    // The now-stale picked-up label is removed from that issue.
    let removed = api.labels_removed.lock().unwrap();
    assert_eq!(removed.len(), 1, "exactly one label remove");
    assert_eq!(removed[0].2, 5);
    assert_eq!(removed[0].3, WORK_PICKED_UP_LABEL.to_string());
}

#[tokio::test]
async fn retire_work_issues_lists_every_label_and_dedups_a_shared_issue() {
    // A multi-label session (epic #594 I4): the executor entry point retires across EACH
    // of the session's labels. The fake listing returns the SAME issue for both labels
    // (an issue carrying two of the session's labels), so the in-pass dedup must retire
    // it EXACTLY once — while both labels are still queried.
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run", WORK_PICKED_UP_LABEL])]);

    retire_work_issues(
        &github,
        &listing,
        &repo(),
        &["alpha".to_string(), "beta".to_string()],
    )
    .await;

    // Both labels were listed (the retire spans the whole set)...
    assert_eq!(listing.list_calls(), 2, "each label's queue is listed");
    // ...but the shared issue is notified/latched/un-latched EXACTLY once (deduped).
    assert_eq!(
        api.comments.lock().unwrap().len(),
        1,
        "a shared issue is retired once across labels"
    );
    assert_eq!(api.labels_added.lock().unwrap().len(), 1);
    assert_eq!(api.labels_removed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn skips_an_already_retired_issue() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    // The issue already carries the retired latch → must be skipped entirely.
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run", SUBSTRATE_RETIRED_LABEL])]);

    retire_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        "fkst-run",
        &mut std::collections::HashSet::new(),
    )
    .await;

    assert!(
        api.comments.lock().unwrap().is_empty(),
        "an already-retired issue is not re-commented"
    );
    assert!(
        api.labels_added.lock().unwrap().is_empty(),
        "an already-retired issue is not re-latched"
    );
    assert!(
        api.labels_removed.lock().unwrap().is_empty(),
        "an already-retired issue is not touched again"
    );
}

#[tokio::test]
async fn no_op_when_the_list_is_empty() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![]);

    retire_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        "fkst-run",
        &mut std::collections::HashSet::new(),
    )
    .await;

    assert_eq!(listing.list_calls(), 1, "the list was queried once");
    assert!(api.comments.lock().unwrap().is_empty());
    assert!(api.labels_added.lock().unwrap().is_empty());
    assert!(api.labels_removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn swallows_a_listing_failure() {
    let api = std::sync::Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let listing = FakeListing::err();

    // Must not panic/propagate — the failure is logged and skipped.
    retire_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        "fkst-run",
        &mut std::collections::HashSet::new(),
    )
    .await;

    assert_eq!(listing.list_calls(), 1, "the list was attempted once");
    assert!(
        api.comments.lock().unwrap().is_empty(),
        "a failed list posts nothing"
    );
    assert!(api.labels_added.lock().unwrap().is_empty());
    assert!(api.labels_removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn swallows_a_comment_failure_but_still_latches_and_unlatches() {
    // Mirrors the announce/ack arms: a best-effort comment failure is swallowed, yet
    // the durable retired latch is still added AND the stale picked-up label removed,
    // so the issue is correctly retired and never endlessly re-processed.
    let api = std::sync::Arc::new(RecordingApi::with_comment_failure());
    let github = tokens(api.clone());
    let listing = FakeListing::ok(vec![issue(5, &["fkst-run", WORK_PICKED_UP_LABEL])]);

    retire_open_work_issues(
        &github,
        &listing,
        &token(),
        &repo(),
        "fkst-run",
        &mut std::collections::HashSet::new(),
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
        "the retired latch is added despite the comment failure"
    );
    assert_eq!(added[0].3, vec![SUBSTRATE_RETIRED_LABEL.to_string()]);
    let removed = api.labels_removed.lock().unwrap();
    assert_eq!(
        removed.len(),
        1,
        "the picked-up label is removed despite the comment failure"
    );
    assert_eq!(removed[0].3, WORK_PICKED_UP_LABEL.to_string());
}
