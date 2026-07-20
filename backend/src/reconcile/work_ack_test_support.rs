//! Shared test harness for the work-issue ack/reject steps, split out so the ack
//! tests ([`super::tests`]) and the R3 authority reject tests
//! ([`super::authz_tests`]) each stay under the 500-line limit. `pub(super)` so both
//! sibling test modules can reuse the recording transport, the fake listing, and the
//! fixtures.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::config::GithubAppConfig;
use crate::github_app::listing::{GithubListing, InstallationSummary, IssueSummary};
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::desired::{SessionDef, SessionRegistration};

// ---- recording fake GitHub transport (mirrors execute_tests) ----------------

/// A recorded issue call: `(owner, repo, issue_number, payload)`.
pub(super) type Call = (String, String, u64, String);
/// A recorded label-add call: `(owner, repo, issue_number, labels)`.
pub(super) type LabelCall = (String, String, u64, Vec<String>);
/// A recorded label-remove call: `(owner, repo, issue_number, label)`.
pub(super) type LabelRemoveCall = (String, String, u64, String);

#[derive(Default)]
pub(super) struct RecordingApi {
    pub(super) comments: Mutex<Vec<Call>>,
    pub(super) labels_added: Mutex<Vec<LabelCall>>,
    pub(super) labels_removed: Mutex<Vec<LabelRemoveCall>>,
    /// When set, `create_issue_comment` fails — exercising the best-effort comment
    /// arm (the latch label must still be added, mirroring the announce arm).
    fail_comment: bool,
    /// When set, `add_issue_labels` fails — exercising the reject latch-first arm
    /// (the reject comment must be SKIPPED so it is never double-posted).
    fail_label: bool,
}

impl RecordingApi {
    pub(super) fn with_comment_failure() -> Self {
        Self {
            fail_comment: true,
            ..Self::default()
        }
    }

    pub(super) fn with_label_failure() -> Self {
        Self {
            fail_label: true,
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
        if self.fail_label {
            return Err(GithubAppError::Http("boom".to_string()));
        }
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

pub(super) fn tokens(api: std::sync::Arc<RecordingApi>) -> GithubAppTokens {
    GithubAppTokens::with_api(&test_config(), api).expect("tokens")
}

// ---- fake listing -----------------------------------------------------------

/// A fake listing whose `list_issues_by_label` result (or error) is fixed per
/// construction, recording how many times it was called. `repo_admins` is the
/// programmable hook (F2) — work_ack itself never calls `list_repo_admins` (the
/// admin set is passed to `ack_open_work_issues` directly), so it defaults empty.
pub(super) struct FakeListing {
    issues: Result<Vec<IssueSummary>, GithubAppError>,
    list_calls: AtomicUsize,
    repo_admins: Vec<GithubActor>,
}

impl FakeListing {
    pub(super) fn ok(issues: Vec<IssueSummary>) -> Self {
        Self {
            issues: Ok(issues),
            list_calls: AtomicUsize::new(0),
            repo_admins: Vec::new(),
        }
    }
    pub(super) fn err() -> Self {
        Self {
            issues: Err(GithubAppError::RateLimited(30)),
            list_calls: AtomicUsize::new(0),
            repo_admins: Vec::new(),
        }
    }
    pub(super) fn list_calls(&self) -> usize {
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

pub(super) fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

/// A work issue with the default author (id 7) == the [`registration`] fixture's
/// trigger author, so it is AUTHORIZED under enforcement unless a test overrides the
/// author via [`issue_by`].
pub(super) fn issue(number: i64, labels: &[&str]) -> IssueSummary {
    issue_by(number, labels, 7, "alice")
}

/// A work issue authored by `(user_id, user_login)` — used by the R3 reject tests to
/// stand up an issue raised by someone other than the session's trigger author.
pub(super) fn issue_by(
    number: i64,
    labels: &[&str],
    user_id: i64,
    user_login: &str,
) -> IssueSummary {
    IssueSummary {
        number,
        title: "work item".to_string(),
        body: String::new(),
        labels: labels.iter().map(|s| s.to_string()).collect(),
        state: "open".to_string(),
        assignees: Vec::new(),
        user_login: user_login.to_string(),
        user_id,
    }
}

pub(super) fn admin(id: i64, login: &str) -> GithubActor {
    GithubActor {
        id,
        login: login.to_string(),
    }
}

pub(super) fn registration(name: &str, work_label: &str) -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: repo(),
        trigger_issue: 1,
        trigger_author_id: 7,
        trigger_author_login: "author-login".to_string(),
        def: SessionDef {
            name: name.to_string(),
            packages: Vec::new(),
            manifest_refs: Vec::new(),
            work_label: Some(work_label.to_string()),
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
        },
        effective_packages: Vec::new(),
        session_id: "sess-1".to_string(),
        config_hash: "hash".to_string(),
        auto_merge: false,
        log_access: vec![],
        collaborators: vec![],
    }
}

pub(super) fn token() -> SecretString {
    SecretString::from("ghs_x".to_string())
}

/// The `session_id -> full work-label set` map the driver threads into
/// `ack_open_work_issues`. Keyed by the [`registration`] fixture's session id
/// (`sess-1`); `labels` is that session's full set for these single-session tests.
pub(super) fn label_map(labels: &[&str]) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    map.insert(
        "sess-1".to_string(),
        labels.iter().map(|s| s.to_string()).collect(),
    );
    map
}
