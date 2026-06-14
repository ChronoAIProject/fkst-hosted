//! Journaling integration tests against an ephemeral Mongo container
//! (testcontainers) and a mocked GitHub Contents API (wiremock), driving the
//! REAL session service + driver state machine with a stub engine that emits
//! `RAISED:` lines on stdout.
//!
//! Every test self-skips when Docker is unavailable so `cargo test` stays
//! green on runners without a daemon.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bson::doc;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::journal::index::{
    ensure_journal_indexes, IDX_RJ_GITHUB_PATH, IDX_RJ_PACKAGE, IDX_SP_PACKAGE, IDX_SP_RECORDED_AT,
    IDX_SP_RUN_IDEM_UNIQ, IDX_SP_SESSION_SEQ,
};
use fkst_hosted_api::journal::model::{
    CompletedEntry, ProgressKind, ProgressRecord, RunJournalDoc, SessionProgressDoc,
    RUN_JOURNALS_COLLECTION, SESSION_PROGRESS_COLLECTION, UNVERIFIED_SHA,
};
use fkst_hosted_api::journal::store::MongoProgressStore;
use fkst_hosted_api::journal::{
    default_identity_pointers, idem_key, package_fingerprint, JournalConfig, Journaler,
    ProgressSignal, SessionCtx,
};
use fkst_hosted_api::models::{SessionDoc, SessionStatus};
use fkst_hosted_api::packages::{NewPackage, PackageFile, PackageRepository};
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use secrecy::SecretString;
use serde_json::json;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

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

/// Mongo image tag — pinned to the same major as `backend/docker-compose.yml`.
const MONGO_TAG: &str = "7";

/// Start an ephemeral Mongo and build a connected `Db` over it, with the
/// journal indexes ensured (the dedupe tests depend on `sp_run_idem_uniq`).
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
    ensure_journal_indexes(&db.database)
        .await
        .expect("journal indexes");
    (container, db)
}

/// Poll `predicate` every 100 ms for up to ~30 s (container tests).
async fn wait_until<F, Fut>(mut predicate: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..300 {
        if predicate().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

fn b64(payload: &str) -> String {
    STANDARD.encode(payload)
}

fn ctx(session_id: &str, token: i64, fingerprint: &str) -> SessionCtx {
    SessionCtx {
        session_id: session_id.to_string(),
        package_name: "demo".to_string(),
        package_fingerprint: fingerprint.to_string(),
        pod_id: Some("pod-a".to_string()),
        fencing_token: token,
    }
}

fn github_cfg(server_uri: &str) -> JournalConfig {
    JournalConfig {
        github_repo: Some("owner/name".to_string()),
        github_api_base: server_uri.to_string(),
        github_token: Some(SecretString::from("test-token".to_string())),
        ..JournalConfig::default()
    }
}

async fn progress_docs(db: &Db) -> Vec<SessionProgressDoc> {
    let mut cursor = db
        .database
        .collection::<SessionProgressDoc>(SESSION_PROGRESS_COLLECTION)
        .find(doc! {})
        .sort(doc! { "seq": 1 })
        .await
        .expect("find progress");
    let mut docs = Vec::new();
    while cursor.advance().await.expect("cursor advance") {
        docs.push(cursor.deserialize_current().expect("deserialize doc"));
    }
    docs
}

// ---------------------------------------------------------------------------
// Indexes + local idempotency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn journal_indexes_are_created_with_stable_names_and_are_idempotent() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    // Idempotent: a second run on an already-indexed DB succeeds.
    ensure_journal_indexes(&db.database)
        .await
        .expect("re-ensure must succeed");

    let progress_names: Vec<String> = db
        .database
        .collection::<SessionProgressDoc>(SESSION_PROGRESS_COLLECTION)
        .list_index_names()
        .await
        .expect("list session_progress indexes");
    for expected in [
        IDX_SP_SESSION_SEQ,
        IDX_SP_RUN_IDEM_UNIQ,
        IDX_SP_PACKAGE,
        IDX_SP_RECORDED_AT,
    ] {
        assert!(
            progress_names.iter().any(|name| name == expected),
            "missing index {expected}: {progress_names:?}"
        );
    }

    let journal_names: Vec<String> = db
        .database
        .collection::<RunJournalDoc>(RUN_JOURNALS_COLLECTION)
        .list_index_names()
        .await
        .expect("list run_journals indexes");
    for expected in [IDX_RJ_PACKAGE, IDX_RJ_GITHUB_PATH] {
        assert!(
            journal_names.iter().any(|name| name == expected),
            "missing index {expected}: {journal_names:?}"
        );
    }
}

#[tokio::test]
async fn duplicate_idem_key_is_a_benign_e11000_and_lifecycle_docs_are_exempt() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let store = MongoProgressStore::new(&db.database);
    let mut journaler = Journaler::start(
        ctx("11111111-1111-4111-8111-111111111111", 1, "fp"),
        JournalConfig {
            github_enabled: false,
            ..JournalConfig::default()
        },
        store,
    )
    .await
    .expect("start");

    let event = json!({"department":"d","source":"s","name":"e1","corr":"c"});
    journaler
        .record(ProgressSignal::Raised {
            event_json: event.clone(),
        })
        .await
        .expect("first insert");
    // The REAL unique partial index fires E11000; the journaler answers Ok.
    journaler
        .record(ProgressSignal::Raised { event_json: event })
        .await
        .expect("duplicate must be a benign no-op");

    // Lifecycle docs omit idem_key entirely and are EXEMPT from the unique
    // constraint: two identical transitions both insert.
    use fkst_hosted_api::journal::{LifecycleEvent, Transition};
    for _ in 0..2 {
        journaler
            .record(ProgressSignal::Lifecycle(LifecycleEvent::now(
                Transition::Running,
            )))
            .await
            .expect("lifecycle inserts are unconstrained");
    }

    let docs = progress_docs(&db).await;
    let raised = docs
        .iter()
        .filter(|d| d.kind == ProgressKind::Raised)
        .count();
    let lifecycle = docs
        .iter()
        .filter(|d| d.kind == ProgressKind::Lifecycle)
        .count();
    assert_eq!(raised, 1, "duplicate raised must not create a second doc");
    assert_eq!(lifecycle, 2, "lifecycle docs are not unique-constrained");
}

// ---------------------------------------------------------------------------
// Redo contract against real Mongo + mocked GitHub
// ---------------------------------------------------------------------------

#[tokio::test]
async fn redo_loads_the_skip_set_mirrors_it_and_reemission_creates_zero_docs() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let pointers = default_identity_pointers();
    let events: Vec<serde_json::Value> = (0..3)
        .map(|i| json!({"department":"d","source":"s","name":format!("e{i}"),"corr":"c"}))
        .collect();

    // "Session A on another pod" already pushed this record to GitHub.
    let mut remote = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
    remote.completed = events
        .iter()
        .map(|event| CompletedEntry {
            idem_key: idem_key("demo", event, &pointers),
            event: event.clone(),
            at: "2026-06-10T00:00:00Z".to_string(),
        })
        .collect();
    remote.max_fencing_token = 1;
    let server = MockServer::start().await;
    let body = json!({
        "content": STANDARD.encode(serde_json::to_vec(&remote).expect("json")),
        "sha": "sha-remote",
        "encoding": "base64"
    });
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    // Session B: the redo on this pod, holding a HIGHER fencing token.
    let store = MongoProgressStore::new(&db.database);
    let mut journaler = Journaler::start(
        ctx("22222222-2222-4222-8222-222222222222", 2, "fp"),
        github_cfg(&server.uri()),
        store,
    )
    .await
    .expect("start");
    let skip = journaler.load_skip_set().await.expect("bootstrap");
    assert_eq!(skip.len(), 3);
    for event in &events {
        assert!(skip.contains(&idem_key("demo", event, &pointers)));
    }
    assert_eq!(
        progress_docs(&db).await.len(),
        3,
        "remote completed[] mirrored into local session_progress"
    );

    // Re-emitting every mirrored event produces ZERO new documents.
    for event in events {
        journaler
            .record(ProgressSignal::Raised { event_json: event })
            .await
            .expect("re-emit");
    }
    assert_eq!(progress_docs(&db).await.len(), 3, "idempotent redo");
    assert_eq!(journaler.buffered(), 0, "nothing newly completed to flush");
}

#[tokio::test]
async fn unreachable_github_fails_open_and_mongo_journaling_continues() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let store = MongoProgressStore::new(&db.database);
    let mut journaler = Journaler::start(
        ctx("33333333-3333-4333-8333-333333333333", 1, "fp"),
        github_cfg("http://127.0.0.1:1"),
        store,
    )
    .await
    .expect("start");

    let skip = journaler.load_skip_set().await.expect("fail-open");
    assert!(skip.is_empty(), "unreachable github => EMPTY skip-set");

    // The head carries the "unverified" sentinel until a successful flush.
    let head: RunJournalDoc = db
        .database
        .collection::<RunJournalDoc>(RUN_JOURNALS_COLLECTION)
        .find_one(doc! { "_id": journaler.run_key() })
        .await
        .expect("head read")
        .expect("head present");
    assert_eq!(head.github.last_commit_sha.as_deref(), Some(UNVERIFIED_SHA));

    // Mongo journaling is unaffected (the durable floor).
    journaler
        .record(ProgressSignal::Raised {
            event_json: json!({"department":"d","source":"s","name":"e","corr":"c"}),
        })
        .await
        .expect("record");
    assert_eq!(progress_docs(&db).await.len(), 1);
}

// ---------------------------------------------------------------------------
// End-to-end: the real driver journals a stub engine's RAISED stdout
// ---------------------------------------------------------------------------

/// Write the stub engine script: conformance passes; supervise emits RAISED
/// traffic on STDOUT and the ready markers on STDERR, then idles.
fn write_stub(dir: &Path) -> PathBuf {
    let e1 = b64(r#"{"department":"hello","source":"raiser","name":"e1","corr":"c-1"}"#);
    let e2 = b64(r#"{"department":"hello","source":"raiser","name":"e2","corr":"c-1"}"#);
    let path = dir.join("stub-framework.sh");
    let script = format!(
        r#"#!/bin/sh
case "$1" in
  conformance)
    echo "PASS graph-scan loaded 1 departments, 1 raisers, 1 queues"
    exit 0
    ;;
  supervise)
    echo "RAISED: {e1}"
    echo "RAISED: {e1}"
    echo "RAISED: {e2}"
    echo "RAISED: !!!not-base64!!!"
    echo "plain engine chatter"
    echo "event runtime running handles=3" >&2
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
    sleep 300
    ;;
esac
"#
    );
    fs::write(&path, script).expect("write stub");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
    path
}

#[tokio::test]
async fn driver_journals_raised_lines_lifecycle_and_run_key_end_to_end() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let engine = EngineConfig {
        framework_bin: write_stub(stub_dir.path()),
        temp_root: temp_root.path().to_path_buf(),
        stop_grace_secs: 5,
        conformance_timeout_secs: 10,
        ready_timeout_secs: 10,
        ..EngineConfig::default()
    };

    let packages = PackageRepository::new(&db.database);
    packages.ensure_indexes().await.expect("package indexes");
    let package = packages
        .create(
            NewPackage {
                name: "demo".to_string(),
                files: vec![PackageFile {
                    path: "departments/hello/main.lua".to_string(),
                    content: "return {}".to_string(),
                }],
                composed_deps: vec![],
            },
            "test-user",
            None,
        )
        .await
        .expect("create package");
    let expected_run_key = fkst_hosted_api::journal::run_key(
        "demo",
        &package_fingerprint(&package.files, &package.composed_deps),
    );

    // Single-pod service with Mongo-only journaling (GitHub disabled): the
    // driver path is identical; only the flush sink differs.
    let sessions = SessionService::new(SessionRepo::new(&db), packages.clone(), engine);
    sessions.enable_journaling(
        JournalConfig {
            github_enabled: false,
            ..JournalConfig::default()
        },
        MongoProgressStore::new(&db.database),
    );

    let created = sessions
        .create(
            "demo",
            fkst_hosted_api::sessions::SessionOwner {
                owner_user_id: "test-user".to_string(),
                org_id: None,
            },
        )
        .await
        .expect("create session");
    let id = created.id;

    // The driver advances to running and the journal fills up.
    assert!(
        wait_until(|| async {
            matches!(
                sessions.get(id).await.expect("get"),
                Some(SessionDoc {
                    status: SessionStatus::Running,
                    ..
                })
            )
        })
        .await,
        "session must reach running"
    );
    assert!(
        wait_until(|| async {
            let docs = progress_docs(&db).await;
            let raised = docs
                .iter()
                .filter(|d| d.kind == ProgressKind::Raised)
                .count();
            let malformed = docs.iter().any(|d| {
                d.lifecycle
                    .as_ref()
                    .map(|l| l.transition == "malformed_raised")
                    .unwrap_or(false)
            });
            raised == 2 && malformed
        })
        .await,
        "exactly 2 raised docs (duplicate deduped) + the malformed anomaly"
    );

    // The run_key is stamped on the session document (narrow write).
    let session = sessions.get(id).await.expect("get").expect("present");
    assert_eq!(session.run_key.as_deref(), Some(expected_run_key.as_str()));
    assert_eq!(session.status, SessionStatus::Running);

    // Stop and verify the terminal journal.
    sessions.request_stop(id).await.expect("stop");
    assert!(
        wait_until(|| async {
            matches!(
                sessions.get(id).await.expect("get"),
                Some(SessionDoc {
                    status: SessionStatus::Stopped,
                    ..
                })
            )
        })
        .await,
        "session must stop"
    );
    assert!(
        wait_until(|| async {
            progress_docs(&db).await.iter().any(|d| {
                d.lifecycle
                    .as_ref()
                    .map(|l| l.transition == "stopped")
                    .unwrap_or(false)
            })
        })
        .await,
        "terminal lifecycle journaled"
    );

    let docs = progress_docs(&db).await;
    // seq is a per-session monotonic total order across BOTH kinds.
    let seqs: Vec<i64> = docs.iter().map(|d| d.seq).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(seqs.len(), sorted.len(), "seq must be unique per session");

    for doc in &docs {
        assert_eq!(doc.session_id, id.to_string());
        assert_eq!(doc.package_name, "demo");
        assert_eq!(doc.run_key, expected_run_key);
        match doc.kind {
            ProgressKind::Raised => {
                let key = doc.idem_key.as_deref().expect("raised carries idem_key");
                assert_eq!(key.len(), 64);
                assert!(key
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
                assert!(doc.event_json.is_some(), "verbatim payload stored");
            }
            ProgressKind::Lifecycle => {
                assert!(doc.idem_key.is_none(), "lifecycle omits idem_key");
                assert!(doc.lifecycle.is_some());
            }
        }
    }
    let transitions: Vec<&str> = docs
        .iter()
        .filter_map(|d| d.lifecycle.as_ref())
        .map(|l| l.transition.as_str())
        .collect();
    for expected in ["validating", "spawned", "running", "stopping", "stopped"] {
        assert!(
            transitions.contains(&expected),
            "missing lifecycle {expected}: {transitions:?}"
        );
    }

    // The run head exists with the GitHub sync disabled posture.
    let head: RunJournalDoc = db
        .database
        .collection::<RunJournalDoc>(RUN_JOURNALS_COLLECTION)
        .find_one(doc! { "_id": &expected_run_key })
        .await
        .expect("head read")
        .expect("head present");
    assert_eq!(head.package_name, "demo");
    assert!(head.github.repo.is_none(), "github disabled => no repo");
}

// ---------------------------------------------------------------------------
// Driver-level invariant: a failing journal sink NEVER fails/alters a session
// ---------------------------------------------------------------------------

/// Stub engine whose `conformance` exits non-zero (a 400-class package error)
/// so the driver fails the session BEFORE the engine ever runs.
fn write_conformance_failing_stub(dir: &Path) -> PathBuf {
    let path = dir.join("stub-conf-fail.sh");
    let script = r#"#!/bin/sh
case "$1" in
  conformance)
    echo "FAIL graph-scan department broken missing M.spec" >&2
    exit 1
    ;;
  supervise)
    echo "supervise must not run after a conformance failure" >&2
    exit 97
    ;;
esac
"#;
    fs::write(&path, script).expect("write stub");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
    path
}

/// A journaling config whose GitHub sink is ENABLED but always fails: every
/// Contents PUT is rejected so each lifecycle/terminal flush takes an error
/// path. `api_base` points at the caller's wiremock server.
fn always_failing_github_cfg(api_base: &str) -> JournalConfig {
    JournalConfig {
        github_enabled: true,
        github_repo: Some("owner/name".to_string()),
        github_api_base: api_base.to_string(),
        github_token: Some(SecretString::from("test-token".to_string())),
        // Keep the CAS budget tiny so a forced flush returns quickly rather
        // than retrying for a long time on each lifecycle transition.
        cas_max_retries: 1,
        ..JournalConfig::default()
    }
}

/// Build the materialize-able package + a single-pod service whose journaling
/// is enabled against `github` (a failing GitHub sink) and a REAL Mongo store.
async fn service_with_failing_github(
    db: &Db,
    engine: EngineConfig,
    github_api_base: &str,
) -> (SessionService, PackageRepository) {
    let packages = PackageRepository::new(&db.database);
    packages.ensure_indexes().await.expect("package indexes");
    packages
        .create(
            NewPackage {
                name: "demo".to_string(),
                files: vec![PackageFile {
                    path: "departments/hello/main.lua".to_string(),
                    content: "return {}".to_string(),
                }],
                composed_deps: vec![],
            },
            "test-user",
            None,
        )
        .await
        .expect("create package");

    let sessions = SessionService::new(SessionRepo::new(db), packages.clone(), engine);
    sessions.enable_journaling(
        always_failing_github_cfg(github_api_base),
        MongoProgressStore::new(&db.database),
    );
    (sessions, packages)
}

#[tokio::test]
async fn failing_journal_sink_never_blocks_a_normal_stop() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;

    // Every GitHub call fails: GET 500, PUT 401 — so each lifecycle flush and
    // the terminal flush traverse an error path the WHOLE session long.
    let github = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&github)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&github)
        .await;

    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let engine = EngineConfig {
        framework_bin: write_stub(stub_dir.path()),
        temp_root: temp_root.path().to_path_buf(),
        stop_grace_secs: 5,
        conformance_timeout_secs: 10,
        ready_timeout_secs: 10,
        ..EngineConfig::default()
    };
    let (sessions, _packages) = service_with_failing_github(&db, engine, &github.uri()).await;

    let created = sessions
        .create(
            "demo",
            fkst_hosted_api::sessions::SessionOwner {
                owner_user_id: "test-user".to_string(),
                org_id: None,
            },
        )
        .await
        .expect("create session");
    let id = created.id;

    // Despite the failing journal sink, the session reaches `running`...
    assert!(
        wait_until(|| async {
            matches!(
                sessions.get(id).await.expect("get"),
                Some(SessionDoc {
                    status: SessionStatus::Running,
                    ..
                })
            )
        })
        .await,
        "a failing journal sink must not block the session reaching running"
    );

    // ...and a normal stop converges to `stopped` with the error field UNSET.
    sessions.request_stop(id).await.expect("stop");
    assert!(
        wait_until(|| async {
            matches!(
                sessions.get(id).await.expect("get"),
                Some(SessionDoc {
                    status: SessionStatus::Stopped,
                    ..
                })
            )
        })
        .await,
        "a failing journal sink must not block a normal stop"
    );

    let session = sessions.get(id).await.expect("get").expect("present");
    assert_eq!(session.status, SessionStatus::Stopped);
    assert!(
        session.error.is_none(),
        "journal failure must not write the session error field: {:?}",
        session.error
    );

    // The Mongo floor still journaled the lifecycle CAS transitions (the
    // session state machine ran to completion regardless of the GitHub sink).
    let docs = progress_docs(&db).await;
    let transitions: Vec<&str> = docs
        .iter()
        .filter_map(|d| d.lifecycle.as_ref())
        .map(|l| l.transition.as_str())
        .collect();
    for expected in ["validating", "running", "stopping", "stopped"] {
        assert!(
            transitions.contains(&expected),
            "lifecycle CAS transition {expected} missing despite the failing sink: {transitions:?}"
        );
    }
}

#[tokio::test]
async fn failing_journal_sink_preserves_a_conformance_failure_disposition() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let (_container, db) = mongo_db().await;

    // The journal sink fails on every call, as above.
    let github = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&github)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&github)
        .await;

    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let engine = EngineConfig {
        framework_bin: write_conformance_failing_stub(stub_dir.path()),
        temp_root: temp_root.path().to_path_buf(),
        stop_grace_secs: 5,
        conformance_timeout_secs: 10,
        ready_timeout_secs: 10,
        ..EngineConfig::default()
    };
    let (sessions, _packages) = service_with_failing_github(&db, engine, &github.uri()).await;

    let created = sessions
        .create(
            "demo",
            fkst_hosted_api::sessions::SessionOwner {
                owner_user_id: "test-user".to_string(),
                org_id: None,
            },
        )
        .await
        .expect("create session");
    let id = created.id;

    // The conformance failure (not the journal failure) decides disposition:
    // the session reaches `failed` carrying the ENGINE's conformance error.
    assert!(
        wait_until(|| async {
            matches!(
                sessions.get(id).await.expect("get"),
                Some(SessionDoc {
                    status: SessionStatus::Failed,
                    ..
                })
            )
        })
        .await,
        "a conformance failure must still fail the session despite the failing sink"
    );

    let session = sessions.get(id).await.expect("get").expect("present");
    assert_eq!(session.status, SessionStatus::Failed);
    let error = session.error.expect("failed session carries an error");
    assert!(
        error.contains("conformance failed"),
        "the session error must be the ENGINE conformance failure, not a journal error: {error}"
    );
    // The journal sink's own failures (auth/network) must never appear in the
    // client-served error field.
    for journal_leak in ["github", "journal", "401", "Bearer"] {
        assert!(
            !error.contains(journal_leak),
            "journal-sink failure leaked into the session error ({journal_leak:?}): {error}"
        );
    }
}
