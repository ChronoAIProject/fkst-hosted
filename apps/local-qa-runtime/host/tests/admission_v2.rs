use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fkst_local_qa_host::{
    parse_startup, serve_with_clock, Clock, FixedClock, Journal, RunError, StartupConfig,
};
use fkst_qa_contracts::{admit_json, canonical_admitted_bytes, sha256_digest};
use rusqlite::Connection;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    expected_request_utf8: String,
    expected_acceptance_utf8: String,
}

struct Host {
    shutdown: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
    port: u16,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let result = join.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../../../packages/qa-contracts/fixtures/qa.local-run-admission/v2/happy-path.json"
    ))
    .unwrap()
}

const DIRECTORY_ALLOCATION_ATTEMPTS: usize = 128;

struct DatabaseFixture {
    directory: PathBuf,
}

impl DatabaseFixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self::reserve(|attempt| {
            std::env::temp_dir().join(format!(
                "fkst-admission-v2-{}-{nonce}-{attempt}",
                std::process::id()
            ))
        })
        .expect("temporary database directory must be reserved")
    }

    fn reserve(mut candidate: impl FnMut(usize) -> PathBuf) -> std::io::Result<Self> {
        for attempt in 0..DIRECTORY_ALLOCATION_ATTEMPTS {
            let directory = candidate(attempt);
            match fs::create_dir(&directory) {
                Ok(()) => return Ok(Self { directory }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "temporary database directory allocation exhausted",
        ))
    }

    fn database_path(&self) -> PathBuf {
        self.directory.join("journal.sqlite")
    }
}

impl Drop for DatabaseFixture {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.directory) {
            eprintln!(
                "could not remove owned database fixture {:?}: {error}",
                self.directory
            );
        }
    }
}

#[test]
fn database_fixture_reserves_distinct_paths_for_the_same_nonce() {
    let root = DatabaseFixture::new();
    let candidate = |attempt| root.directory.join(format!("fixed-nonce-6092-{attempt}"));
    let neighbor = candidate(0);
    fs::create_dir(&neighbor).unwrap();
    let sentinel = neighbor.join("journal.sqlite");
    fs::write(&sentinel, b"pre-existing neighbor database").unwrap();
    let barrier = std::sync::Barrier::new(7);
    let mut fixtures = thread::scope(|scope| {
        let workers: Vec<_> = (0..7)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    DatabaseFixture::reserve(candidate).unwrap()
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    let paths: Vec<_> = fixtures
        .iter()
        .map(DatabaseFixture::database_path)
        .collect();
    let distinct: std::collections::HashSet<_> = paths.iter().collect();
    assert_eq!(
        distinct.len(),
        paths.len(),
        "each fixture must own its path"
    );
    for fixture in &fixtures {
        assert!(fixture.directory.is_dir());
        assert_ne!(fixture.directory, neighbor);
        fs::write(fixture.database_path(), b"owned database").unwrap();
    }
    let first = fixtures.pop().unwrap();
    let first_directory = first.directory.clone();
    drop(first);
    assert!(!first_directory.exists());
    for fixture in &fixtures {
        assert_eq!(
            fs::read(fixture.database_path()).unwrap(),
            b"owned database"
        );
    }
    drop(fixtures);
    for path in paths {
        assert!(!path.parent().unwrap().exists());
    }
    assert_eq!(
        fs::read(&sentinel).unwrap(),
        b"pre-existing neighbor database"
    );
    assert_eq!(fs::read_dir(&root.directory).unwrap().count(), 1);

    let mut attempts = 0;
    let exhausted = DatabaseFixture::reserve(|_| {
        attempts += 1;
        neighbor.clone()
    });
    assert_eq!(
        exhausted.err().unwrap().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(attempts, DIRECTORY_ALLOCATION_ATTEMPTS);
    assert_eq!(
        fs::read(&sentinel).unwrap(),
        b"pre-existing neighbor database"
    );

    let mut attempts = 0;
    let invalid_parent = DatabaseFixture::reserve(|_| {
        attempts += 1;
        sentinel.join("not-a-directory")
    });
    assert!(invalid_parent.is_err());
    assert_eq!(attempts, 1, "only name collisions may be retried");
    assert_eq!(
        fs::read(&sentinel).unwrap(),
        b"pre-existing neighbor database"
    );
}

#[test]
fn database_fixture_keeps_wal_and_shm_until_users_close_then_cleans_on_unwind() {
    let root = DatabaseFixture::new();
    let directory = root.directory.join("wal-lifetime");
    let result = std::panic::catch_unwind(|| {
        let temporary = DatabaseFixture::reserve(|_| directory.clone()).unwrap();
        let database = temporary.database_path();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; CREATE TABLE sentinel(value TEXT); \
             INSERT INTO sentinel VALUES ('retained through Host shutdown');",
            )
            .unwrap();
        let host = start_mvp0_host(&database);
        let wal = temporary.directory.join("journal.sqlite-wal");
        let shm = temporary.directory.join("journal.sqlite-shm");
        assert!(database.is_file());
        assert!(wal.is_file());
        assert!(shm.is_file());
        drop(host);
        assert!(
            wal.is_file(),
            "the remaining SQLite connection still owns WAL"
        );
        assert!(
            shm.is_file(),
            "the remaining SQLite connection still owns SHM"
        );
        let value: String = connection
            .query_row("SELECT value FROM sentinel", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "retained through Host shutdown");
        let _restarted = start_mvp0_host(&database);
        panic!("exercise fixture cleanup while unwinding");
    });
    let panic = result.expect_err("the fixture scope must unwind");
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"exercise fixture cleanup while unwinding")
    );
    assert!(
        !directory.exists(),
        "owned database and all sidecars must be removed"
    );
    assert!(root.directory.is_dir(), "cleanup must preserve the parent");
}

type ServeWithClock =
    fn(StartupConfig, Arc<AtomicBool>, Arc<dyn Clock + Send + Sync>) -> Result<(), RunError>;

fn start_host_with(database: &Path, serve: ServeWithClock) -> Host {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let config = parse_startup([
        "local-demo".into(),
        "--listen".into(),
        format!("127.0.0.1:{port}").into(),
        "--database".into(),
        database.as_os_str().to_owned(),
    ])
    .unwrap();
    let shutdown = Arc::new(AtomicBool::new(false));
    let thread_shutdown = Arc::clone(&shutdown);
    let join = thread::spawn(move || {
        serve(
            config,
            thread_shutdown,
            Arc::new(FixedClock::new("2026-08-25T16:00:01Z").unwrap()),
        )
        .unwrap();
    });
    let host = Host {
        shutdown,
        join: Some(join),
        port,
    };
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return host;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("host did not start")
}

fn start_mvp0_host(database: &Path) -> Host {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = parse_startup([
        "local-demo".into(),
        "--listen".into(),
        format!("127.0.0.1:{port}").into(),
        "--database".into(),
        database.as_os_str().to_owned(),
    ])
    .unwrap();
    let shutdown = Arc::new(AtomicBool::new(false));
    let thread_shutdown = Arc::clone(&shutdown);
    let (ready, received) = std::sync::mpsc::channel();
    let join = thread::spawn(move || {
        fkst_local_qa_host::serve_mvp0_with_listener_for_test(
            config,
            thread_shutdown,
            Arc::new(FixedClock::new("2026-08-25T16:00:01Z").unwrap()),
            listener,
            ready,
        )
        .unwrap();
    });
    let host = Host {
        shutdown,
        join: Some(join),
        port,
    };
    assert_eq!(
        received
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .port(),
        port
    );
    host
}

fn start_production_host(database: &Path) -> Host {
    start_host_with(database, serve_with_clock)
}

fn request(port: u16, method: &str, key: &str, body: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "{method} /v1/runs/00000000-0000-0000-0000-000000000002 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nIdempotency-Key: {key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}

fn malformed_content_type_request(port: u16, body: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "PUT /v1/runs/00000000-0000-0000-0000-000000000002 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type application/json\r\nIdempotency-Key: idem_0002\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}

fn http_1_0_request(port: u16, body: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "PUT /v1/runs/00000000-0000-0000-0000-000000000002 HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nIdempotency-Key: idem_0002\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}

fn body(response: &[u8]) -> &[u8] {
    let offset = response
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    &response[offset..]
}

fn with_idempotency_key(body: &str, key: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(body).unwrap();
    value["idempotency_key"] = key.into();
    canonical_with_digest(value)
}

fn canonical_with_digest(mut value: serde_json::Value) -> String {
    value.as_object_mut().unwrap().remove("content_digest");
    let projected = serde_json::to_vec(&value).unwrap();
    let admitted = admit_json(&projected).unwrap();
    let digest = sha256_digest(&canonical_admitted_bytes(&admitted).unwrap());
    value["content_digest"] = digest.into();
    let admitted = admit_json(&serde_json::to_vec(&value).unwrap()).unwrap();
    String::from_utf8(canonical_admitted_bytes(&admitted).unwrap()).unwrap()
}

fn assert_admission_tables_empty(database: &Path) {
    let connection = Connection::open(database).unwrap();
    for table in [
        "accepted_requests",
        "runs",
        "events",
        "admission_v2_records",
        "active_run_slot",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} must remain empty");
    }
}

#[test]
fn admits_replays_conflicts_and_recovers_one_v2_request() {
    let fixture = fixture();
    assert_eq!(fixture.expected_request_utf8.len(), 1947);
    let expected_body = format!("{}\n", fixture.expected_acceptance_utf8).into_bytes();
    assert_eq!(expected_body.len(), 740);
    let mut expected_created = b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 740\r\nConnection: close\r\n\r\n".to_vec();
    expected_created.extend_from_slice(&expected_body);
    let temporary = DatabaseFixture::new();
    let database = temporary.database_path();
    {
        let host = start_mvp0_host(&database);
        let created = request(
            host.port,
            "PUT",
            "idem_0002",
            fixture.expected_request_utf8.as_bytes(),
        );
        assert_eq!(created, expected_created);
        let replay = request(
            host.port,
            "PUT",
            "idem_0002",
            fixture.expected_request_utf8.as_bytes(),
        );
        assert!(replay.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert_eq!(body(&replay), body(&created));
        let changed = with_idempotency_key(&fixture.expected_request_utf8, "different");
        let conflict = request(host.port, "PUT", "different", changed.as_bytes());
        assert!(conflict.starts_with(b"HTTP/1.1 409 Conflict\r\n"));
        let post = request(
            host.port,
            "POST",
            "idem_0002",
            fixture.expected_request_utf8.as_bytes(),
        );
        assert!(post.starts_with(b"HTTP/1.1 405 Method Not Allowed\r\n"));
    }
    let journal = Journal::open(&database).unwrap();
    let stored = journal
        .stored_v2_admission("00000000-0000-0000-0000-000000000002")
        .unwrap()
        .unwrap();
    assert_eq!(stored.acceptance_bytes, expected_body);
    drop(journal);

    let connection = Connection::open(&database).unwrap();
    let accepted: (String, String, String, Vec<u8>) = connection
        .query_row(
            "SELECT run_id, idempotency_key, request_digest, response_json FROM accepted_requests",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        accepted,
        (
            "00000000-0000-0000-0000-000000000002".to_owned(),
            "idem_0002".to_owned(),
            "sha256:466f393b6846c658687a0d77e17567fb6c4f403c2ebc7c44c8fb33d9f63321b5".to_owned(),
            expected_body.clone(),
        )
    );
    let run: (String, i64, Option<String>) = connection
        .query_row(
            "SELECT state, admission_version, execution_outcome FROM runs",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(run, ("accepted".to_owned(), 2, None));
    let event: (i64, String) = connection
        .query_row("SELECT sequence, event_type FROM events", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(event, (1, "run.accepted".to_owned()));
    let admission_records: i64 = connection
        .query_row("SELECT COUNT(*) FROM admission_v2_records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(admission_records, 1);
    let active_slot: (i64, String) = connection
        .query_row("SELECT slot, run_id FROM active_run_slot", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(
        active_slot,
        (1, "00000000-0000-0000-0000-000000000002".to_owned())
    );
    let user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 10);
    drop(connection);

    let restarted = start_mvp0_host(&database);
    let replay = request(
        restarted.port,
        "PUT",
        "idem_0002",
        fixture.expected_request_utf8.as_bytes(),
    );
    assert!(replay.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert_eq!(body(&replay), expected_body);
    drop(restarted);
}

#[test]
fn restart_reconstructs_complete_canonical_admitted_input() {
    let fixture = fixture();
    let temporary = DatabaseFixture::new();
    let database = temporary.database_path();
    {
        let host = start_mvp0_host(&database);
        // Formatting is admitted, but only canonical validated JSON is retained.
        let value: serde_json::Value =
            serde_json::from_str(&fixture.expected_request_utf8).unwrap();
        let formatted = serde_json::to_vec_pretty(&value).unwrap();
        let created = request(host.port, "PUT", "idem_0002", &formatted);
        assert!(created.starts_with(b"HTTP/1.1 201 Created\r\n"));
    }
    let connection = Connection::open(&database).unwrap();
    let stored: Vec<u8> = connection
        .query_row("SELECT request_json FROM admission_v2_records", [], |row| {
            row.get(0)
        })
        .expect("accepted input must survive Host shutdown");
    assert_eq!(stored, fixture.expected_request_utf8.as_bytes());
    drop(connection);
    let journal = Journal::open(&database).unwrap();
    let reconstructed = journal.reconstruct_v2_request(RUN_ID).unwrap().unwrap();
    let expected: fkst_qa_contracts::LocalQARunRequestV2 =
        serde_json::from_str(&fixture.expected_request_utf8).unwrap();
    assert_eq!(reconstructed, expected);
    assert!(journal
        .reconstruct_v2_request("00000000-0000-0000-0000-000000000099")
        .unwrap()
        .is_none());
    drop(journal);
    let restarted = start_production_host(&database);
    let replay = request(
        restarted.port,
        "PUT",
        "idem_0002",
        fixture.expected_request_utf8.as_bytes(),
    );
    assert!(replay.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert_eq!(
        body(&replay),
        format!("{}\n", fixture.expected_acceptance_utf8).as_bytes()
    );
    drop(restarted);
    let connection = Connection::open(database).unwrap();
    let effects: i64 = connection
        .query_row("SELECT COUNT(*) FROM execution_attempts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        effects, 0,
        "reconstruction and response replay cannot claim v2"
    );
}

const RUN_ID: &str = "00000000-0000-0000-0000-000000000002";
const SECRET_CANARY: &str = "SECRET_CANARY_6092_DO_NOT_ECHO";

fn admitted_database() -> DatabaseFixture {
    let temporary = DatabaseFixture::new();
    let host = start_mvp0_host(&temporary.database_path());
    let created = request(
        host.port,
        "PUT",
        "idem_0002",
        fixture().expected_request_utf8.as_bytes(),
    );
    assert!(created.starts_with(b"HTTP/1.1 201 Created\r\n"));
    drop(host);
    temporary
}

#[test]
fn reconstruction_rejects_corrupt_input_and_relations_without_mutation_or_echo() {
    let temporary = admitted_database();
    let database = temporary.database_path();
    let journal = Journal::open(&database).unwrap();
    let connection = Connection::open(&database).unwrap();
    let original = fixture().expected_request_utf8;
    let mut unknown: serde_json::Value = serde_json::from_str(&original).unwrap();
    unknown["bearer_token"] = SECRET_CANARY.into();
    let mut missing: serde_json::Value = serde_json::from_str(&original).unwrap();
    missing.as_object_mut().unwrap().remove("environment");
    let inputs = [
        b"{".to_vec(),
        serde_json::to_vec(&unknown).unwrap(),
        serde_json::to_vec(&missing).unwrap(),
        original
            .replacen('{', &format!("{{\"nonce\":\"{SECRET_CANARY}\","), 1)
            .into_bytes(),
        original.replace("source-0002", "source-0003").into_bytes(),
        original
            .replace(RUN_ID, "00000000-0000-0000-0000-000000000099")
            .into_bytes(),
        with_idempotency_key(&original, "different-key").into_bytes(),
        serde_json::to_vec_pretty(&serde_json::from_str::<serde_json::Value>(&original).unwrap())
            .unwrap(),
        format!("{original}\n").into_bytes(),
        vec![b' '; 65_537],
    ];
    for bytes in inputs {
        connection
            .execute("UPDATE admission_v2_records SET request_json=?1", [bytes])
            .unwrap();
        let before = connection.total_changes();
        let error = journal.reconstruct_v2_request(RUN_ID).unwrap_err();
        assert!(matches!(
            error,
            RunError::InvalidJournal("invalid persisted v2 admission input")
        ));
        assert!(!error.to_string().contains(SECRET_CANARY));
        assert_eq!(connection.total_changes(), before);
    }
    connection
        .execute(
            "UPDATE admission_v2_records SET request_json=?1",
            [original.as_bytes()],
        )
        .unwrap();
    assert!(journal.reconstruct_v2_request(RUN_ID).unwrap().is_some());
    for sql in [
        "UPDATE accepted_requests SET idempotency_key='other'",
        "UPDATE accepted_requests SET request_digest='other'",
        "UPDATE accepted_requests SET response_json=X'7B7D'",
        "UPDATE admission_v2_records SET binding_json=X'7B7D'",
        "UPDATE admission_v2_records SET selection_json=X'7B7D'",
        "UPDATE admission_v2_records SET request_json='text is not a blob'",
        "UPDATE admission_v2_records SET request_json=zeroblob(10000000)",
        "UPDATE admission_v2_records SET binding_json=zeroblob(10000000)",
        "UPDATE admission_v2_records SET selection_json=zeroblob(10000000)",
        "UPDATE accepted_requests SET response_json=zeroblob(10000000)",
        "UPDATE runs SET admission_version=1",
        "UPDATE runs SET executor_run_id='00000000-0000-0000-0000-000000000099'",
        "DELETE FROM runs",
        "DELETE FROM accepted_requests",
        "DELETE FROM admission_v2_records",
    ] {
        connection.execute_batch("BEGIN").unwrap();
        connection.execute_batch(sql).unwrap();
        connection.execute_batch("COMMIT").unwrap();
        let error = journal.reconstruct_v2_request(RUN_ID).unwrap_err();
        assert!(matches!(error, RunError::InvalidJournal(_)), "{sql}");
        restore_admission_rows(&connection, &original);
    }
    let original_value: serde_json::Value = serde_json::from_str(&original).unwrap();
    for (column, pointer, replacement) in [
        ("binding_json", "/worker_id", "worker-foreign"),
        ("binding_json", "/deadline", "2026-08-25T16:06:00Z"),
        ("selection_json", "/executor_version", "2.0.0"),
    ] {
        let field = if column == "binding_json" {
            "attempt_binding"
        } else {
            "executor_selection"
        };
        let mut changed = original_value[field].clone();
        *changed.pointer_mut(pointer).unwrap() = replacement.into();
        connection
            .execute(
                &format!("UPDATE admission_v2_records SET {column}=?1"),
                [serde_json::to_vec(&changed).unwrap()],
            )
            .unwrap();
        assert!(matches!(
            journal.reconstruct_v2_request(RUN_ID),
            Err(RunError::InvalidJournal(_))
        ));
        restore_admission_rows(&connection, &original);
    }
    for accepted_at in ["2026-08-25T15:59:59Z", "2026-08-25T16:05:00Z"] {
        let mut changed: serde_json::Value =
            serde_json::from_str(&fixture().expected_acceptance_utf8).unwrap();
        changed["created_at"] = accepted_at.into();
        changed["accepted_at"] = accepted_at.into();
        let bytes = format!("{}\n", canonical_with_digest(changed)).into_bytes();
        fkst_qa_contracts::validate_run_acceptance_v2(&bytes[..bytes.len() - 1]).unwrap();
        connection
            .execute("UPDATE accepted_requests SET response_json=?1", [bytes])
            .unwrap();
        assert!(matches!(
            journal.reconstruct_v2_request(RUN_ID),
            Err(RunError::InvalidJournal(_))
        ));
        restore_admission_rows(&connection, &original);
    }
    for pointer in [
        "/run_id",
        "/source/id",
        "/environment/id",
        "/executor_selection/executor_version",
    ] {
        let mut changed = original_value.clone();
        *changed.pointer_mut(pointer).unwrap() = if pointer == "/run_id" {
            "00000000-0000-0000-0000-000000000099".into()
        } else if pointer == "/executor_selection/executor_version" {
            "2.0.0".into()
        } else {
            "foreign-reference".into()
        };
        let bytes = canonical_with_digest(changed).into_bytes();
        fkst_qa_contracts::validate_local_qa_run_request_v2(&bytes).unwrap();
        connection
            .execute("UPDATE admission_v2_records SET request_json=?1", [bytes])
            .unwrap();
        assert!(matches!(
            journal.reconstruct_v2_request(RUN_ID),
            Err(RunError::InvalidJournal(_))
        ));
        restore_admission_rows(&connection, &original);
    }
    assert!(journal.reconstruct_v2_request(RUN_ID).unwrap().is_some());
}

fn restore_admission_rows(connection: &Connection, original: &str) {
    let value: serde_json::Value = serde_json::from_str(original).unwrap();
    connection.execute("INSERT OR REPLACE INTO accepted_requests (run_id,idempotency_key,request_digest,response_json) VALUES (?1,'idem_0002',?2,?3)", rusqlite::params![RUN_ID, value["content_digest"].as_str().unwrap(), format!("{}\n", fixture().expected_acceptance_utf8).as_bytes()]).unwrap();
    connection.execute("INSERT OR REPLACE INTO runs (run_id,executor_run_id,state,admission_version) VALUES (?1,?1,'accepted',2)", [RUN_ID]).unwrap();
    connection.execute("INSERT OR REPLACE INTO admission_v2_records (run_id,binding_json,selection_json,request_json) VALUES (?1,?2,?3,?4)", rusqlite::params![RUN_ID, serde_json::to_vec(&value["attempt_binding"]).unwrap(), serde_json::to_vec(&value["executor_selection"]).unwrap(), original.as_bytes()]).unwrap();
}

#[test]
fn legacy_null_input_replays_exact_response_without_current_claim() {
    let temporary = admitted_database();
    let database = temporary.database_path();
    let connection = Connection::open(&database).unwrap();
    connection
        .execute("UPDATE admission_v2_records SET request_json=NULL", [])
        .unwrap();
    drop(connection);
    let journal = Journal::open(&database).unwrap();
    assert!(journal.reconstruct_v2_request(RUN_ID).unwrap().is_none());
    drop(journal);
    let host = start_production_host(&database);
    let fixture = fixture();
    let replay = request(
        host.port,
        "PUT",
        "idem_0002",
        fixture.expected_request_utf8.as_bytes(),
    );
    assert!(replay.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert_eq!(
        body(&replay),
        format!("{}\n", fixture.expected_acceptance_utf8).as_bytes()
    );
}

#[test]
fn input_insert_failure_rolls_back_acceptance_slot_and_events_without_echo() {
    for table in ["admission_v2_records", "active_run_slot"] {
        let temporary = DatabaseFixture::new();
        let database = temporary.database_path();
        drop(Journal::open(&database).unwrap());
        let connection = Connection::open(&database).unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER fail_insert BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, '{SECRET_CANARY}'); END;")).unwrap();
        let host = start_mvp0_host(&database);
        let failed = request(
            host.port,
            "PUT",
            "idem_0002",
            fixture().expected_request_utf8.as_bytes(),
        );
        assert!(failed.starts_with(b"HTTP/1.1 500 Internal Server Error\r\n"));
        assert!(!String::from_utf8_lossy(&failed).contains(SECRET_CANARY));
        assert_admission_tables_empty(&database);
        connection
            .execute_batch("DROP TRIGGER fail_insert")
            .unwrap();
        let accepted = request(
            host.port,
            "PUT",
            "idem_0002",
            fixture().expected_request_utf8.as_bytes(),
        );
        assert!(accepted.starts_with(b"HTTP/1.1 201 Created\r\n"));
        drop(host);
        assert!(Journal::open(&database)
            .unwrap()
            .reconstruct_v2_request(RUN_ID)
            .unwrap()
            .is_some());
    }
}

#[test]
fn unknown_secret_and_path_fields_are_rejected_before_any_durable_write() {
    let temporary = DatabaseFixture::new();
    let database = temporary.database_path();
    let host = start_mvp0_host(&database);
    for pointer in [
        "/bearer_token",
        "/authorization",
        "/lease_credential",
        "/source/path",
        "/environment/token",
    ] {
        let mut value: serde_json::Value =
            serde_json::from_str(&fixture().expected_request_utf8).unwrap();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        value.pointer_mut(parent).unwrap()[key] = SECRET_CANARY.into();
        let rejected = request(
            host.port,
            "PUT",
            "idem_0002",
            &serde_json::to_vec(&value).unwrap(),
        );
        assert!(rejected.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
        assert!(!String::from_utf8_lossy(&rejected).contains(SECRET_CANARY));
        assert_admission_tables_empty(&database);
    }
}

#[test]
fn production_rejects_v2_when_current_claim_authority_is_unavailable() {
    let fixture = fixture();
    let temporary = DatabaseFixture::new();
    let database = temporary.database_path();
    {
        let host = start_production_host(&database);
        let rejected = request(
            host.port,
            "PUT",
            "idem_0002",
            fixture.expected_request_utf8.as_bytes(),
        );
        assert!(
            rejected.starts_with(b"HTTP/1.1 503 Service Unavailable\r\n"),
            "unexpected production response: {}",
            String::from_utf8_lossy(&rejected)
        );
        assert!(body(&rejected).starts_with(
            b"{\"type\":\"about:blank\",\"title\":\"Service Unavailable\",\"status\":503"
        ));
    }

    assert_admission_tables_empty(&database);
}

#[test]
fn maintained_parser_rejects_malformed_header_before_admission_mutation() {
    let fixture = fixture();
    let temporary = DatabaseFixture::new();
    let database = temporary.database_path();
    {
        let host = start_mvp0_host(&database);
        let rejected =
            malformed_content_type_request(host.port, fixture.expected_request_utf8.as_bytes());
        assert!(rejected.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
    }

    assert_admission_tables_empty(&database);
}

#[test]
fn maintained_parser_rejects_http_1_0_before_admission_mutation() {
    let fixture = fixture();
    let temporary = DatabaseFixture::new();
    let database = temporary.database_path();
    {
        let host = start_mvp0_host(&database);
        let rejected = http_1_0_request(host.port, fixture.expected_request_utf8.as_bytes());
        assert!(rejected.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
    }

    assert_admission_tables_empty(&database);
}

#[test]
fn rejects_trailing_newline_before_admission_mutation() {
    let fixture = fixture();
    let temporary = DatabaseFixture::new();
    let database = temporary.database_path();
    {
        let host = start_mvp0_host(&database);
        let mut request_body = fixture.expected_request_utf8.into_bytes();
        request_body.push(b'\n');
        let rejected = request(host.port, "PUT", "idem_0002", &request_body);
        assert!(rejected.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
    }

    assert_admission_tables_empty(&database);
}

#[test]
fn rejects_the_old_non_canonical_fence_token_without_mutation() {
    let fixture = fixture();
    let temporary = DatabaseFixture::new();
    let database = temporary.database_path();
    {
        let host = start_mvp0_host(&database);
        let request_body = fixture
            .expected_request_utf8
            .replace("dGVzdC1mZW5jZS0wMDAwMDAwMg", "test-fence-00000002");
        let rejected = request(host.port, "PUT", "idem_0002", request_body.as_bytes());
        assert!(rejected.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
    }

    assert_admission_tables_empty(&database);
}

#[test]
fn accepts_every_idempotency_key_character_allowed_by_the_transport_contract() {
    let fixture = fixture();
    let keys = [
        "-".to_owned(),
        "_".to_owned(),
        "A".to_owned(),
        "0".to_owned(),
        "-".repeat(64),
    ];
    for key in keys {
        let temporary = DatabaseFixture::new();
        let database = temporary.database_path();
        {
            let host = start_mvp0_host(&database);
            let request_body = with_idempotency_key(&fixture.expected_request_utf8, &key);
            let accepted = request(host.port, "PUT", &key, request_body.as_bytes());
            assert!(
                accepted.starts_with(b"HTTP/1.1 201 Created\r\n"),
                "key {key}"
            );
        }
    }
}
