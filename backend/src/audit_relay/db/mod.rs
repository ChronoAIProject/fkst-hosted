//! The relay's storage handle: one writer, a bounded reader pool, and a
//! fail-closed health flag.
//!
//! ```text
//! async caller ──write()──> bounded queue ──> ONE writer thread ──> SQLite (WAL)
//!              ──read()───> permit ──> pooled read connection (spawn_blocking)
//! ```
//!
//! ## Why a dedicated writer thread rather than `spawn_blocking`
//!
//! SQLite serializes writers anyway, and the durability contract is
//! `synchronous=FULL`: every commit fsyncs. Running that on the async executor
//! would stall unrelated tasks, and running it on the blocking pool would let an
//! unbounded number of fsyncs pile up behind each other with no visible queue.
//! One thread with one BOUNDED queue makes the backlog a number
//! ([`Database::writer_queue_depth`]) instead of a mystery, and makes
//! "one writer" a property of the process rather than of SQLite's locking.
//!
//! ## Fail-closed readiness
//!
//! A corrupt database, a read-only volume, a full disk, or a failed migration
//! flips [`Database::ingress_ready`] to false permanently for that process. The
//! relay then reports `/ready` as `503`, which in `required` mode makes the
//! control plane refuse audited requests rather than run handlers whose calls it
//! could never record. A PostHog outage deliberately does NOT do this: the whole
//! purpose of an outbox is to keep accepting while the destination is down.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, ErrorCode, OpenFlags, Transaction};
use tokio::sync::{mpsc, oneshot, Semaphore};

pub mod delivery;
pub mod ingest;
pub mod read;
pub mod row;
pub mod schema;

/// Why a storage operation could not be performed. Bounded by construction:
/// every variant carries a compile-time constant, never a SQLite message, a
/// path, or a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DbError {
    /// The event id exists with different immutable content. Never an overwrite.
    #[error("event id conflicts with an existing record")]
    Conflict,
    /// A completion arrived for an event id with no registered start.
    #[error("no registered start for this event id")]
    NoStart,
    /// The configured record ceiling was reached.
    #[error("the relay is at its configured record capacity")]
    Capacity,
    /// Storage is unusable for this process (corrupt, read-only, full, I/O).
    #[error("relay storage is unavailable ({0})")]
    Unavailable(&'static str),
    /// A contended write did not get the lock inside the busy timeout.
    #[error("relay storage is busy")]
    Busy,
    /// Anything else, categorized but never quoted.
    #[error("relay storage failed ({0})")]
    Internal(&'static str),
}

impl DbError {
    /// The bounded metric/log label.
    pub fn as_str(self) -> &'static str {
        match self {
            DbError::Conflict => "conflict",
            DbError::NoStart => "no_start",
            DbError::Capacity => "capacity",
            DbError::Unavailable(kind) => kind,
            DbError::Busy => "busy",
            DbError::Internal(kind) => kind,
        }
    }

    /// Whether this failure means the process can no longer accept records.
    pub fn is_fatal_storage(self) -> bool {
        matches!(self, DbError::Unavailable(_))
    }
}

/// Map a rusqlite failure onto the bounded vocabulary above.
///
/// The SQLite message is deliberately dropped: it can quote a file path, a
/// column value, or an index name, none of which belong in a relay log.
pub fn classify(error: &rusqlite::Error) -> DbError {
    let rusqlite::Error::SqliteFailure(failure, _) = error else {
        return DbError::Internal("query");
    };
    match failure.code {
        ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => DbError::Unavailable("corrupt"),
        ErrorCode::ReadOnly => DbError::Unavailable("read_only"),
        ErrorCode::DiskFull => DbError::Unavailable("disk_full"),
        ErrorCode::CannotOpen => DbError::Unavailable("cannot_open"),
        ErrorCode::SystemIoFailure => DbError::Unavailable("io"),
        ErrorCode::PermissionDenied => DbError::Unavailable("permission"),
        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => DbError::Busy,
        ErrorCode::ConstraintViolation => DbError::Conflict,
        _ => DbError::Internal("query"),
    }
}

/// Work handed to the single writer thread.
type WriterJob = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// A pool of read-only connections, handed out under a semaphore so the number
/// of concurrent blocking reads is a configured number rather than "however many
/// requests arrived".
struct ReaderPool {
    permits: Semaphore,
    free: Mutex<Vec<Connection>>,
}

/// The storage handle. Cloneable; every clone shares one writer and one pool.
#[derive(Clone)]
pub struct Database {
    writer: mpsc::Sender<WriterJob>,
    readers: Arc<ReaderPool>,
    ready: Arc<AtomicBool>,
    queue_depth: Arc<AtomicU64>,
    max_records: u64,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("ingress_ready", &self.ingress_ready())
            .field("writer_queue_depth", &self.writer_queue_depth())
            .field("max_records", &self.max_records)
            .finish()
    }
}

/// The knobs [`Database::open`] needs. A struct rather than five positional
/// arguments so a future knob cannot be silently swapped with its neighbour.
#[derive(Clone, Copy, Debug)]
pub struct DatabaseSettings {
    pub busy_timeout_ms: u64,
    pub writer_queue_capacity: usize,
    pub read_concurrency: usize,
    pub max_records: u64,
}

impl Database {
    /// Open (creating if needed) the database at `path`, apply the pragmas,
    /// migrate, and start the writer thread.
    ///
    /// Every failure here is a startup failure: a relay that cannot prove its
    /// storage is writable and migrated must not begin accepting records.
    pub fn open(path: &Path, settings: DatabaseSettings) -> Result<Self, DbError> {
        prepare_directory(path)?;
        let mut writer_connection = open_connection(path, true, settings.busy_timeout_ms)?;
        let journal_mode = schema::apply_pragmas(&writer_connection, settings.busy_timeout_ms)
            .map_err(|error| classify(&error))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            tracing::error!(
                journal_mode = %journal_mode,
                "audit relay: the database refused WAL mode"
            );
            return Err(DbError::Unavailable("journal_mode"));
        }
        schema::migrate(&mut writer_connection).map_err(|error| {
            tracing::error!(
                reason = classify(&error).as_str(),
                "audit relay: migration failed"
            );
            classify(&error)
        })?;
        restrict_file_permissions(path)?;

        let mut free = Vec::with_capacity(settings.read_concurrency);
        for _ in 0..settings.read_concurrency {
            let reader = open_connection(path, false, settings.busy_timeout_ms)?;
            // WAL readers need the same busy timeout; the mode itself is a
            // database-level property already established by the writer.
            reader
                .busy_timeout(std::time::Duration::from_millis(settings.busy_timeout_ms))
                .map_err(|error| classify(&error))?;
            free.push(reader);
        }

        let (sender, receiver) = mpsc::channel::<WriterJob>(settings.writer_queue_capacity);
        let queue_depth = Arc::new(AtomicU64::new(0));
        spawn_writer(writer_connection, receiver, queue_depth.clone());

        Ok(Self {
            writer: sender,
            readers: Arc::new(ReaderPool {
                permits: Semaphore::new(settings.read_concurrency),
                free: Mutex::new(free),
            }),
            ready: Arc::new(AtomicBool::new(true)),
            queue_depth,
            max_records: settings.max_records,
        })
    }

    /// Whether durable ingress can still be promised.
    pub fn ingress_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// Records still queued for the single writer.
    pub fn writer_queue_depth(&self) -> u64 {
        self.queue_depth.load(Ordering::Relaxed)
    }

    /// The configured capacity guard.
    pub fn max_records(&self) -> u64 {
        self.max_records
    }

    /// Run `job` inside ONE transaction on the writer thread and await its
    /// result. A returned `Err` rolls the transaction back.
    pub async fn write<T, F>(&self, job: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, DbError> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
        let queued: WriterJob = Box::new(move |connection| {
            let outcome = match connection.transaction() {
                Ok(transaction) => match job(&transaction) {
                    Ok(value) => transaction.commit().map(|()| value).map_err(|error| {
                        let classified = classify(&error);
                        tracing::error!(reason = classified.as_str(), "audit relay: commit failed");
                        classified
                    }),
                    Err(error) => Err(error),
                },
                Err(error) => Err(classify(&error)),
            };
            // The receiver is gone only when the caller was cancelled; the
            // transaction already committed or rolled back either way.
            let _ = tx.send(outcome);
        });
        if self.writer.send(queued).await.is_err() {
            self.queue_depth.fetch_sub(1, Ordering::Relaxed);
            self.mark_unavailable("writer_stopped");
            return Err(DbError::Unavailable("writer_stopped"));
        }
        let outcome = rx.await.unwrap_or(Err(DbError::Internal("writer_dropped")));
        if let Err(error) = &outcome {
            if error.is_fatal_storage() {
                self.mark_unavailable(error.as_str());
            }
        }
        outcome
    }

    /// Run `job` on a pooled read connection, off the async executor.
    pub async fn read<T, F>(&self, job: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, DbError> + Send + 'static,
    {
        let readers = self.readers.clone();
        let permit = readers
            .permits
            .acquire()
            .await
            .map_err(|_| DbError::Internal("reader_closed"))?;
        // A permit exists for every pooled connection, so the pop cannot miss.
        let Some(connection) = readers.take() else {
            return Err(DbError::Internal("reader_missing"));
        };
        let (connection, outcome) =
            tokio::task::spawn_blocking(move || (job(&connection), connection))
                .await
                .map(|(outcome, connection)| (connection, outcome))
                .map_err(|_| DbError::Internal("reader_panicked"))?;
        readers.give_back(connection);
        drop(permit);
        outcome
    }

    /// Latch this process into "durable ingress impossible".
    fn mark_unavailable(&self, reason: &'static str) {
        if self.ready.swap(false, Ordering::Relaxed) {
            tracing::error!(
                reason,
                "audit relay: durable ingress is no longer available; readiness is now false"
            );
        }
    }
}

impl ReaderPool {
    fn take(&self) -> Option<Connection> {
        self.free
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .pop()
    }

    fn give_back(&self, connection: Connection) {
        self.free
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(connection);
    }
}

/// Start the single writer thread. It owns the connection for the process's
/// lifetime; when the queue closes it returns and the handle reports fatal.
fn spawn_writer(
    mut connection: Connection,
    mut receiver: mpsc::Receiver<WriterJob>,
    queue_depth: Arc<AtomicU64>,
) {
    std::thread::Builder::new()
        .name("fkst-audit-relay-writer".to_string())
        .spawn(move || {
            while let Some(job) = receiver.blocking_recv() {
                job(&mut connection);
                queue_depth.fetch_sub(1, Ordering::Relaxed);
            }
            tracing::info!("audit relay: writer thread stopped");
        })
        // A process that cannot start its only writer has no durable ingress at
        // all; there is no degraded mode worth inventing.
        .expect("audit relay writer thread must start");
}

/// Create the parent directory `0700` if it does not exist.
fn prepare_directory(path: &Path) -> Result<(), DbError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    if !parent.exists() {
        std::fs::create_dir_all(parent).map_err(|error| {
            tracing::error!(
                kind = io_kind(&error),
                "audit relay: cannot create the data directory"
            );
            DbError::Unavailable("cannot_open")
        })?;
    }
    set_mode(parent, 0o700)
}

/// Keep the database file itself `0600`. Applied after migration so the file
/// exists; the WAL/SHM siblings inherit the directory's `0700`.
fn restrict_file_permissions(path: &Path) -> Result<(), DbError> {
    set_mode(path, 0o600)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), DbError> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions).map_err(|error| {
        tracing::error!(
            kind = io_kind(&error),
            mode,
            "audit relay: cannot restrict permissions"
        );
        DbError::Unavailable("permission")
    })
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), DbError> {
    // The relay ships as a Linux container; on any other platform the mode is a
    // no-op rather than a build-time absence, so tests still run locally.
    Ok(())
}

/// A bounded category for an I/O failure — never the path or the OS message.
fn io_kind(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => "permission_denied",
        std::io::ErrorKind::NotFound => "not_found",
        std::io::ErrorKind::AlreadyExists => "already_exists",
        _ => "io",
    }
}

/// Open one connection. `writable` selects the writer's flags; readers open
/// read-only so a bug in a read path cannot mutate the outbox.
fn open_connection(
    path: &Path,
    writable: bool,
    busy_timeout_ms: u64,
) -> Result<Connection, DbError> {
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let connection = Connection::open_with_flags(path, flags).map_err(|error| {
        let classified = classify(&error);
        tracing::error!(
            reason = classified.as_str(),
            writable,
            "audit relay: cannot open the database"
        );
        classified
    })?;
    connection
        .busy_timeout(std::time::Duration::from_millis(busy_timeout_ms))
        .map_err(|error| classify(&error))?;
    Ok(connection)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
