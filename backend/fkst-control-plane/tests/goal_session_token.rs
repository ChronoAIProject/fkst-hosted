//! Regression test for issue #106: a goal session's substrate engine must
//! receive a GitHub App installation token at t=0.
//!
//! Before #106 the session driver called `runner.start(&prepared)` (which
//! hardcodes `goal: None`), so the engine was spawned with NO GitHub
//! credential and the trigger-time preflight token was discarded. The fix
//! makes the driver mint the token and build a `GoalContext` BEFORE starting
//! the engine (the `build_goal_context` -> `start_with_spec(goal: Some(..))`
//! path).
//!
//! After issue #115 (Mongo package store removed), sessions are created
//! exclusively via `POST /api/v1/goals/:id/trigger`; the driver now clones
//! the goal repo's `.fkst/packages/` at spawn. No package doc in Mongo is
//! needed: `create_for_goal` only inserts the session doc, and the driver
//! proceeds to `build_goal_context` (token mint) before the clone attempt.
//!
//! The facts under test:
//!   1. POSITIVE — a goal session mints the installation token for the goal's
//!      repo (mint count >= 1), and the session's failure reason is the
//!      downstream engine/clone failure, NOT a mint failure (proving
//!      `build_goal_context` succeeded and the token reached the start path).
//!
//! The NEGATIVE case (`package_session_does_not_mint_github_token`) was deleted
//! by #115: `SessionService::create` (classic package session create) no longer
//! exists; sessions are created only via a goal trigger.
//!
//! Every test gets a fresh container and self-skips when Docker is unavailable
//! so `cargo test` stays green on runners without a daemon.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use fkst_control_plane::db::Db;
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
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;

mod support;

/// True when a Docker daemon answers `docker info`.
fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Mongo image tag — Mongo 7 (the integration-test datastore major, until issue 143 removes Mongo).
const MONGO_TAG: &str = "7";

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

/// Everything a test needs, with the container kept alive for its lifetime.
struct TestCtx {
    _container: ContainerAsync<Mongo>,
    db: Db,
    sessions: SessionService,
    goals: GoalIssueStore,
    github_api: Arc<CountingFakeApi>,
}

/// Start an ephemeral Mongo and build a single-pod `SessionService` with goal
/// support (backed by the counting fake) and the test vault wired in. The
/// engine binary stays the absent default so a driven session fails at start —
/// after `build_goal_context` has already run and minted the token.
async fn ctx() -> TestCtx {
    let container = Mongo::default()
        .with_tag(MONGO_TAG)
        .start()
        .await
        .expect("start mongo");
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(27017)
        .await
        .expect("container port");
    let config = fkst_control_plane::config::Config {
        mongodb_uri: format!("mongodb://{host}:{port}"),
        mongodb_server_selection_timeout_ms: 5000,
        ..fkst_control_plane::config::Config::default()
    };
    let db = Db::connect(&config).await.expect("connect + ping");

    // PackageRepository is no longer an arg to SessionService::new (#115);
    // packages are resolved from the goal repo clone at spawn, not from Mongo.
    let sessions = SessionService::new(
        SessionRepo::new(&db),
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
    sessions.enable_vault(support::test_vault(&db));

    TestCtx {
        _container: container,
        db,
        sessions,
        goals,
        github_api,
    }
}

/// Insert a NotStarted goal bound to `repo` naming `package` into the
/// in-memory store. Returns its id. (#137: goals no longer live in Mongo.)
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

/// Poll the session document until it leaves `pending`/`validating`, returning
/// the terminal-ish doc. Panics after ~20s. The driver fails the session at
/// the clone/engine start (no binary), so this resolves quickly to `failed`.
async fn poll_until_settled(
    repo: &SessionRepo,
    id: bson::Uuid,
) -> fkst_control_plane::models::SessionDoc {
    for _ in 0..200 {
        if let Some(doc) = repo.get(id).await.expect("get session") {
            if !matches!(
                doc.status,
                SessionStatus::Pending | SessionStatus::Validating
            ) {
                return doc;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("session {id} never left pending/validating");
}

/// #106 — a goal session mints the GitHub App installation token for the goal's
/// repo at startup, and the failure is the downstream engine/clone start (not a
/// mint failure), proving the driver now delivers a token at t=0.
///
/// After #115: no Mongo package doc is seeded — `create_for_goal` only needs
/// the goal doc and repo info; the driver resolves packages from the git clone
/// AFTER minting the token. The clone will fail (no real GitHub credential /
/// repo), but the mint is observable regardless.
#[tokio::test]
async fn goal_session_mints_github_token_at_startup() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let ctx = ctx().await;
    let repo = RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    };
    // No seed_package needed: packages come from the repo clone, not Mongo (#115).
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

    let repo_handle = SessionRepo::new(&ctx.db);
    let doc = poll_until_settled(&repo_handle, result.session_id).await;

    // The driver minted a token for the goal's repo BEFORE starting the engine.
    assert!(
        ctx.github_api.mint_count() >= 1,
        "the driver must mint an installation token for the goal session at t=0 (#106)"
    );
    assert!(
        ctx.github_api
            .resolved_repos()
            .iter()
            .any(|(owner, name)| owner == "acme" && name == "site"),
        "the mint must target the goal's repo (acme/site)"
    );

    // The session failed at the engine/clone start, NOT at the token mint —
    // proving build_goal_context succeeded and the token reached the start path.
    // The error string is the runner's/clone's, never the mint sentinel.
    assert_eq!(
        doc.status,
        SessionStatus::Failed,
        "engine/clone start fails (no binary / real repo)"
    );
    let error = doc.error.unwrap_or_default();
    assert!(
        !error.contains("failed to mint github token"),
        "the failure must be downstream of the mint, got: {error}"
    );
}

// package_session_does_not_mint_github_token was deleted by #115:
// `SessionService::create` (the classic package session create, bypassing a
// goal) no longer exists. Sessions are created exclusively via a goal trigger
// (`create_for_goal`), which always carries a GoalContext and therefore always
// reaches `build_goal_context`. The NEGATIVE assertion (non-goal session never
// mints) has no plausible code path after #115 and cannot be expressed with
// the surviving API.
