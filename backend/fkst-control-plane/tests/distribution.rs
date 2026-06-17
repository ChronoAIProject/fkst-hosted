//! Distribution-layer integration tests against an ephemeral Mongo
//! container (testcontainers): least-loaded placement, lease-fenced
//! takeover (redo), and the boot orphan sweep. Multiple in-process
//! `Distributor` instances with distinct pod identities and fake
//! `HealthView`s stand in for pods; expiry is always FORCED via a direct
//! `expires_at` write, never by sleeping out a TTL.
//!
//! Every test self-skips when Docker is unavailable so `cargo test` stays
//! green on runners without a daemon.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bson::doc;
use fkst_control_plane::config::Config;
use fkst_control_plane::db::Db;
use fkst_control_plane::distribution::{
    DistributionConfig, Distributor, DriverHost, HealthView, PlacementError, PodLoad,
};
use fkst_control_plane::leases::{AcquireOutcome, LeaseStore, PoolConfig, PoolError};
use fkst_control_plane::models::{LeaseDoc, SessionDoc, SessionStatus};
use fkst_control_plane::sessions::repo::ORPHANED_ERROR;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;

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

/// Generous TTL: leases never expire on their own inside a test run; expiry
/// is forced via [`force_expires_at`].
const TTL: Duration = Duration::from_secs(30);

/// Start an ephemeral Mongo and build a connected `Db` over it.
async fn mongo_db() -> (ContainerAsync<Mongo>, Db) {
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
    let config = Config {
        mongodb_uri: format!("mongodb://{host}:{port}"),
        mongodb_server_selection_timeout_ms: 5000,
        ..Config::default()
    };
    let db = Db::connect(&config).await.expect("connect + ping");
    (container, db)
}

/// Fixed healthy-pod set + loads.
struct FakeHealth(Vec<PodLoad>);

#[async_trait]
impl HealthView for FakeHealth {
    async fn healthy_pods_and_loads(&self) -> Result<Vec<PodLoad>, PoolError> {
        Ok(self.0.clone())
    }
}

/// Errors exactly once (the first call), then behaves like [`FakeHealth`].
struct FlakyHealth {
    fail_next: AtomicBool,
    pods: Vec<PodLoad>,
}

#[async_trait]
impl HealthView for FlakyHealth {
    async fn healthy_pods_and_loads(&self) -> Result<Vec<PodLoad>, PoolError> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(PoolError::Mongo(mongodb::error::Error::from(
                std::io::Error::other("injected transient failure"),
            )));
        }
        Ok(self.pods.clone())
    }
}

/// Reaper seam stub: scan-2 pickup is the sessions service's concern and is
/// covered by its own suite; here it must only never panic.
struct NoopHost;

#[async_trait]
impl DriverHost for NoopHost {
    async fn ensure_driver(&self, _session: &SessionDoc) {}
}

fn pod_load(pod_id: &str, active_sessions: u64) -> PodLoad {
    PodLoad {
        pod_id: pod_id.to_string(),
        active_sessions,
    }
}

/// A `Distributor` for `pod_id` whose health view reports `pods` healthy.
/// Zero grace so a forced-expired lease is immediately scannable.
fn distributor(db: &Db, pod_id: &str, pods: Vec<PodLoad>, max_load: u64) -> Distributor {
    let pool = PoolConfig {
        pod_id: pod_id.to_string(),
        lease_ttl: TTL,
    };
    let cfg = DistributionConfig {
        pool: pool.clone(),
        renew_interval: Duration::from_secs(10),
        scan_interval: Duration::from_secs(5),
        grace: Duration::ZERO,
        max_load,
    };
    Distributor::new(
        db.clone(),
        LeaseStore::new(db, &pool),
        Arc::new(FakeHealth(pods)),
        cfg,
    )
}

/// A lease store impersonating `pod_id` (seeding leases "held" by pods that
/// do not exist as processes).
fn store(db: &Db, pod_id: &str) -> LeaseStore {
    LeaseStore::new(
        db,
        &PoolConfig {
            pod_id: pod_id.to_string(),
            lease_ttl: TTL,
        },
    )
}

fn session_doc(package_name: &str, status: SessionStatus) -> SessionDoc {
    SessionDoc {
        id: bson::Uuid::new(),
        package_name: package_name.to_string(),
        status,
        pod_id: None,
        fencing_token: None,
        pid: None,
        runtime_dir: None,
        error: None,
        run_key: None,
        owner_user_id: None,
        org_id: None,
        package_names: vec![],
        goal_id: None,
        repo: None,
        env_scope: None,
        triggered_by: None,
        nyxid_key_id: None,
        nyxid_key_prefix: None,
        ornn_skills: None,
        terminal_cause: None,
        created_at: bson::DateTime::now(),
        started_at: None,
        stopped_at: None,
    }
}

async fn insert_session(db: &Db, session: &SessionDoc) {
    db.sessions()
        .insert_one(session.clone())
        .await
        .expect("insert session");
}

async fn raw_lease(db: &Db, package: &str) -> Option<LeaseDoc> {
    db.leases()
        .find_one(doc! { "_id": package })
        .await
        .expect("find lease")
}

async fn raw_session(db: &Db, id: bson::Uuid) -> SessionDoc {
    db.sessions()
        .find_one(doc! { "_id": id })
        .await
        .expect("find session")
        .expect("session present")
}

/// Test-only direct write forcing `expires_at` (the no-sleep expiry lever).
async fn force_expires_at(db: &Db, package: &str, at: bson::DateTime) {
    let updated = db
        .leases()
        .update_one(
            doc! { "_id": package },
            doc! { "$set": { "expires_at": at } },
        )
        .await
        .expect("force expires_at");
    assert_eq!(updated.matched_count, 1, "lease to expire must exist");
}

/// A timestamp comfortably in the past (dead under `expires_at <= now`).
fn past() -> bson::DateTime {
    bson::DateTime::from_millis(bson::DateTime::now().timestamp_millis() - 60_000)
}

/// Seed an active session owned by `holder` together with its lease.
/// Returns `(session, lease)`.
async fn seed_owned_session(
    db: &Db,
    package: &str,
    holder: &str,
    status: SessionStatus,
) -> (SessionDoc, LeaseDoc) {
    let mut session = session_doc(package, status);
    let lease = match store(db, holder)
        .acquire(package, session.id)
        .await
        .expect("seed acquire")
    {
        AcquireOutcome::Acquired(lease) => lease,
        AcquireOutcome::NotAcquired => panic!("seed acquire must win"),
    };
    session.pod_id = Some(holder.to_string());
    session.fencing_token = Some(lease.fencing_token);
    session.pid = Some(4242);
    session.runtime_dir = Some("/tmp/run".to_string());
    insert_session(db, &session).await;
    (session, lease)
}

#[tokio::test]
async fn place_assigns_least_loaded() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let session = session_doc("pkg", SessionStatus::Pending);
    insert_session(&db, &session).await;

    let dist = distributor(
        &db,
        "pod-coord",
        vec![
            pod_load("pod-a", 2),
            pod_load("pod-b", 0),
            pod_load("pod-c", 1),
        ],
        0,
    );
    let placement = dist.place("pkg", session.id).await.expect("place");

    assert_eq!(placement.pod_id, "pod-b", "load-0 pod wins");
    assert_eq!(placement.fencing_token, 1, "fresh lease starts at 1");
    assert_eq!(placement.session_id, session.id);

    let lease = raw_lease(&db, "pkg").await.expect("lease acquired");
    assert_eq!(lease.holder_pod, "pod-b");
    assert_eq!(lease.session_id, session.id);

    let stored = raw_session(&db, session.id).await;
    assert_eq!(stored.pod_id.as_deref(), Some("pod-b"));
    assert_eq!(stored.fencing_token, Some(1));
    assert_eq!(
        stored.status,
        SessionStatus::Pending,
        "placement never advances status"
    );
}

#[tokio::test]
async fn place_invalid_name() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let dist = distributor(&db, "pod-a", vec![pod_load("pod-a", 0)], 0);

    let err = dist
        .place("bad name!", bson::Uuid::new())
        .await
        .expect_err("invalid name must fail");
    assert!(matches!(err, PlacementError::InvalidPackageName));

    // No Mongo writes happened: the leases collection stays empty.
    let leases = db
        .leases()
        .count_documents(doc! {})
        .await
        .expect("count leases");
    assert_eq!(leases, 0, "no lease written for an invalid name");
}

#[tokio::test]
async fn place_conflicts_on_live_lease() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let (_other_session, lease) =
        seed_owned_session(&db, "pkg", "pod-other", SessionStatus::Running).await;

    let mine = session_doc("pkg", SessionStatus::Pending);
    insert_session(&db, &mine).await;
    let dist = distributor(&db, "pod-a", vec![pod_load("pod-a", 0)], 0);

    let err = dist
        .place("pkg", mine.id)
        .await
        .expect_err("live lease must conflict");
    assert!(matches!(err, PlacementError::AlreadyRunning(_)));

    // The live lease is untouched.
    let stored = raw_lease(&db, "pkg").await.expect("lease present");
    assert_eq!(
        stored, lease,
        "a conflicting place must not modify the lease"
    );
}

/// #24 review blocker regression: two CONCURRENT placements for one
/// package (two fresh sessions, same pod) must yield exactly one winner.
/// The pre-read in `place` is a non-atomic fast path, so both calls can
/// pass it; the atomic acquire filter (live lease winnable only by the same
/// holder + session) is what prevents the second placement from stealing
/// the first one's lease — without it both would "win", two engines would
/// run one package, and the superseded driver would exit on `Lost` leaving
/// its document stuck non-terminal forever.
#[tokio::test]
async fn concurrent_places_for_one_package_yield_one_winner() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let session_1 = session_doc("pkg", SessionStatus::Pending);
    let session_2 = session_doc("pkg", SessionStatus::Pending);
    insert_session(&db, &session_1).await;
    insert_session(&db, &session_2).await;

    let dist = distributor(&db, "pod-a", vec![pod_load("pod-a", 0)], 0);
    let (first, second) = tokio::join!(
        dist.place("pkg", session_1.id),
        dist.place("pkg", session_2.id),
    );

    // Exactly one Placement; the other is the AlreadyRunning conflict.
    let (winner, loser_err) = match (first, second) {
        (Ok(placement), Err(err)) => (placement, err),
        (Err(err), Ok(placement)) => (placement, err),
        (Ok(a), Ok(b)) => panic!("both placements won: {a:?} / {b:?}"),
        (Err(a), Err(b)) => panic!("both placements failed: {a} / {b}"),
    };
    assert!(
        matches!(loser_err, PlacementError::AlreadyRunning(_)),
        "the loser must surface AlreadyRunning (the 409 the create path \
         converges on), got: {loser_err}"
    );

    // Single lease document, bound to the winner's session at token 1 (no
    // steal, no second bump).
    let leases = db
        .leases()
        .count_documents(doc! {})
        .await
        .expect("count leases");
    assert_eq!(leases, 1, "exactly one lease document");
    let lease = raw_lease(&db, "pkg").await.expect("lease present");
    assert_eq!(lease.session_id, winner.session_id);
    assert_eq!(lease.fencing_token, winner.fencing_token);
    assert_eq!(lease.fencing_token, 1, "no steal ever bumped the token");

    // Only the winner's document carries ownership — only one driver would
    // spawn. The loser's document stays unassigned (the create path then
    // converges it to failed and answers 409).
    let winner_doc = raw_session(&db, winner.session_id).await;
    assert_eq!(winner_doc.pod_id.as_deref(), Some("pod-a"));
    assert_eq!(winner_doc.fencing_token, Some(winner.fencing_token));
    let loser_id = if winner.session_id == session_1.id {
        session_2.id
    } else {
        session_1.id
    };
    let loser_doc = raw_session(&db, loser_id).await;
    assert_eq!(loser_doc.pod_id, None, "the loser never gains ownership");
    assert_eq!(loser_doc.fencing_token, None);
}

#[tokio::test]
async fn place_idempotent() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let session = session_doc("pkg", SessionStatus::Pending);
    insert_session(&db, &session).await;

    let dist = distributor(&db, "pod-a", vec![pod_load("pod-a", 0)], 0);
    let first = dist.place("pkg", session.id).await.expect("first place");
    let second = dist.place("pkg", session.id).await.expect("replayed place");

    assert_eq!(first, second, "replay returns the identical Placement");
    assert_eq!(
        raw_lease(&db, "pkg").await.expect("lease").fencing_token,
        first.fencing_token,
        "the token must not bump on replay"
    );
}

#[tokio::test]
async fn place_no_capacity() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let session = session_doc("pkg", SessionStatus::Pending);
    insert_session(&db, &session).await;

    // Every healthy pod is at the cap.
    let dist = distributor(
        &db,
        "pod-a",
        vec![pod_load("pod-a", 3), pod_load("pod-b", 5)],
        3,
    );
    let err = dist
        .place("pkg", session.id)
        .await
        .expect_err("all-at-cap must fail");
    assert!(matches!(err, PlacementError::NoCapacity));

    // The session stays pending and unassigned; no lease was taken.
    let stored = raw_session(&db, session.id).await;
    assert_eq!(stored.status, SessionStatus::Pending);
    assert_eq!(stored.pod_id, None);
    assert!(raw_lease(&db, "pkg").await.is_none());
}

#[tokio::test]
async fn takeover_only_after_expiry() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let (session, lease) = seed_owned_session(&db, "pkg", "pod-dead", SessionStatus::Running).await;
    let dist = distributor(&db, "pod-b", vec![pod_load("pod-b", 0)], 0);

    // Live lease: no takeover, lease untouched.
    let won = dist.reap_and_takeover(&NoopHost).await.expect("reap");
    assert!(won.is_empty(), "a live lease must never be taken over");
    assert_eq!(raw_lease(&db, "pkg").await.expect("lease"), lease);

    // Expired lease: exactly this takeover.
    force_expires_at(&db, "pkg", past()).await;
    let won = dist.reap_and_takeover(&NoopHost).await.expect("reap");
    assert_eq!(won.len(), 1, "one takeover won");
    assert_eq!(won[0].session_id, session.id);
    assert_eq!(won[0].pod_id, "pod-b");
    assert!(
        won[0].fencing_token > lease.fencing_token,
        "the redo token must be strictly greater"
    );

    let stored = raw_session(&db, session.id).await;
    assert_eq!(
        stored.status,
        SessionStatus::Pending,
        "redo normalizes to pending"
    );
    assert_eq!(stored.pod_id.as_deref(), Some("pod-b"));
    assert_eq!(stored.fencing_token, Some(won[0].fencing_token));
    assert_eq!(stored.pid, None, "dead pod's pid cleared");
    assert_eq!(stored.runtime_dir, None, "dead pod's runtime_dir cleared");

    let new_lease = raw_lease(&db, "pkg").await.expect("lease survives");
    assert_eq!(new_lease.holder_pod, "pod-b");
    assert_eq!(
        new_lease.session_id, session.id,
        "the SAME session is redone; the lease binding must not change"
    );
}

#[tokio::test]
async fn takeover_single_winner() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let (session, _lease) =
        seed_owned_session(&db, "pkg", "pod-dead", SessionStatus::Running).await;
    force_expires_at(&db, "pkg", past()).await;

    let dist_a = distributor(&db, "pod-a", vec![pod_load("pod-a", 0)], 0);
    let dist_b = distributor(&db, "pod-b", vec![pod_load("pod-b", 0)], 0);

    let (won_a, won_b) = tokio::join!(
        dist_a.reap_and_takeover(&NoopHost),
        dist_b.reap_and_takeover(&NoopHost),
    );
    let won_a = won_a.expect("pod-a pass");
    let won_b = won_b.expect("pod-b pass");
    assert_eq!(
        won_a.len() + won_b.len(),
        1,
        "exactly one survivor wins the takeover: a={won_a:?} b={won_b:?}"
    );

    let winner = won_a.first().or(won_b.first()).expect("one winner");
    let stored = raw_session(&db, session.id).await;
    assert_eq!(stored.pod_id.as_deref(), Some(winner.pod_id.as_str()));
    let lease = raw_lease(&db, "pkg").await.expect("lease present");
    assert_eq!(lease.holder_pod, winner.pod_id, "lease and session agree");
}

#[tokio::test]
async fn healthy_holder_not_taken_over() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let (session, lease) =
        seed_owned_session(&db, "pkg", "pod-other", SessionStatus::Running).await;
    force_expires_at(&db, "pkg", past()).await;
    let expired = raw_lease(&db, "pkg").await.expect("lease present");

    // Health reports the lapsed holder as healthy: fail closed, skip.
    let dist = distributor(
        &db,
        "pod-b",
        vec![pod_load("pod-b", 0), pod_load("pod-other", 1)],
        0,
    );
    let won = dist.reap_and_takeover(&NoopHost).await.expect("reap");
    assert!(won.is_empty(), "a healthy holder must not be taken over");
    assert_eq!(
        raw_lease(&db, "pkg").await.expect("lease present"),
        expired,
        "the lapsed-but-healthy holder's lease is untouched"
    );
    let stored = raw_session(&db, session.id).await;
    assert_eq!(stored.pod_id.as_deref(), Some("pod-other"));
    assert_eq!(stored.fencing_token, Some(lease.fencing_token));
}

#[tokio::test]
async fn self_holder_lapsed_is_reclaimed() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    // The lease is held by THIS pod but lapsed (e.g. a long stall): the old
    // process is us, so reclaiming can never double-run — always allowed.
    let (session, lease) = seed_owned_session(&db, "pkg", "pod-a", SessionStatus::Running).await;
    force_expires_at(&db, "pkg", past()).await;

    let dist = distributor(&db, "pod-a", vec![pod_load("pod-a", 1)], 0);
    let won = dist.reap_and_takeover(&NoopHost).await.expect("reap");
    assert_eq!(won.len(), 1, "own lapsed lease is reclaimed");
    assert_eq!(won[0].pod_id, "pod-a");
    assert!(won[0].fencing_token > lease.fencing_token);

    let stored = raw_session(&db, session.id).await;
    assert_eq!(stored.status, SessionStatus::Pending);
    assert_eq!(stored.fencing_token, Some(won[0].fencing_token));
}

#[tokio::test]
async fn takeover_session_went_terminal() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let (session, _lease) =
        seed_owned_session(&db, "pkg", "pod-dead", SessionStatus::Running).await;
    force_expires_at(&db, "pkg", past()).await;

    // The session went terminal while its (now expired) lease lingered.
    db.sessions()
        .update_one(
            doc! { "_id": session.id },
            doc! { "$set": { "status": "failed", "error": "boom" } },
        )
        .await
        .expect("flip terminal");

    let dist = distributor(&db, "pod-b", vec![pod_load("pod-b", 0)], 0);
    let won = dist.reap_and_takeover(&NoopHost).await.expect("reap");
    assert!(won.is_empty(), "a terminal session is never redone");
    assert!(
        raw_lease(&db, "pkg").await.is_none(),
        "the lingering lease must be released"
    );
    let stored = raw_session(&db, session.id).await;
    assert_eq!(stored.status, SessionStatus::Failed, "terminal state kept");
}

#[tokio::test]
async fn reaper_survives_transient_error() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let (session, _lease) =
        seed_owned_session(&db, "pkg", "pod-dead", SessionStatus::Running).await;
    force_expires_at(&db, "pkg", past()).await;

    let pool = PoolConfig {
        pod_id: "pod-b".to_string(),
        lease_ttl: TTL,
    };
    let dist = Distributor::new(
        db.clone(),
        LeaseStore::new(&db, &pool),
        Arc::new(FlakyHealth {
            fail_next: AtomicBool::new(true),
            pods: vec![pod_load("pod-b", 0)],
        }),
        DistributionConfig {
            pool: pool.clone(),
            renew_interval: Duration::from_secs(10),
            scan_interval: Duration::from_secs(5),
            grace: Duration::ZERO,
            max_load: 0,
        },
    );

    // First tick: the injected failure surfaces as an error (the loop logs
    // and keeps ticking)...
    dist.reap_and_takeover(&NoopHost)
        .await
        .expect_err("injected failure must surface");
    // ...and the next tick succeeds and does the work.
    let won = dist
        .reap_and_takeover(&NoopHost)
        .await
        .expect("second tick");
    assert_eq!(won.len(), 1, "the takeover happens on the next tick");
    assert_eq!(won[0].session_id, session.id);
}

/// The boot sweep fails THIS pod's pre-terminal sessions and releases their
/// leases — and is scoped to this pod: sessions owned by other pods (their
/// holder may be healthy) and unassigned pending sessions are untouched.
#[tokio::test]
async fn fail_orphans_releases_lease() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let (session, _lease) =
        seed_owned_session(&db, "pkg", "pod-dead", SessionStatus::Running).await;
    // A foreign pod's live session + lease and an unassigned pending
    // session: both must survive pod-dead's boot sweep.
    let (foreign_session, foreign_lease) =
        seed_owned_session(&db, "pkg-foreign", "pod-other", SessionStatus::Running).await;
    let unassigned = session_doc("pkg-unassigned", SessionStatus::Pending);
    insert_session(&db, &unassigned).await;

    let dist = distributor(&db, "pod-dead", vec![pod_load("pod-dead", 0)], 0);
    let failed = dist.fail_orphans_at_boot().await.expect("boot sweep");
    assert_eq!(failed, 1, "exactly this pod's active session is swept");

    let stored = raw_session(&db, session.id).await;
    assert_eq!(stored.status, SessionStatus::Failed);
    assert_eq!(stored.error.as_deref(), Some(ORPHANED_ERROR));
    assert!(
        raw_lease(&db, "pkg").await.is_none(),
        "the orphaned session's lease must be released at boot"
    );

    // The foreign pod's session and lease are untouched (its recovery, if
    // its holder really is dead, belongs to the lease-expiry takeover).
    let foreign = raw_session(&db, foreign_session.id).await;
    assert_eq!(foreign.status, SessionStatus::Running);
    assert_eq!(foreign.pod_id.as_deref(), Some("pod-other"));
    assert_eq!(
        raw_lease(&db, "pkg-foreign").await.expect("lease present"),
        foreign_lease,
        "a foreign pod's lease must not be released by this pod's boot"
    );

    // The unassigned pending session stays pending for placement.
    let pending = raw_session(&db, unassigned.id).await;
    assert_eq!(pending.status, SessionStatus::Pending);
    assert_eq!(pending.pod_id, None);
}
