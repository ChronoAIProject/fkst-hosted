//! PackageRepository integration tests against an ephemeral Mongo container
//! (testcontainers). Every test gets a fresh container and self-skips when
//! Docker is unavailable so `cargo test` stays green on runners without a
//! Docker daemon.

use bson::{doc, Bson};
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::packages::{
    NewPackage, PackageError, PackageFile, PackageRepository, PACKAGES_COLLECTION,
};
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

/// Mongo image tag — pinned to the same major as `backend/docker-compose.yml`
/// so integration tests and local dev exercise the same server line.
const MONGO_TAG: &str = "7";

/// Start an ephemeral Mongo and build a `PackageRepository` over it.
async fn repo() -> (ContainerAsync<Mongo>, mongodb::Database, PackageRepository) {
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
    let repository = PackageRepository::new(&db.database);
    (container, db.database, repository)
}

fn file(path: &str, content: &str) -> PackageFile {
    PackageFile {
        path: path.to_string(),
        content: content.to_string(),
    }
}

/// A valid creation input: department + raiser entries, a `core.lua`, a
/// dotted-segment lib file, multibyte UTF-8 content, and two deps.
fn sample_new_package(name: &str) -> NewPackage {
    NewPackage {
        name: name.to_string(),
        files: vec![
            file("departments/router/main.lua", "-- héllo 你好 🚀\nreturn {}"),
            file("raisers/cron.lua", "return { --[[ sources ]] }"),
            file("core.lua", "-- shared helpers"),
            // Dots inside a segment are path-safe (only "." / ".." exact
            // segments are rejected).
            file("lib/util.v2.lua", "-- dotted segment"),
        ],
        composed_deps: vec!["base".to_string(), "extra-dep".to_string()],
    }
}

#[tokio::test]
async fn ensure_indexes_is_idempotent_and_adds_nothing_beyond_implicit_id() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, database, repository) = repo().await;

    // Callable repeatedly, including before the collection exists.
    repository.ensure_indexes().await.expect("first ensure");
    repository.ensure_indexes().await.expect("second ensure");

    // Materialize the collection, ensure again, then prove only `_id_`.
    repository
        .create(sample_new_package("demo-package"))
        .await
        .expect("create");
    repository.ensure_indexes().await.expect("third ensure");

    let mut cursor = database
        .collection::<bson::Document>(PACKAGES_COLLECTION)
        .list_indexes()
        .await
        .expect("list_indexes");
    let mut names = Vec::new();
    while cursor.advance().await.expect("cursor advance") {
        let model = cursor.deserialize_current().expect("index model");
        names.push(model.options.and_then(|o| o.name).expect("index name"));
    }
    assert_eq!(
        names,
        vec!["_id_".to_string()],
        "only the implicit _id index"
    );
}

#[tokio::test]
async fn create_then_get_round_trips_deeply() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _database, repository) = repo().await;
    let input = sample_new_package("demo-package");

    let created = repository.create(input.clone()).await.expect("create");
    assert_eq!(created.name, input.name);
    assert_eq!(created.files, input.files);
    assert_eq!(created.composed_deps, input.composed_deps);
    assert_eq!(
        created.created_at, created.updated_at,
        "both timestamps from a single clock read"
    );

    let found = repository
        .get("demo-package")
        .await
        .expect("get")
        .expect("package must exist");
    // Deep equality: files (order + byte-for-byte content, multibyte UTF-8
    // included), composed_deps, and timestamps.
    assert_eq!(found, created);
    assert_eq!(
        found.files[0].content, "-- héllo 你好 🚀\nreturn {}",
        "multibyte content byte-identical"
    );
}

#[tokio::test]
async fn stored_bson_shape_matches_canon() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, database, repository) = repo().await;

    // Empty composed_deps on purpose: must still be stored as an array.
    let input = NewPackage {
        name: "shape-pkg".to_string(),
        files: vec![
            file("departments/router/main.lua", "return {}"),
            file("core.lua", "-- core"),
        ],
        composed_deps: Vec::new(),
    };
    repository.create(input).await.expect("create");

    let raw = database
        .collection::<bson::Document>(PACKAGES_COLLECTION)
        .find_one(doc! { "_id": "shape-pkg" })
        .await
        .expect("find_one")
        .expect("raw document present");

    assert_eq!(
        raw.get("_id").expect("_id"),
        &Bson::String("shape-pkg".to_string())
    );
    match raw.get("files").expect("files present") {
        Bson::Array(items) => {
            assert_eq!(items.len(), 2);
            for item in items {
                let sub = item.as_document().expect("files entries are subdocs");
                assert!(matches!(sub.get("path"), Some(Bson::String(_))));
                assert!(matches!(sub.get("content"), Some(Bson::String(_))));
            }
        }
        other => panic!("expected files as Bson::Array, got {other:?}"),
    }
    assert_eq!(
        raw.get("composed_deps").expect("composed_deps present"),
        &Bson::Array(Vec::new()),
        "empty composed_deps stored as an array, not null/absent"
    );
    assert!(matches!(raw.get("created_at"), Some(Bson::DateTime(_))));
    assert!(matches!(raw.get("updated_at"), Some(Bson::DateTime(_))));
}

#[tokio::test]
async fn duplicate_create_returns_duplicate_not_db() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _database, repository) = repo().await;

    repository
        .create(sample_new_package("dup-pkg"))
        .await
        .expect("first create");
    // Exercises the real is_duplicate_key path against a real server 11000.
    let err = repository
        .create(sample_new_package("dup-pkg"))
        .await
        .expect_err("second create must fail");
    match err {
        PackageError::Duplicate(name) => assert_eq!(name, "dup-pkg"),
        other => panic!("expected PackageError::Duplicate, got {other:?}"),
    }
}

#[tokio::test]
async fn concurrent_creates_of_the_same_name_yield_one_ok_one_duplicate() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _database, repository) = repo().await;

    let (left, right) = tokio::join!(
        repository.create(sample_new_package("race-pkg")),
        repository.create(sample_new_package("race-pkg")),
    );
    let outcomes = [left, right];
    let oks = outcomes.iter().filter(|r| r.is_ok()).count();
    let duplicates = outcomes
        .iter()
        .filter(|r| matches!(r, Err(PackageError::Duplicate(name)) if name == "race-pkg"))
        .count();
    assert_eq!(
        (oks, duplicates),
        (1, 1),
        "exactly one create wins, the other observes Duplicate: {outcomes:?}"
    );
}

#[tokio::test]
async fn list_returns_only_names_sorted_ascending() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _database, repository) = repo().await;

    // Insert out of order to prove the sort.
    repository
        .create(sample_new_package("beta-pkg"))
        .await
        .expect("create beta");
    repository
        .create(sample_new_package("alpha-pkg"))
        .await
        .expect("create alpha");

    let names = repository.list().await.expect("list");
    assert_eq!(names, vec!["alpha-pkg".to_string(), "beta-pkg".to_string()]);
}

#[tokio::test]
async fn get_missing_is_none_and_exists_reports_presence() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _database, repository) = repo().await;

    assert_eq!(repository.get("missing").await.expect("get"), None);
    assert!(!repository.exists("missing").await.expect("exists"));

    repository
        .create(sample_new_package("present-pkg"))
        .await
        .expect("create");
    assert!(repository.exists("present-pkg").await.expect("exists"));
    assert!(!repository.exists("missing").await.expect("exists"));
}

#[tokio::test]
async fn invalid_input_is_rejected_before_any_write() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let (_container, _database, repository) = repo().await;

    let input = NewPackage {
        name: "bad-pkg".to_string(),
        files: vec![file("../escape.lua", "x")],
        composed_deps: Vec::new(),
    };
    let err = repository
        .create(input)
        .await
        .expect_err("traversal path must be rejected");
    match err {
        PackageError::Validation(reason) => {
            assert!(reason.starts_with("unsafe path component"), "got: {reason}");
        }
        other => panic!("expected PackageError::Validation, got {other:?}"),
    }

    // Validation failures must never reach Mongo.
    assert_eq!(repository.list().await.expect("list"), Vec::<String>::new());
}
