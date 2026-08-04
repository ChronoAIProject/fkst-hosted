use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Deserialize;

const CANONICAL_REQUEST_DIGEST: &str =
    "c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428";
const MAX_BODY_BYTES: usize = 64;
const MAX_HEADER_BYTES: usize = 16 * 1024;

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
    Database(rusqlite::Error),
    Io(io::Error),
    UnsupportedDatabaseVersion(i64),
    JournalMode(String),
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "database error: {error}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::UnsupportedDatabaseVersion(version) => {
                write!(formatter, "unsupported database version: {version}")
            }
            Self::JournalMode(mode) => write!(formatter, "SQLite WAL mode unavailable: {mode}"),
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
    let mut journal = Journal::open(&config.database_path)?;
    let listener = TcpListener::bind(config.listen)?;
    let assigned_address = listener.local_addr()?;

    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "fkst-local-qa-host: listening on {assigned_address}"
    )?;
    stdout.flush()?;
    drop(stdout);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                let _ = handle_connection(&mut stream, &mut journal);
            }
            Err(error) => return Err(RunError::Io(error)),
        }
    }
    Ok(())
}

struct Journal {
    connection: Connection,
}

enum Admission {
    Created(Vec<u8>),
    Replay(Vec<u8>),
    DifferentKey,
}

impl Journal {
    fn open(path: &Path) -> Result<Self, RunError> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let foreign_keys: i64 =
            connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
        if foreign_keys != 1 {
            return Err(RunError::Database(rusqlite::Error::InvalidQuery));
        }

        let journal_mode: String =
            connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            connection.pragma_update(None, "journal_mode", "WAL")?;
            let confirmed_mode: String =
                connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
            if !confirmed_mode.eq_ignore_ascii_case("wal") {
                return Err(RunError::JournalMode(confirmed_mode));
            }
        }

        let mut journal = Self { connection };
        journal.migrate()?;
        Ok(journal)
    }

    fn migrate(&mut self) -> Result<(), RunError> {
        let version: i64 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                let transaction = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                transaction.execute_batch(
                    "CREATE TABLE accepted_requests (
                        run_id TEXT PRIMARY KEY NOT NULL,
                        idempotency_key TEXT NOT NULL,
                        request_digest TEXT NOT NULL,
                        response_json BLOB NOT NULL,
                        UNIQUE (run_id, idempotency_key)
                    );
                    CREATE TABLE runs (
                        run_id TEXT PRIMARY KEY NOT NULL,
                        state TEXT NOT NULL
                    );
                    CREATE TABLE events (
                        run_id TEXT NOT NULL,
                        sequence INTEGER NOT NULL,
                        event_type TEXT NOT NULL,
                        event_json TEXT NOT NULL,
                        PRIMARY KEY (run_id, sequence),
                        FOREIGN KEY (run_id) REFERENCES runs(run_id)
                    );
                    PRAGMA user_version = 1;",
                )?;
                transaction.commit()?;
                Ok(())
            }
            1 => Ok(()),
            other => Err(RunError::UnsupportedDatabaseVersion(other)),
        }
    }

    fn admit(&mut self, run_id: &str, idempotency_key: &str) -> Result<Admission, RunError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored = transaction
            .query_row(
                "SELECT idempotency_key, request_digest, response_json
                 FROM accepted_requests WHERE run_id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((stored_key, stored_digest, response_json)) = stored {
            let admission =
                if stored_key == idempotency_key && stored_digest == CANONICAL_REQUEST_DIGEST {
                    Admission::Replay(response_json)
                } else {
                    Admission::DifferentKey
                };
            transaction.commit()?;
            return Ok(admission);
        }

        let existing_run: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE run_id = ?1)",
            [run_id],
            |row| row.get(0),
        )?;
        if existing_run {
            transaction.commit()?;
            return Ok(Admission::DifferentKey);
        }

        let response_json =
            format!("{{\"run_id\":\"{run_id}\",\"state\":\"accepted\",\"event_sequence\":1}}\n")
                .into_bytes();
        let event_json = format!("{{\"run_id\":\"{run_id}\",\"state\":\"accepted\"}}");
        transaction.execute(
            "INSERT INTO accepted_requests
             (run_id, idempotency_key, request_digest, response_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                run_id,
                idempotency_key,
                CANONICAL_REQUEST_DIGEST,
                response_json
            ],
        )?;
        transaction.execute(
            "INSERT INTO runs (run_id, state) VALUES (?1, 'accepted')",
            [run_id],
        )?;
        transaction.execute(
            "INSERT INTO events (run_id, sequence, event_type, event_json)
             VALUES (?1, 1, 'run.accepted', ?2)",
            params![run_id, event_json],
        )?;
        transaction.commit()?;
        Ok(Admission::Created(response_json))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitBody {
    kind: String,
}

struct Request {
    run_id: String,
    idempotency_key: String,
    body: Vec<u8>,
}

fn handle_connection(stream: &mut TcpStream, journal: &mut Journal) -> io::Result<()> {
    let request = match read_request(stream)? {
        Ok(request) => request,
        Err(response) => return write_response(stream, response),
    };

    let valid_body =
        serde_json::from_slice::<SubmitBody>(&request.body).is_ok_and(|body| body.kind == "inert");
    if !valid_body {
        return write_response(
            stream,
            problem_response(400, "Bad Request", "invalid submit request"),
        );
    }

    let response = match journal.admit(&request.run_id, &request.idempotency_key) {
        Ok(Admission::Created(body)) => Response::new(201, "Created", "application/json", body),
        Ok(Admission::Replay(body)) => Response::new(200, "OK", "application/json", body),
        Ok(Admission::DifferentKey) => problem_response(
            409,
            "Conflict",
            "run_id is already accepted under a different Idempotency-Key",
        ),
        Err(_) => problem_response(500, "Internal Server Error", "journal operation failed"),
    };
    write_response(stream, response)
}

fn read_request(stream: &mut TcpStream) -> io::Result<Result<Request, Response>> {
    let mut received = Vec::new();
    let header_end = loop {
        if let Some(position) = find_header_end(&received) {
            break position;
        }
        if received.len() >= MAX_HEADER_BYTES {
            return Ok(Err(problem_response(
                400,
                "Bad Request",
                "invalid submit request",
            )));
        }
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(Err(problem_response(
                400,
                "Bad Request",
                "invalid submit request",
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
                "invalid submit request",
            )))
        }
    };
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split(' ');
    let method = request_parts.next().unwrap_or_default();
    let target = request_parts.next().unwrap_or_default();
    let version = request_parts.next().unwrap_or_default();
    if request_parts.next().is_some()
        || method != "PUT"
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
    {
        return Ok(Err(problem_response(
            400,
            "Bad Request",
            "invalid submit request",
        )));
    }
    let Some(run_id) = target.strip_prefix("/v1/runs/") else {
        return Ok(Err(problem_response(404, "Not Found", "route not found")));
    };
    if !valid_run_id(run_id) {
        return Ok(Err(problem_response(
            400,
            "Bad Request",
            "invalid submit request",
        )));
    }

    let mut content_types = Vec::new();
    let mut idempotency_keys = Vec::new();
    let mut content_lengths = Vec::new();
    let mut has_transfer_encoding = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Ok(Err(problem_response(
                400,
                "Bad Request",
                "invalid submit request",
            )));
        };
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

    if content_types.len() != 1 || !valid_content_type(content_types[0]) {
        return Ok(Err(problem_response(
            415,
            "Unsupported Media Type",
            "Content-Type must be application/json",
        )));
    }
    if idempotency_keys.len() != 1 || !valid_idempotency_key(idempotency_keys[0]) {
        return Ok(Err(problem_response(
            400,
            "Bad Request",
            "invalid submit request",
        )));
    }
    if has_transfer_encoding || content_lengths.len() != 1 {
        return Ok(Err(problem_response(
            400,
            "Bad Request",
            "invalid submit request",
        )));
    }
    let content_length = match content_lengths[0].parse::<usize>() {
        Ok(value) => value,
        Err(_) => {
            return Ok(Err(problem_response(
                400,
                "Bad Request",
                "invalid submit request",
            )))
        }
    };
    if content_length > MAX_BODY_BYTES {
        return Ok(Err(problem_response(
            413,
            "Payload Too Large",
            "request body exceeds 64 bytes",
        )));
    }

    let body_start = header_end + 4;
    let mut body = received[body_start..].to_vec();
    if body.len() < content_length {
        let mut remaining = vec![0_u8; content_length - body.len()];
        stream.read_exact(&mut remaining)?;
        body.extend_from_slice(&remaining);
    }
    body.truncate(content_length);

    Ok(Ok(Request {
        run_id: run_id.to_owned(),
        idempotency_key: idempotency_keys[0].to_owned(),
        body,
    }))
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
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

fn problem_response(status: u16, title: &'static str, detail: &'static str) -> Response {
    Response::new(
        status,
        match status {
            400 => "Bad Request",
            404 => "Not Found",
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
