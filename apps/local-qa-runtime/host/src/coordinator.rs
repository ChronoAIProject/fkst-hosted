use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::executor::{ExecutorRegistry, ExecutorRequest, ExecutorSelection};
use crate::journal::Journal;
use crate::RunError;

enum CoordinatorMessage {
    Stop,
}

pub(crate) struct CoordinatorHandle {
    sender: Sender<CoordinatorMessage>,
    join: Option<JoinHandle<Result<(), RunError>>>,
}

impl CoordinatorHandle {
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

    match receiver.recv() {
        Ok(CoordinatorMessage::Stop) | Err(_) => Ok(()),
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

    use super::{run_coordinator, CoordinatorHandle};
    use crate::executor::{
        DeterministicExecutor, ExecutorDescriptor, ExecutorRegistry, ExecutorRequest,
        ExecutorResult, ExecutorSelection, VersionedExecutor,
    };
    use crate::journal::{Admission, Journal, V2AdmissionRecord};
    use crate::RunError;

    const TEST_REQUEST_DIGEST: &str =
        "c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428";
    const TEST_BINDING_JSON: &[u8] = br#"{"qa_task_id":"qa-task-0002","qa_attempt_id":"qa-attempt-0002","machine_id":"machine-0002","worker_id":"worker-0002","installation_id":"installation-0002","generation":1,"fence_token":"test-fence-00000002","deadline":"2026-08-25T16:05:00Z"}"#;
    const TEST_SELECTION_JSON: &[u8] = br#"{"schema_version":"qa.local-executor/v1","executor_id":"fake.api","executor_version":"1.0.0","capability_digest":"sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335","required_capability":"api.request"}"#;

    struct BlockingExecutor {
        descriptor: ExecutorDescriptor,
        calls: Arc<AtomicUsize>,
        entered: mpsc::Sender<String>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl VersionedExecutor for BlockingExecutor {
        fn descriptor(&self) -> &ExecutorDescriptor {
            &self.descriptor
        }

        fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered
                .send(request.run_id.clone())
                .map_err(|_| RunError::Contract("blocking executor entry signal failed"))?;
            self.release
                .lock()
                .map_err(|_| RunError::Contract("blocking executor release lock poisoned"))?
                .recv()
                .map_err(|_| RunError::Contract("blocking executor release signal failed"))?;
            Ok(ExecutorResult {
                schema_version: "qa.local-executor/v1".to_owned(),
                run_id: request.run_id.clone(),
                executor_id: self.descriptor.executor_id.clone(),
                executor_version: self.descriptor.executor_version.clone(),
                capability_digest: self.descriptor.capability_digest.clone(),
                execution_outcome: "passed".to_owned(),
            })
        }
    }

    struct RecordingVersionedExecutor {
        descriptor: ExecutorDescriptor,
        requests: Mutex<mpsc::Sender<ExecutorRequest>>,
        result_behavior: ResultBehavior,
    }

    #[derive(Clone, Copy)]
    enum ResultBehavior {
        Valid,
        Malformed,
        RelationMismatch,
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
                schema_version: if matches!(self.result_behavior, ResultBehavior::Malformed) {
                    "qa.local-executor/v2".to_owned()
                } else {
                    "qa.local-executor/v1".to_owned()
                },
                run_id: if matches!(self.result_behavior, ResultBehavior::RelationMismatch) {
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
                "sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335".to_owned(),
        }
    }

    fn api_selection() -> ExecutorSelection {
        ExecutorSelection {
            schema_version: "qa.local-executor/v1".to_owned(),
            executor_id: "fake.api".to_owned(),
            executor_version: "1.0.0".to_owned(),
            capability_digest:
                "sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335".to_owned(),
            required_capability: "api.request".to_owned(),
        }
    }

    fn admit_v2(
        journal: &mut Journal,
        run_id: &str,
        idempotency_key: &str,
        request_digest: &str,
    ) -> Result<Admission, RunError> {
        journal.admit_v2(V2AdmissionRecord {
            run_id,
            idempotency_key,
            request_digest,
            acceptance_bytes: b"{}",
            binding_json: TEST_BINDING_JSON,
            selection_json: TEST_SELECTION_JSON,
        })
    }

    #[test]
    fn active_execution_is_at_most_once_and_completion_is_atomic() {
        let directory = temporary_directory("blocking-executor");
        let database_path = directory.join("journal.sqlite");
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let executor = BlockingExecutor {
            descriptor: api_descriptor(),
            calls: Arc::clone(&calls),
            entered: entered_sender,
            release: Mutex::new(release_receiver),
        };
        let registry =
            ExecutorRegistry::new(vec![Box::new(executor)]).expect("registry must be valid");
        let run_id = "00000000-0000-0000-0000-000000000000";
        let mut journal = Journal::open(&database_path).expect("HTTP journal opens");
        journal
            .seed_executable_v1(run_id, "idem-001", TEST_REQUEST_DIGEST)
            .expect("executable v1 fixture must be seeded");
        let coordinator_database_path = database_path.clone();
        let coordinator_start = thread::spawn(move || {
            CoordinatorHandle::start_versioned(
                &coordinator_database_path,
                registry,
                api_selection(),
            )
        });
        let executor_run_id = entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("executor must be entered");
        assert_eq!(executor_run_id, run_id);
        assert_eq!(
            journal
                .connection
                .query_row(
                    "SELECT executor_run_id FROM runs WHERE run_id = '00000000-0000-0000-0000-000000000000'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("executor run ID must be readable"),
            executor_run_id
        );

        assert!(journal
            .claim_next()
            .expect("claim check must succeed")
            .is_none());
        assert!(matches!(
            journal.cancel(run_id, "cancel-active"),
            Err(RunError::ActiveAttempt)
        ));
        let connection = Connection::open(&database_path).expect("inspection journal opens");
        assert_eq!(row_count(&connection, "cancel_requests"), 0);
        assert_eq!(row_count(&connection, "execution_attempts"), 1);
        assert_eq!(row_count(&connection, "events"), 4);
        let before = lifecycle_facts(&connection, run_id);
        assert_eq!(before.0, "executing");
        assert_eq!(before.1, None);
        assert_eq!(before.2, 4);
        assert_eq!(before.3, "claimed");
        assert_eq!(before.4, None);

        release_sender.send(()).expect("executor must be released");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let facts = lifecycle_facts(&connection, run_id);
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
        let mut coordinator = coordinator_start
            .join()
            .expect("coordinator startup thread joins")
            .expect("coordinator starts after processing executable v1 work");
        coordinator.shutdown().expect("coordinator joins");
        drop(connection);
        drop(journal);
        let reopened = Journal::open(&database_path).expect("journal must reopen");
        assert_eq!(
            reopened
                .connection
                .query_row(
                    "SELECT executor_run_id FROM runs WHERE run_id = '00000000-0000-0000-0000-000000000000'",
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
            result_behavior: ResultBehavior::Valid,
        })])
        .expect("registry must be valid");
        let selection = api_selection();
        let mut journal = Journal::open(&database_path).expect("journal opens");
        journal
            .seed_executable_v1(run_id, "idem-001", TEST_REQUEST_DIGEST)
            .expect("executable v1 fixture must be seeded");
        let mut coordinator =
            CoordinatorHandle::start_versioned(&database_path, registry, selection.clone())
                .expect("versioned coordinator starts");
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
    fn startup_selection_failure_does_not_invoke_or_fall_back() {
        let directory = temporary_directory("selection-failure");
        let database_path = directory.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000002";
        let mut journal = Journal::open(&database_path).expect("journal opens");
        journal
            .seed_executable_v1(run_id, "idem-001", TEST_REQUEST_DIGEST)
            .expect("executable v1 fixture must be seeded");
        drop(journal);
        let (request_sender, request_receiver) = mpsc::channel();
        let registry = ExecutorRegistry::new(vec![Box::new(RecordingVersionedExecutor {
            descriptor: api_descriptor(),
            requests: Mutex::new(request_sender),
            result_behavior: ResultBehavior::Valid,
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
    fn startup_relation_mismatch_never_completes_the_journal() {
        let directory = temporary_directory("result-mismatch");
        let database_path = directory.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000003";
        let mut journal = Journal::open(&database_path).expect("journal opens");
        journal
            .seed_executable_v1(run_id, "idem-001", TEST_REQUEST_DIGEST)
            .expect("executable v1 fixture must be seeded");
        drop(journal);
        let (request_sender, request_receiver) = mpsc::channel();
        let registry = ExecutorRegistry::new(vec![Box::new(RecordingVersionedExecutor {
            descriptor: api_descriptor(),
            requests: Mutex::new(request_sender),
            result_behavior: ResultBehavior::RelationMismatch,
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
    fn startup_malformed_result_never_completes_the_journal() {
        let directory = temporary_directory("malformed-result");
        let database_path = directory.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000004";
        let mut journal = Journal::open(&database_path).expect("journal opens");
        journal
            .seed_executable_v1(run_id, "idem-001", TEST_REQUEST_DIGEST)
            .expect("executable v1 fixture must be seeded");
        drop(journal);
        let (request_sender, request_receiver) = mpsc::channel();
        let registry = ExecutorRegistry::new(vec![Box::new(RecordingVersionedExecutor {
            descriptor: api_descriptor(),
            requests: Mutex::new(request_sender),
            result_behavior: ResultBehavior::Malformed,
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
    fn v2_admission_remains_durable_and_unexecuted_after_restart() {
        let directory = temporary_directory("v2-inert");
        let database_path = directory.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000005";
        let mut journal = Journal::open(&database_path).expect("journal opens");
        assert!(matches!(
            admit_v2(&mut journal, run_id, "idem-001", TEST_REQUEST_DIGEST),
            Ok(Admission::Created(_))
        ));
        drop(journal);

        let mut journal = Journal::open(&database_path).expect("journal reopens");
        assert!(matches!(
            admit_v2(&mut journal, run_id, "idem-001", TEST_REQUEST_DIGEST),
            Ok(Admission::Replay(_))
        ));
        assert!(journal
            .claim_next()
            .expect("claim check must succeed")
            .is_none());
        assert_eq!(
            journal
                .connection
                .query_row(
                    "SELECT run_id FROM active_run_slot WHERE slot = 1",
                    [],
                    |row| { row.get::<_, String>(0) }
                )
                .expect("active slot must remain durable"),
            run_id
        );
        drop(journal);

        let (request_sender, request_receiver) = mpsc::channel();
        let registry = ExecutorRegistry::new(vec![Box::new(RecordingVersionedExecutor {
            descriptor: api_descriptor(),
            requests: Mutex::new(request_sender),
            result_behavior: ResultBehavior::Valid,
        })])
        .expect("registry must be valid");
        let mut coordinator =
            CoordinatorHandle::start_versioned(&database_path, registry, api_selection())
                .expect("coordinator starts without executing v2");
        assert!(request_receiver.try_recv().is_err());
        coordinator.shutdown().expect("coordinator joins");

        let journal = Journal::open(&database_path).expect("journal reopens after coordinator");
        let snapshot = journal
            .snapshot(run_id)
            .expect("snapshot reads")
            .expect("run exists");
        assert_eq!(snapshot.state, "accepted");
        assert_eq!(snapshot.execution_outcome, None);
        assert_eq!(snapshot.latest_event_sequence, 1);
        assert_eq!(row_count(&journal.connection, "execution_attempts"), 0);
        fs::remove_dir_all(directory).expect("temporary directory must be removed");
    }

    #[test]
    fn idle_shutdown_joins_without_hanging() {
        let directory = temporary_directory("coordinator-shutdown");
        let database_path = directory.join("journal.sqlite");
        let registry = ExecutorRegistry::new(vec![Box::new(DeterministicExecutor::api())])
            .expect("registry must be valid");
        let mut coordinator =
            CoordinatorHandle::start_versioned(&database_path, registry, api_selection())
                .expect("coordinator starts");
        let started = Instant::now();
        coordinator.shutdown().expect("coordinator joins");
        assert!(started.elapsed() < Duration::from_secs(1));
        fs::remove_dir_all(directory).expect("temporary directory must be removed");
    }

    #[test]
    fn sender_disconnect_stops_the_coordinator() {
        let directory = temporary_directory("coordinator-disconnect");
        let database_path = directory.join("journal.sqlite");
        let journal = Journal::open(&database_path).expect("journal opens");
        let registry = ExecutorRegistry::new(vec![Box::new(DeterministicExecutor::api())])
            .expect("registry must be valid");
        let (sender, receiver) = mpsc::channel();
        let (startup_sender, startup_receiver) = mpsc::sync_channel(0);
        let coordinator = thread::spawn(move || {
            run_coordinator(
                journal,
                registry,
                api_selection(),
                receiver,
                Some(startup_sender),
            )
        });

        assert!(startup_receiver.recv().expect("startup signal arrives"));
        assert!(!coordinator.is_finished());
        drop(sender);
        coordinator
            .join()
            .expect("coordinator thread joins")
            .expect("sender disconnect stops the coordinator");
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
        run_id: &str,
    ) -> (String, Option<String>, i64, String, Option<String>) {
        connection
            .query_row(
                "SELECT runs.state, runs.execution_outcome,
                        (SELECT MAX(sequence) FROM events WHERE events.run_id = runs.run_id),
                        execution_attempts.status, execution_attempts.execution_outcome
                 FROM runs JOIN execution_attempts
                   ON execution_attempts.run_id = runs.run_id
                 WHERE runs.run_id = ?1",
                [run_id],
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
