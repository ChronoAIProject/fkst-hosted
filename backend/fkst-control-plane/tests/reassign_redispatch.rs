//! Integration test for reassignment re-dispatch (#140 / "#140b").
//!
//! Proves the END-TO-END real re-dispatch seam: a goal session is placed +
//! dispatched to worker A, then a reassignment (driven through the
//! [`ReassignDriver`] over the real [`DispatchRedispatch`]) re-resolves a FRESH
//! [`ResolvedDispatch`] with a BUMPED fence and queues it to worker B's outbound
//! control channel. This exercises the same Mongo-backed resolve path the live
//! placement uses, so it needs a Mongo — it uses an ephemeral testcontainers
//! Mongo and self-skips when Docker is unavailable (so `cargo test` stays green
//! on daemonless runners), mirroring `activation_dispatch.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;

use fkst_control_plane::controller::{ClaimMap, ControllerHandle, ReassignDriver, WorkerRegistry};
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
const WORKER_A: &str = "worker-a";
const WORKER_B: &str = "worker-b";
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

/// Minimal fake GitHub API: every mint returns a distinct `ghs_…` token (so the
/// re-dispatched dispatch carries a real token, and re-resolution mints again).
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
            token: SecretString::from(format!("ghs_redispatch_token_{n}")),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        })
    }
}

// ---- builders -------------------------------------------------------------

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
        package_names: vec!["pkg-a".to_string()],
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
        package_names: vec!["pkg-a".to_string()],
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

struct Harness {
    _container: ContainerAsync<Mongo>,
    sessions: SessionService,
    goals: GoalIssueStore,
}

/// Start an ephemeral Mongo + a session service with goal support, seed `goal`.
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

/// Trigger a goal on the controller path so a session is placed + dispatched onto
/// `WORKER_A`, returning the created `session_id`, the registry (shared with the
/// handle), the claim map, and the reassign driver over the REAL re-dispatch seam.
async fn place_and_dispatch(
    h: &Harness,
    goal_id: bson::Uuid,
) -> (bson::Uuid, WorkerRegistry, Arc<ClaimMap>, ReassignDriver) {
    let registry = WorkerRegistry::new(Duration::from_secs(60));
    let claims = Arc::new(ClaimMap::new());
    // Only WORKER_A is live at trigger time, so placement lands there.
    registry.register(&reg(WORKER_A)).await;
    // dispatch_mode = true: this suite exercises the worker-dispatch +
    // reassignment path, so placement must route to a worker (#198-ii).
    let handle = ControllerHandle::new(claims.clone(), registry.clone(), 0, true);
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

    // Drain WORKER_A's initial dispatch (place_and_dispatch's job is only to set
    // up the placed state; the reassignment is asserted by the caller).
    let initial = registry.take_control(WORKER_A).await;
    assert_eq!(initial.len(), 1, "the initial dispatch landed on worker A");

    // The reassign driver over the REAL DispatchRedispatch seam, sharing the same
    // registry + claims as the placement handle.
    let redispatch = h.sessions.make_redispatch(registry.clone());
    let driver = ReassignDriver::new(claims.clone(), registry.clone(), 0, redispatch);
    (result.session_id, registry, claims, driver)
}

// ---- tests ----------------------------------------------------------------

/// A dead worker's session is reassigned to a live worker AND a fresh dispatch
/// (bumped fence, new worker_id) is queued to that worker's outbound channel.
#[tokio::test]
async fn dead_worker_redispatches_to_a_live_worker_with_bumped_fence() {
    if !docker_available() {
        eprintln!("skipping: docker unavailable");
        return;
    }
    let goal_id = bson::Uuid::new();
    let h = harness(&goal_doc(goal_id)).await;
    let (session_id, registry, claims, driver) = place_and_dispatch(&h, goal_id).await;

    let lease_key = claims
        .lease_key_for_session(session_id)
        .expect("the placed session has a claim");
    let fence_before = claims.get(&lease_key).unwrap().fencing_id;
    assert_eq!(claims.get(&lease_key).unwrap().owner_worker, WORKER_A);

    // Bring up a survivor, then reassign WORKER_A's work (the abrupt-death path).
    registry.register(&reg(WORKER_B)).await;
    let n = driver.on_worker_dead(WORKER_A).await;
    assert_eq!(n, 1, "the dead worker's one session was reassigned");

    // The claim moved to WORKER_B on a strictly-greater fence.
    let after = claims.get(&lease_key).unwrap();
    assert_eq!(after.owner_worker, WORKER_B);
    assert!(after.fencing_id > fence_before, "fence bumped on reassign");

    // A fresh ResolvedDispatch is queued to WORKER_B (delivered on next heartbeat),
    // stamped with the new worker + the bumped fence, for the SAME session.
    let drained = registry.take_control(WORKER_B).await;
    assert_eq!(
        drained.len(),
        1,
        "exactly one re-dispatch queued to worker B"
    );
    match &drained[0] {
        ControlMessage::ResolvedDispatch(dispatch) => {
            assert_eq!(dispatch.session_id, session_id.to_string());
            assert_eq!(dispatch.worker_id, WORKER_B);
            assert_eq!(dispatch.fencing_id, after.fencing_id);
        }
        other => panic!("expected a ResolvedDispatch, got {other:?}"),
    }
    // The dead worker A got nothing new.
    assert!(
        registry.take_control(WORKER_A).await.is_empty(),
        "no dispatch re-queued to the dead worker"
    );
}

/// A per-session `Released` (graceful drain) reassigns exactly that session and
/// re-dispatches it to a live worker with a bumped fence.
#[tokio::test]
async fn released_session_redispatches_to_a_live_worker() {
    if !docker_available() {
        eprintln!("skipping: docker unavailable");
        return;
    }
    let goal_id = bson::Uuid::new();
    let h = harness(&goal_doc(goal_id)).await;
    let (session_id, registry, claims, driver) = place_and_dispatch(&h, goal_id).await;

    let lease_key = claims.lease_key_for_session(session_id).unwrap();
    let fence_before = claims.get(&lease_key).unwrap().fencing_id;

    // Worker A drains; bring up B, then deliver A's per-session Released ack.
    registry.register(&reg(WORKER_B)).await;
    let reassigned = driver.on_session_released(session_id).await;
    assert!(reassigned, "the released session reassigned");

    let after = claims.get(&lease_key).unwrap();
    assert_eq!(after.owner_worker, WORKER_B);
    assert!(after.fencing_id > fence_before);

    let drained = registry.take_control(WORKER_B).await;
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        ControlMessage::ResolvedDispatch(dispatch) => {
            assert_eq!(dispatch.session_id, session_id.to_string());
            assert_eq!(dispatch.worker_id, WORKER_B);
            assert_eq!(dispatch.fencing_id, after.fencing_id);
        }
        other => panic!("expected a ResolvedDispatch, got {other:?}"),
    }
}
