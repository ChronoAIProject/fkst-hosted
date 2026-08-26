use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fkst_local_qa_host::{parse_startup, serve_with_clock, FixedClock, Journal};
use fkst_qa_contracts::{admit_json, canonical_admitted_bytes, sha256_digest};
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

fn start_host(database: &PathBuf) -> Host {
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

fn request(port: u16, method: &str, key: &str, body: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "{method} /v1/runs/00000000-0000-0000-0000-000000000002 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nIdempotency-Key: {key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}\n",
        body.len() + 1
    )
    .unwrap();
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

#[test]
fn admits_replays_conflicts_and_recovers_one_v2_request() {
    let fixture = fixture();
    let database = database_path();
    {
        let host = start_host(&database);
        let created = request(
            host.port,
            "PUT",
            "idem_0002",
            &fixture.expected_request_utf8,
        );
        assert!(created.starts_with(b"HTTP/1.1 201 Created\r\n"));
        assert_eq!(
            body(&created),
            format!("{}\n", fixture.expected_acceptance_utf8).as_bytes()
        );
        let replay = request(
            host.port,
            "PUT",
            "idem_0002",
            &fixture.expected_request_utf8,
        );
        assert!(replay.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert_eq!(body(&replay), body(&created));
        let changed = with_idempotency_key(&fixture.expected_request_utf8, "different");
        let conflict = request(host.port, "PUT", "different", &changed);
        assert!(conflict.starts_with(b"HTTP/1.1 409 Conflict\r\n"));
        let post = request(
            host.port,
            "POST",
            "idem_0002",
            &fixture.expected_request_utf8,
        );
        assert!(post.starts_with(b"HTTP/1.1 405 Method Not Allowed\r\n"));
    }
    let journal = Journal::open(&database).unwrap();
    let stored = journal
        .stored_v2_admission("00000000-0000-0000-0000-000000000002")
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.acceptance_bytes,
        format!("{}\n", fixture.expected_acceptance_utf8).as_bytes()
    );
    drop(journal);
    let restarted = start_host(&database);
    let replay = request(
        restarted.port,
        "PUT",
        "idem_0002",
        &fixture.expected_request_utf8,
    );
    assert!(replay.starts_with(b"HTTP/1.1 200 OK\r\n"));
    let _ = fs::remove_file(database);
}
