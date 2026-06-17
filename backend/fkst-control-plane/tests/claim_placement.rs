//! In-process placement cutover (#198-ii): the controller's in-memory `ClaimMap`
//! is the SINGLE claim authority for the DEFAULT in-process execution path.
//!
//! These prove the two load-bearing properties of the cutover:
//! 1. A goal trigger CLAIMS its lease key in the `ClaimMap` (owner = the
//!    controller's pod id) and spawns the in-process driver; a conflicting live
//!    claim makes the trigger fail `409 Conflict` and spawns NOTHING extra.
//! 2. NO STRANDED CLAIM — a session that reaches a terminal driver exit RELEASES
//!    its claim, so the lease key frees up and the goal can be re-triggered.
//!
//! Docker-free: the session store is in-memory (#198-i) and the goal store is
//! in-memory, so no datastore is needed. Dispatch mode is OFF (the default), so
//! placement takes the in-process claim path (never the worker-dispatch path).

use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;

use fkst_control_plane::controller::{ClaimMap, ControllerHandle, WorkerRegistry};
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::error::AppError;
use fkst_control_plane::goals::{GoalDoc, GoalIssueStore, GoalStatus, RepoRef};
use fkst_control_plane::models::SessionStatus;
use fkst_control_plane::sessions::{GoalTriggerInfo, SessionRepo, SessionService};

const OWNER: &str = "acme";
const REPO: &str = "site";
const OWNER_USER: &str = "user-1";

// ---- builders -------------------------------------------------------------

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

/// Lease key the in-process claim path uses for a goal session (mirrors
/// `SessionDoc::lease_key`).
fn lease_key(goal_id: bson::Uuid) -> String {
    format!("goal-{goal_id}")
}

/// A session service with the controller enabled in IN-PROCESS mode
/// (`dispatch_mode = false`), plus the goal seeded into an in-memory goal store.
/// Goal support is intentionally NOT wired: the spawned driver then fails fast at
/// `build_goal_context` (no GitHub token to mint), reaching a terminal exit
/// quickly without any network — exactly what the no-stranded-claim test needs.
async fn harness(goal: &GoalDoc) -> (SessionService, GoalIssueStore, Arc<ClaimMap>) {
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());
    let claims = Arc::new(ClaimMap::new());
    let registry = WorkerRegistry::new(Duration::from_secs(60));
    // dispatch_mode = false: placement claims + spawns in-process.
    let handle = ControllerHandle::new(claims.clone(), registry, 0, false);
    sessions.enable_controller(handle);

    let goals = GoalIssueStore::new(None);
    goals.insert(goal).await.expect("seed goal");
    (sessions, goals, claims)
}

/// Poll `claims` until the entry for `key` is gone (the driver released it on its
/// terminal exit), or fail after a generous timeout. The driver runs detached, so
/// the release is asynchronous.
async fn await_claim_released(claims: &ClaimMap, key: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if claims.get(key).is_none() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("claim for {key} was never released (STRANDED CLAIM)");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ---- tests ----------------------------------------------------------------

/// In-process placement: a goal trigger CLAIMS its lease key (owner = the
/// controller pod id) and the created session is stamped with the claim's fence.
/// A SECOND trigger whose lease key is already claimed by a DIFFERENT session
/// fails `409 Conflict` (the `AlreadyClaimed` arm) and does NOT spawn — the
/// original claim stays intact, bound to its session.
#[tokio::test]
async fn in_process_trigger_claims_then_conflicts_on_a_busy_lease_key() {
    let goal_id = bson::Uuid::new();
    let (sessions, goals, claims) = harness(&goal_doc(goal_id)).await;

    let result = sessions
        .create_for_goal(
            &goals,
            trigger_info(goal_id),
            Some(SecretString::from("user-tok")),
        )
        .await
        .expect("first trigger claims + spawns");

    // The claim exists for the goal's lease key, bound to the created session and
    // owned by the controller (a non-blank owner). The created session is fenced
    // by that claim.
    let key = lease_key(goal_id);
    let entry = claims.get(&key).expect("the goal's lease key is claimed");
    assert_eq!(
        entry.session_id, result.session_id,
        "claim binds the session"
    );
    assert!(!entry.owner_worker.is_empty(), "claim owner is never blank");
    assert_eq!(
        entry.goal_id,
        Some(goal_id),
        "the goal id rides on the claim"
    );

    // Now drive a CONFLICT through the same code path: a second goal whose lease
    // key is already taken by a DIFFERENT session must surface 409. The goal CAS
    // alone would block a re-trigger of the SAME goal, so to reach the
    // `AlreadyClaimed` arm specifically, pre-occupy a fresh goal's lease key with
    // an unrelated session, then trigger it.
    let goal2 = bson::Uuid::new();
    goals.insert(&goal_doc(goal2)).await.expect("seed goal 2");
    let key2 = lease_key(goal2);
    let intruder = bson::Uuid::new();
    claims
        .claim(&key2, intruder, Some(goal2), "someone-else")
        .expect("pre-occupy goal2's lease key");

    match sessions
        .create_for_goal(&goals, trigger_info(goal2), None)
        .await
    {
        Err(AppError::Conflict(_)) => {}
        Ok(_) => panic!("a busy lease key must conflict, not succeed"),
        Err(other) => panic!("AlreadyClaimed maps to 409 Conflict, got {other:?}"),
    }

    // The conflict did NOT disturb the pre-existing claim: it is still bound to
    // the intruder session, never re-pointed at the rejected trigger.
    let still = claims
        .get(&key2)
        .expect("intruder claim survives the conflict");
    assert_eq!(
        still.session_id, intruder,
        "the conflicting trigger never stole the claim"
    );
}

/// NO STRANDED CLAIM (the load-bearing invariant): a triggered in-process session
/// that reaches a terminal driver exit RELEASES its claim, so the lease key frees
/// up and the goal becomes re-triggerable. Here the driver fails fast (no goal
/// support => no token to mint), which is a terminal exit, and the teardown MUST
/// still release the claim.
#[tokio::test]
async fn terminal_exit_releases_the_claim_so_the_goal_can_be_retriggered() {
    let goal_id = bson::Uuid::new();
    let (sessions, goals, claims) = harness(&goal_doc(goal_id)).await;
    let key = lease_key(goal_id);

    let result = sessions
        .create_for_goal(
            &goals,
            trigger_info(goal_id),
            Some(SecretString::from("user-tok")),
        )
        .await
        .expect("trigger claims + spawns");
    let first_session = result.session_id;
    // The claim is taken synchronously by `create_for_goal` before the driver
    // spawns, so it is present immediately after the trigger returns.
    assert!(
        claims.get(&key).is_some(),
        "the claim is held while the session is live"
    );

    // The detached driver fails fast (build_goal_context errors with no goal
    // support) and reaches its terminal teardown, which releases the claim.
    await_claim_released(&claims, &key).await;

    // The session itself is terminal (Failed), confirming a real terminal exit
    // drove the release rather than the claim never being taken.
    let doc = sessions
        .get(first_session)
        .await
        .expect("get session")
        .expect("session exists");
    assert!(
        matches!(doc.status, SessionStatus::Failed | SessionStatus::Stopped),
        "the session reached a terminal status, got {:?}",
        doc.status
    );

    // The released claim means the lease key is free again: a brand-new session
    // can re-claim it (proving the goal is not stuck forever).
    let next_session = bson::Uuid::new();
    let reclaimed = claims
        .claim(&key, next_session, Some(goal_id), "controller")
        .expect("the freed lease key is re-claimable after a terminal exit");
    assert_eq!(
        reclaimed.session_id, next_session,
        "the lease key is genuinely free for a new claim"
    );
}
