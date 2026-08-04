use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};

const CREATED_BODY: &[u8] =
    b"{\"run_id\":\"run-001\",\"state\":\"accepted\",\"event_sequence\":1}\n";
const CONFLICT_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Conflict\",\"status\":409,\"detail\":\"run_id is already accepted under a different Idempotency-Key\"}\n";
const REQUEST_DIGEST: &str = "c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428";

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new(label: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "fkst-local-qa-host-{label}-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("temporary directory must be created");
        Self { path }
    }

    fn database_path(&self) -> PathBuf {
        self.path.join("journal.sqlite")
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("temporary directory must be removed");
    }
}

struct HostProcess {
    child: Child,
    port: u16,
}

impl HostProcess {
    fn start(database_path: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_fkst-local-qa-host"))
            .args(["local-demo", "--listen", "127.0.0.1:0", "--database"])
            .arg(database_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("fkst-local-qa-host must start");
        let stdout = child.stdout.take().expect("host stdout must be piped");
        let mut reader = BufReader::new(stdout);
        let mut readiness = String::new();
        reader
            .read_line(&mut readiness)
            .expect("host readiness line must be readable");
        let prefix = "fkst-local-qa-host: listening on 127.0.0.1:";
        let port = readiness
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix('\n'))
            .expect("host readiness line must have the exact IPv4 grammar")
            .parse::<u16>()
            .expect("assigned port must be a decimal u16");
        assert_ne!(port, 0, "the kernel must assign a nonzero port");
        Self { child, port }
    }

    fn stop(mut self) {
        self.child.kill().expect("host process must terminate");
        let status = self.child.wait().expect("host process must be reaped");
        assert!(
            !status.success(),
            "a killed host must not exit successfully"
        );
    }
}

impl Drop for HostProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct HttpResponse {
    status_line: String,
    content_type: String,
    body: Vec<u8>,
}

fn submit(port: u16, run_id: &str, idempotency_key: &str) -> HttpResponse {
    let body = b"{\"kind\":\"inert\"}";
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("loopback host must accept connections");
    write!(
        stream,
        "PUT /v1/runs/{run_id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nIdempotency-Key: {idempotency_key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("request headers must be written");
    stream
        .write_all(body)
        .expect("request body must be written");
    stream.flush().expect("request must be flushed");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("response must be readable");
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response must contain an HTTP header terminator");
    let headers =
        std::str::from_utf8(&response[..header_end]).expect("response headers must be UTF-8");
    let mut lines = headers.split("\r\n");
    let status_line = lines
        .next()
        .expect("response must contain a status line")
        .to_owned();
    let content_type = lines
        .find_map(|line| line.strip_prefix("Content-Type: "))
        .expect("response must contain Content-Type")
        .to_owned();
    HttpResponse {
        status_line,
        content_type,
        body: response[header_end + 4..].to_vec(),
    }
}

#[test]
fn acceptance_restarts_replays_and_rejects_a_different_key_without_mutation() {
    let temp = TempDirectory::new("restart-replay");
    let database_path = temp.database_path();

    let first_host = HostProcess::start(&database_path);
    let created = submit(first_host.port, "run-001", "idem-001");
    assert_eq!(created.status_line, "HTTP/1.1 201 Created");
    assert_eq!(created.content_type, "application/json");
    assert_eq!(created.body, CREATED_BODY);
    first_host.stop();

    let restarted_host = HostProcess::start(&database_path);
    let replay = submit(restarted_host.port, "run-001", "idem-001");
    assert_eq!(replay.status_line, "HTTP/1.1 200 OK");
    assert_eq!(replay.content_type, "application/json");
    assert_eq!(replay.body, CREATED_BODY);

    let conflict = submit(restarted_host.port, "run-001", "idem-002");
    assert_eq!(conflict.status_line, "HTTP/1.1 409 Conflict");
    assert_eq!(conflict.content_type, "application/problem+json");
    assert_eq!(conflict.body, CONFLICT_BODY);
    restarted_host.stop();

    assert_exact_journal(&database_path, "idem-001");
}

#[test]
fn concurrent_different_keys_create_exactly_one_acceptance() {
    let temp = TempDirectory::new("concurrent-keys");
    let database_path = temp.database_path();
    let host = HostProcess::start(&database_path);
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for key in ["idem-001", "idem-002"] {
        let barrier = Arc::clone(&barrier);
        let port = host.port;
        threads.push(thread::spawn(move || {
            barrier.wait();
            (key, submit(port, "run-001", key))
        }));
    }
    barrier.wait();

    let mut results = threads
        .into_iter()
        .map(|thread| thread.join().expect("submit thread must complete"))
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.1.status_line.cmp(&right.1.status_line));
    assert_eq!(results[0].1.status_line, "HTTP/1.1 201 Created");
    assert_eq!(results[0].1.body, CREATED_BODY);
    assert_eq!(results[1].1.status_line, "HTTP/1.1 409 Conflict");
    assert_eq!(results[1].1.body, CONFLICT_BODY);
    let accepted_key = results[0].0;
    host.stop();

    assert_exact_journal(&database_path, accepted_key);
}

fn assert_exact_journal(database_path: &Path, accepted_key: &str) {
    let connection = Connection::open(database_path).expect("journal must open after host exit");
    let accepted = connection
        .query_row(
            "SELECT run_id, idempotency_key, request_digest, response_json
             FROM accepted_requests",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            },
        )
        .optional()
        .expect("accepted request query must succeed");
    assert_eq!(
        accepted,
        Some((
            "run-001".to_owned(),
            accepted_key.to_owned(),
            REQUEST_DIGEST.to_owned(),
            CREATED_BODY.to_vec(),
        ))
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM accepted_requests", [], |row| row
                .get::<_, i64>(0))
            .expect("accepted request count must be readable"),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT run_id, state FROM runs", [], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("Run row must be readable"),
        ("run-001".to_owned(), "accepted".to_owned())
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT run_id, sequence, event_type, event_json FROM events",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .expect("Event row must be readable"),
        (
            "run-001".to_owned(),
            1,
            "run.accepted".to_owned(),
            "{\"run_id\":\"run-001\",\"state\":\"accepted\"}".to_owned(),
        )
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row
                .get::<_, i64>(0))
            .expect("Event count must be readable"),
        1
    );
}
