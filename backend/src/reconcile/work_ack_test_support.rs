use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use crate::access_policy::AccessPolicy;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::config::GithubAppConfig;
use crate::github_app::listing::{GithubListing, InstallationSummary, IssueSummary};
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::desired::{SessionDef, SessionRegistration};

pub(super) type Call = (String, String, i64, String);
pub(super) type LabelCall = (String, String, i64, Vec<String>);

#[derive(Default)]
pub(super) struct RecordingApi {
    pub(super) comments: Mutex<Vec<Call>>,
    pub(super) labels_added: Mutex<Vec<LabelCall>>,
    pub(super) labels_removed: Mutex<Vec<Call>>,
    pub(super) events: Mutex<Vec<&'static str>>,
    fail_comment: bool,
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
        self.events.lock().unwrap().push("comment");
        self.comments.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number as i64,
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
        self.events.lock().unwrap().push("label");
        self.labels_added.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number as i64,
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
        self.events.lock().unwrap().push("remove");
        self.labels_removed.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number as i64,
            label.to_string(),
        ));
        Ok(())
    }
}

pub(super) fn tokens(api: Arc<RecordingApi>) -> GithubAppTokens {
    use rand::rngs::OsRng;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;

    let private = RsaPrivateKey::new(&mut OsRng, 2048).expect("key");
    let config = GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(
            private
                .to_pkcs8_pem(LineEnding::LF)
                .expect("pem")
                .to_string(),
        ),
        app_slug: Some("fkst-test".to_string()),
        webhook_secret: None,
        api_base: "https://api.github.com".to_string(),
    };
    GithubAppTokens::with_api(&config, api).expect("tokens")
}

pub(super) struct FakeListing {
    issues: Result<Vec<IssueSummary>, GithubAppError>,
    list_calls: AtomicUsize,
}

impl FakeListing {
    pub(super) fn ok(issues: Vec<IssueSummary>) -> Self {
        Self {
            issues: Ok(issues),
            list_calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn err() -> Self {
        Self {
            issues: Err(GithubAppError::RateLimited(30)),
            list_calls: AtomicUsize::new(0),
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
        Ok(Vec::new())
    }
}

pub(super) fn repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

pub(super) fn issue(number: i64, labels: &[&str]) -> IssueSummary {
    issue_by(number, labels, 7, "alice", &["alice"])
}

pub(super) fn issue_by(
    number: i64,
    labels: &[&str],
    user_id: i64,
    user_login: &str,
    assignees: &[&str],
) -> IssueSummary {
    IssueSummary {
        number,
        title: "work item".to_string(),
        body: "content is intentionally ignored".to_string(),
        labels: labels.iter().map(|value| value.to_string()).collect(),
        state: "open".to_string(),
        assignees: assignees.iter().map(|value| value.to_string()).collect(),
        user_login: user_login.to_string(),
        user_id,
    }
}

pub(super) fn registration(name: &str, work_label: &str) -> SessionRegistration {
    registration_for(name, work_label, "alice", Some(7), "sess-1")
}

pub(super) fn registration_for(
    name: &str,
    work_label: &str,
    creator_login: &str,
    creator_id: Option<i64>,
    session_id: &str,
) -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: repo(),
        trigger_issue: 1,
        trigger_author_id: creator_id.unwrap_or(9000),
        trigger_author_login: creator_login.to_string(),
        creator_login: creator_login.to_string(),
        creator_id,
        def: SessionDef {
            name: name.to_string(),
            packages: Vec::new(),
            manifest_refs: Vec::new(),
            work_label: Some(work_label.to_string()),
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
            source_branch: None,
            target_branch: None,
        },
        effective_packages: Vec::new(),
        session_id: session_id.to_string(),
        config_hash: "hash".to_string(),
        auto_merge: false,
        log_access: vec![],
        collaborators: vec![],
    }
}

pub(super) fn token() -> SecretString {
    SecretString::from("ghs_x".to_string())
}

pub(super) fn label_map(entries: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(session, labels)| {
            (
                (*session).to_string(),
                labels.iter().map(|value| (*value).to_string()).collect(),
            )
        })
        .collect()
}

pub(super) fn one_label_map(labels: &[&str]) -> HashMap<String, Vec<String>> {
    label_map(&[("sess-1", labels)])
}

pub(super) fn access(global_admins: &str) -> AccessPolicy {
    AccessPolicy::from_vars(&[("FKST_GLOBAL_ADMINS".to_string(), global_admins.to_string())])
        .expect("access")
}
