use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::executor::{
    CleanupReceipt, ExecutorControlReport, ExecutorControlRequest, ExecutorRegistry,
    ExecutorRequest, ExecutorSelection,
};
use crate::journal::{Cancellation, CancellationControl, Journal};
use crate::RunError;

enum CoordinatorMessage {
    Stop,
}

pub(crate) struct CoordinatorHandle {
    sender: Sender<CoordinatorMessage>,
    join: Option<JoinHandle<Result<(), RunError>>>,
    registry: ExecutorRegistry,
    selection: ExecutorSelection,
}

impl CoordinatorHandle {
    pub(crate) fn start_versioned(
        database_path: &Path,
        registry: ExecutorRegistry,
        selection: ExecutorSelection,
    ) -> Result<Self, RunError> {
        registry.resolve(&selection)?;
        let journal = Journal::open(database_path)?;
        let (sender, receiver) = mpsc::channel();
        let coordinator_registry = registry.clone();
        let coordinator_selection = selection.clone();
        let join = thread::Builder::new()
            .name("fkst-local-qa-run-coordinator".to_owned())
            .spawn(move || {
                run_coordinator(
                    journal,
                    coordinator_registry,
                    coordinator_selection,
                    receiver,
                    None,
                )
            })?;
        Ok(Self {
            sender,
            join: Some(join),
            registry,
            selection,
        })
    }

    pub(crate) fn cancel(
        &self,
        journal: &mut Journal,
        run_id: &str,
        idempotency_key: &str,
    ) -> Result<Cancellation, RunError> {
        let selection_json = serde_json::to_string(&self.selection)
            .map_err(|_| RunError::Contract("executor selection serialization failed"))?;
        let cancellation =
            journal.cancel_with_control(run_id, idempotency_key, &selection_json)?;
        if matches!(cancellation, Cancellation::Accepted { .. }) {
            let control = journal
                .cancellation_control(run_id)?
                .ok_or(RunError::InvalidJournal("cancel control obligation missing"))?;
            reconcile_control(journal, &self.registry, &control)?;
        }
        Ok(cancellation)
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
    for control in journal.incomplete_cancellation_controls()? {
        reconcile_control(&mut journal, &registry, &control)?;
    }
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

fn reconcile_control(
    journal: &mut Journal,
    registry: &ExecutorRegistry,
    control: &CancellationControl,
) -> Result<(), RunError> {
    let selection = serde_json::from_str::<ExecutorSelection>(&control.selection_json)
        .map_err(|_| RunError::InvalidJournal("invalid persisted executor selection"))?;
    registry.resolve(&selection)?;
    let executor_run_id = control
        .executor_run_id
        .clone()
        .ok_or(RunError::InvalidJournal("cancel control executor Run ID missing"))?;
    let report = if journal
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM execution_attempts WHERE run_id = ?1)",
            [&control.run_id],
            |row| row.get::<_, bool>(0),
        )?
    {
        registry.control(&ExecutorControlRequest {
            schema_version: "qa.local-executor-control/v1".to_owned(),
            control_id: control.control_id.clone(),
            run_id: control.run_id.clone(),
            executor_run_id,
            selection,
            deadline_utc: control.deadline_utc.clone(),
        })?
    } else {
        ExecutorControlReport {
            schema_version: "qa.local-executor-control/v1".to_owned(),
            control_id: control.control_id.clone(),
            run_id: control.run_id.clone(),
            executor_run_id,
            executor_id: selection.executor_id,
            executor_version: selection.executor_version,
            capability_digest: selection.capability_digest,
            status: "accepted".to_owned(),
            control_acknowledged: true,
            worker_stop_observed: true,
            effect_disposition: "not_started".to_owned(),
            execution_outcome: "not_started".to_owned(),
            evidence_outcome: "not_started".to_owned(),
            upload_outcome: "not_started".to_owned(),
            cleanup_outcome: "not_required".to_owned(),
            cleanup_receipt: Some(CleanupReceipt {
                receipt_id: control.control_id.clone(),
                no_resources_remain: true,
                resource_handles: Vec::new(),
            }),
            residual: None,
        }
    };
    journal.record_control_report(&report)
}

fn process_available(
    journal: &mut Journal,
    registry: &ExecutorRegistry,
    selection: &ExecutorSelection,
) -> Result<(), RunError> {
    while let Some(claimed) = journal.claim_next()? {
        if !journal.transition(&claimed.run_id, "preparing", "ready", 3)? {
            return Ok(());
        }
        if !journal.transition(&claimed.run_id, "ready", "executing", 4)? {
            return Ok(());
        }
        let request = ExecutorRequest {
            schema_version: "qa.local-executor/v1".to_owned(),
            run_id: claimed.executor_run_id,
            selection: selection.clone(),
        };
        if !journal.admit_effect(&claimed.run_id, "executor.execute")? {
            return Ok(());
        }
        let outcome = registry.execute(&request)?;
        if !journal.transition(&claimed.run_id, "executing", "staging_evidence", 5)? {
            return Ok(());
        }
        if !journal.admit_effect(&claimed.run_id, "evidence.stage")? {
            return Ok(());
        }
        if !journal.transition(
            &claimed.run_id,
            "staging_evidence",
            "cleaning_up_execution",
            6,
        )? {
            return Ok(());
        }
        if !journal.admit_effect(&claimed.run_id, "cleanup.execute")? {
            return Ok(());
        }
        if !journal.transition(&claimed.run_id, "cleaning_up_execution", "uploading", 7)? {
            return Ok(());
        }
        if !journal.admit_effect(&claimed.run_id, "upload.admit")? {
            return Ok(());
        }
        if !journal.transition(&claimed.run_id, "uploading", "finalizing_local", 8)? {
            return Ok(());
        }
        if !journal.complete(&claimed.run_id, &outcome)? {
            return Ok(());
        }
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
        CleanupReceipt, DeterministicExecutor, ExecutorControlReport, ExecutorControlRequest,
        ExecutorDescriptor, ExecutorRegistry, ExecutorRequest, ExecutorResult, ExecutorSelection,
        VersionedExecutor,
    };
    use crate::journal::{Admission, Journal, V2AdmissionRecord};
    use crate::RunError;

    const TEST_REQUEST_DIGEST: &str =
        "c6da30d2cbe81af624c4e364e21cdad9dc2510d2e2ff9a02bb5bd6c325a25428";
    const TEST_BINDING_JSON: &[u8] = br#"{"qa_task_id":"qa-task-0002","qa_attempt_id":"qa-attempt-0002","machine_id":"machine-0002","worker_id":"worker-0002","installation_id":"installation-0002","generation":1,"fence_token":"dGVzdC1mZW5jZS0wMDAwMDAwMg","deadline":"2026-08-25T16:05:00Z"}"#;
    const TEST_SELECTION_JSON: &[u8] = br#"{"schema_version":"qa.local-executor/v1","executor_id":"fake.api","executor_version":"1.0.0","capability_digest":"sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335","required_capability":"api.request"}"#;

    struct BlockingExecutor {
        descriptor: ExecutorDescriptor,
        calls: Arc<AtomicUsize>,
        cancellations: Arc<AtomicUsize>,
        entered: mpsc::Sender<String>,
        release: Mutex<mpsc::Receiver<()>>,
        cancel_entered: mpsc::Sender<()>,
        cancel_release: Mutex<mpsc::Receiver<()>>,
        database_path: std::path::PathBuf,
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

        fn control(
            &self,
            request: &ExecutorControlRequest,
        ) -> Result<ExecutorControlReport, RunError> {
            let inspection =
                Journal::open(&self.database_path).expect("independent callback Journal must open");
            let persisted = inspection
                .connection
                .query_row(
                    "SELECT cancel_requests.event_sequence, events.event_type
                     FROM cancel_requests JOIN events
                       ON events.run_id = cancel_requests.run_id
                      AND events.sequence = cancel_requests.event_sequence
                     WHERE cancel_requests.run_id = ?1",
                    [&request.run_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .expect("cancel intent and Event must be committed before callback");
            assert_eq!(persisted, (5, "run.cancel_requested".to_owned()));
            self.cancellations.fetch_add(1, Ordering::SeqCst);
            self.cancel_entered
                .send(())
                .expect("cancel callback entry signal must be received");
            self.cancel_release
                .lock()
                .expect("cancel callback release lock must remain valid")
                .recv()
                .expect("cancel callback release signal must be received");
            Ok(control_report(request, &self.descriptor))
        }
    }

    struct CountingExecutor {
        descriptor: ExecutorDescriptor,
        executions: Arc<AtomicUsize>,
        cancellations: Arc<AtomicUsize>,
    }

    impl VersionedExecutor for CountingExecutor {
        fn descriptor(&self) -> &ExecutorDescriptor {
            &self.descriptor
        }

        fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            Ok(ExecutorResult {
                schema_version: "qa.local-executor/v1".to_owned(),
                run_id: request.run_id.clone(),
                executor_id: self.descriptor.executor_id.clone(),
                executor_version: self.descriptor.executor_version.clone(),
                capability_digest: self.descriptor.capability_digest.clone(),
                execution_outcome: "passed".to_owned(),
            })
        }

        fn control(
            &self,
            request: &ExecutorControlRequest,
        ) -> Result<ExecutorControlReport, RunError> {
            self.cancellations.fetch_add(1, Ordering::SeqCst);
            Ok(control_report(request, &self.descriptor))
        }
    }

    fn control_report(
        request: &ExecutorControlRequest,
        descriptor: &ExecutorDescriptor,
    ) -> ExecutorControlReport {
        ExecutorControlReport {
            schema_version: "qa.local-executor-control/v1".to_owned(),
            control_id: request.control_id.clone(),
            run_id: request.run_id.clone(),
            executor_run_id: request.executor_run_id.clone(),
            executor_id: descriptor.executor_id.clone(),
            executor_version: descriptor.executor_version.clone(),
            capability_digest: descriptor.capability_digest.clone(),
            status: "accepted".to_owned(),
            control_acknowledged: true,
            worker_stop_observed: true,
            effect_disposition: "uncertain".to_owned(),
            execution_outcome: "lost_or_inconclusive".to_owned(),
            evidence_outcome: "not_started".to_owned(),
            upload_outcome: "not_started".to_owned(),
            cleanup_outcome: "completed".to_owned(),
            cleanup_receipt: Some(CleanupReceipt {
                receipt_id: request.control_id.clone(),
                no_resources_remain: true,
                resource_handles: Vec::new(),
            }),
            residual: None,
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
    fn active_cancel_commits_before_one_executor_callback_and_survives_restart() {
        let directory = temporary_directory("blocking-executor-cancel");
        let database_path = directory.join("journal.sqlite");
        let calls = Arc::new(AtomicUsize::new(0));
        let cancellations = Arc::new(AtomicUsize::new(0));
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let (cancel_entered_sender, cancel_entered_receiver) = mpsc::channel();
        let (cancel_release_sender, cancel_release_receiver) = mpsc::channel();
        let executor = BlockingExecutor {
            descriptor: api_descriptor(),
            calls: Arc::clone(&calls),
            cancellations: Arc::clone(&cancellations),
            entered: entered_sender,
            release: Mutex::new(release_receiver),
            cancel_entered: cancel_entered_sender,
            cancel_release: Mutex::new(cancel_release_receiver),
            database_path: database_path.clone(),
        };
        let registry =
            ExecutorRegistry::new(vec![Box::new(executor)]).expect("registry must be valid");
        let run_id = "00000000-0000-0000-0000-000000000000";
        let mut journal = Journal::open(&database_path).expect("HTTP Journal opens");
        journal
            .seed_executable_v1(run_id, "idem-001", TEST_REQUEST_DIGEST)
            .expect("executable v1 fixture must be seeded");
        let coordinator =
            CoordinatorHandle::start_versioned(&database_path, registry, api_selection())
                .expect("coordinator starts before execution completes");
        assert_eq!(
            entered_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("executor must be entered"),
            run_id
        );

        let cancellation_database_path = database_path.clone();
        let cancellation = thread::spawn(move || {
            let mut cancellation_journal =
                Journal::open(&cancellation_database_path).expect("cancellation Journal opens");
            let result = coordinator.cancel(&mut cancellation_journal, run_id, "cancel-active");
            (coordinator, result)
        });
        cancel_entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("cancel callback must run after the durable commit");

        let events = journal
            .events(run_id, 0, 10)
            .expect("events read")
            .expect("run events exist");
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_type.as_str())
                .collect::<Vec<_>>(),
            [
                "run.accepted",
                "run.state_changed",
                "run.state_changed",
                "run.state_changed",
                "run.cancel_requested",
            ]
        );
        assert_eq!(row_count(&journal.connection, "cancel_requests"), 1);
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);

        cancel_release_sender
            .send(())
            .expect("cancel callback must be released");
        let (mut coordinator, cancellation_result) =
            cancellation.join().expect("cancellation thread joins");
        assert!(matches!(
            cancellation_result,
            Ok(crate::journal::Cancellation::Accepted {
                event_sequence: 5,
                active_executor_run_id: Some(ref executor_run_id),
            }) if executor_run_id == run_id
        ));
        let settled_events = journal
            .events(run_id, 0, 20)
            .expect("settled events read")
            .expect("settled events exist");
        assert_eq!(
            settled_events
                .iter()
                .map(|event| event.event_type.as_str())
                .collect::<Vec<_>>(),
            [
                "run.accepted",
                "run.state_changed",
                "run.state_changed",
                "run.state_changed",
                "run.cancel_requested",
                "control.reported",
                "worker.stop_observed",
                "cleanup.receipt",
                "run.cancel_settled",
            ]
        );
        assert!(matches!(
            coordinator.cancel(&mut journal, run_id, "cancel-repeat"),
            Ok(crate::journal::Cancellation::Terminal(9))
        ));
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);

        release_sender.send(()).expect("executor must be released");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let facts = lifecycle_facts(&journal.connection, run_id);
            if facts.2 == 9 {
                assert_eq!(facts.0, "terminal");
                assert_eq!(facts.1.as_deref(), Some("cancelled"));
                assert_eq!(facts.3, "completed");
                assert_eq!(facts.4.as_deref(), Some("cancelled"));
                break;
            }
            assert!(Instant::now() < deadline, "cancelled Run did not settle");
            thread::yield_now();
        }
        coordinator.shutdown().expect("coordinator joins");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let restart_registry = ExecutorRegistry::new(vec![Box::new(CountingExecutor {
            descriptor: api_descriptor(),
            executions: Arc::clone(&calls),
            cancellations: Arc::clone(&cancellations),
        })])
        .expect("restart registry must be valid");
        let mut restarted =
            CoordinatorHandle::start_versioned(&database_path, restart_registry, api_selection())
                .expect("cancelled Journal restarts");
        restarted.shutdown().expect("restarted coordinator joins");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);

        drop(journal);
        fs::remove_dir_all(directory).expect("temporary directory must be removed");
    }

    #[test]
    fn terminal_completion_wins_without_cancel_mutation_or_callback() {
        let directory = temporary_directory("completion-wins-cancel-race");
        let database_path = directory.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000006";
        let executions = Arc::new(AtomicUsize::new(0));
        let cancellations = Arc::new(AtomicUsize::new(0));
        let registry = ExecutorRegistry::new(vec![Box::new(CountingExecutor {
            descriptor: api_descriptor(),
            executions: Arc::clone(&executions),
            cancellations: Arc::clone(&cancellations),
        })])
        .expect("registry must be valid");
        let mut journal = Journal::open(&database_path).expect("Journal opens");
        journal
            .seed_executable_v1(run_id, "idem-001", TEST_REQUEST_DIGEST)
            .expect("executable v1 fixture must be seeded");
        let mut coordinator =
            CoordinatorHandle::start_versioned(&database_path, registry, api_selection())
                .expect("coordinator starts");
        coordinator
            .shutdown()
            .expect("coordinator completes the Run");
        assert_terminal(&journal.connection, run_id, "passed");

        assert!(matches!(
            coordinator.cancel(&mut journal, run_id, "cancel-terminal"),
            Ok(crate::journal::Cancellation::Terminal(9))
        ));
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert_eq!(cancellations.load(Ordering::SeqCst), 0);
        assert_eq!(row_count(&journal.connection, "cancel_requests"), 0);
        assert_eq!(row_count(&journal.connection, "events"), 9);

        drop(journal);
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
        coordinator
            .shutdown()
            .expect("coordinator completes the Run");
        assert_terminal(&journal.connection, run_id, "passed");
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
            "accepted"
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
        let mut coordinator =
            CoordinatorHandle::start_versioned(&database_path, registry, api_selection())
                .expect("coordinator starts before executor result validation");
        assert_eq!(
            request_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("executor must be invoked once")
                .run_id,
            run_id
        );
        assert!(matches!(
            wait_for_coordinator_error(&mut coordinator),
            RunError::Contract("executor result relation failed")
        ));
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
        let mut coordinator =
            CoordinatorHandle::start_versioned(&database_path, registry, api_selection())
                .expect("coordinator starts before executor result validation");
        assert_eq!(
            request_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("executor must be invoked once")
                .run_id,
            run_id
        );
        assert!(matches!(
            wait_for_coordinator_error(&mut coordinator),
            RunError::Contract("invalid executor result")
        ));
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
    fn cancel_before_claim_settles_without_executor_effect_and_releases_slot() {
        let directory = temporary_directory("cancel-before-claim");
        let database_path = directory.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000005";
        let mut journal = Journal::open(&database_path).expect("journal opens");
        assert!(matches!(
            admit_v2(&mut journal, run_id, "idem-001", TEST_REQUEST_DIGEST),
            Ok(Admission::Created(_))
        ));
        let registry = ExecutorRegistry::new(vec![Box::new(DeterministicExecutor::api())])
            .expect("registry must be valid");
        let mut coordinator =
            CoordinatorHandle::start_versioned(&database_path, registry, api_selection())
                .expect("coordinator starts");

        assert!(matches!(
            coordinator.cancel(&mut journal, run_id, "cancel-001"),
            Ok(crate::journal::Cancellation::Accepted {
                event_sequence: 2,
                active_executor_run_id: None,
            })
        ));
        let snapshot = journal
            .snapshot(run_id)
            .expect("snapshot reads")
            .expect("run exists");
        assert_eq!(snapshot.state, "terminal");
        assert_eq!(snapshot.execution_outcome.as_deref(), Some("cancelled"));
        assert_eq!(snapshot.latest_event_sequence, 6);
        assert_eq!(row_count(&journal.connection, "execution_attempts"), 0);
        assert_eq!(row_count(&journal.connection, "active_run_slot"), 0);
        assert_eq!(row_count(&journal.connection, "effect_admissions"), 0);

        coordinator.shutdown().expect("coordinator joins");
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

    fn wait_for_coordinator_error(coordinator: &mut CoordinatorHandle) -> RunError {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match coordinator.check() {
                Ok(()) => {
                    assert!(
                        Instant::now() < deadline,
                        "coordinator did not report its execution error"
                    );
                    thread::yield_now();
                }
                Err(error) => return error,
            }
        }
    }

    fn assert_terminal(connection: &Connection, run_id: &str, outcome: &str) {
        let snapshot = connection
            .query_row(
                "SELECT state, execution_outcome FROM runs WHERE run_id = ?1",
                [run_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .expect("run lifecycle must be readable");
        assert_eq!(snapshot.0, "terminal");
        assert_eq!(snapshot.1.as_deref(), Some(outcome));
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
