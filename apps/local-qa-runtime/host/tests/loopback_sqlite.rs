use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};

const CREATED_BODY: &[u8] =
    b"{\"run_id\":\"run-001\",\"state\":\"accepted\",\"event_sequence\":1}\n";
const CONFLICT_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Conflict\",\"status\":409,\"detail\":\"run_id is already accepted under a different Idempotency-Key\"}\n";
const REQUEST_DIGEST: &str = "c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428";
const HEALTH_BODY: &[u8] =
    b"{\"service\":\"fkst-local-qa-host\",\"version\":\"0.0.0\",\"alive\":true}\n";
const NOT_FOUND_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Not Found\",\"status\":404,\"detail\":\"run not found\"}\n";
const INVALID_READ_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Bad Request\",\"status\":400,\"detail\":\"invalid read request\"}\n";
const INVALID_CANCEL_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Bad Request\",\"status\":400,\"detail\":\"invalid cancel request\"}\n";
const METHOD_NOT_ALLOWED_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Method Not Allowed\",\"status\":405,\"detail\":\"method not allowed\"}\n";
const ENDPOINT_NOT_FOUND_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Not Found\",\"status\":404,\"detail\":\"endpoint not found\"}\n";

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

fn request(
    port: u16,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> HttpResponse {
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("loopback host must accept connections");
    write!(
        stream,
        "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
    )
    .expect("request line must be written");
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").expect("request header must be written");
    }
    write!(stream, "Connection: close\r\n\r\n").expect("request terminator must be written");
    stream
        .write_all(body)
        .expect("request body must be written");
    stream.flush().expect("request must be flushed");

    read_response(&mut stream)
}

fn read_response(stream: &mut TcpStream) -> HttpResponse {
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

fn submit(port: u16, run_id: &str, idempotency_key: &str) -> HttpResponse {
    request(
        port,
        "PUT",
        &format!("/v1/runs/{run_id}"),
        &[
            ("Content-Type", "application/json"),
            ("Idempotency-Key", idempotency_key),
            ("Content-Length", "16"),
        ],
        b"{\"kind\":\"inert\"}",
    )
}

fn get(port: u16, target: &str) -> HttpResponse {
    request(port, "GET", target, &[], b"")
}

fn cancel(port: u16, run_id: &str, key: &str) -> HttpResponse {
    request(
        port,
        "POST",
        &format!("/v1/runs/{run_id}:cancel"),
        &[("Idempotency-Key", key), ("Content-Length", "0")],
        b"",
    )
}

fn assert_response(response: HttpResponse, status: &str, content_type: &str, body: &[u8]) {
    assert_eq!(response.status_line, status);
    assert_eq!(response.content_type, content_type);
    assert_eq!(response.body, body);
}

#[test]
fn delayed_nonempty_cancel_body_returns_complete_error_without_mutation() {
    let temp = TempDirectory::new("delayed-cancel-body");
    let database_path = temp.database_path();
    let host = HostProcess::start(&database_path);
    assert_response(
        submit(host.port, "run-001", "idem-001"),
        "HTTP/1.1 201 Created",
        "application/json",
        CREATED_BODY,
    );

    let mut stream = TcpStream::connect(("127.0.0.1", host.port))
        .expect("loopback host must accept connections");
    write!(
        stream,
        "POST /v1/runs/run-001:cancel HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nIdempotency-Key: cancel-001\r\nContent-Length: 1\r\nConnection: close\r\n\r\n",
        host.port
    )
    .expect("cancel request headers must be written");
    stream
        .flush()
        .expect("cancel request headers must be flushed");
    thread::sleep(Duration::from_millis(100));
    stream
        .write_all(b"x")
        .expect("delayed cancel request body must be written");
    stream
        .shutdown(Shutdown::Write)
        .expect("cancel request write side must close");
    assert_response(
        read_response(&mut stream),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_CANCEL_BODY,
    );
    host.stop();

    let connection = Connection::open(&database_path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cancel_requests", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn oversized_cancel_body_declaration_is_rejected_without_waiting_for_body() {
    let temp = TempDirectory::new("oversized-cancel-body");
    let database_path = temp.database_path();
    let host = HostProcess::start(&database_path);

    let mut stream = TcpStream::connect(("127.0.0.1", host.port))
        .expect("loopback host must accept connections");
    write!(
        stream,
        "POST /v1/runs/run-001:cancel HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nIdempotency-Key: cancel-001\r\nContent-Length: 65\r\nConnection: close\r\n\r\n",
        host.port
    )
    .expect("oversized cancel request headers must be written");
    stream
        .shutdown(Shutdown::Write)
        .expect("oversized cancel request write side must close");
    assert_response(
        read_response(&mut stream),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_CANCEL_BODY,
    );
    host.stop();

    let connection = Connection::open(&database_path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cancel_requests", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
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

#[test]
fn reads_cancellation_and_restart_match_the_durable_contract() {
    let temp = TempDirectory::new("reads-cancel-restart");
    let database_path = temp.database_path();
    let host = HostProcess::start(&database_path);

    assert_response(
        get(host.port, "/v1/health"),
        "HTTP/1.1 200 OK",
        "application/json",
        HEALTH_BODY,
    );
    assert_response(
        submit(host.port, "run-001", "idem-001"),
        "HTTP/1.1 201 Created",
        "application/json",
        CREATED_BODY,
    );
    assert_response(
        get(host.port, "/v1/runs/run-001"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"run-001\",\"state\":\"accepted\",\"latest_event_sequence\":1}\n",
    );
    assert_response(
        get(host.port, "/v1/runs/run-001/events?after=0&limit=100"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"run-001\",\"after\":0,\"events\":[{\"sequence\":1,\"event_type\":\"run.accepted\",\"event\":{\"run_id\":\"run-001\",\"state\":\"accepted\"}}],\"next_after\":1}\n",
    );
    assert_response(
        get(host.port, "/v1/runs/run-001/events?limit=1&after=1"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"run-001\",\"after\":1,\"events\":[],\"next_after\":1}\n",
    );
    assert_response(
        cancel(host.port, "run-001", "cancel-001"),
        "HTTP/1.1 202 Accepted",
        "application/json",
        b"{\"run_id\":\"run-001\",\"disposition\":\"accepted\",\"event_sequence\":2}\n",
    );
    assert_response(
        cancel(host.port, "run-001", "cancel-002"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"run-001\",\"disposition\":\"already_accepted\",\"event_sequence\":2}\n",
    );
    assert_response(
        get(host.port, "/v1/runs/run-001/events?after=1&limit=1"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"run-001\",\"after\":1,\"events\":[{\"sequence\":2,\"event_type\":\"run.cancel_requested\",\"event\":{\"run_id\":\"run-001\",\"state\":\"accepted\"}}],\"next_after\":2}\n",
    );
    host.stop();

    let restarted = HostProcess::start(&database_path);
    assert_response(
        submit(restarted.port, "run-001", "idem-001"),
        "HTTP/1.1 200 OK",
        "application/json",
        CREATED_BODY,
    );
    assert_response(
        get(restarted.port, "/v1/runs/run-001"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"run-001\",\"state\":\"accepted\",\"latest_event_sequence\":2}\n",
    );
    assert_response(
        cancel(restarted.port, "run-001", "cancel-003"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"run-001\",\"disposition\":\"already_accepted\",\"event_sequence\":2}\n",
    );
    restarted.stop();

    let connection = Connection::open(&database_path).expect("journal must open");
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cancel_requests", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn concurrent_cancellation_has_one_winner() {
    let temp = TempDirectory::new("concurrent-cancel");
    let database_path = temp.database_path();
    let host = HostProcess::start(&database_path);
    assert_eq!(
        submit(host.port, "run-001", "idem-001").status_line,
        "HTTP/1.1 201 Created"
    );
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();
    for index in 0..4 {
        let barrier = Arc::clone(&barrier);
        let port = host.port;
        threads.push(thread::spawn(move || {
            barrier.wait();
            cancel(port, "run-001", &format!("cancel-{index}"))
        }));
    }
    barrier.wait();
    let responses = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        responses
            .iter()
            .filter(|response| response.status_line == "HTTP/1.1 202 Accepted")
            .count(),
        1
    );
    assert_eq!(
        responses
            .iter()
            .filter(|response| response.status_line == "HTTP/1.1 200 OK")
            .count(),
        3
    );
    host.stop();
    let connection = Connection::open(&database_path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cancel_requests", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'run.cancel_requested'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn terminal_absent_and_invalid_requests_are_mutation_free() {
    let temp = TempDirectory::new("negative-contracts");
    let database_path = temp.database_path();
    let host = HostProcess::start(&database_path);
    assert_eq!(
        submit(host.port, "run-001", "idem-001").status_line,
        "HTTP/1.1 201 Created"
    );
    host.stop();
    let connection = Connection::open(&database_path).unwrap();
    connection
        .execute(
            "UPDATE runs SET state = 'terminal' WHERE run_id = 'run-001'",
            [],
        )
        .unwrap();
    drop(connection);

    let host = HostProcess::start(&database_path);
    assert_response(
        cancel(host.port, "run-001", "cancel-terminal"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"run-001\",\"disposition\":\"terminal\",\"event_sequence\":1}\n",
    );
    assert_response(
        get(host.port, "/v1/runs/missing"),
        "HTTP/1.1 404 Not Found",
        "application/problem+json",
        NOT_FOUND_BODY,
    );
    assert_response(
        cancel(host.port, "missing", "cancel-001"),
        "HTTP/1.1 404 Not Found",
        "application/problem+json",
        NOT_FOUND_BODY,
    );
    for target in [
        "/v1/runs/run-001/events?after_sequence=0&limit=1",
        "/v1/runs/run-001/events?after=0&limit=0",
        "/v1/runs/run-001/events?after=0&limit=1&extra=1",
        "/v1/runs/run-001/events?after=%GG&limit=1",
    ] {
        assert_response(
            get(host.port, target),
            "HTTP/1.1 400 Bad Request",
            "application/problem+json",
            INVALID_READ_BODY,
        );
    }
    assert_response(
        request(
            host.port,
            "POST",
            "/v1/runs/run-001:cancel",
            &[("Content-Length", "0")],
            b"",
        ),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_CANCEL_BODY,
    );
    assert_response(
        request(
            host.port,
            "POST",
            "/v1/runs/run-001:cancel?x=1",
            &[("Idempotency-Key", "cancel-001"), ("Content-Length", "0")],
            b"",
        ),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_CANCEL_BODY,
    );
    assert_response(
        request(
            host.port,
            "POST",
            "/v1/runs/run-001:cancel",
            &[("Idempotency-Key", "cancel-001"), ("Content-Length", "1")],
            b"x",
        ),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_CANCEL_BODY,
    );
    assert_response(
        request(host.port, "DELETE", "/v1/runs/run-001", &[], b""),
        "HTTP/1.1 405 Method Not Allowed",
        "application/problem+json",
        METHOD_NOT_ALLOWED_BODY,
    );
    assert_response(
        get(host.port, "/v1/unknown"),
        "HTTP/1.1 404 Not Found",
        "application/problem+json",
        ENDPOINT_NOT_FOUND_BODY,
    );
    host.stop();

    let connection = Connection::open(&database_path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cancel_requests", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn version_one_database_migrates_without_changing_accepted_bytes() {
    let temp = TempDirectory::new("v1-migration");
    let database_path = temp.database_path();
    let connection = Connection::open(&database_path).unwrap();
    connection.execute_batch(
        "CREATE TABLE accepted_requests (run_id TEXT PRIMARY KEY NOT NULL, idempotency_key TEXT NOT NULL, request_digest TEXT NOT NULL, response_json BLOB NOT NULL, UNIQUE (run_id, idempotency_key));
         CREATE TABLE runs (run_id TEXT PRIMARY KEY NOT NULL, state TEXT NOT NULL);
         CREATE TABLE events (run_id TEXT NOT NULL, sequence INTEGER NOT NULL, event_type TEXT NOT NULL, event_json TEXT NOT NULL, PRIMARY KEY (run_id, sequence), FOREIGN KEY (run_id) REFERENCES runs(run_id));
         INSERT INTO accepted_requests VALUES ('run-001', 'idem-001', 'c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428', X'7B2272756E5F6964223A2272756E2D303031222C227374617465223A226163636570746564222C226576656E745F73657175656E6365223A317D0A');
         INSERT INTO runs VALUES ('run-001', 'accepted');
         INSERT INTO events VALUES ('run-001', 1, 'run.accepted', '{\"run_id\":\"run-001\",\"state\":\"accepted\"}');
         PRAGMA user_version = 1;"
    ).unwrap();
    drop(connection);

    let host = HostProcess::start(&database_path);
    assert_response(
        submit(host.port, "run-001", "idem-001"),
        "HTTP/1.1 200 OK",
        "application/json",
        CREATED_BODY,
    );
    host.stop();
    let connection = Connection::open(&database_path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row("SELECT response_json FROM accepted_requests", [], |row| row
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        CREATED_BODY
    );
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
