//! Journaling integration tests against an ephemeral Mongo container
//! (testcontainers) and a mocked GitHub Contents API (wiremock), driving the
//! REAL session service + driver state machine with a stub engine that emits
//! `RAISED:` lines on stdout.
//!
//! Every test self-skips when Docker is unavailable so `cargo test` stays
//! green on runners without a daemon.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bson::doc;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
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
    default_identity_pointers, idem_key, JournalConfig, Journaler, ProgressSignal, SessionCtx,
};
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
