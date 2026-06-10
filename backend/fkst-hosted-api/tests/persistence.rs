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
use fkst_hosted_api::models::{PackageDoc, PackageFile, SessionDoc, SessionStatus};
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::state::AppState;
use http_body_util::BodyExt;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
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

/// Start an ephemeral Mongo and build a connected `Db` over it.
async fn mongo_db(selection_timeout_ms: u64) -> (ContainerAsync<Mongo>, Config, Db) {
    let container = Mongo::default().start().await.expect("start mongo");
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

/// Collect the index names of a collection via the driver's own cursor API.
async fn index_names<T: Send + Sync>(coll: &mongodb::Collection<T>) -> Vec<String> {
    let mut cursor = coll.list_indexes().await.expect("list_indexes");
    let mut names = Vec::new();
    while cursor.advance().await.expect("cursor advance") {
        let model = cursor.deserialize_current().expect("index model");
        let options = model.options.expect("index options present");
        names.push(options.name.expect("index name present"));
        // No extra unique index beyond the implicit `_id`.
        if let Some(unique) = options.unique {
            assert!(!unique, "unexpected unique index declared");
        }
    }
    names.sort();
    names
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
        created_at: bson::DateTime::from_millis(1_700_000_000_000),
        started_at: Some(bson::DateTime::from_millis(1_700_000_000_500)),
        stopped_at: None,
    }
}

fn sample_package() -> PackageDoc {
    PackageDoc {
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

    db.ensure_indexes().await.expect("first ensure_indexes");

    let mut expected_sessions = vec![
        "_id_".to_string(),
        IDX_SESSIONS_PACKAGE_NAME.to_string(),
        IDX_SESSIONS_STATUS.to_string(),
        IDX_SESSIONS_POD_ID.to_string(),
    ];
    expected_sessions.sort();
    let mut expected_leases = vec!["_id_".to_string(), IDX_LEASES_EXPIRES_AT.to_string()];
    expected_leases.sort();

    assert_eq!(index_names(&db.sessions()).await, expected_sessions);
    assert_eq!(index_names(&db.leases()).await, expected_leases);
    // No secondary index is ever declared for packages; the collection may
    // not even exist yet (older servers answer NamespaceNotFound), so at
    // most the implicit `_id` index is present.
    match db.packages().list_indexes().await {
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

    // Second run: Ok, identical set (idempotency, no duplicates).
    db.ensure_indexes().await.expect("second ensure_indexes");
    assert_eq!(index_names(&db.sessions()).await, expected_sessions);
    assert_eq!(index_names(&db.leases()).await, expected_leases);

    // Concurrent runs (two pods racing startup): both Ok, same set.
    let (left, right) = tokio::join!(db.ensure_indexes(), db.ensure_indexes());
    left.expect("concurrent ensure_indexes (left)");
    right.expect("concurrent ensure_indexes (right)");
    assert_eq!(index_names(&db.sessions()).await, expected_sessions);
    assert_eq!(index_names(&db.leases()).await, expected_leases);
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

    db.packages()
        .insert_one(package.clone())
        .await
        .expect("insert package");

    let found = db
        .packages()
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
    let router = build_router(AppState { config, db });

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
