use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::executor::Executor;
use crate::journal::Journal;
use crate::RunError;

enum CoordinatorMessage {
    Wake,
    Stop,
}

pub(crate) struct CoordinatorHandle {
    sender: Sender<CoordinatorMessage>,
    join: Option<JoinHandle<Result<(), RunError>>>,
}

impl CoordinatorHandle {
    pub(crate) fn start(
        database_path: &Path,
        executor: Box<dyn Executor>,
    ) -> Result<Self, RunError> {
        let journal = Journal::open(database_path)?;
        let (sender, receiver) = mpsc::channel();
        let (startup_sender, startup_receiver) = mpsc::sync_channel(0);
        let join = thread::Builder::new()
            .name("fkst-local-qa-run-coordinator".to_owned())
            .spawn(move || run_coordinator(journal, executor, receiver, Some(startup_sender)))?;

        match startup_receiver.recv() {
            Ok(true) => Ok(Self {
                sender,
                join: Some(join),
            }),
            Ok(false) | Err(_) => match join.join() {
                Ok(Err(error)) => Err(error),
                Ok(Ok(())) => Err(RunError::CoordinatorStopped),
                Err(_) => Err(RunError::CoordinatorPanicked),
            },
        }
    }

    pub(crate) fn wake(&self) {
        let _ = self.sender.send(CoordinatorMessage::Wake);
    }

    pub(crate) fn check(&mut self) -> Result<(), RunError> {
        if !self
            .join
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            return Ok(());
        }
        match self
            .join
            .take()
            .expect("finished coordinator must exist")
            .join()
        {
            Ok(Err(error)) => Err(error),
            Ok(Ok(())) => Err(RunError::CoordinatorStopped),
            Err(_) => Err(RunError::CoordinatorPanicked),
        }
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), RunError> {
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        let _ = self.sender.send(CoordinatorMessage::Stop);
        match join.join() {
            Ok(result) => result,
            Err(_) => Err(RunError::CoordinatorPanicked),
        }
    }
}

impl Drop for CoordinatorHandle {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn run_coordinator(
    mut journal: Journal,
    mut executor: Box<dyn Executor>,
    receiver: Receiver<CoordinatorMessage>,
    startup_sender: Option<mpsc::SyncSender<bool>>,
) -> Result<(), RunError> {
    if let Err(error) = process_available(&mut journal, executor.as_mut()) {
        if let Some(sender) = startup_sender {
            let _ = sender.send(false);
        }
        return Err(error);
    }
    if let Some(sender) = startup_sender {
        let _ = sender.send(true);
    }

    loop {
        match receiver.recv() {
            Ok(CoordinatorMessage::Wake) => process_available(&mut journal, executor.as_mut())?,
            Ok(CoordinatorMessage::Stop) | Err(_) => return Ok(()),
        }
    }
}

fn process_available(journal: &mut Journal, executor: &mut dyn Executor) -> Result<(), RunError> {
    while let Some(claimed) = journal.claim_next()? {
        journal.transition(&claimed.run_id, "preparing", "ready", 3)?;
        journal.transition(&claimed.run_id, "ready", "executing", 4)?;
        let outcome = executor.execute(&claimed.run_id)?;
        journal.transition(&claimed.run_id, "executing", "staging_evidence", 5)?;
        journal.transition(
            &claimed.run_id,
            "staging_evidence",
            "cleaning_up_execution",
            6,
        )?;
        journal.transition(&claimed.run_id, "cleaning_up_execution", "uploading", 7)?;
        journal.transition(&claimed.run_id, "uploading", "finalizing_local", 8)?;
        journal.complete(&claimed.run_id, &outcome)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use super::CoordinatorHandle;
    use crate::executor::{ExecutionOutcome, Executor};
    use crate::journal::{Admission, Journal};
    use crate::{RunError, CANONICAL_REQUEST_DIGEST};

    struct BlockingExecutor {
        calls: Arc<AtomicUsize>,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    impl Executor for BlockingExecutor {
        fn execute(&mut self, _run_id: &str) -> Result<ExecutionOutcome, RunError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered
                .send(())
                .map_err(|_| RunError::Contract("blocking executor entry signal failed"))?;
            self.release
                .recv()
                .map_err(|_| RunError::Contract("blocking executor release signal failed"))?;
            ExecutionOutcome::passed()
        }
    }

    #[test]
    fn replay_during_execution_is_at_most_once_and_completion_is_atomic() {
        let directory = temporary_directory("blocking-executor");
        let database_path = directory.join("journal.sqlite");
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let executor = BlockingExecutor {
            calls: Arc::clone(&calls),
            entered: entered_sender,
            release: release_receiver,
        };
        let mut coordinator = CoordinatorHandle::start(&database_path, Box::new(executor))
            .expect("coordinator starts");
        let mut journal = Journal::open(&database_path).expect("HTTP journal opens");
        assert!(matches!(
            journal.admit("run-001", "idem-001", CANONICAL_REQUEST_DIGEST),
            Ok(Admission::Created(_))
        ));
        coordinator.wake();
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("executor must be entered");

        assert!(matches!(
            journal.admit("run-001", "idem-001", CANONICAL_REQUEST_DIGEST),
            Ok(Admission::Replay(_))
        ));
        assert!(matches!(
            journal.cancel("run-001", "cancel-active"),
            Err(RunError::ActiveAttempt)
        ));
        coordinator.wake();
        let connection = Connection::open(&database_path).expect("inspection journal opens");
        assert_eq!(row_count(&connection, "cancel_requests"), 0);
        assert_eq!(row_count(&connection, "execution_attempts"), 1);
        assert_eq!(row_count(&connection, "events"), 4);
        let before = lifecycle_facts(&connection);
        assert_eq!(before.0, "executing");
        assert_eq!(before.1, None);
        assert_eq!(before.2, 4);
        assert_eq!(before.3, "claimed");
        assert_eq!(before.4, None);

        release_sender.send(()).expect("executor must be released");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let facts = lifecycle_facts(&connection);
            if facts.0 == "terminal" {
                assert_eq!(facts.1.as_deref(), Some("passed"));
                assert_eq!(facts.2, 9);
                assert_eq!(facts.3, "completed");
                assert_eq!(facts.4.as_deref(), Some("passed"));
                break;
            }
            assert!(facts.1.is_none());
            assert_eq!(facts.3, "claimed");
            assert!(facts.4.is_none());
            assert!(Instant::now() < deadline, "Run did not reach terminal");
            thread::yield_now();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(row_count(&connection, "execution_attempts"), 1);
        assert_eq!(row_count(&connection, "events"), 9);
        coordinator.shutdown().expect("coordinator joins");
        drop(connection);
        drop(journal);
        fs::remove_dir_all(directory).expect("temporary directory must be removed");
    }

    #[test]
    fn idle_shutdown_joins_without_hanging() {
        let directory = temporary_directory("coordinator-shutdown");
        let database_path = directory.join("journal.sqlite");
        let mut coordinator =
            CoordinatorHandle::start(&database_path, Box::new(crate::executor::PassingExecutor))
                .expect("coordinator starts");
        let started = Instant::now();
        coordinator.shutdown().expect("coordinator joins");
        assert!(started.elapsed() < Duration::from_secs(1));
        fs::remove_dir_all(directory).expect("temporary directory must be removed");
    }

    fn lifecycle_facts(
        connection: &Connection,
    ) -> (String, Option<String>, i64, String, Option<String>) {
        connection
            .query_row(
                "SELECT runs.state, runs.execution_outcome,
                        (SELECT MAX(sequence) FROM events WHERE events.run_id = runs.run_id),
                        execution_attempts.status, execution_attempts.execution_outcome
                 FROM runs JOIN execution_attempts
                   ON execution_attempts.run_id = runs.run_id
                 WHERE runs.run_id = 'run-001'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("lifecycle facts must be readable")
    }

    fn row_count(connection: &Connection, table: &str) -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("row count must be readable")
    }

    fn temporary_directory(label: &str) -> std::path::PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "fkst-local-qa-host-{label}-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("temporary directory must be created");
        directory
    }
}
