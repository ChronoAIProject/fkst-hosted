//! Integration test for dispatch activation (#151 i7b).
//!
//! Proves the controller-backed placement path of [`SessionService::create_for_goal`]:
//! when `enable_controller` is wired, a goal trigger PLACES the session on a live
//! worker, RESOLVES a [`ResolvedDispatch`], and ENQUEUES it onto that worker's
//! outbound control queue (delivered on the worker's next heartbeat) — instead of
//! spawning the engine in-process. The negative case proves the no-controller
//! fallback enqueues NOTHING (it spawns the driver in-process, #198-ii).
//!
//! `create_for_goal` inserts + transitions real session documents, so this needs
//! a Mongo. It uses an ephemeral testcontainers Mongo and self-skips when Docker
//! is unavailable, so `cargo test` stays green on daemonless runners.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;

use fkst_control_plane::controller::{ClaimMap, ControllerHandle, WorkerRegistry};
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use fkst_control_plane::github_app::{GithubAppConfig, GithubAppError, GithubAppTokens};
use fkst_control_plane::goals::{GoalDoc, GoalIssueStore, GoalStatus, RepoRef};
use fkst_control_plane::sessions::{GoalTriggerInfo, SessionRepo, SessionService};

use fkst_shared::protocol::{ControlMessage, RegisterRequest, PROTOCOL_VERSION};

const OWNER: &str = "acme";
const REPO: &str = "site";
const OWNER_USER: &str = "user-1";
const WORKER: &str = "worker-7";
const MONGO_TAG: &str = "7";

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

// ---- fakes ----------------------------------------------------------------

/// Minimal fake GitHub API: every mint returns a distinct `ghs_…` token (mirrors
/// the dispatch unit-test fake so the resolved dispatch carries a real token).
#[derive(Default)]
struct FakeGithubApi {
    mint_count: AtomicUsize,
}

#[async_trait]
impl GithubApi for FakeGithubApi {
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        Ok(InstallationId(99))
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        _id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        let n = self.mint_count.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(InstallationToken {
            token: SecretString::from(format!("ghs_dispatch_token_{n}")),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        })
    }
}

// ---- builders -------------------------------------------------------------

/// An RSA-backed test GitHub-App config (the encoding key must parse).
fn github_config() -> GithubAppConfig {
    use rand::rngs::OsRng;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;
    let private = RsaPrivateKey::new(&mut OsRng, 2048).expect("rsa key");
    let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");
    GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(pem.to_string()),
        app_slug: Some("fkst".to_string()),
        webhook_secret: None,
        api_base: "https://api.github.test".to_string(),
    }
}

fn goal_doc(goal_id: bson::Uuid) -> GoalDoc {
    GoalDoc {
        id: goal_id,
        title: "Ship the thing".to_string(),
        description: "ENGINE-PROMPT-BODY".to_string(),
        package_names: vec!["pkg-a".to_string(), "pkg-b".to_string()],
        repo: None,
        status: GoalStatus::NotStarted,
        owner_user_id: OWNER_USER.to_string(),
        org_id: None,
        active_session_id: None,
        created_at: bson::DateTime::now(),
        updated_at: bson::DateTime::now(),
    }
}

fn trigger_info(goal_id: bson::Uuid) -> GoalTriggerInfo {
    GoalTriggerInfo {
        goal_id,
        repo: RepoRef {
            owner: OWNER.to_string(),
            name: REPO.to_string(),
        },
        package_names: vec!["pkg-a".to_string(), "pkg-b".to_string()],
        owner_user_id: OWNER_USER.to_string(),
        org_id: None,
        prior_status: GoalStatus::NotStarted,
        ornn_skills: None,
    }
}

fn reg(id: &str) -> RegisterRequest {
    RegisterRequest {
        worker_id: id.to_string(),
        protocol_version: PROTOCOL_VERSION,
        capacity: 4,
        engine_temp_root: "/tmp/e".to_string(),
    }
}

/// Everything a test needs, with the Mongo container kept alive.
struct Harness {
    _container: ContainerAsync<Mongo>,
    sessions: SessionService,
    goals: GoalIssueStore,
}

/// Start an ephemeral Mongo, build a session service with goal support, and seed
/// `goal` into the (in-memory) goal store. No vault/codex/ornn is wired — the
/// minimal resolve path (token + clone spec + nonce) is enough to assert the
/// dispatch is enqueued.
async fn harness(goal: &GoalDoc) -> Harness {
    let container = Mongo::default()
        .with_tag(MONGO_TAG)
        .start()
        .await
        .expect("start mongo");
    // The session store is now in-memory (#198): the SessionRepo no longer takes a
    // `Db`, so this harness no longer connects to the container. The container is
    // still started (kept alive via `_container`) so the suite's docker self-skip
    // gate stays meaningful and the wiring is ready for the rest of AppState.
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());
    let github_app =
        GithubAppTokens::with_api(&github_config(), Arc::new(FakeGithubApi::default()))
            .expect("github app");
    let goals = GoalIssueStore::new(None);
    goals.insert(goal).await.expect("seed goal");
    sessions.enable_goal_support(goals.clone(), github_app);

    Harness {
        _container: container,
        sessions,
        goals,
    }
}

/// A registry with exactly ONE registered live worker, plus a fresh claim map,
/// and a [`ControllerHandle`] over a CLONE of that registry so the test can drain
/// the SAME shared outbound queue the dispatch is enqueued onto.
fn one_worker_controller() -> (WorkerRegistry, ControllerHandle) {
    let registry = WorkerRegistry::new(Duration::from_secs(60));
    let claims = Arc::new(ClaimMap::new());
    // max_load 0 == uncapped (the default), matching the production wiring in
    // main.rs. dispatch_mode = true so the trigger takes the worker-dispatch path
    // (this suite asserts a ResolvedDispatch is enqueued, #198-ii).
    let handle = ControllerHandle::new(claims, registry.clone(), 0, true);
    (registry, handle)
}

// ---- tests ----------------------------------------------------------------

/// With the controller enabled, a goal trigger resolves + enqueues exactly one
/// `ResolvedDispatch` to the placed worker, whose `session_id` matches the
/// created session.
#[tokio::test]
async fn controller_enabled_trigger_enqueues_a_dispatch_to_the_worker() {
    if !docker_available() {
        eprintln!("skipping: docker unavailable");
        return;
    }
    let goal_id = bson::Uuid::new();
    let h = harness(&goal_doc(goal_id)).await;

    let (registry, handle) = one_worker_controller();
    registry.register(&reg(WORKER)).await;
    h.sessions.enable_controller(handle);

    let result = h
        .sessions
        .create_for_goal(
            &h.goals,
            trigger_info(goal_id),
            Some(SecretString::from("user-tok")),
        )
        .await
        .expect("trigger succeeds");

    // The placed worker's outbound queue now holds exactly one ResolvedDispatch
    // for the just-created session (delivered on the worker's next heartbeat).
    let drained = registry.take_control(WORKER).await;
    assert_eq!(
        drained.len(),
        1,
        "exactly one dispatch queued to the worker"
    );
    match &drained[0] {
        ControlMessage::ResolvedDispatch(dispatch) => {
            assert_eq!(
                dispatch.session_id,
                result.session_id.to_string(),
                "the dispatch targets the created session"
            );
            assert_eq!(dispatch.worker_id, WORKER);
        }
        other => panic!("expected a ResolvedDispatch, got {other:?}"),
    }
    // The queue is drained once-only.
    assert!(
        registry.take_control(WORKER).await.is_empty(),
        "the dispatch is delivered exactly once"
    );
}

/// Without a controller wired, the SAME trigger spawns the driver in-process and
/// enqueues NOTHING — the worker's queue stays empty (the no-controller
/// fallback, #198-ii).
#[tokio::test]
async fn controller_disabled_trigger_enqueues_nothing() {
    if !docker_available() {
        eprintln!("skipping: docker unavailable");
        return;
    }
    let goal_id = bson::Uuid::new();
    let h = harness(&goal_doc(goal_id)).await;

    // A registry with a live worker exists, but the controller is NEVER enabled,
    // so placement does not route through it and nothing is queued.
    let registry = WorkerRegistry::new(Duration::from_secs(60));
    registry.register(&reg(WORKER)).await;

    h.sessions
        .create_for_goal(
            &h.goals,
            trigger_info(goal_id),
            Some(SecretString::from("user-tok")),
        )
        .await
        .expect("trigger succeeds");

    assert!(
        registry.take_control(WORKER).await.is_empty(),
        "no controller => no dispatch enqueued (in-process path)"
    );
}
