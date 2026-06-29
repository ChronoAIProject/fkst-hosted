//! API-only control plane: a goal trigger RECORDS a `Pending` session and runs
//! NOTHING in-process.
//!
//! Before the single control-plane refactor, `create_for_goal` claimed the goal
//! and spawned an in-process driver that minted a GitHub App installation token
//! at t=0 (issue #106). That driver is gone: the control plane is API-only, so a
//! trigger only inserts a `Pending` `SessionDoc` and links it to the goal —
//! pod-per-session execution (milestone #9) is what will later clone the repo
//! and mint credentials.
//!
//! The facts under test:
//!   1. a goal trigger records a `Pending` session, moves the goal to
//!      `Triggered`, and links the session as the goal's active session;
//!   2. NO GitHub App installation token is minted at trigger time (mint
//!      count == 0) — credential work is deferred to pod-per-session execution.
//!
//! The control plane is datastore-free: the harness builds a `SessionService`
//! over an in-memory store with no external datastore, so the test runs
//! unconditionally on any runner.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use fkst_control_plane::github_app::{GithubAppConfig, GithubAppTokens};
use fkst_control_plane::goals::{GoalDoc, GoalIssueStore, GoalStatus};
use fkst_control_plane::models::{RepoRef, SessionStatus};
use fkst_control_plane::sessions::service::GoalTriggerInfo;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use secrecy::SecretString;

mod support;

/// Counting fake GitHub transport. Records every installation-token mint and
/// the `(owner, repo)` it resolved an installation for, so a test can prove the
/// driver minted a token for the goal's repo at startup — never exposing a
/// token value. Mirrors the `FakeApi` pattern in `src/github_app/mod.rs` tests.
#[derive(Debug, Default)]
struct CountingFakeApi {
    installation_id: InstallationId,
    mint_count: AtomicUsize,
    /// `(owner, repo)` pairs the fake resolved an installation for, in order.
    resolved_repos: Mutex<Vec<(String, String)>>,
}

impl CountingFakeApi {
    fn new(id: u64) -> Self {
        Self {
            installation_id: InstallationId(id),
            ..Self::default()
        }
    }

    fn mint_count(&self) -> usize {
        self.mint_count.load(Ordering::SeqCst)
    }

    fn resolved_repos(&self) -> Vec<(String, String)> {
        self.resolved_repos.lock().expect("resolved repos").clone()
    }
}

#[async_trait]
impl GithubApi for CountingFakeApi {
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<InstallationId, fkst_control_plane::github_app::GithubAppError> {
        self.resolved_repos
            .lock()
            .expect("resolved repos")
            .push((owner.to_string(), repo.to_string()));
        Ok(self.installation_id)
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, fkst_control_plane::github_app::GithubAppError> {
        let count = self.mint_count.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(InstallationToken {
            // Distinct, clearly-fake value; never a real token. The session
            // never surfaces it, so it only ever flows into the engine spec.
            token: SecretString::from(format!("ghs_fake_{}_{count}", id.0)),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        })
    }
}

/// Build a `GithubAppConfig` with a freshly generated RSA key (the `with_api`
/// path still validates the PEM). Mirrors `test_config()` in the unit tests.
fn test_github_config() -> GithubAppConfig {
    use rand::rngs::OsRng;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;
    let mut rng = OsRng;
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa key");
    let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");
    GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(pem.to_string()),
        app_slug: Some("fkst-test".to_string()),
        webhook_secret: None,
        api_base: "https://api.github.com".to_string(),
    }
}

/// Everything a test needs to drive a goal session.
struct TestCtx {
    sessions: SessionService,
    goals: GoalIssueStore,
    github_api: Arc<CountingFakeApi>,
}

/// Build a single-pod, datastore-free (#143) `SessionService` with goal support
/// (backed by the counting fake) and the test vault wired in. The engine binary
/// stays the absent default so a driven session fails at start — after
/// `build_goal_context` has already run and minted the token.
async fn ctx() -> TestCtx {
    // PackageRepository is no longer an arg to SessionService::new (#115);
    // packages are resolved from the goal repo clone at spawn. The store is
    // fully in-memory (#143), so no external datastore is provisioned.
    let sessions = SessionService::new(
        SessionRepo::new(),
        // Default engine config -> framework_bin is the absent
        // /usr/local/bin/fkst-framework, so the engine start fails (downstream
        // of the token mint). That is the expected terminal state here.
        EngineConfig::default(),
    );

    let github_api = Arc::new(CountingFakeApi::new(7));
    let github_app = GithubAppTokens::with_api(&test_github_config(), github_api.clone())
        .expect("github app tokens service");
    // One in-memory store shared between the test (seeding goals) and the
    // sessions service. `GoalIssueStore` is `Arc`-backed, so the clone shares
    // the same map — seeded goals are visible to `create_for_goal`.
    let goals = GoalIssueStore::new(None);
    sessions.enable_goal_support(goals.clone(), github_app);
    sessions.enable_vault(support::test_vault());

    TestCtx {
        sessions,
        goals,
        github_api,
    }
}

/// Insert a NotStarted goal bound to `repo` naming `package` into the
/// in-memory store. Returns its id. (#137/#143: goals live in the in-memory
/// store, not an external datastore.)
async fn seed_goal(goals: &GoalIssueStore, repo: RepoRef, package: &str) -> bson::Uuid {
    let id = bson::Uuid::new();
    let now = bson::DateTime::now();
    let goal = GoalDoc {
        id,
        title: "Wire the token".to_string(),
        description: "Goal that needs a GitHub App token at t=0".to_string(),
        package_names: vec![package.to_string()],
        repo: Some(repo),
        status: GoalStatus::NotStarted,
        owner_user_id: "owner-1".to_string(),
        org_id: None,
        active_session_id: None,
        created_at: now,
        updated_at: now,
    };
    goals.insert(&goal).await.expect("seed goal");
    id
}

/// A goal trigger records a `Pending` session and mints NOTHING in-process: the
/// API-only control plane defers all credential/engine work to pod-per-session
/// execution (milestone #9). Replaces the pre-refactor #106 assertion that the
/// in-process driver minted a token at t=0.
#[tokio::test]
async fn goal_trigger_records_pending_session_without_minting() {
    let ctx = ctx().await;
    let repo = RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    };
    let goal_id = seed_goal(&ctx.goals, repo.clone(), "goal-pkg").await;

    let result = ctx
        .sessions
        .create_for_goal(
            &ctx.goals,
            GoalTriggerInfo {
                goal_id,
                repo: repo.clone(),
                package_names: vec!["goal-pkg".to_string()],
                owner_user_id: "owner-1".to_string(),
                org_id: None,
                prior_status: GoalStatus::NotStarted,
                ornn_skills: None,
            },
            Some(SecretString::from("test-raw-token".to_string())),
        )
        .await
        .expect("create_for_goal");

    // The goal moved to Triggered and the session is the goal's active session.
    assert_eq!(result.goal_status, GoalStatus::Triggered);
    assert_eq!(
        ctx.goals.active_session(goal_id).await,
        Some(result.session_id),
        "the session must be linked as the goal's active session"
    );

    // The session is recorded as Pending and is NEVER run in-process: with no
    // driver, it stays Pending across the whole window.
    let repo_handle = ctx.sessions.repo().clone();
    let doc = repo_handle
        .get(result.session_id)
        .await
        .expect("get session")
        .expect("session exists");
    assert_eq!(
        doc.status,
        SessionStatus::Pending,
        "the trigger only records a pending session; nothing runs it"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let still = repo_handle
        .get(result.session_id)
        .await
        .expect("get session")
        .expect("session exists");
    assert_eq!(
        still.status,
        SessionStatus::Pending,
        "no in-process driver: the session must remain pending"
    );

    // No GitHub App installation token is minted at trigger time — credential
    // work is deferred to pod-per-session execution.
    assert_eq!(
        ctx.github_api.mint_count(),
        0,
        "the API-only control plane must NOT mint a token at trigger time"
    );
    assert!(
        ctx.github_api.resolved_repos().is_empty(),
        "no installation should be resolved at trigger time"
    );
}
