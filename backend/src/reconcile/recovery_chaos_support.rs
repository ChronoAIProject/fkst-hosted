//! Durable fakes for the composed recovery-chaos tests.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use base64::Engine;
use secrecy::SecretString;

use super::full_resync_once;
use crate::config::Config;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest, RemoteFile,
};
use crate::github_app::listing::{GithubListing, InstallationSummary, IssueSummary};
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::log_access::LogAccessRegistry;
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::desired::KillReason;
use crate::reconcile::{
    new_active_repos, new_ensured_templates, reconcile_channel, reconcile_repo,
};

#[path = "recovery_chaos_runtime.rs"]
mod runtime;

use runtime::{
    test_app_config, BackendEvents, ChaosBackend, FixtureEnvironmentStore, RuntimeRecord,
};
pub(super) use runtime::{BackendProfile, EnsureEvent};

pub(super) const INSTALLATION_ID: i64 = 42;
pub(super) const AUTHOR_ID: i64 = 101;
pub(super) const TRIGGER: i64 = 10;
pub(super) const WORK: i64 = 20;
pub(super) const WORK_LABEL: &str = "fkst-demo";
pub(super) const ENVIRONMENT: &str = "recovery";

pub(super) fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

pub(super) fn trigger_body(name: &str, work_label: &str) -> String {
    format!(
        "### Session Name\n{name}\n\n\
         ### Packages\nacme/tools@main:pkg/demo\n\n\
         ### Work Label\n{work_label}\n\n\
         ### Environment\n{ENVIRONMENT}\n"
    )
}

pub(super) fn issue(
    number: i64,
    body: impl Into<String>,
    labels: &[&str],
    login: &str,
    user_id: i64,
) -> IssueSummary {
    IssueSummary {
        number,
        title: format!("fixture #{number}"),
        body: body.into(),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
        state: "open".to_string(),
        assignees: Vec::new(),
        user_login: login.to_string(),
        user_id,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct GithubEffects {
    pub comments: usize,
    pub label_adds: usize,
    pub label_removes: usize,
}

#[derive(Default)]
struct LedgerState {
    issues: BTreeMap<i64, IssueSummary>,
    comments: BTreeMap<i64, Vec<String>>,
    effects: GithubEffects,
    missing_branches: HashSet<String>,
    branch_transport_error: bool,
}

/// GitHub state is intentionally independent of a controller context. Labels and
/// comments therefore remain the authoritative restart latches.
pub(super) struct GithubLedger {
    state: Mutex<LedgerState>,
}

impl GithubLedger {
    fn new() -> Self {
        Self {
            state: Mutex::new(LedgerState::default()),
        }
    }

    pub fn put(&self, issue: IssueSummary) {
        self.state
            .lock()
            .unwrap()
            .issues
            .insert(issue.number, issue);
    }

    pub fn set_state(&self, number: i64, state: &str) {
        self.state
            .lock()
            .unwrap()
            .issues
            .get_mut(&number)
            .unwrap()
            .state = state.to_string();
    }

    pub fn set_body(&self, number: i64, body: String) {
        self.state
            .lock()
            .unwrap()
            .issues
            .get_mut(&number)
            .unwrap()
            .body = body;
    }

    pub fn set_assignees(&self, number: i64, assignees: &[&str]) {
        self.state
            .lock()
            .unwrap()
            .issues
            .get_mut(&number)
            .unwrap()
            .assignees = assignees.iter().map(|login| login.to_string()).collect();
    }

    pub fn labels(&self, number: i64) -> Vec<String> {
        self.state.lock().unwrap().issues[&number].labels.clone()
    }

    pub fn comments(&self, number: i64) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .comments
            .get(&number)
            .cloned()
            .unwrap_or_default()
    }

    pub fn effects(&self) -> GithubEffects {
        self.state.lock().unwrap().effects
    }

    pub fn set_branch_exists(&self, branch: &str, exists: bool) {
        let mut state = self.state.lock().unwrap();
        if exists {
            state.missing_branches.remove(branch);
        } else {
            state.missing_branches.insert(branch.to_string());
        }
    }

    pub fn set_branch_transport_error(&self, enabled: bool) {
        self.state.lock().unwrap().branch_transport_error = enabled;
    }

    fn matching_open_issues(&self, label: &str) -> Vec<IssueSummary> {
        self.state
            .lock()
            .unwrap()
            .issues
            .values()
            .filter(|issue| issue.state == "open" && issue.labels.iter().any(|l| l == label))
            .cloned()
            .collect()
    }

    fn matching_open_issues_assignee(&self, label: &str, assignee: &str) -> Vec<IssueSummary> {
        self.matching_open_issues(label)
            .into_iter()
            .filter(|issue| {
                issue
                    .assignees
                    .iter()
                    .any(|login| login.eq_ignore_ascii_case(assignee))
            })
            .collect()
    }
}

#[async_trait]
impl GithubListing for GithubLedger {
    async fn list_issues_by_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        label: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError> {
        Ok(self.matching_open_issues(label))
    }

    async fn count_open_issues_with_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        label: &str,
    ) -> Result<u64, GithubAppError> {
        Ok(self.matching_open_issues(label).len() as u64)
    }

    async fn list_issues_by_label_assignee(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        label: &str,
        assignee: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError> {
        Ok(self.matching_open_issues_assignee(label, assignee))
    }

    async fn count_open_issues_with_label_assignee(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        label: &str,
        assignee: &str,
    ) -> Result<u64, GithubAppError> {
        Ok(self.matching_open_issues_assignee(label, assignee).len() as u64)
    }

    async fn get_collaborator_role(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _username: &str,
    ) -> Result<Option<String>, GithubAppError> {
        // Harness trigger authors model established repo maintainers. Tests that
        // exercise rejection by the deployment allowlist are skipped before this
        // role gate, preserving that intentionally silent behavior.
        Ok(Some("maintain".to_string()))
    }

    async fn list_installations(
        &self,
        _app_jwt: &SecretString,
    ) -> Result<Vec<InstallationSummary>, GithubAppError> {
        Ok(vec![InstallationSummary {
            id: INSTALLATION_ID,
            account_login: "acme".to_string(),
        }])
    }

    async fn list_installation_repos(
        &self,
        _token: &SecretString,
    ) -> Result<Vec<RepoRef>, GithubAppError> {
        Ok(vec![repo()])
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

#[async_trait]
impl GithubApi for GithubLedger {
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        Ok(InstallationId(INSTALLATION_ID as u64))
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        _id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        Ok(InstallationToken {
            token: SecretString::from("ghs_chaos_fixture".to_string()),
            expires_at: SystemTime::now() + Duration::from_secs(3_600),
        })
    }

    async fn create_issue_comment(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        number: u64,
        body: &str,
    ) -> Result<(), GithubAppError> {
        let mut state = self.state.lock().unwrap();
        state
            .comments
            .entry(number as i64)
            .or_default()
            .push(body.to_string());
        state.effects.comments += 1;
        Ok(())
    }

    async fn add_issue_labels(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        number: u64,
        labels: &[String],
    ) -> Result<(), GithubAppError> {
        let mut state = self.state.lock().unwrap();
        let issue = state.issues.get_mut(&(number as i64)).unwrap();
        for label in labels {
            if !issue.labels.contains(label) {
                issue.labels.push(label.clone());
            }
        }
        state.effects.label_adds += 1;
        Ok(())
    }

    async fn remove_issue_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        number: u64,
        label: &str,
    ) -> Result<(), GithubAppError> {
        let mut state = self.state.lock().unwrap();
        state
            .issues
            .get_mut(&(number as i64))
            .unwrap()
            .labels
            .retain(|existing| existing != label);
        state.effects.label_removes += 1;
        Ok(())
    }

    async fn list_issue_comments(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        number: u64,
    ) -> Result<Vec<String>, GithubAppError> {
        Ok(self.comments(number as i64))
    }

    async fn content_file(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _path: &str,
        _git_ref: Option<&str>,
    ) -> Result<Option<RemoteFile>, GithubAppError> {
        let content = base64::engine::general_purpose::STANDARD.encode(format!(
            "# fkst-issue-templates-version: {}\n",
            crate::github_app::FKST_ISSUE_TEMPLATES_VERSION
        ));
        Ok(Some(RemoteFile {
            sha: "template-fixture".to_string(),
            content_base64: content,
        }))
    }

    async fn repo_default_branch(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<String, GithubAppError> {
        Ok("main".to_string())
    }

    async fn branch_head_sha(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        branch: &str,
    ) -> Result<Option<String>, GithubAppError> {
        let state = self.state.lock().unwrap();
        if state.branch_transport_error {
            return Err(GithubAppError::Http(
                "fixture branch lookup transport failure".to_string(),
            ));
        }
        if state.missing_branches.contains(branch) {
            Ok(None)
        } else {
            Ok(Some(format!("head-sha-{branch}")))
        }
    }
}

pub(super) struct ChaosHarness {
    pub ledger: Arc<GithubLedger>,
    profile: BackendProfile,
    runtimes: Arc<Mutex<HashMap<String, RuntimeRecord>>>,
    events: Arc<Mutex<BackendEvents>>,
    config: Config,
    ctx: crate::reconcile::ReconcileCtx,
}

impl ChaosHarness {
    pub fn new(
        profile: BackendProfile,
        github_api_base: &str,
        allowed_users: Option<&str>,
    ) -> Self {
        let mut vars = vec![
            ("FKST_LLM_API_KEY", "fixture-llm-value"),
            ("FKST_STORAGE_BASE_URL", "https://storage.invalid/proxy"),
            ("FKST_STORAGE_BUCKET", "fixture-bucket"),
            ("FKST_NYXID_TOKEN_URL", "https://identity.invalid/token"),
            ("FKST_NYXID_CLIENT_ID", "fixture-client"),
            ("FKST_NYXID_CLIENT_SECRET", "fixture-client-value"),
        ];
        if let Some(allowed) = allowed_users {
            vars.push(("FKST_ACCESS_ALLOWED_USERS", allowed));
        }
        let mut config = Config::from_vars(
            vars.into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string())),
        )
        .expect("fixture config");
        config.github_api_base_url = github_api_base.to_string();
        let ledger = Arc::new(GithubLedger::new());
        let runtimes = Arc::new(Mutex::new(HashMap::new()));
        let events = Arc::new(Mutex::new(BackendEvents::default()));
        let ctx = Self::controller_ctx(
            profile,
            runtimes.clone(),
            events.clone(),
            ledger.clone(),
            config.clone(),
        );
        Self {
            ledger,
            profile,
            runtimes,
            events,
            config,
            ctx,
        }
    }

    fn controller_ctx(
        profile: BackendProfile,
        runtimes: Arc<Mutex<HashMap<String, RuntimeRecord>>>,
        events: Arc<Mutex<BackendEvents>>,
        ledger: Arc<GithubLedger>,
        config: Config,
    ) -> crate::reconcile::ReconcileCtx {
        let backend = Arc::new(ChaosBackend {
            profile,
            runtimes,
            events,
            credential_cache: Mutex::new(HashMap::new()),
        });
        let github = GithubAppTokens::with_api(&test_app_config(), ledger.clone()).expect("tokens");
        crate::reconcile::ReconcileCtx {
            backend,
            env_store: Arc::new(FixtureEnvironmentStore),
            github,
            listing: ledger,
            http: reqwest::Client::new(),
            config,
            active_repos: new_active_repos(),
            ensured_templates: new_ensured_templates(),
            log_registry: LogAccessRegistry::new(),
            disposable_environments: Default::default(),
        }
    }

    pub fn restart_controller(&mut self) {
        self.ctx = Self::controller_ctx(
            self.profile,
            self.runtimes.clone(),
            self.events.clone(),
            self.ledger.clone(),
            self.config.clone(),
        );
    }

    pub fn delete_runtime(&self, session_id: &str) {
        self.runtimes.lock().unwrap().remove(session_id);
    }

    pub fn runtime_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.runtimes.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn ensures(&self) -> Vec<EnsureEvent> {
        self.events.lock().unwrap().ensures.clone()
    }

    pub fn stops(&self) -> Vec<(String, KillReason)> {
        self.events.lock().unwrap().stops.clone()
    }

    pub async fn full_resync(&self) {
        let (handle, mut rx) = reconcile_channel(8);
        let summary = full_resync_once(&self.ctx, &handle)
            .await
            .expect("full resync");
        assert!(summary.is_complete());
        assert_eq!(summary.repositories_enqueued, 1);
        while let Ok((installation, repo)) = rx.try_recv() {
            reconcile_repo(installation, &repo, &self.ctx)
                .await
                .expect("repository reconcile");
        }
    }

    pub async fn reconcile_repo_result(&self) -> Result<(), crate::error::AppError> {
        reconcile_repo(INSTALLATION_ID, &repo(), &self.ctx).await
    }
}
