mod coordinator;
mod executor;
mod journal;
pub mod ownership;

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use coordinator::CoordinatorHandle;
use executor::PassingExecutor;
use fkst_qa_contracts::{validate_cancel_disposition, validate_event_cursor};
use journal::{Admission, Cancellation, EventPayload};
pub use journal::{Journal, OwnedHandle, ResourceIntent};
pub use ownership::{
    reconcile_environment, Clock, CreateRequest, EnvironmentProvider, EnvironmentRequest,
    FixedClock, ProviderResource, SystemClock,
};
use serde::{Deserialize, Serialize};

const CANONICAL_REQUEST_DIGEST: &str =
    "c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428";
const MAX_BODY_BYTES: usize = 64;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_REJECTION_DRAIN_BYTES: usize = MAX_HEADER_BYTES + MAX_BODY_BYTES;
const REJECTION_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupConfig {
    listen: SocketAddr,
    database_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupError {
    NoSupportedConfiguration,
}

impl fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSupportedConfiguration => formatter.write_str("no supported configuration"),
        }
    }
}

#[derive(Debug)]
pub enum RunError {
    ActiveAttempt,
    Contract(&'static str),
    CoordinatorPanicked,
    CoordinatorStopped,
    Database(rusqlite::Error),
    InvalidJournal(&'static str),
    Io(io::Error),
    JournalMode(String),
    ShutdownHandler(ctrlc::Error),
    UnsupportedDatabaseVersion(i64),
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActiveAttempt => {
                formatter.write_str("cancellation during execution is unsupported")
            }
            Self::Contract(detail) => write!(formatter, "contract error: {detail}"),
            Self::CoordinatorPanicked => formatter.write_str("Run coordinator panicked"),
            Self::CoordinatorStopped => formatter.write_str("Run coordinator stopped unexpectedly"),
            Self::Database(error) => write!(formatter, "database error: {error}"),
            Self::InvalidJournal(detail) => write!(formatter, "invalid journal: {detail}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::JournalMode(mode) => write!(formatter, "SQLite WAL mode unavailable: {mode}"),
            Self::ShutdownHandler(error) => {
                write!(formatter, "shutdown handler error: {error}")
            }
            Self::UnsupportedDatabaseVersion(version) => {
                write!(formatter, "unsupported database version: {version}")
            }
        }
    }
}

impl From<io::Error> for RunError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for RunError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<ctrlc::Error> for RunError {
    fn from(error: ctrlc::Error) -> Self {
        Self::ShutdownHandler(error)
    }
}

pub fn parse_startup<I>(arguments: I) -> Result<StartupConfig, StartupError>
where
    I: IntoIterator<Item = OsString>,
{
    let arguments: Vec<OsString> = arguments.into_iter().collect();
    if arguments.len() != 5 || arguments[0].to_str() != Some("local-demo") {
        return Err(StartupError::NoSupportedConfiguration);
    }

    let mut listen = None;
    let mut database_path = None;
    let mut index = 1;
    while index < arguments.len() {
        let option = arguments[index]
            .to_str()
            .ok_or(StartupError::NoSupportedConfiguration)?;
        let value = &arguments[index + 1];
        if value.is_empty() {
            return Err(StartupError::NoSupportedConfiguration);
        }

        match option {
            "--listen" if listen.is_none() => {
                let value = value
                    .to_str()
                    .ok_or(StartupError::NoSupportedConfiguration)?;
                listen = Some(parse_loopback_address(value)?);
            }
            "--database" if database_path.is_none() => {
                database_path = Some(PathBuf::from(value));
            }
            _ => return Err(StartupError::NoSupportedConfiguration),
        }
        index += 2;
    }

    Ok(StartupConfig {
        listen: listen.ok_or(StartupError::NoSupportedConfiguration)?,
        database_path: database_path.ok_or(StartupError::NoSupportedConfiguration)?,
    })
}

fn parse_loopback_address(value: &str) -> Result<SocketAddr, StartupError> {
    if let Some(port) = value.strip_prefix("127.0.0.1:") {
        return Ok(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            parse_port(port)?,
        ));
    }
    if let Some(port) = value.strip_prefix("[::1]:") {
        return Ok(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            parse_port(port)?,
        ));
    }
    Err(StartupError::NoSupportedConfiguration)
}

fn parse_port(value: &str) -> Result<u16, StartupError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(StartupError::NoSupportedConfiguration);
    }
    value
        .parse()
        .map_err(|_| StartupError::NoSupportedConfiguration)
}

pub fn run(config: StartupConfig) -> Result<(), RunError> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let signal_shutdown = Arc::clone(&shutdown);
    ctrlc::set_handler(move || signal_shutdown.store(true, Ordering::SeqCst))?;
    serve(config, shutdown)
}

fn serve(config: StartupConfig, shutdown: Arc<AtomicBool>) -> Result<(), RunError> {
    let mut journal = Journal::open(&config.database_path)?;
    let listener = TcpListener::bind(config.listen)?;
    listener.set_nonblocking(true)?;
    let assigned_address = listener.local_addr()?;
    let mut coordinator =
        CoordinatorHandle::start(&config.database_path, Box::new(PassingExecutor))?;

    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "fkst-local-qa-host: listening on {assigned_address}"
    )?;
    stdout.flush()?;
    drop(stdout);

    let serve_result = loop {
        if shutdown.load(Ordering::SeqCst) {
            break Ok(());
        }
        if let Err(error) = coordinator.check() {
            break Err(error);
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                let _ = handle_connection(&mut stream, &mut journal, &coordinator);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => break Err(RunError::Io(error)),
        }
    };
    let shutdown_result = coordinator.shutdown();
    serve_result.and(shutdown_result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitBody {
    kind: String,
}

#[derive(Serialize)]
struct HealthResponse<'a> {
    service: &'a str,
    version: &'a str,
    alive: bool,
}

#[derive(Serialize)]
struct SnapshotResponse<'a> {
    run_id: &'a str,
    state: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_outcome: Option<&'a str>,
    latest_event_sequence: i64,
}

#[derive(Serialize)]
struct EventResponse {
    sequence: i64,
    event_type: String,
    event: EventPayload,
}

#[derive(Serialize)]
struct EventsResponse<'a> {
    run_id: &'a str,
    after: i64,
    events: Vec<EventResponse>,
    next_after: i64,
}

#[derive(Serialize)]
struct CancelResponse<'a> {
    run_id: &'a str,
    disposition: &'a str,
    event_sequence: i64,
}

enum Route {
    Health,
    Submit {
        run_id: String,
    },
    Snapshot {
        run_id: String,
    },
    Events {
        run_id: String,
        after: i64,
        limit: i64,
    },
    Cancel {
        run_id: String,
    },
}

struct Request {
    route: Route,
    idempotency_key: Option<String>,
    body: Vec<u8>,
}

fn handle_connection(
    stream: &mut TcpStream,
    journal: &mut Journal,
    coordinator: &CoordinatorHandle,
) -> io::Result<()> {
    let request = match read_request(stream)? {
        Ok(request) => request,
        Err(response) => return write_rejection_response(stream, response),
    };

    let response = match request.route {
        Route::Health => json_response(
            200,
            "OK",
            &HealthResponse {
                service: "fkst-local-qa-host",
                version: env!("CARGO_PKG_VERSION"),
                alive: true,
            },
        ),
        Route::Submit { run_id } => handle_submit(
            journal,
            coordinator,
            &run_id,
            request.idempotency_key.as_deref().unwrap_or_default(),
            &request.body,
        ),
        Route::Snapshot { run_id } => match journal.snapshot(&run_id) {
            Ok(Some(snapshot)) => json_response(
                200,
                "OK",
                &SnapshotResponse {
                    run_id: &run_id,
                    state: &snapshot.state,
                    execution_outcome: snapshot.execution_outcome.as_deref(),
                    latest_event_sequence: snapshot.latest_event_sequence,
                },
            ),
            Ok(None) => run_not_found(),
            Err(_) => journal_failure(),
        },
        Route::Events {
            run_id,
            after,
            limit,
        } => match journal.events(&run_id, after, limit) {
            Ok(Some(events)) => {
                let events = events
                    .into_iter()
                    .map(|event| EventResponse {
                        sequence: event.sequence,
                        event_type: event.event_type,
                        event: event.event,
                    })
                    .collect::<Vec<_>>();
                let next_after = events.last().map_or(after, |event| event.sequence);
                json_response(
                    200,
                    "OK",
                    &EventsResponse {
                        run_id: &run_id,
                        after,
                        events,
                        next_after,
                    },
                )
            }
            Ok(None) => run_not_found(),
            Err(_) => journal_failure(),
        },
        Route::Cancel { run_id } => match journal.cancel(
            &run_id,
            request.idempotency_key.as_deref().unwrap_or_default(),
        ) {
            Ok(Cancellation::Accepted(sequence)) => {
                cancel_response(202, "Accepted", &run_id, "accepted", sequence)
            }
            Ok(Cancellation::AlreadyAccepted(sequence)) => {
                cancel_response(200, "OK", &run_id, "already_accepted", sequence)
            }
            Ok(Cancellation::Terminal(sequence)) => {
                cancel_response(200, "OK", &run_id, "terminal", sequence)
            }
            Ok(Cancellation::NotFound) => run_not_found(),
            Err(_) => journal_failure(),
        },
    };
    write_response(stream, response)
}

fn handle_submit(
    journal: &mut Journal,
    coordinator: &CoordinatorHandle,
    run_id: &str,
    idempotency_key: &str,
    body: &[u8],
) -> Response {
    let valid_body =
        serde_json::from_slice::<SubmitBody>(body).is_ok_and(|body| body.kind == "inert");
    if !valid_body {
        return problem_response(400, "Bad Request", "invalid submit request");
    }

    match journal.admit(run_id, idempotency_key, CANONICAL_REQUEST_DIGEST) {
        Ok(Admission::Created(body)) => {
            coordinator.wake();
            Response::new(201, "Created", "application/json", body)
        }
        Ok(Admission::Replay(body)) => {
            coordinator.wake();
            Response::new(200, "OK", "application/json", body)
        }
        Ok(Admission::DifferentKey) => problem_response(
            409,
            "Conflict",
            "run_id is already accepted under a different Idempotency-Key",
        ),
        Err(_) => journal_failure(),
    }
}

fn read_request(stream: &mut TcpStream) -> io::Result<Result<Request, Response>> {
    let mut received = Vec::new();
    let header_end = loop {
        if let Some(position) = find_header_end(&received) {
            if position + 4 > MAX_HEADER_BYTES {
                return Ok(Err(oversized_header_response(&received)));
            }
            break position;
        }
        if received.len() >= MAX_HEADER_BYTES {
            return Ok(Err(oversized_header_response(&received)));
        }
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(Err(problem_response(
                400,
                "Bad Request",
                "invalid read request",
            )));
        }
        received.extend_from_slice(&chunk[..read]);
    };

    let header_bytes = &received[..header_end];
    let header_text = match std::str::from_utf8(header_bytes) {
        Ok(value) => value,
        Err(_) => {
            return Ok(Err(problem_response(
                400,
                "Bad Request",
                "invalid read request",
            )))
        }
    };
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split(' ');
    let method = request_parts.next().unwrap_or_default();
    let target = request_parts.next().unwrap_or_default();
    let version = request_parts.next().unwrap_or_default();
    if request_parts.next().is_some() || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Ok(Err(problem_response(
            400,
            "Bad Request",
            "invalid read request",
        )));
    }
    let route = match classify_route(method, target) {
        Ok(route) => route,
        Err(response) => return Ok(Err(response)),
    };

    let mut content_types = Vec::new();
    let mut idempotency_keys = Vec::new();
    let mut content_lengths = Vec::new();
    let mut has_transfer_encoding = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Ok(Err(invalid_request(&route)));
        };
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Ok(Err(invalid_request(&route)));
        }
        let value = value.trim_matches([' ', '\t']);
        if name.eq_ignore_ascii_case("content-type") {
            content_types.push(value);
        } else if name.eq_ignore_ascii_case("idempotency-key") {
            idempotency_keys.push(value);
        } else if name.eq_ignore_ascii_case("content-length") {
            content_lengths.push(value);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            has_transfer_encoding = true;
        }
    }

    let idempotency_key = match &route {
        Route::Submit { .. } => {
            if content_types.len() != 1 || !valid_content_type(content_types[0]) {
                return Ok(Err(problem_response(
                    415,
                    "Unsupported Media Type",
                    "Content-Type must be application/json",
                )));
            }
            if idempotency_keys.len() != 1 || !valid_idempotency_key(idempotency_keys[0]) {
                return Ok(Err(invalid_request(&route)));
            }
            Some(idempotency_keys[0].to_owned())
        }
        Route::Cancel { .. } => {
            if idempotency_keys.len() != 1 || !valid_idempotency_key(idempotency_keys[0]) {
                return Ok(Err(invalid_request(&route)));
            }
            Some(idempotency_keys[0].to_owned())
        }
        _ => None,
    };

    if has_transfer_encoding || content_lengths.len() > 1 {
        return Ok(Err(invalid_request(&route)));
    }
    let content_length = match content_lengths.first() {
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            match value.parse::<usize>() {
                Ok(value) => value,
                Err(_) => return Ok(Err(invalid_request(&route))),
            }
        }
        Some(_) => return Ok(Err(invalid_request(&route))),
        None => 0,
    };
    if matches!(route, Route::Submit { .. }) && content_lengths.len() != 1 {
        return Ok(Err(invalid_request(&route)));
    }
    if content_length > MAX_BODY_BYTES {
        let response = if matches!(route, Route::Submit { .. }) {
            problem_response(413, "Payload Too Large", "request body exceeds 64 bytes")
        } else {
            invalid_request(&route)
        };
        return Ok(Err(response));
    }

    let body_start = header_end + 4;
    let mut body = received[body_start..].to_vec();
    if body.len() > content_length {
        return Ok(Err(invalid_request(&route)));
    }
    if body.len() < content_length {
        let mut remaining = vec![0_u8; content_length - body.len()];
        if let Err(error) = stream.read_exact(&mut remaining) {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(Err(invalid_request(&route)));
            }
            return Err(error);
        }
        body.extend_from_slice(&remaining);
    }
    if !matches!(route, Route::Submit { .. }) && content_length != 0 {
        return Ok(Err(invalid_request(&route)));
    }

    Ok(Ok(Request {
        route,
        idempotency_key,
        body,
    }))
}

fn classify_route(method: &str, target: &str) -> Result<Route, Response> {
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (target, None),
    };

    if path == "/v1/health" {
        if method != "GET" {
            return Err(method_not_allowed());
        }
        if query.is_some() {
            return Err(invalid_read());
        }
        return Ok(Route::Health);
    }

    let Some(remainder) = path.strip_prefix("/v1/runs/") else {
        return Err(endpoint_not_found());
    };
    if let Some(run_id) = remainder.strip_suffix(":cancel") {
        if method != "POST" {
            return Err(method_not_allowed());
        }
        if query.is_some() || !valid_run_id(run_id) {
            return Err(invalid_cancel());
        }
        return Ok(Route::Cancel {
            run_id: run_id.to_owned(),
        });
    }
    if let Some(run_id) = remainder.strip_suffix("/events") {
        if method != "GET" {
            return Err(method_not_allowed());
        }
        if !valid_run_id(run_id) {
            return Err(invalid_read());
        }
        let Some(query) = query else {
            return Err(invalid_read());
        };
        let Some((after, limit)) = parse_event_query(query) else {
            return Err(invalid_read());
        };
        return Ok(Route::Events {
            run_id: run_id.to_owned(),
            after,
            limit,
        });
    }

    if remainder.contains('/') {
        return Err(endpoint_not_found());
    }

    match method {
        "PUT" => {
            if query.is_some() || !valid_run_id(remainder) {
                return Err(problem_response(
                    400,
                    "Bad Request",
                    "invalid submit request",
                ));
            }
            Ok(Route::Submit {
                run_id: remainder.to_owned(),
            })
        }
        "GET" => {
            if query.is_some() || !valid_run_id(remainder) {
                return Err(invalid_read());
            }
            Ok(Route::Snapshot {
                run_id: remainder.to_owned(),
            })
        }
        _ if !remainder.contains('/') && valid_run_id(remainder) && query.is_none() => {
            Err(method_not_allowed())
        }
        _ => Err(endpoint_not_found()),
    }
}

fn parse_event_query(query: &str) -> Option<(i64, i64)> {
    let mut after = None;
    let mut limit = None;
    let mut count = 0;
    for parameter in query.split('&') {
        count += 1;
        let (name, value) = parameter.split_once('=')?;
        let name = percent_decode_ascii(name)?;
        let value = percent_decode_ascii(value)?;
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        match name.as_str() {
            "after" if after.is_none() => {
                validate_event_cursor(value.as_bytes()).ok()?;
                after = Some(value.parse::<i64>().ok()?);
            }
            "limit" if limit.is_none() => {
                let parsed = value.parse::<i64>().ok()?;
                if !(1..=100).contains(&parsed) {
                    return None;
                }
                limit = Some(parsed);
            }
            _ => return None,
        }
    }
    if count != 2 {
        return None;
    }
    Some((after?, limit?))
}

fn percent_decode_ascii(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    if !decoded.is_ascii() {
        return None;
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn oversized_header_response(bytes: &[u8]) -> Response {
    let Some(request_line_end) = bytes.windows(2).position(|window| window == b"\r\n") else {
        return invalid_read();
    };
    let Ok(request_line) = std::str::from_utf8(&bytes[..request_line_end]) else {
        return invalid_read();
    };
    let mut request_parts = request_line.split(' ');
    let method = request_parts.next().unwrap_or_default();
    let target = request_parts.next().unwrap_or_default();
    let version = request_parts.next().unwrap_or_default();
    if request_parts.next().is_some() || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return invalid_read();
    }
    match classify_route(method, target) {
        Ok(route) => invalid_request(&route),
        Err(_) => invalid_read(),
    }
}

fn valid_run_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_idempotency_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

struct Response {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn new(status: u16, reason: &'static str, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            reason,
            content_type,
            body,
        }
    }
}

fn json_response<T: Serialize>(status: u16, reason: &'static str, value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(mut body) => {
            body.push(b'\n');
            Response::new(status, reason, "application/json", body)
        }
        Err(_) => journal_failure(),
    }
}

fn cancel_response(
    status: u16,
    reason: &'static str,
    run_id: &str,
    disposition: &str,
    event_sequence: i64,
) -> Response {
    let Ok(encoded_disposition) = serde_json::to_vec(disposition) else {
        return journal_failure();
    };
    if validate_cancel_disposition(&encoded_disposition).is_err() {
        return journal_failure();
    }
    json_response(
        status,
        reason,
        &CancelResponse {
            run_id,
            disposition,
            event_sequence,
        },
    )
}

fn invalid_request(route: &Route) -> Response {
    match route {
        Route::Submit { .. } => problem_response(400, "Bad Request", "invalid submit request"),
        Route::Cancel { .. } => invalid_cancel(),
        _ => invalid_read(),
    }
}

fn invalid_read() -> Response {
    problem_response(400, "Bad Request", "invalid read request")
}

fn invalid_cancel() -> Response {
    problem_response(400, "Bad Request", "invalid cancel request")
}

fn run_not_found() -> Response {
    problem_response(404, "Not Found", "run not found")
}

fn endpoint_not_found() -> Response {
    problem_response(404, "Not Found", "endpoint not found")
}

fn method_not_allowed() -> Response {
    problem_response(405, "Method Not Allowed", "method not allowed")
}

fn journal_failure() -> Response {
    problem_response(500, "Internal Server Error", "journal operation failed")
}

fn problem_response(status: u16, title: &'static str, detail: &'static str) -> Response {
    Response::new(
        status,
        match status {
            400 => "Bad Request",
            404 => "Not Found",
            405 => "Method Not Allowed",
            409 => "Conflict",
            413 => "Payload Too Large",
            415 => "Unsupported Media Type",
            _ => "Internal Server Error",
        },
        "application/problem+json",
        format!(
            "{{\"type\":\"about:blank\",\"title\":\"{title}\",\"status\":{status},\"detail\":\"{detail}\"}}\n"
        )
        .into_bytes(),
    )
}

fn write_response(stream: &mut TcpStream, response: Response) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn write_rejection_response(stream: &mut TcpStream, response: Response) -> io::Result<()> {
    write_response(stream, response)?;
    stream.shutdown(Shutdown::Write)?;
    stream.set_read_timeout(Some(REJECTION_DRAIN_TIMEOUT))?;

    let mut drained = 0;
    let mut buffer = [0_u8; 1024];
    while drained < MAX_REJECTION_DRAIN_BYTES {
        let available = (MAX_REJECTION_DRAIN_BYTES - drained).min(buffer.len());
        match stream.read(&mut buffer[..available]) {
            Ok(0) => break,
            Ok(read) => drained += read,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::ConnectionReset
                ) =>
            {
                break;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::path::PathBuf;

    use super::{parse_startup, StartupConfig, StartupError};

    #[test]
    fn zero_arguments_are_rejected() {
        assert_eq!(
            parse_startup(Vec::new()),
            Err(StartupError::NoSupportedConfiguration)
        );
    }

    #[test]
    fn unsupported_configuration_error_has_exact_display_text() {
        assert_eq!(
            StartupError::NoSupportedConfiguration.to_string(),
            "no supported configuration"
        );
    }

    #[test]
    fn explicit_ipv4_and_ipv6_loopback_are_accepted() {
        let cases = [
            (
                "127.0.0.1:0",
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            ),
            (
                "[::1]:65535",
                SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 65535),
            ),
        ];
        for (listen, expected) in cases {
            assert_eq!(
                parse_startup([
                    OsString::from("local-demo"),
                    OsString::from("--database"),
                    OsString::from("journal.sqlite"),
                    OsString::from("--listen"),
                    OsString::from(listen),
                ]),
                Ok(StartupConfig {
                    listen: expected,
                    database_path: PathBuf::from("journal.sqlite"),
                })
            );
        }
    }

    #[test]
    fn malformed_startup_forms_are_rejected() {
        let cases: &[&[&str]] = &[
            &["local-demo"],
            &["local-demo", "--listen", "127.0.0.1:0"],
            &[
                "local-demo",
                "--listen",
                "0.0.0.0:0",
                "--database",
                "journal.sqlite",
            ],
            &[
                "local-demo",
                "--listen",
                "localhost:0",
                "--database",
                "journal.sqlite",
            ],
            &[
                "local-demo",
                "--listen",
                "127.0.0.1:65536",
                "--database",
                "journal.sqlite",
            ],
            &[
                "local-demo",
                "--listen",
                "127.0.0.1:0",
                "--listen",
                "127.0.0.1:1",
            ],
        ];

        for arguments in cases {
            assert_eq!(
                parse_startup(arguments.iter().map(OsString::from)),
                Err(StartupError::NoSupportedConfiguration),
                "arguments: {arguments:?}"
            );
        }
    }
}
