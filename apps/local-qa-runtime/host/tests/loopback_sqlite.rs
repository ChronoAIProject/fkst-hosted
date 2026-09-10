use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fkst_local_qa_host::{
    parse_startup, serve_mvp0_with_listener_for_test, serve_passive_with_listener_for_test,
    FixedClock, Journal, RunError, StartupConfig,
};
use fkst_qa_contracts::{admit_json, canonical_admitted_bytes, sha256_digest};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;

const CREATED_BODY: &[u8] =
    b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"accepted\",\"event_sequence\":1}\n";
const TERMINAL_BODY: &[u8] = b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"terminal\",\"execution_outcome\":\"blocked\",\"latest_event_sequence\":9}\n";
const LEGACY_CREATED_BODY: &[u8] =
    b"{\"run_id\":\"run-001\",\"state\":\"accepted\",\"event_sequence\":1}\n";
const LEGACY_ACCEPTED_SNAPSHOT_BODY: &[u8] =
    b"{\"run_id\":\"run-001\",\"state\":\"accepted\",\"latest_event_sequence\":1}\n";
const EVENTS_BODY: &[u8] = b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"after\":0,\"events\":[{\"sequence\":1,\"event_type\":\"run.accepted\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"accepted\"}},{\"sequence\":2,\"event_type\":\"run.state_changed\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"preparing\"}},{\"sequence\":3,\"event_type\":\"run.state_changed\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"ready\"}},{\"sequence\":4,\"event_type\":\"run.state_changed\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"executing\"}},{\"sequence\":5,\"event_type\":\"run.state_changed\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"staging_evidence\"}},{\"sequence\":6,\"event_type\":\"run.state_changed\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"cleaning_up_execution\"}},{\"sequence\":7,\"event_type\":\"run.state_changed\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"uploading\"}},{\"sequence\":8,\"event_type\":\"run.state_changed\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"finalizing_local\"}},{\"sequence\":9,\"event_type\":\"run.completed\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"terminal\",\"execution_outcome\":\"blocked\"}}],\"next_after\":9}\n";
const REQUEST_DIGEST: &str = "c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428";
const HEALTH_BODY: &[u8] =
    b"{\"service\":\"fkst-local-qa-host\",\"version\":\"0.0.0\",\"alive\":true}\n";
const NOT_FOUND_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Not Found\",\"status\":404,\"detail\":\"run not found\"}\n";
const INVALID_READ_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Bad Request\",\"status\":400,\"detail\":\"invalid read request\"}\n";
const INVALID_CANCEL_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Bad Request\",\"status\":400,\"detail\":\"invalid cancel request\"}\n";
const INVALID_SUBMIT_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Bad Request\",\"status\":400,\"detail\":\"invalid submit request\"}\n";
const METHOD_NOT_ALLOWED_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Method Not Allowed\",\"status\":405,\"detail\":\"method not allowed\"}\n";
const ENDPOINT_NOT_FOUND_BODY: &[u8] = b"{\"type\":\"about:blank\",\"title\":\"Not Found\",\"status\":404,\"detail\":\"endpoint not found\"}\n";

#[derive(Deserialize)]
struct AdmissionFixture {
    expected_request_utf8: String,
    expected_acceptance_utf8: String,
}

fn admission_fixture() -> AdmissionFixture {
    serde_json::from_str(include_str!(
        "../../../../packages/qa-contracts/fixtures/qa.local-run-admission/v2/happy-path.json"
    ))
    .expect("admission-v2 fixture must decode")
}

fn v2_request_with_key(body: &str, key: &str) -> Vec<u8> {
    let mut value: serde_json::Value =
        serde_json::from_str(body).expect("fixture request must decode");
    value["idempotency_key"] = key.into();
    value
        .as_object_mut()
        .expect("fixture request must be an object")
        .remove("content_digest");
    let projected = serde_json::to_vec(&value).expect("projected request must encode");
    let admitted = admit_json(&projected).expect("projected request must admit");
    value["content_digest"] =
        sha256_digest(&canonical_admitted_bytes(&admitted).expect("request must canonicalize"))
            .into();
    let admitted = admit_json(&serde_json::to_vec(&value).expect("request must encode"))
        .expect("request must admit");
    canonical_admitted_bytes(&admitted).expect("request must canonicalize")
}

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
        let signal_status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .expect("SIGTERM command must execute");
        assert!(signal_status.success(), "SIGTERM must be delivered");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().expect("host status must be readable") {
                assert!(
                    status.success(),
                    "host must shut down successfully: {status}"
                );
                return;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("host did not join its coordinator after SIGTERM");
            }
            thread::sleep(Duration::from_millis(10));
        }
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

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug)]
enum FixtureMode {
    Passive,
    FixedClock,
}

struct OwnedHost {
    shutdown: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<Result<(), RunError>>>,
    done: mpsc::Receiver<()>,
    ready: Option<mpsc::Receiver<Result<SocketAddr, String>>>,
    release: Option<mpsc::Sender<()>>,
    mode: FixtureMode,
    port: u16,
}

fn fixture_config(database_path: &Path, address: SocketAddr) -> StartupConfig {
    parse_startup([
        "local-demo".into(),
        "--listen".into(),
        address.to_string().into(),
        "--database".into(),
        database_path.as_os_str().to_owned(),
    ])
    .expect("fixture configuration must parse")
}

impl OwnedHost {
    fn spawn(database_path: &Path, mode: FixtureMode, gated: bool) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("loopback port must bind");
        let config = fixture_config(database_path, listener.local_addr().unwrap());
        Self::spawn_config(config, listener, mode, gated)
    }

    fn spawn_config(
        config: StartupConfig,
        listener: TcpListener,
        mode: FixtureMode,
        gated: bool,
    ) -> Self {
        let port = listener.local_addr().unwrap().port();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let (completed, done) = mpsc::channel();
        let (sender, ready) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let join = thread::spawn(move || {
            if gated && gate.recv_timeout(STARTUP_TIMEOUT).is_err() {
                return Err(RunError::Lifecycle("fixture start gate timed out"));
            }
            let result = match mode {
                FixtureMode::Passive => {
                    serve_passive_with_listener_for_test(config, thread_shutdown, listener, sender)
                }
                FixtureMode::FixedClock => serve_mvp0_with_listener_for_test(
                    config,
                    thread_shutdown,
                    Arc::new(FixedClock::new("2026-08-25T16:00:01Z").unwrap()),
                    listener,
                    sender,
                ),
            };
            let _ = completed.send(());
            result
        });
        Self {
            shutdown,
            join: Some(join),
            done,
            ready: Some(ready),
            release: Some(release),
            mode,
            port,
        }
    }

    fn release_start(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }

    fn wait_ready(mut self) -> Result<Self, String> {
        let receipt = self
            .ready
            .as_ref()
            .expect("startup receiver must exist")
            .recv_timeout(STARTUP_TIMEOUT);
        match receipt {
            Ok(Ok(address)) if address == SocketAddr::from(([127, 0, 0, 1], self.port)) => Ok(self),
            failure => {
                let result = self.finish();
                Err(format!(
                    "{:?} Host {} startup receipt: {failure:?}; thread result: {result:?}",
                    self.mode, self.port
                ))
            }
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        self.shutdown.store(true, Ordering::SeqCst);
        self.release_start();
        let Some(join) = self.join.as_ref() else {
            return Ok(());
        };
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let _ = self.done.recv_timeout(STARTUP_TIMEOUT);
        while !join.is_finished() {
            if Instant::now() >= deadline {
                // Rust cannot cancel synchronous initialization. Fail this test process
                // rather than detach a writer and delete its database during unwinding.
                eprintln!(
                    "{:?} Host {} cleanup deadline exceeded",
                    self.mode, self.port
                );
                std::process::abort();
            }
            thread::sleep(Duration::from_millis(1));
        }
        self.join
            .take()
            .unwrap()
            .join()
            .map_err(|_| "Host thread panicked".to_owned())?
            .map_err(|error| error.to_string())
    }

    fn stop(mut self) {
        self.finish().expect("fixture Host must stop");
    }
}

impl Drop for OwnedHost {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

struct PassiveHost;

impl PassiveHost {
    fn start(database_path: &Path) -> OwnedHost {
        OwnedHost::spawn(database_path, FixtureMode::Passive, false)
            .wait_ready()
            .unwrap_or_else(|error| panic!("passive Host startup failed: {error}"))
    }
}

struct FixedClockHost;

impl FixedClockHost {
    fn start(database_path: &Path) -> OwnedHost {
        OwnedHost::spawn(database_path, FixtureMode::FixedClock, false)
            .wait_ready()
            .unwrap_or_else(|error| panic!("fixed-clock Host startup failed: {error}"))
    }
}

#[test]
fn owned_startup_requires_initialized_host_receipt() {
    for mode in [FixtureMode::Passive, FixtureMode::FixedClock] {
        let temporary = TempDirectory::new("owned-startup");
        let database = temporary.database_path();
        let mut pending = OwnedHost::spawn(&database, mode, true);
        let address = SocketAddr::from(([127, 0, 0, 1], pending.port));
        let foreign = TcpListener::bind(address).ok();
        let port_stolen = foreign.is_some();
        let connected = TcpStream::connect(address).expect("bound port must be connectable");
        let premature_ready = pending.ready.as_ref().unwrap().try_recv();
        let database_before_release = database.exists();
        drop(connected);
        drop(foreign);
        pending.release_start();
        let host = pending
            .wait_ready()
            .unwrap_or_else(|error| panic!("startup failed: {error}"));
        let connection = Connection::open(&database).unwrap();
        let schema_present: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'accepted_requests')",
            [], |row| row.get(0),
        ).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        drop(connection);
        let response = request(host.port, "GET", "/v1/health", &[], &[]);
        host.stop();
        assert_eq!(response.status_line, "HTTP/1.1 200 OK");
        assert_eq!(response.body, HEALTH_BODY);
        assert_eq!(version, 9);
        assert_eq!(
            premature_ready,
            Err(mpsc::TryRecvError::Empty),
            "a connectable port must not signal intended Host readiness"
        );
        assert!(
            !port_stolen,
            "pending Host must retain ownership of its port"
        );
        assert!(
            !database_before_release,
            "gated Host must not have initialized Journal"
        );
        assert!(
            schema_present,
            "actual Host ready receipt must follow Journal migrations"
        );
    }
}

#[test]
fn owned_startup_reports_initialization_failure_and_releases_listener() {
    for mode in [FixtureMode::Passive, FixtureMode::FixedClock] {
        let temporary = TempDirectory::new("owned-init-error");
        let mut pending = OwnedHost::spawn(&temporary.path, mode, false);
        let address = SocketAddr::from(([127, 0, 0, 1], pending.port));
        assert_eq!(
            pending
                .ready
                .as_ref()
                .unwrap()
                .recv_timeout(STARTUP_TIMEOUT)
                .unwrap(),
            Err("host initialization failed".to_owned()),
        );
        let result = pending.finish();
        assert!(
            result.is_err(),
            "original initialization error must survive: {result:?}"
        );
        assert_eq!(
            pending.ready.as_ref().unwrap().try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        );
        TcpListener::bind(address).expect("failed startup must release listener");
    }
}

#[test]
fn owned_startup_rejects_listener_mismatch_before_journal_effects() {
    for mode in [FixtureMode::Passive, FixtureMode::FixedClock] {
        for mismatch in ["port", "ip", "nonloopback"] {
            let temporary = TempDirectory::new("owned-listener-error");
            let database = temporary.database_path();
            let ip = if mismatch == "nonloopback" {
                "0.0.0.0"
            } else {
                "127.0.0.1"
            };
            let listener = TcpListener::bind((ip, 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let configured = match mismatch {
                "port" => {
                    SocketAddr::from(([127, 0, 0, 1], if address.port() == 1 { 2 } else { 1 }))
                }
                "ip" => SocketAddr::new(std::net::Ipv6Addr::LOCALHOST.into(), address.port()),
                _ => SocketAddr::from(([127, 0, 0, 1], address.port())),
            };
            let config = fixture_config(&database, configured);
            let mut pending = OwnedHost::spawn_config(config, listener, mode, false);
            assert_eq!(
                pending
                    .ready
                    .as_ref()
                    .unwrap()
                    .recv_timeout(STARTUP_TIMEOUT)
                    .unwrap(),
                Err("host initialization failed".to_owned()),
            );
            let result = pending.finish();
            assert!(
                result
                    .as_ref()
                    .is_err_and(|error| error.contains("fixture listener mismatch")),
                "{result:?}"
            );
            assert!(
                !database.exists(),
                "{mismatch} must fail before opening Journal"
            );
            assert_eq!(
                pending.ready.as_ref().unwrap().try_recv(),
                Err(mpsc::TryRecvError::Disconnected)
            );
            TcpListener::bind(address).expect("rejected listener must be released");
        }
    }
}

#[test]
fn owned_startup_receiver_disconnect_stops_host_and_coordinator() {
    for mode in [FixtureMode::Passive, FixtureMode::FixedClock] {
        let temporary = TempDirectory::new("owned-receiver-error");
        let database = temporary.database_path();
        let mut pending = OwnedHost::spawn(&database, mode, true);
        let address = SocketAddr::from(([127, 0, 0, 1], pending.port));
        drop(pending.ready.take());
        pending.release_start();
        pending
            .done
            .recv_timeout(STARTUP_TIMEOUT)
            .expect("Host must exit without external shutdown");
        assert!(!pending.shutdown.load(Ordering::SeqCst));
        let result = pending.finish();
        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.contains("startup receiver disconnected")),
            "{result:?}"
        );
        TcpListener::bind(address).expect("disconnected receiver must release listener");
        Journal::open(&database).expect("initialized Journal must reopen after coordinator joins");
    }
}

#[test]
fn owned_startup_drop_releases_gate_and_joins_before_database_cleanup() {
    for mode in [FixtureMode::Passive, FixtureMode::FixedClock] {
        let temporary = TempDirectory::new("owned-gate-drop");
        let database = temporary.database_path();
        let pending = OwnedHost::spawn(&database, mode, true);
        let address = SocketAddr::from(([127, 0, 0, 1], pending.port));
        assert!(!database.exists());
        drop(pending);
        TcpListener::bind(address).expect("dropping gated Host must release listener");
        Journal::open(&database).expect("Host must finish initialization before fixture cleanup");
    }
}

struct HttpResponse {
    status_line: String,
    content_type: String,
    body: Vec<u8>,
}

fn connect_loopback(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => return stream,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Err(error) => panic!("loopback host must accept connections: {error}"),
        }
    }
}

fn request(
    port: u16,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> HttpResponse {
    let mut stream = connect_loopback(port);
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

fn submit_v2(port: u16, idempotency_key: &str, body: &[u8]) -> HttpResponse {
    let content_length = body.len().to_string();
    request(
        port,
        "PUT",
        "/v1/runs/00000000-0000-0000-0000-000000000002",
        &[
            ("Content-Type", "application/json"),
            ("Idempotency-Key", idempotency_key),
            ("Content-Length", &content_length),
        ],
        body,
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

fn wait_for_exact_get(port: u16, target: &str, body: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let response = get(port, target);
        if response.status_line == "HTTP/1.1 200 OK"
            && response.content_type == "application/json"
            && response.body == body
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "GET {target} did not reach the expected body; last status={} body={}",
            response.status_line,
            String::from_utf8_lossy(&response.body)
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn initialize_and_insert_unclaimed_run(database_path: &Path) {
    drop(Journal::open(database_path).expect("journal schema must initialize"));
    insert_unclaimed_run(database_path);
}

fn insert_unclaimed_run(database_path: &Path) {
    let connection = Connection::open(database_path).expect("journal must open");
    connection
        .execute_batch(
            "INSERT INTO accepted_requests VALUES (
                 '00000000-0000-0000-0000-000000000001',
                 'idem-001',
                 'c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428',
                 CAST('{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"accepted\",\"event_sequence\":1}' || char(10) AS BLOB)
             );
             INSERT INTO runs (run_id, executor_run_id, state, execution_outcome)
             VALUES (
                 '00000000-0000-0000-0000-000000000001',
                 '00000000-0000-0000-0000-000000000001',
                 'accepted',
                 NULL
             );
             INSERT INTO events VALUES (
                 '00000000-0000-0000-0000-000000000001', 1, 'run.accepted',
                 '{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"accepted\"}'
             );",
        )
        .expect("unclaimed Run fixture must be inserted");
}

fn submit_with_total_head_bytes(port: u16, total_head_bytes: usize, body: &[u8]) -> HttpResponse {
    let prefix = format!(
        "PUT /v1/runs/00000000-0000-0000-0000-000000000002 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nIdempotency-Key: idem_0002\r\nContent-Length: {}\r\nX-Fill: ",
        body.len()
    );
    let suffix = "\r\nConnection: close\r\n\r\n";
    assert!(
        prefix.len() + suffix.len() <= total_head_bytes,
        "requested head size must fit the bounded filler header"
    );
    let mut head = prefix.into_bytes();
    head.resize(total_head_bytes - suffix.len(), b'a');
    head.extend_from_slice(suffix.as_bytes());
    assert_eq!(head.len(), total_head_bytes);

    let mut stream = connect_loopback(port);
    stream
        .write_all(&head)
        .expect("bounded submit request head must be written");
    stream
        .write_all(body)
        .expect("bounded submit request body must be written");
    stream.flush().expect("bounded submit must be flushed");
    read_response(&mut stream)
}

fn assert_empty_journal(database_path: &Path) {
    let connection = Connection::open(database_path).expect("journal must open after host exit");
    for table in [
        "accepted_requests",
        "runs",
        "events",
        "cancel_requests",
        "execution_attempts",
        "admission_v2_records",
        "active_run_slot",
        "cancellation_controls",
        "effect_admissions",
    ] {
        let count = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("journal row count must be readable");
        assert_eq!(count, 0, "{table} must remain empty");
    }
}

#[test]
fn submit_accepts_exact_maximum_total_head_bytes() {
    let temp = TempDirectory::new("maximum-submit-head");
    let database_path = temp.database_path();
    let host = FixedClockHost::start(&database_path);
    let fixture = admission_fixture();
    let expected_body = format!("{}\n", fixture.expected_acceptance_utf8);
    assert_response(
        submit_with_total_head_bytes(host.port, 16_384, fixture.expected_request_utf8.as_bytes()),
        "HTTP/1.1 201 Created",
        "application/json",
        expected_body.as_bytes(),
    );
    host.stop();
    let connection = Connection::open(&database_path).expect("journal must open");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM admission_v2_records", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
}

#[test]
fn submit_rejects_total_head_one_byte_over_limit_without_mutation() {
    let temp = TempDirectory::new("oversized-submit-head");
    let database_path = temp.database_path();
    let host = FixedClockHost::start(&database_path);
    let fixture = admission_fixture();
    assert_response(
        submit_with_total_head_bytes(host.port, 16_385, fixture.expected_request_utf8.as_bytes()),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_SUBMIT_BODY,
    );
    host.stop();
    assert_empty_journal(&database_path);
}

#[test]
fn truncated_submit_body_returns_complete_error_without_mutation() {
    let temp = TempDirectory::new("truncated-submit-body");
    let database_path = temp.database_path();
    let host = HostProcess::start(&database_path);
    let mut stream = TcpStream::connect(("127.0.0.1", host.port))
        .expect("loopback host must accept connections");
    write!(
        stream,
        "PUT /v1/runs/00000000-0000-0000-0000-000000000001 HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nIdempotency-Key: idem-001\r\nContent-Length: 16\r\nConnection: close\r\n\r\n",
        host.port
    )
    .expect("truncated submit request head must be written");
    stream
        .write_all(b"{\"kind\":\"inert\"")
        .expect("partial submit request body must be written");
    stream
        .shutdown(Shutdown::Write)
        .expect("submit request write side must close");
    assert_response(
        read_response(&mut stream),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_SUBMIT_BODY,
    );
    host.stop();
    assert_empty_journal(&database_path);
}

#[test]
fn delayed_nonempty_cancel_body_returns_complete_error_without_mutation() {
    let temp = TempDirectory::new("delayed-cancel-body");
    let database_path = temp.database_path();
    let host = PassiveHost::start(&database_path);
    insert_unclaimed_run(&database_path);

    let mut stream = TcpStream::connect(("127.0.0.1", host.port))
        .expect("loopback host must accept connections");
    write!(
        stream,
        "POST /v1/runs/00000000-0000-0000-0000-000000000001:cancel HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nIdempotency-Key: cancel-001\r\nContent-Length: 1\r\nConnection: close\r\n\r\n",
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
        "POST /v1/runs/00000000-0000-0000-0000-000000000001:cancel HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nIdempotency-Key: cancel-001\r\nContent-Length: 65\r\nConnection: close\r\n\r\n",
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
fn exact_submission_completes_replays_and_restarts_without_duplicate_work() {
    let temp = TempDirectory::new("complete-replay-restart");
    let database_path = temp.database_path();

    initialize_and_insert_unclaimed_run(&database_path);
    let first_host = HostProcess::start(&database_path);
    wait_for_exact_get(
        first_host.port,
        "/v1/runs/00000000-0000-0000-0000-000000000001",
        TERMINAL_BODY,
    );
    assert_response(
        get(
            first_host.port,
            "/v1/runs/00000000-0000-0000-0000-000000000001/events?after=0&limit=100",
        ),
        "HTTP/1.1 200 OK",
        "application/json",
        EVENTS_BODY,
    );
    first_host.stop();
    assert_exact_journal(&database_path, "idem-001");

    let restarted_host = HostProcess::start(&database_path);
    assert_response(
        get(
            restarted_host.port,
            "/v1/runs/00000000-0000-0000-0000-000000000001",
        ),
        "HTTP/1.1 200 OK",
        "application/json",
        TERMINAL_BODY,
    );
    assert_response(
        get(
            restarted_host.port,
            "/v1/runs/00000000-0000-0000-0000-000000000001/events?after=0&limit=100",
        ),
        "HTTP/1.1 200 OK",
        "application/json",
        EVENTS_BODY,
    );
    restarted_host.stop();
    assert_exact_journal(&database_path, "idem-001");
}

#[test]
fn concurrent_different_keys_create_exactly_one_acceptance() {
    let temp = TempDirectory::new("concurrent-keys");
    let database_path = temp.database_path();
    let host = FixedClockHost::start(&database_path);
    let fixture = admission_fixture();
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for key in ["idem_0002", "idem_other"] {
        let barrier = Arc::clone(&barrier);
        let port = host.port;
        let body = v2_request_with_key(&fixture.expected_request_utf8, key);
        threads.push(thread::spawn(move || {
            barrier.wait();
            (key, submit_v2(port, key, &body))
        }));
    }
    barrier.wait();

    let mut results = threads
        .into_iter()
        .map(|thread| thread.join().expect("submit thread must complete"))
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.1.status_line.cmp(&right.1.status_line));
    assert_eq!(results[0].1.status_line, "HTTP/1.1 201 Created");
    assert_eq!(results[1].1.status_line, "HTTP/1.1 409 Conflict");
    let accepted_key = results[0].0;
    host.stop();

    let connection = Connection::open(&database_path).expect("journal must open");
    assert_eq!(
        connection
            .query_row("SELECT idempotency_key FROM accepted_requests", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        accepted_key
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM active_run_slot", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
}

#[test]
fn reads_cancellation_and_restart_match_the_durable_contract() {
    let temp = TempDirectory::new("reads-cancel-restart");
    let database_path = temp.database_path();
    let host = PassiveHost::start(&database_path);

    assert_response(
        get(host.port, "/v1/health"),
        "HTTP/1.1 200 OK",
        "application/json",
        HEALTH_BODY,
    );
    insert_unclaimed_run(&database_path);
    let executor_run_id_before_restart = Connection::open(&database_path)
        .expect("journal must open")
        .query_row(
            "SELECT executor_run_id FROM runs WHERE run_id = '00000000-0000-0000-0000-000000000001'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("executor run ID must be present");
    fkst_qa_contracts::validate_scalar("UUID", &executor_run_id_before_restart).unwrap();
    assert_response(
        get(host.port, "/v1/runs/00000000-0000-0000-0000-000000000001"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"accepted\",\"latest_event_sequence\":1}\n",
    );
    assert_response(
        get(host.port, "/v1/runs/00000000-0000-0000-0000-000000000001/events?after=0&limit=100"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"after\":0,\"events\":[{\"sequence\":1,\"event_type\":\"run.accepted\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"accepted\"}}],\"next_after\":1}\n",
    );
    assert_response(
        get(host.port, "/v1/runs/00000000-0000-0000-0000-000000000001/events?limit=1&after=1"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"after\":1,\"events\":[],\"next_after\":1}\n",
    );
    assert_response(
        cancel(host.port, "00000000-0000-0000-0000-000000000001", "cancel-001"),
        "HTTP/1.1 202 Accepted",
        "application/json",
        b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"disposition\":\"accepted\",\"event_sequence\":2}\n",
    );
    assert_response(
        cancel(host.port, "00000000-0000-0000-0000-000000000001", "cancel-002"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"disposition\":\"already_accepted\",\"event_sequence\":2}\n",
    );
    assert_response(
        get(host.port, "/v1/runs/00000000-0000-0000-0000-000000000001/events?after=1&limit=1"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"after\":1,\"events\":[{\"sequence\":2,\"event_type\":\"run.cancel_requested\",\"event\":{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"accepted\"}}],\"next_after\":2}\n",
    );
    host.stop();

    let restarted = PassiveHost::start(&database_path);
    assert_response(
        get(restarted.port, "/v1/runs/00000000-0000-0000-0000-000000000001"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"accepted\",\"latest_event_sequence\":2}\n",
    );
    assert_response(
        cancel(restarted.port, "00000000-0000-0000-0000-000000000001", "cancel-003"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"disposition\":\"already_accepted\",\"event_sequence\":2}\n",
    );
    restarted.stop();

    let connection = Connection::open(&database_path).expect("journal must open");
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT executor_run_id FROM runs WHERE run_id = '00000000-0000-0000-0000-000000000001'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        executor_run_id_before_restart
    );
    assert_eq!(
        connection
            .query_row(
                r#"SELECT "notnull" FROM pragma_table_info('runs') WHERE name = 'executor_run_id'"#,
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                r#"SELECT COUNT(*) FROM pragma_index_list('runs') WHERE name = 'runs_executor_run_id_unique' AND "unique" = 1"#,
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
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
    insert_unclaimed_run(&database_path);
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();
    for index in 0..4 {
        let barrier = Arc::clone(&barrier);
        let port = host.port;
        threads.push(thread::spawn(move || {
            barrier.wait();
            cancel(
                port,
                "00000000-0000-0000-0000-000000000001",
                &format!("cancel-{index}"),
            )
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
fn terminal_and_invalid_requests_are_mutation_free() {
    let temp = TempDirectory::new("negative-contracts");
    let database_path = temp.database_path();
    initialize_and_insert_unclaimed_run(&database_path);
    let host = HostProcess::start(&database_path);
    assert_response(
        submit(host.port, "run-001", "idem-legacy"),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_SUBMIT_BODY,
    );
    assert_eq!(
        get(host.port, "/v1/runs/00000000-0000-0000-0000-000000000001").status_line,
        "HTTP/1.1 200 OK"
    );
    wait_for_exact_get(
        host.port,
        "/v1/runs/00000000-0000-0000-0000-000000000001",
        TERMINAL_BODY,
    );
    assert_response(
        cancel(host.port, "00000000-0000-0000-0000-000000000001", "cancel-terminal"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"disposition\":\"terminal\",\"event_sequence\":9}\n",
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
        "/v1/runs/00000000-0000-0000-0000-000000000001/events?after_sequence=0&limit=1",
        "/v1/runs/00000000-0000-0000-0000-000000000001/events?after=0&limit=0",
        "/v1/runs/00000000-0000-0000-0000-000000000001/events?after=0&limit=1&extra=1",
        "/v1/runs/00000000-0000-0000-0000-000000000001/events?after=%GG&limit=1",
        "/v1/runs/00000000-0000-0000-0000-000000000001/events?after=9007199254740992&limit=1",
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
            "/v1/runs/00000000-0000-0000-0000-000000000001:cancel",
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
            "/v1/runs/00000000-0000-0000-0000-000000000001:cancel?x=1",
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
            "/v1/runs/00000000-0000-0000-0000-000000000001:cancel",
            &[("Idempotency-Key", "cancel-001"), ("Content-Length", "1")],
            b"x",
        ),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_CANCEL_BODY,
    );
    assert_response(
        request(
            host.port,
            "DELETE",
            "/v1/runs/00000000-0000-0000-0000-000000000001",
            &[],
            b"",
        ),
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
        9
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

    let host = PassiveHost::start(&database_path);
    assert_response(
        submit(host.port, "run-001", "idem-001"),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_SUBMIT_BODY,
    );
    assert_response(
        get(host.port, "/v1/runs/run-001"),
        "HTTP/1.1 200 OK",
        "application/json",
        LEGACY_ACCEPTED_SNAPSHOT_BODY,
    );
    let executor_run_id = Connection::open(&database_path)
        .unwrap()
        .query_row(
            "SELECT executor_run_id FROM runs WHERE run_id = 'run-001'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    fkst_qa_contracts::validate_scalar("UUID", &executor_run_id).unwrap();
    assert_ne!(executor_run_id, "run-001");
    host.stop();
    let restarted = PassiveHost::start(&database_path);
    restarted.stop();
    let connection = Connection::open(&database_path).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT executor_run_id FROM runs WHERE run_id = 'run-001'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        executor_run_id
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert_eq!(
        connection
            .query_row("SELECT response_json FROM accepted_requests", [], |row| row
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        LEGACY_CREATED_BODY
    );
}

#[test]
fn version_two_database_migrates_without_rewriting_durable_data() {
    let temp = TempDirectory::new("v2-migration");
    let database_path = temp.database_path();
    let connection = Connection::open(&database_path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE accepted_requests (run_id TEXT PRIMARY KEY NOT NULL, idempotency_key TEXT NOT NULL, request_digest TEXT NOT NULL, response_json BLOB NOT NULL, UNIQUE (run_id, idempotency_key));
             CREATE TABLE runs (run_id TEXT PRIMARY KEY NOT NULL, state TEXT NOT NULL);
             CREATE TABLE events (run_id TEXT NOT NULL, sequence INTEGER NOT NULL, event_type TEXT NOT NULL, event_json TEXT NOT NULL, PRIMARY KEY (run_id, sequence), FOREIGN KEY (run_id) REFERENCES runs(run_id));
             CREATE TABLE cancel_requests (run_id TEXT PRIMARY KEY NOT NULL, idempotency_key TEXT NOT NULL, event_sequence INTEGER NOT NULL, FOREIGN KEY (run_id) REFERENCES runs(run_id));
             INSERT INTO accepted_requests VALUES ('run-001', 'idem-001', 'c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428', X'7B2272756E5F6964223A2272756E2D303031222C227374617465223A226163636570746564222C226576656E745F73657175656E6365223A317D0A');
             INSERT INTO runs VALUES ('run-001', 'accepted');
             INSERT INTO events VALUES ('run-001', 1, 'run.accepted', '{\"run_id\":\"run-001\",\"state\":\"accepted\"}');
             INSERT INTO events VALUES ('run-001', 2, 'run.cancel_requested', '{\"run_id\":\"run-001\",\"state\":\"accepted\"}');
             INSERT INTO cancel_requests VALUES ('run-001', 'cancel-001', 2);
             PRAGMA user_version = 2;",
        )
        .unwrap();
    drop(connection);

    let host = HostProcess::start(&database_path);
    assert_response(
        submit(host.port, "run-001", "idem-001"),
        "HTTP/1.1 400 Bad Request",
        "application/problem+json",
        INVALID_SUBMIT_BODY,
    );
    assert_response(
        get(host.port, "/v1/runs/run-001"),
        "HTTP/1.1 200 OK",
        "application/json",
        b"{\"run_id\":\"run-001\",\"state\":\"accepted\",\"latest_event_sequence\":2}\n",
    );
    host.stop();

    let connection = Connection::open(&database_path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT run_id, idempotency_key, request_digest, response_json FROM accepted_requests",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Vec<u8>>(3)?))
            )
            .unwrap(),
        (
            "run-001".to_owned(),
            "idem-001".to_owned(),
            REQUEST_DIGEST.to_owned(),
            LEGACY_CREATED_BODY.to_vec(),
        )
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT run_id, state, execution_outcome FROM runs",
                [],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?
                ))
            )
            .unwrap(),
        ("run-001".to_owned(), "accepted".to_owned(), None)
    );
    let events = connection
        .prepare("SELECT run_id, sequence, event_type, event_json FROM events ORDER BY sequence")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        events,
        vec![
            (
                "run-001".to_owned(),
                1,
                "run.accepted".to_owned(),
                "{\"run_id\":\"run-001\",\"state\":\"accepted\"}".to_owned(),
            ),
            (
                "run-001".to_owned(),
                2,
                "run.cancel_requested".to_owned(),
                "{\"run_id\":\"run-001\",\"state\":\"accepted\"}".to_owned(),
            ),
        ]
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT run_id, idempotency_key, event_sequence FROM cancel_requests",
                [],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?
                ))
            )
            .unwrap(),
        ("run-001".to_owned(), "cancel-001".to_owned(), 2)
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM execution_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
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
            "00000000-0000-0000-0000-000000000001".to_owned(),
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
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("journal version must be readable"),
        9
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT run_id, executor_run_id, state, execution_outcome FROM runs",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                }
            )
            .expect("Run row must be readable"),
        (
            "00000000-0000-0000-0000-000000000001".to_owned(),
            "00000000-0000-0000-0000-000000000001".to_owned(),
            "terminal".to_owned(),
            Some("blocked".to_owned()),
        )
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT run_id, status, execution_outcome FROM execution_attempts",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                }
            )
            .expect("attempt row must be readable"),
        (
            "00000000-0000-0000-0000-000000000001".to_owned(),
            "completed".to_owned(),
            Some("blocked".to_owned()),
        )
    );
    let mut statement = connection
        .prepare("SELECT sequence, event_type FROM events ORDER BY sequence")
        .expect("Event query must prepare");
    let events = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .expect("Events must be queryable")
        .collect::<Result<Vec<_>, _>>()
        .expect("Events must be readable");
    assert_eq!(
        events,
        vec![
            (1, "run.accepted".to_owned()),
            (2, "run.state_changed".to_owned()),
            (3, "run.state_changed".to_owned()),
            (4, "run.state_changed".to_owned()),
            (5, "run.state_changed".to_owned()),
            (6, "run.state_changed".to_owned()),
            (7, "run.state_changed".to_owned()),
            (8, "run.state_changed".to_owned()),
            (9, "run.completed".to_owned()),
        ]
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT event_json FROM events WHERE sequence = 9",
                [],
                |row| row.get::<_, String>(0)
            )
            .expect("completed Event must be readable"),
        "{\"run_id\":\"00000000-0000-0000-0000-000000000001\",\"state\":\"terminal\",\"execution_outcome\":\"blocked\"}"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM cancel_requests", [], |row| row
                .get::<_, i64>(0))
            .expect("cancellation count must be readable"),
        0
    );
}
