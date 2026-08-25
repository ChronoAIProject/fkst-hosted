use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::executor::{
    legacy_executor_descriptor, legacy_executor_selection, Executor, ExecutorRegistry,
    ExecutorRequest, ExecutorSelection, LegacyExecutorAdapter,
};
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
        let registry = ExecutorRegistry::new(vec![Box::new(LegacyExecutorAdapter::new(
            executor,
            legacy_executor_descriptor(),
        ))])?;
        Self::start_versioned(database_path, registry, legacy_executor_selection())
    }

    pub(crate) fn start_versioned(
        database_path: &Path,
        registry: ExecutorRegistry,
        selection: ExecutorSelection,
    ) -> Result<Self, RunError> {
        let journal = Journal::open(database_path)?;
        let (sender, receiver) = mpsc::channel();
        let (startup_sender, startup_receiver) = mpsc::sync_channel(0);
        let join = thread::Builder::new()
            .name("fkst-local-qa-run-coordinator".to_owned())
            .spawn(move || {
                run_coordinator(journal, registry, selection, receiver, Some(startup_sender))
            })?;

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
    registry: ExecutorRegistry,
    selection: ExecutorSelection,
    receiver: Receiver<CoordinatorMessage>,
    startup_sender: Option<mpsc::SyncSender<bool>>,
) -> Result<(), RunError> {
    if let Err(error) = process_available(&mut journal, &registry, &selection) {
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
            Ok(CoordinatorMessage::Wake) => {
                process_available(&mut journal, &registry, &selection)?
            }
            Ok(CoordinatorMessage::Stop) | Err(_) => return Ok(()),
        }
    }
}

fn process_available(
    journal: &mut Journal,
    registry: &ExecutorRegistry,
    selection: &ExecutorSelection,
) -> Result<(), RunError> {
    while let Some(claimed) = journal.claim_next()? {
        journal.transition(&claimed.run_id, "preparing", "ready", 3)?;
        journal.transition(&claimed.run_id, "ready", "executing", 4)?;
        let request = ExecutorRequest {
            schema_version: "qa.local-executor/v1".to_owned(),
            run_id: claimed.executor_run_id,
            selection: selection.clone(),
        };
        let outcome = registry.execute(&request)?;
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
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use super::CoordinatorHandle;
    use crate::executor::{
        ExecutionOutcome, Executor, ExecutorDescriptor, ExecutorRegistry, ExecutorRequest,
        ExecutorResult, ExecutorSelection, VersionedExecutor,
    };
    use crate::journal::{Admission, Journal};
    use crate::{RunError, CANONICAL_REQUEST_DIGEST};

    struct BlockingExecutor {
        calls: Arc<AtomicUsize>,
        entered: mpsc::Sender<String>,
        release: mpsc::Receiver<()>,
    }

    impl Executor for BlockingExecutor {
        fn execute(&mut self, run_id: &str) -> Result<ExecutionOutcome, RunError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered
                .send(run_id.to_owned())
                .map_err(|_| RunError::Contract("blocking executor entry signal failed"))?;
            self.release
                .recv()
                .map_err(|_| RunError::Contract("blocking executor release signal failed"))?;
            ExecutionOutcome::passed()
        }
    }

    struct RecordingVersionedExecutor {
        descriptor: ExecutorDescriptor,
        requests: Mutex<mpsc::Sender<ExecutorRequest>>,
        mismatched_result: bool,
    }

    impl VersionedExecutor for RecordingVersionedExecutor {
        fn descriptor(&self) -> &ExecutorDescriptor {
            &self.descriptor
        }

        fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError> {
            self.requests
                .lock()
                .map_err(|_| RunError::Contract("recording executor poisoned"))?
                .send(request.clone())
                .map_err(|_| RunError::Contract("recording executor request signal failed"))?;
            Ok(ExecutorResult {
                schema_version: "qa.local-executor/v1".to_owned(),
                run_id: if self.mismatched_result {
                    "00000000-0000-0000-0000-000000000099".to_owned()
                } else {
                    request.run_id.clone()
                },
                executor_id: self.descriptor.executor_id.clone(),
                executor_version: self.descriptor.executor_version.clone(),
                capability_digest: self.descriptor.capability_digest.clone(),
                execution_outcome: "passed".to_owned(),
            })
        }
    }

    fn api_descriptor() -> ExecutorDescriptor {
        ExecutorDescriptor {
            schema_version: "qa.local-executor/v1".to_owned(),
            executor_id: "fake.api".to_owned(),
            executor_version: "1.0.0".to_owned(),
            capabilities: vec!["api.request".to_owned()],
            capability_digest:
                "sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335"
                    .to_owned(),
        }
    }

    fn api_selection() -> ExecutorSelection {
        ExecutorSelection {
            schema_version: "qa.local-executor/v1".to_owned(),
            executor_id: "fake.api".to_owned(),
            executor_version: "1.0.0".to_owned(),
            capability_digest:
                "sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335"
                    .to_owned(),
            required_capability: "api.request".to_owned(),
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
        let executor_run_id = entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("executor must be entered");
        fkst_qa_contracts::validate_scalar("UUID", &executor_run_id)
            .expect("legacy run must receive a canonical executor UUID");
        assert_ne!(executor_run_id, "run-001");
        assert_eq!(
            journal
                .connection
                .query_row(
                    "SELECT executor_run_id FROM runs WHERE run_id = 'run-001'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("executor run ID must be readable"),
            executor_run_id
        );

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
        let reopened = Journal::open(&database_path).expect("journal must reopen");
        assert_eq!(
            reopened
                .connection
                .query_row(
                    "SELECT executor_run_id FROM runs WHERE run_id = 'run-001'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("executor run ID must survive restart"),
            executor_run_id
        );
        drop(reopened);
        fs::remove_dir_all(directory).expect("temporary directory must be removed");
    }

    #[test]
    fn versioned_coordinator_uses_the_exact_pinned_request() {
        let directory = temporary_directory("versioned-request");
        let database_path = directory.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000001";
        let (request_sender, request_receiver) = mpsc::channel();
        let registry = ExecutorRegistry::new(vec![Box::new(RecordingVersionedExecutor {
            descriptor: api_descriptor(),
            requests: Mutex::new(request_sender),
            mismatched_result: false,
        })])
        .expect("registry must be valid");
        let selection = api_selection();
        let mut coordinator =
            CoordinatorHandle::start_versioned(&database_path, registry, selection.clone())
                .expect("versioned coordinator starts");
        let mut journal = Journal::open(&database_path).expect("journal opens");
        assert!(matches!(
            journal.admit(run_id, "idem-001", CANONICAL_REQUEST_DIGEST),
            Ok(Admission::Created(_))
        ));
        coordinator.wake();
        assert_eq!(
            request_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("versioned executor must receive a request"),
            ExecutorRequest {
                schema_version: "qa.local-executor/v1".to_owned(),
                run_id: run_id.to_owned(),
                selection,
            }
        );
        wait_for_terminal(&journal.connection, run_id, "passed");
        coordinator.shutdown().expect("coordinator joins");
        fs::remove_dir_all(directory).expect("temporary directory must be removed");
    }

    #[test]
    fn selection_failure_does_not_invoke_or_fall_back() {
        let directory = temporary_directory("selection-failure");
        let database_path = directory.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000002";
        let mut journal = Journal::open(&database_path).expect("journal opens");
        assert!(matches!(
            journal.admit(run_id, "idem-001", CANONICAL_REQUEST_DIGEST),
            Ok(Admission::Created(_))
        ));
        drop(journal);
        let (request_sender, request_receiver) = mpsc::channel();
        let registry = ExecutorRegistry::new(vec![Box::new(RecordingVersionedExecutor {
            descriptor: api_descriptor(),
            requests: Mutex::new(request_sender),
            mismatched_result: false,
        })])
        .expect("registry must be valid");
        let mut selection = api_selection();
        selection.required_capability = "api.unknown".to_owned();
        assert!(CoordinatorHandle::start_versioned(&database_path, registry, selection).is_err());
        assert!(request_receiver.try_recv().is_err());
        let journal = Journal::open(&database_path).expect("journal reopens");
        assert_eq!(
            journal
                .snapshot(run_id)
                .expect("snapshot reads")
                .expect("run exists")
                .state,
            "executing"
        );
        fs::remove_dir_all(directory).expect("temporary directory must be removed");
    }

    #[test]
    fn relation_mismatched_result_never_completes_the_journal() {
        let directory = temporary_directory("result-mismatch");
        let database_path = directory.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000003";
        let mut journal = Journal::open(&database_path).expect("journal opens");
        assert!(matches!(
            journal.admit(run_id, "idem-001", CANONICAL_REQUEST_DIGEST),
            Ok(Admission::Created(_))
        ));
        drop(journal);
        let (request_sender, request_receiver) = mpsc::channel();
        let registry = ExecutorRegistry::new(vec![Box::new(RecordingVersionedExecutor {
            descriptor: api_descriptor(),
            requests: Mutex::new(request_sender),
            mismatched_result: true,
        })])
        .expect("registry must be valid");
        assert!(
            CoordinatorHandle::start_versioned(&database_path, registry, api_selection()).is_err()
        );
        assert_eq!(
            request_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("executor must be invoked once")
                .run_id,
            run_id
        );
        let journal = Journal::open(&database_path).expect("journal reopens");
        let snapshot = journal
            .snapshot(run_id)
            .expect("snapshot reads")
            .expect("run exists");
        assert_eq!(snapshot.state, "executing");
        assert_eq!(snapshot.execution_outcome, None);
        assert_eq!(snapshot.latest_event_sequence, 4);
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

    fn wait_for_terminal(connection: &Connection, run_id: &str, outcome: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = connection
                .query_row(
                    "SELECT state, execution_outcome FROM runs WHERE run_id = ?1",
                    [run_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .expect("run lifecycle must be readable");
            if snapshot.0 == "terminal" {
                assert_eq!(snapshot.1.as_deref(), Some(outcome));
                return;
            }
            assert!(Instant::now() < deadline, "Run did not reach terminal");
            thread::yield_now();
        }
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
