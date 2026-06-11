//! Lease coordination integration tests against an ephemeral Mongo container
//! (testcontainers). Every test gets a fresh container and self-skips when
//! Docker is unavailable so `cargo test` stays green on runners without a
//! Docker daemon.
//!
//! Expiry is always FORCED via a direct test-only write on `expires_at` —
//! never by sleeping out a TTL or shrinking the TTL.

use std::time::Duration;

use bson::doc;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::{Db, IDX_LEASES_EXPIRES_AT};
use fkst_hosted_api::leases::{
    AcquireOutcome, LeaseStore, PoolConfig, ReleaseOutcome, RenewOutcome, IDX_LEASES_HOLDER_POD,
};
use fkst_hosted_api::models::LeaseDoc;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;
use tokio::task::JoinSet;

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

/// Mongo image tag — pinned to the same major as `backend/docker-compose.yml`
/// so integration tests and local dev exercise the same server line.
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

/// A `LeaseStore` bound to `db` under the given pod identity.
fn store(db: &Db, pod_id: &str) -> LeaseStore {
    LeaseStore::new(
        db,
        &PoolConfig {
            pod_id: pod_id.to_string(),
            lease_ttl: TTL,
        },
    )
}

/// Raw read of the lease document, bypassing the store.
async fn raw_lease(db: &Db, package: &str) -> Option<LeaseDoc> {
    db.leases()
        .find_one(doc! { "_id": package })
        .await
        .expect("find_one")
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

/// Busy-wait until the millisecond clock has strictly advanced past `after`,
/// so strict `>` assertions on stored timestamps are deterministic even when
/// two operations land within the same millisecond. This is NOT a TTL wait
/// (expiry is always forced); it only guarantees a clock tick (<= 1ms).
fn wait_clock_tick(after: bson::DateTime) {
    while bson::DateTime::now() <= after {
        std::hint::spin_loop();
    }
}

fn acquired(outcome: AcquireOutcome) -> LeaseDoc {
    match outcome {
        AcquireOutcome::Acquired(lease) => lease,
        AcquireOutcome::NotAcquired => panic!("expected Acquired, got NotAcquired"),
    }
}

fn renewed(outcome: RenewOutcome) -> LeaseDoc {
    match outcome {
        RenewOutcome::Renewed(lease) => lease,
        RenewOutcome::Lost => panic!("expected Renewed, got Lost"),
    }
}

/// AC3: empty collection -> Acquired, token 1, all fields ours, live expiry.
#[tokio::test]
async fn acquire_fresh() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");
    let session = bson::Uuid::new();

    let before = bson::DateTime::now();
    let lease = acquired(pod_a.acquire("pkg", session).await.expect("acquire"));

    assert_eq!(lease.fencing_token, 1, "first token of a fresh doc is 1");
    assert_eq!(lease.package_name, "pkg");
    assert_eq!(lease.holder_pod, "pod-a");
    assert_eq!(lease.session_id, session);
    assert!(
        lease.expires_at > before,
        "expires_at must be in the future"
    );
    assert!(lease.renewed_at >= before);
    // The returned post-image is exactly the stored document.
    assert_eq!(raw_lease(&db, "pkg").await.expect("doc present"), lease);
}

/// AC4: a live lease held by A is untouchable by B -> NotAcquired, doc
/// byte-for-byte unchanged.
#[tokio::test]
async fn acquire_contended() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");
    let pod_b = store(&db, "pod-b");

    let lease_a = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("A acquires"),
    );

    let outcome = pod_b
        .acquire("pkg", bson::Uuid::new())
        .await
        .expect("contention is an outcome, not an error");
    assert_eq!(outcome, AcquireOutcome::NotAcquired);

    assert_eq!(
        raw_lease(&db, "pkg").await.expect("doc present"),
        lease_a,
        "a contended acquire must not modify the lease in any field"
    );
}

/// AC5: an expired lease is taken over by another pod; the token continues
/// monotonically (prev + 1) because the document survived.
#[tokio::test]
async fn acquire_after_expiry() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");
    let pod_b = store(&db, "pod-b");

    let lease_a = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("A acquires"),
    );
    force_expires_at(&db, "pkg", past()).await;

    let session_b = bson::Uuid::new();
    let lease_b = acquired(pod_b.acquire("pkg", session_b).await.expect("B takes over"));

    assert_eq!(lease_b.fencing_token, lease_a.fencing_token + 1);
    assert_eq!(lease_b.holder_pod, "pod-b");
    assert_eq!(lease_b.session_id, session_b);
}

/// AC6: a self-reacquire bumps the token, rebinds session_id, and extends
/// the expiry — callers must re-read the returned token.
#[tokio::test]
async fn self_reacquire_bumps_token() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");

    let first = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("first"),
    );
    assert_eq!(first.fencing_token, 1);

    wait_clock_tick(first.renewed_at);
    let session_2 = bson::Uuid::new();
    let second = acquired(pod_a.acquire("pkg", session_2).await.expect("re-acquire"));

    assert_eq!(second.fencing_token, 2, "self-reacquire bumps the token");
    assert_eq!(second.session_id, session_2, "session_id is rebound");
    assert!(second.expires_at > first.expires_at, "expiry extended");
    assert!(second.renewed_at > first.renewed_at);
    assert_eq!(second.holder_pod, "pod-a");
}

/// AC7: renew keeps the token (and session) and strictly advances
/// `expires_at` / `renewed_at`.
#[tokio::test]
async fn renew_keeps_token_extends_expiry() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");

    let lease = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("acquire"),
    );

    wait_clock_tick(lease.renewed_at);
    let renewed_lease = renewed(
        pod_a
            .renew("pkg", lease.fencing_token)
            .await
            .expect("renew"),
    );

    assert_eq!(
        renewed_lease.fencing_token, lease.fencing_token,
        "token unchanged"
    );
    assert_eq!(
        renewed_lease.session_id, lease.session_id,
        "session unchanged"
    );
    assert!(
        renewed_lease.expires_at > lease.expires_at,
        "expiry extended"
    );
    assert!(
        renewed_lease.renewed_at > lease.renewed_at,
        "renewed_at advanced"
    );
}

/// AC8: a stale heartbeat after a fence is Lost and modifies nothing; a
/// never-existed token is Lost too (equality pin).
#[tokio::test]
async fn renew_lost_after_fence() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");
    let pod_b = store(&db, "pod-b");

    let lease_a = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("A acquires"),
    );
    force_expires_at(&db, "pkg", past()).await;
    let lease_b = acquired(
        pod_b
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("B takes over"),
    );
    assert_eq!(lease_b.fencing_token, lease_a.fencing_token + 1);

    // A's stale heartbeat at its old token: Lost, doc untouched.
    let outcome = pod_a
        .renew("pkg", lease_a.fencing_token)
        .await
        .expect("stale renew is an outcome");
    assert_eq!(outcome, RenewOutcome::Lost);
    assert_eq!(
        raw_lease(&db, "pkg").await.expect("doc present"),
        lease_b,
        "a lost renew must not modify the document"
    );

    // A token that never existed is Lost even for the live holder pod.
    let outcome = pod_b.renew("pkg", 9999).await.expect("renew");
    assert_eq!(outcome, RenewOutcome::Lost);
    assert_eq!(raw_lease(&db, "pkg").await.expect("doc present"), lease_b);

    // Sanity: the holder at the correct token still renews.
    renewed(
        pod_b
            .renew("pkg", lease_b.fencing_token)
            .await
            .expect("renew"),
    );
}

/// AC8 (expiry half): renew never resurrects a dead lease — once
/// `expires_at <= now`, the holder's own renew at the right token is Lost.
#[tokio::test]
async fn renew_lost_after_expiry() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");

    let lease = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("acquire"),
    );
    force_expires_at(&db, "pkg", past()).await;

    let outcome = pod_a
        .renew("pkg", lease.fencing_token)
        .await
        .expect("expired renew is an outcome");
    assert_eq!(
        outcome,
        RenewOutcome::Lost,
        "a dead lease cannot be renewed"
    );
}

/// AC9: release deletes the doc; re-release and wrong-pod/wrong-token
/// releases are NotHeld and delete nothing.
#[tokio::test]
async fn release_then_notheld() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");
    let pod_b = store(&db, "pod-b");

    let lease = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("acquire"),
    );

    // Wrong pod: NotHeld, doc unaffected.
    let outcome = pod_b
        .release("pkg", lease.fencing_token)
        .await
        .expect("release");
    assert_eq!(outcome, ReleaseOutcome::NotHeld);
    assert_eq!(raw_lease(&db, "pkg").await.expect("doc present"), lease);

    // Wrong token: NotHeld, doc unaffected.
    let outcome = pod_a
        .release("pkg", lease.fencing_token + 1)
        .await
        .expect("release");
    assert_eq!(outcome, ReleaseOutcome::NotHeld);
    assert_eq!(raw_lease(&db, "pkg").await.expect("doc present"), lease);

    // The holder at the right token releases: doc gone.
    let outcome = pod_a
        .release("pkg", lease.fencing_token)
        .await
        .expect("release");
    assert_eq!(outcome, ReleaseOutcome::Released);
    assert!(raw_lease(&db, "pkg").await.is_none(), "document deleted");

    // Idempotent: an immediate second release is NotHeld, not an error.
    let outcome = pod_a
        .release("pkg", lease.fencing_token)
        .await
        .expect("re-release");
    assert_eq!(outcome, ReleaseOutcome::NotHeld);
}

/// AC10: release deletes the document, so the next acquire starts a fresh
/// lease whose first token is 1 (the documented reset boundary).
#[tokio::test]
async fn release_resets_token() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");

    acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("acquire"),
    );
    let second = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("re-acquire"),
    );
    assert_eq!(second.fencing_token, 2);

    let outcome = pod_a
        .release("pkg", second.fencing_token)
        .await
        .expect("release");
    assert_eq!(outcome, ReleaseOutcome::Released);

    let session_3 = bson::Uuid::new();
    let fresh = acquired(
        pod_a
            .acquire("pkg", session_3)
            .await
            .expect("fresh acquire"),
    );
    assert_eq!(
        fresh.fencing_token, 1,
        "token resets with the fresh document"
    );
    assert_eq!(fresh.session_id, session_3);
}

/// Consensus pin: fencing tokens are compared by EQUALITY ONLY. After a
/// release resets the counter (live token 1), a renew with the OLD, HIGHER
/// pre-release token must be Lost — an ordering comparison (e.g. stored <=
/// presented) would wrongly accept it.
#[tokio::test]
async fn stale_higher_token_after_release_renews_lost() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");

    // Raise the token to 3, then release at 3.
    acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("acquire 1"),
    );
    acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("acquire 2"),
    );
    let old = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("acquire 3"),
    );
    assert_eq!(old.fencing_token, 3);
    assert_eq!(
        pod_a.release("pkg", 3).await.expect("release"),
        ReleaseOutcome::Released
    );

    // Fresh lease: token 1 — numerically LOWER than the stale token 3.
    let fresh = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("fresh"),
    );
    assert_eq!(fresh.fencing_token, 1);

    // The stale higher token must be rejected (equality-only comparison).
    let outcome = pod_a.renew("pkg", 3).await.expect("stale renew");
    assert_eq!(outcome, RenewOutcome::Lost);
    assert_eq!(
        raw_lease(&db, "pkg").await.expect("doc present"),
        fresh,
        "the stale renew must not modify the fresh lease"
    );

    // And the current token still works.
    renewed(pod_a.renew("pkg", 1).await.expect("current renew"));
}

/// AC11: holds_current is true ONLY for {current holder + current token +
/// live}; false for wrong token, wrong pod, expired, and missing doc.
#[tokio::test]
async fn holds_current_matrix() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");
    let pod_b = store(&db, "pod-b");

    let lease = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("acquire"),
    );
    let token = lease.fencing_token;

    assert!(pod_a.holds_current("pkg", token).await.expect("check"));
    assert!(
        !pod_a.holds_current("pkg", token + 1).await.expect("check"),
        "wrong token must be false"
    );
    assert!(
        !pod_b.holds_current("pkg", token).await.expect("check"),
        "wrong pod must be false"
    );
    assert!(
        !pod_a
            .holds_current("missing-pkg", token)
            .await
            .expect("check"),
        "missing document must be false"
    );

    force_expires_at(&db, "pkg", past()).await;
    assert!(
        !pod_a.holds_current("pkg", token).await.expect("check"),
        "expired lease must be false even for the right holder + token"
    );
}

/// AC12: the boundary instant `expires_at == now` is DEAD — acquirable by
/// `$lte`, not live for `holds_current`'s `$gt` (exact complements).
#[tokio::test]
async fn expiry_boundary_is_dead() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");
    let pod_b = store(&db, "pod-b");

    let lease = acquired(
        pod_a
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("acquire"),
    );

    // Pin expires_at to the current instant: from this millisecond on the
    // lease is dead (`expires_at <= now`), never live (`expires_at > now`).
    let boundary = bson::DateTime::now();
    force_expires_at(&db, "pkg", boundary).await;

    assert!(
        !pod_a
            .holds_current("pkg", lease.fencing_token)
            .await
            .expect("check"),
        "at the boundary the lease is not live"
    );

    let takeover = acquired(
        pod_b
            .acquire("pkg", bson::Uuid::new())
            .await
            .expect("takeover"),
    );
    assert_eq!(takeover.fencing_token, lease.fencing_token + 1);
    assert_eq!(takeover.holder_pod, "pod-b");
}

/// AC15/AC16 (CANON exactly-one): M >= 5 distinct-pod stores race concurrent
/// acquires. Per round exactly ONE wins and nobody surfaces an Err (the
/// E11000 insert race resolves to NotAcquired). The same package is raced
/// over several rounds — the token increases strictly (by exactly 1) per
/// round while the document lives — and the whole race is repeated on fresh
/// packages to re-exercise the first-insert E11000 path.
#[tokio::test]
async fn two_pods_exactly_one_wins() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;

    const PODS: usize = 6;
    const ROUNDS: i64 = 3;
    const PACKAGES: usize = 3;

    let stores: Vec<LeaseStore> = (0..PODS).map(|i| store(&db, &format!("pod-{i}"))).collect();

    for package_index in 0..PACKAGES {
        let package = format!("race-pkg-{package_index}");
        for round in 1..=ROUNDS {
            let mut join_set = JoinSet::new();
            for racer in &stores {
                let racer = racer.clone();
                let package = package.clone();
                join_set.spawn(async move { racer.acquire(&package, bson::Uuid::new()).await });
            }

            let mut winners = Vec::new();
            let mut losers = 0_usize;
            while let Some(joined) = join_set.join_next().await {
                let outcome = joined
                    .expect("task must not panic")
                    .expect("acquire must never surface an Err under contention");
                match outcome {
                    AcquireOutcome::Acquired(lease) => winners.push(lease),
                    AcquireOutcome::NotAcquired => losers += 1,
                }
            }

            assert_eq!(
                winners.len(),
                1,
                "{package} round {round}: exactly one Acquired, got {winners:?}"
            );
            assert_eq!(losers, PODS - 1);
            assert_eq!(
                winners[0].fencing_token, round,
                "{package} round {round}: token strictly increasing across rounds"
            );

            let stored = raw_lease(&db, &package).await.expect("doc present");
            assert_eq!(
                stored, winners[0],
                "stored doc matches the winner's post-image"
            );
        }
    }
}

/// AC2: LeaseStore::ensure_indexes is idempotent, creates the stable-named
/// holder_pod + expires_at secondaries with exact key specs, never a TTL
/// option, and coexists with the startup path's identical expires_at spec.
#[tokio::test]
async fn ensure_indexes_idempotent() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");

    // The wire-level names are pinned as string literals.
    assert_eq!(IDX_LEASES_HOLDER_POD, "leases_holder_pod");
    assert_eq!(IDX_LEASES_EXPIRES_AT, "leases_expires_at");

    pod_a.ensure_indexes().await.expect("first ensure_indexes");
    pod_a.ensure_indexes().await.expect("second ensure_indexes");

    // The startup path declares the identical {expires_at: 1} spec under the
    // same name, so running it before/after the store's ensure is a no-op.
    db.ensure_indexes()
        .await
        .expect("startup ensure_indexes coexists");

    let mut cursor = db.leases().list_indexes().await.expect("list_indexes");
    let mut specs = Vec::new();
    while cursor.advance().await.expect("cursor advance") {
        let model = cursor.deserialize_current().expect("index model");
        let options = model.options.expect("index options present");
        let name = options.name.expect("index name present");
        assert!(
            options.expire_after.is_none(),
            "{name} must not be a TTL index"
        );
        if let Some(unique) = options.unique {
            assert!(!unique, "unexpected unique index declared: {name}");
        }
        specs.push((name, model.keys));
    }
    specs.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(
        specs,
        vec![
            ("_id_".to_string(), doc! { "_id": 1 }),
            ("leases_expires_at".to_string(), doc! { "expires_at": 1 }),
            ("leases_holder_pod".to_string(), doc! { "holder_pod": 1 }),
        ],
        "exactly the implicit _id plus the two declared secondaries"
    );
}

/// Helper AC: reap_expired deletes exactly the dead leases, leaves live ones,
/// and is idempotent (second run deletes nothing).
#[tokio::test]
async fn reap_expired_counts() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pod_a = store(&db, "pod-a");

    acquired(
        pod_a
            .acquire("pkg-a", bson::Uuid::new())
            .await
            .expect("acquire a"),
    );
    acquired(
        pod_a
            .acquire("pkg-b", bson::Uuid::new())
            .await
            .expect("acquire b"),
    );
    let live = acquired(
        pod_a
            .acquire("pkg-c", bson::Uuid::new())
            .await
            .expect("acquire c"),
    );

    force_expires_at(&db, "pkg-a", past()).await;
    force_expires_at(&db, "pkg-b", past()).await;

    let reaped = pod_a.reap_expired().await.expect("reap");
    assert_eq!(reaped, 2, "exactly the two dead leases are reaped");
    assert!(raw_lease(&db, "pkg-a").await.is_none());
    assert!(raw_lease(&db, "pkg-b").await.is_none());
    assert_eq!(
        raw_lease(&db, "pkg-c").await.expect("live doc remains"),
        live
    );

    let reaped_again = pod_a.reap_expired().await.expect("second reap");
    assert_eq!(reaped_again, 0, "idempotent: nothing left to reap");
}
