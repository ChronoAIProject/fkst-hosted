use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fkst_local_qa_host::{parse_startup, serve_with_clock, FixedClock, Journal};
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
            join.join().unwrap();
        }
    }
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../../../packages/qa-contracts/fixtures/qa.local-run-admission/v2/happy-path.json"
    ))
    .unwrap()
}

fn database_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "fkst-admission-v2-{}-{nonce}.sqlite",
        std::process::id()
    ))
}

fn start_host(database: &Path) -> Host {
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
        serve_with_clock(
            config,
            thread_shutdown,
            Arc::new(FixedClock::new("2026-08-25T16:00:01Z").unwrap()),
        )
        .unwrap();
    });
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Host {
                shutdown,
                join: Some(join),
                port,
            };
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("host did not start")
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
    assert_eq!(fixture.expected_request_utf8.len(), 1940);
    let expected_body = format!("{}\n", fixture.expected_acceptance_utf8).into_bytes();
    assert_eq!(expected_body.len(), 740);
    let mut expected_created = b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 740\r\nConnection: close\r\n\r\n".to_vec();
    expected_created.extend_from_slice(&expected_body);
    let database = database_path();
    {
        let host = start_host(&database);
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
            "sha256:42711db690b0ce483e28161924a8371ac9f498fd9f81ad90fc29bb15b9e96e30".to_owned(),
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
    assert_eq!(user_version, 6);
    drop(connection);

    let restarted = start_host(&database);
    let replay = request(
        restarted.port,
        "PUT",
        "idem_0002",
        fixture.expected_request_utf8.as_bytes(),
    );
    assert!(replay.starts_with(b"HTTP/1.1 200 OK\r\n"));
    let _ = fs::remove_file(database);
}

#[test]
fn maintained_parser_rejects_malformed_header_before_admission_mutation() {
    let fixture = fixture();
    let database = database_path();
    {
        let host = start_host(&database);
        let rejected =
            malformed_content_type_request(host.port, fixture.expected_request_utf8.as_bytes());
        assert!(rejected.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
    }

    assert_admission_tables_empty(&database);
    let _ = fs::remove_file(database);
}

#[test]
fn rejects_trailing_newline_before_admission_mutation() {
    let fixture = fixture();
    let database = database_path();
    {
        let host = start_host(&database);
        let mut request_body = fixture.expected_request_utf8.into_bytes();
        request_body.push(b'\n');
        let rejected = request(host.port, "PUT", "idem_0002", &request_body);
        assert!(rejected.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
    }

    assert_admission_tables_empty(&database);
    let _ = fs::remove_file(database);
}
