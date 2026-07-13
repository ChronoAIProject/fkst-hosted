//! Shared test fixtures for the reconciler tests (the executor issue-effect +
//! action-routing tests and the loop tests), reconcile-wide so each test file stays
//! under the 500-line limit and all of them can build a `ReconcileCtx`. The shared
//! session-backend fake lives in [`crate::session_backend::test_support`] and is
//! re-exported here.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use super::ReconcileCtx;
use crate::config::Config;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::config::GithubAppConfig;
use crate::github_app::listing::{GithubListing, InstallationSummary, IssueSummary};
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::goals::trigger_parse::PackageRef;
use crate::k8s::env_store::EnvStore;
use crate::log_access::LogAccessRegistry;
use crate::models::RepoRef;
use crate::reconcile::desired::{SessionDef, SessionRegistration};
use crate::session_backend::SessionBackend;

// ---- recording fake GitHub transport ---------------------------------------

/// A recorded issue call: `(owner, repo, issue_number, payload)`.
pub(super) type Call = (String, String, u64, String);
/// A recorded label-add call: `(owner, repo, issue_number, labels)`.
pub(super) type LabelCall = (String, String, u64, Vec<String>);

#[derive(Default)]
pub(super) struct RecordingApi {
    pub(super) comments: Mutex<Vec<Call>>,
    pub(super) labels_added: Mutex<Vec<LabelCall>>,
    pub(super) labels_removed: Mutex<Vec<Call>>,
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

pub(super) fn test_config() -> GithubAppConfig {
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

pub(super) fn tokens(api: Arc<RecordingApi>) -> GithubAppTokens {
    GithubAppTokens::with_api(&test_config(), api).expect("tokens")
}

// ---- ctx builder -----------------------------------------------------------

/// The shared recording [`SessionBackend`] fake, promoted to
/// [`crate::session_backend::test_support`] so the reconcile loop tests reuse it.
pub(crate) use crate::session_backend::test_support::FakeSessionBackend;

/// A trivial [`GithubListing`] the routing tests never actually call (only the pod
/// effects run through the faked backend); present so a `ReconcileCtx` is buildable.
#[derive(Default)]
pub(super) struct FakeListing;

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

/// Build a [`ReconcileCtx`] wired to `backend`; every other field is a trivial fake
/// the pod-effect routing tests do not exercise.
pub(crate) fn test_ctx(backend: Arc<dyn SessionBackend>) -> ReconcileCtx {
    ReconcileCtx {
        backend,
        env_store: EnvStore::fake(),
        github: tokens(Arc::new(RecordingApi::default())),
        listing: Arc::new(FakeListing),
        http: reqwest::Client::new(),
        config: Config::default(),
        active_repos: crate::reconcile::new_active_repos(),
        ensured_templates: crate::reconcile::new_ensured_templates(),
        log_registry: LogAccessRegistry::new(),
    }
}

pub(super) fn test_repo() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

/// A representative valid registration used by the pure spec-assembly test and the
/// spawn-routing test.
pub(super) fn registration() -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        trigger_issue: 7,
        trigger_author_id: 583231,
        def: SessionDef {
            name: "site".to_string(),
            packages: vec![
                PackageRef {
                    owner: "ChronoAIProject".to_string(),
                    repo: "fkst-packages".to_string(),
                    git_ref: "dev".to_string(),
                    path: "packages/github-devloop".to_string(),
                },
                PackageRef {
                    owner: "acme".to_string(),
                    repo: "pkgs".to_string(),
                    git_ref: "main".to_string(),
                    path: "packages/proxy".to_string(),
                },
            ],
            work_label: "fkst-run".to_string(),
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
        },
        session_id: "sess-abc".to_string(),
        config_hash: "hash123".to_string(),
        auto_merge: false,
        log_access: vec![],
    }
}
