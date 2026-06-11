//! Mongo integration tests against an ephemeral container (testcontainers).
//!
//! Every test self-skips when Docker is unavailable so `cargo test` stays
//! green on runners without a Docker daemon.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bson::doc;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::{
    Db, IDX_LEASES_EXPIRES_AT, IDX_SESSIONS_PACKAGE_NAME, IDX_SESSIONS_POD_ID, IDX_SESSIONS_STATUS,
};
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::leases::{LeaseStore, PoolConfig, IDX_LEASES_HOLDER_POD};
use fkst_hosted_api::models::{SessionDoc, SessionStatus};
use fkst_hosted_api::packages::{Package, PackageFile, PackageRepository, PACKAGES_COLLECTION};
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use fkst_hosted_api::state::AppState;
use http_body_util::BodyExt;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;
use tower::ServiceExt;

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

/// Start an ephemeral Mongo and build a connected `Db` over it.
async fn mongo_db(selection_timeout_ms: u64) -> (ContainerAsync<Mongo>, Config, Db) {
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
        mongodb_server_selection_timeout_ms: selection_timeout_ms,
        ..Config::default()
    };
    let db = Db::connect(&config).await.expect("connect + ping");
    (container, config, db)
}

/// Collect `(name, key document)` pairs of a collection's indexes via the
/// driver's own cursor API, sorted by name.
async fn index_specs<T: Send + Sync>(
    coll: &mongodb::Collection<T>,
) -> Vec<(String, bson::Document)> {
    let mut cursor = coll.list_indexes().await.expect("list_indexes");
    let mut specs = Vec::new();
    while cursor.advance().await.expect("cursor advance") {
        let model = cursor.deserialize_current().expect("index model");
        let options = model.options.expect("index options present");
        let name = options.name.expect("index name present");
        // No extra unique index beyond the implicit `_id`.
        if let Some(unique) = options.unique {
            assert!(!unique, "unexpected unique index declared: {name}");
        }
        specs.push((name, model.keys));
    }
    specs.sort_by(|a, b| a.0.cmp(&b.0));
    specs
}

fn sample_session() -> SessionDoc {
    SessionDoc {
        id: bson::Uuid::new(),
        package_name: "demo-package".to_string(),
        status: SessionStatus::Running,
        pod_id: Some("pod-0".to_string()),
        fencing_token: Some(42),
        pid: Some(4242),
        runtime_dir: Some("/tmp/run".to_string()),
        error: None,
        run_key: None,
        created_at: bson::DateTime::from_millis(1_700_000_000_000),
        started_at: Some(bson::DateTime::from_millis(1_700_000_000_500)),
        stopped_at: None,
    }
}

fn sample_package() -> Package {
    Package {
        name: "demo-package".to_string(),
        files: vec![
            PackageFile {
                path: "init.lua".to_string(),
                content: "return {}".to_string(),
            },
            PackageFile {
                path: "lib/util.lua".to_string(),
                content: "-- util".to_string(),
            },
        ],
        composed_deps: vec!["base".to_string()],
        created_at: bson::DateTime::from_millis(1_700_000_000_000),
        updated_at: bson::DateTime::from_millis(1_700_000_001_000),
    }
}

#[tokio::test]
async fn connect_and_ping_succeed_against_a_real_mongo() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _config, db) = mongo_db(5000).await;
    db.ping().await.expect("ping must succeed");
}

#[tokio::test]
async fn ensure_indexes_creates_exact_stable_names_and_is_idempotent() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _config, db) = mongo_db(5000).await;

    // Mirror the startup path exactly: the base ensure plus the lease
    // store's own ensure (main.rs runs both before binding).
    let lease_store = LeaseStore::new(
        &db,
        &PoolConfig {
            pod_id: "pod-test".to_string(),
            lease_ttl: Duration::from_secs(30),
        },
    );
    db.ensure_indexes().await.expect("first ensure_indexes");
    lease_store
        .ensure_indexes()
        .await
        .expect("first lease ensure_indexes");

    // The wire-level names are asserted as STRING LITERALS (not only via the
    // IDX_* constants) so a constant rename or a key swap fails this test;
    // these asserts pin the constants to the literals.
    assert_eq!(IDX_SESSIONS_PACKAGE_NAME, "sessions_package_name");
    assert_eq!(IDX_SESSIONS_STATUS, "sessions_status");
    assert_eq!(IDX_SESSIONS_POD_ID, "sessions_pod_id");
    assert_eq!(IDX_LEASES_EXPIRES_AT, "leases_expires_at");
    assert_eq!(IDX_LEASES_HOLDER_POD, "leases_holder_pod");

    // EXACTLY the implicit `_id` plus the declared secondaries, with their
    // exact key documents (sorted by name).
    let expected_sessions = vec![
        ("_id_".to_string(), doc! { "_id": 1 }),
        (
            "sessions_package_name".to_string(),
            doc! { "package_name": 1 },
        ),
        ("sessions_pod_id".to_string(), doc! { "pod_id": 1 }),
        ("sessions_status".to_string(), doc! { "status": 1 }),
    ];
    let expected_leases = vec![
        ("_id_".to_string(), doc! { "_id": 1 }),
        ("leases_expires_at".to_string(), doc! { "expires_at": 1 }),
        ("leases_holder_pod".to_string(), doc! { "holder_pod": 1 }),
    ];

    assert_eq!(index_specs(&db.sessions()).await, expected_sessions);
    assert_eq!(index_specs(&db.leases()).await, expected_leases);
    // No secondary index is ever declared for packages and nothing has been
    // inserted, so the collection does not exist; mongo:7 answers
    // NamespaceNotFound for list_indexes on a missing namespace (verified
    // empirically). Stay tolerant of an existing-but-empty collection too.
    match db
        .collection::<Package>(PACKAGES_COLLECTION)
        .list_indexes()
        .await
    {
        Ok(mut cursor) => {
            let mut package_names = Vec::new();
            while cursor.advance().await.expect("cursor advance") {
                let model = cursor.deserialize_current().expect("index model");
                package_names.push(model.options.and_then(|o| o.name).expect("index name"));
            }
            assert!(
                package_names.is_empty() || package_names == vec!["_id_".to_string()],
                "unexpected packages indexes: {package_names:?}"
            );
        }
        Err(error) => {
            let rendered = error.to_string();
            assert!(
                rendered.contains("NamespaceNotFound") || rendered.contains("ns does not exist"),
                "unexpected list_indexes error for packages: {rendered}"
            );
        }
    }

    // Second run: Ok, identical specs (idempotency, no duplicates).
    db.ensure_indexes().await.expect("second ensure_indexes");
    lease_store
        .ensure_indexes()
        .await
        .expect("second lease ensure_indexes");
    assert_eq!(index_specs(&db.sessions()).await, expected_sessions);
    assert_eq!(index_specs(&db.leases()).await, expected_leases);

    // Concurrent runs (two pods racing startup): both Ok, same specs.
    let (left, right) = tokio::join!(db.ensure_indexes(), db.ensure_indexes());
    left.expect("concurrent ensure_indexes (left)");
    right.expect("concurrent ensure_indexes (right)");
    assert_eq!(index_specs(&db.sessions()).await, expected_sessions);
    assert_eq!(index_specs(&db.leases()).await, expected_leases);
}

#[tokio::test]
async fn session_doc_round_trips_by_uuid_id() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _config, db) = mongo_db(5000).await;
    let session = sample_session();

    db.sessions()
        .insert_one(session.clone())
        .await
        .expect("insert session");

    // Binary subtype-4 regression guard: a string `_id` would never match.
    let found = db
        .sessions()
        .find_one(doc! { "_id": session.id })
        .await
        .expect("find_one")
        .expect("session must be found by bson::Uuid _id");
    assert_eq!(found, session, "full field equality after round-trip");
}

#[tokio::test]
async fn package_doc_round_trips_with_id_as_name() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _config, db) = mongo_db(5000).await;
    let package = sample_package();

    db.collection::<Package>(PACKAGES_COLLECTION)
        .insert_one(package.clone())
        .await
        .expect("insert package");

    let found = db
        .collection::<Package>(PACKAGES_COLLECTION)
        .find_one(doc! { "_id": &package.name })
        .await
        .expect("find_one")
        .expect("package must be found by name _id");
    assert_eq!(found, package, "_id mapping and files array intact");
    assert_eq!(found.files.len(), 2);
}

#[tokio::test]
async fn health_endpoints_reflect_mongo_liveness() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    // Short selection timeout so the post-stop request fails fast.
    let (container, config, db) = mongo_db(500).await;
    let packages = PackageRepository::new(&db.database);
    let sessions = SessionService::new(
        SessionRepo::new(&db),
        packages.clone(),
        EngineConfig::default(),
    );
    let router = build_router(AppState {
        config,
        db,
        packages,
        sessions,
    });

    // Mongo up: exact 200 body.
    let response = router
        .clone()
        .oneshot(
            Request::get("/api/v1/health")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let expected_up = format!(
        r#"{{"status":"ok","mongo":"up","version":"{}"}}"#,
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(std::str::from_utf8(&body).unwrap(), expected_up);

    // Stop Mongo: /health must degrade within a bounded time (no hang).
    container.stop().await.expect("stop mongo container");
    let response = tokio::time::timeout(
        Duration::from_secs(8),
        router.oneshot(
            Request::get("/health")
                .body(Body::empty())
                .expect("request builds"),
        ),
    )
    .await
    .expect("degraded health must answer within 8s, not hang")
    .expect("router must respond");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let expected_down = format!(
        r#"{{"status":"degraded","mongo":"down","version":"{}"}}"#,
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(std::str::from_utf8(&body).unwrap(), expected_down);
}
