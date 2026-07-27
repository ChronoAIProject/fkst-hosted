//! Shared test fixtures for the reconciler tests (the executor issue-effect +
//! action-routing tests and the loop tests), reconcile-wide so each test file stays
//! under the 500-line limit and all of them can build a `ReconcileCtx`. The shared
//! session-backend fake lives in [`crate::session_backend::test_support`] and is
//! re-exported here.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use super::ReconcileCtx;
use crate::config::Config;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest, TokenPermissions,
};
use crate::github_app::config::GithubAppConfig;
use crate::github_app::listing::{GithubListing, InstallationSummary, IssueSummary};
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::goals::trigger_parse::PackageRef;
use crate::k8s::env_store::EnvStore;
use crate::log_access::LogAccessRegistry;
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::desired::{SessionDef, SessionRegistration};
use crate::session_backend::SessionBackend;

// ---- recording fake GitHub transport ---------------------------------------

/// A recorded issue call: `(owner, repo, issue_number, payload)`.
pub(super) type Call = (String, String, u64, String);
/// A recorded label-add call: `(owner, repo, issue_number, labels)`.
pub(super) type LabelCall = (String, String, u64, Vec<String>);

/// The lifetime a minted installation token carries unless a test scripts a shorter
/// one — GitHub's real one-hour installation-token TTL.
pub(super) const FULL_TOKEN_TTL: Duration = Duration::from_secs(3600);

#[derive(Default)]
pub(super) struct RecordingApi {
    pub(super) comments: Mutex<Vec<Call>>,
    pub(super) labels_added: Mutex<Vec<LabelCall>>,
    pub(super) labels_removed: Mutex<Vec<Call>>,
    pub(super) events: Mutex<Vec<&'static str>>,
    /// `None` means every branch exists (the compatibility default). `Some(map)`
    /// makes branch lookup explicit so provisioning tests can model absence.
    pub(super) branch_heads: Mutex<Option<HashMap<String, String>>>,
    pub(super) create_refs: Mutex<Vec<(String, String)>>,
    pub(super) create_ref_error: Mutex<Option<GithubAppError>>,
    /// Remaining lifetimes handed to successive token mints; an empty/exhausted queue
    /// falls back to [`FULL_TOKEN_TTL`]. Lets a test reproduce the near-expiry token
    /// that #3410 leaked into the shared cache.
    mint_lifetimes: Mutex<VecDeque<Duration>>,
    /// The permissions each mint requested, in call order. Only an
    /// installation-wide mint records `None`; every repo-scoped mint records
    /// `Some(..)`, with a `perms: None` caller recording `default_permissions()`
    /// (the service substitutes it before building the request).
    mint_perms: Mutex<Vec<Option<TokenPermissions>>>,
    mint_count: AtomicUsize,
}

impl RecordingApi {
    /// Script the remaining lifetime of the next mints (the rest are
    /// [`FULL_TOKEN_TTL`]).
    pub(super) fn with_mint_lifetimes(self, lifetimes: impl IntoIterator<Item = Duration>) -> Self {
        *self.mint_lifetimes.lock().unwrap() = lifetimes.into_iter().collect();
        self
    }

    /// How many mints requested exactly `perms`.
    ///
    /// NOT a session-vs-reconciler discriminator: `session_permissions()` is
    /// structurally equal to `default_permissions()`, so this also counts the
    /// reconciler's own repo-scoped reads. It answers only "how many mints carried
    /// these permissions" — which is what a caller wants when it has already primed
    /// the cache, making every other call on the path a hit.
    pub(super) fn mints_with_perms(&self, perms: &TokenPermissions) -> usize {
        self.mint_perms
            .lock()
            .unwrap()
            .iter()
            .filter(|requested| requested.as_ref() == Some(perms))
            .count()
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
        req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        let nth = self.mint_count.fetch_add(1, Ordering::SeqCst) + 1;
        self.mint_perms
            .lock()
            .unwrap()
            .push(req.permissions.clone());
        let lifetime = self
            .mint_lifetimes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(FULL_TOKEN_TTL);
        // The mint ordinal makes successive tokens distinguishable, so a test can prove
        // WHICH mint's token was delivered rather than only that one happened.
        Ok(InstallationToken {
            token: SecretString::from(format!("ghs_fake_{nth}")),
            expires_at: SystemTime::now() + lifetime,
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
        self.events.lock().unwrap().push("comment");
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
        self.events.lock().unwrap().push("label-add");
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
        self.events.lock().unwrap().push("label-remove");
        self.labels_removed.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number,
            label.to_string(),
        ));
        Ok(())
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
        match &*self.branch_heads.lock().unwrap() {
            Some(heads) => Ok(heads.get(branch).cloned()),
            None => Ok(Some("head-sha".to_string())),
        }
    }

    async fn create_ref(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<(), GithubAppError> {
        self.create_refs
            .lock()
            .unwrap()
            .push((branch.to_string(), sha.to_string()));
        match self.create_ref_error.lock().unwrap().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
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

    async fn list_repo_admins(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<Vec<GithubActor>, GithubAppError> {
        Ok(Vec::new())
    }
}

/// Build a [`ReconcileCtx`] wired to `backend`; every other field is a trivial fake
/// the pod-effect routing tests do not exercise.
pub(crate) fn test_ctx(backend: Arc<dyn SessionBackend>) -> ReconcileCtx {
    test_ctx_with_github(backend, tokens(Arc::new(RecordingApi::default())))
}

/// [`test_ctx`] with the GitHub token service supplied, so a test can pre-seed its
/// cache or observe its mints.
pub(crate) fn test_ctx_with_github(
    backend: Arc<dyn SessionBackend>,
    github: GithubAppTokens,
) -> ReconcileCtx {
    ReconcileCtx {
        backend,
        env_store: Arc::new(EnvStore::fake()),
        github,
        listing: Arc::new(FakeListing),
        http: reqwest::Client::new(),
        config: Config::default(),
        active_repos: crate::reconcile::new_active_repos(),
        ensured_templates: crate::reconcile::new_ensured_templates(),
        log_registry: LogAccessRegistry::new(),
        disposable_environments: Default::default(),
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
        trigger_author_login: "author-login".to_string(),
        creator_login: "author-login".to_string(),
        creator_id: Some(583231),
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
            manifest_refs: vec![],
            work_label: Some("fkst-run".to_string()),
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
            source_branch: None,
            target_branch: None,
        },
        // A manifest-free registration: the effective set equals the explicit packages, so
        // `package_roots` + reachability read exactly these two refs.
        effective_packages: vec![
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
        session_id: "sess-abc".to_string(),
        config_hash: "hash123".to_string(),
        auto_merge: false,
        log_access: vec![],
        collaborators: vec![],
    }
}
