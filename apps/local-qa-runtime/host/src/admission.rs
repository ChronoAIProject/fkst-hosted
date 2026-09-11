use fkst_qa_contracts::{
    build_initial_run_acceptance_v2, canonical_bytes, validate_local_qa_run_request_v2,
    AttemptBindingV2, LocalQARunRequestV2,
};

use crate::executor::{ExecutorRegistry, ExecutorSelection};
use crate::journal::{Admission, Journal, V2AdmissionRecord};
use crate::Response;

const PRODUCER_VERSION: &str = "fkst-local-qa-host/0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurrentClaimVerification {
    Verified,
    Denied,
    Unavailable,
}

pub(crate) trait CurrentClaimVerifier: Send + Sync {
    fn verify(&self, binding: &AttemptBindingV2, now: &str) -> CurrentClaimVerification;
}

pub(crate) struct UnavailableCurrentClaimVerifier;

impl CurrentClaimVerifier for UnavailableCurrentClaimVerifier {
    fn verify(&self, _binding: &AttemptBindingV2, _now: &str) -> CurrentClaimVerification {
        CurrentClaimVerification::Unavailable
    }
}

pub(crate) struct Mvp0DeterministicCurrentClaimVerifier;

impl CurrentClaimVerifier for Mvp0DeterministicCurrentClaimVerifier {
    fn verify(&self, binding: &AttemptBindingV2, now: &str) -> CurrentClaimVerification {
        let expected = binding.qa_task_id == "qa-task-0002"
            && binding.qa_attempt_id == "qa-attempt-0002"
            && binding.generation == 1
            && binding.fence_token == "dGVzdC1mZW5jZS0wMDAwMDAwMg"
            && binding.machine_id == "machine-0002"
            && binding.worker_id == "worker-0002"
            && binding.installation_id == "installation-0002"
            && binding.deadline == "2026-08-25T16:05:00Z"
            && now == "2026-08-25T16:00:01Z"
            && now < binding.deadline.as_str();
        if expected {
            CurrentClaimVerification::Verified
        } else {
            CurrentClaimVerification::Denied
        }
    }
}

pub(crate) fn admit_v2(
    journal: &mut Journal,
    registry: &ExecutorRegistry,
    verifier: &dyn CurrentClaimVerifier,
    now: &str,
    path_run_id: &str,
    header_idempotency_key: &str,
    body: &[u8],
) -> Response {
    let validated = match validate_local_qa_run_request_v2(body) {
        Ok(value) => value,
        Err(_) => return crate::problem_response(400, "Bad Request", "invalid submit request"),
    };
    let request: LocalQARunRequestV2 = match serde_json::from_value(validated.value().clone()) {
        Ok(value) => value,
        Err(_) => return crate::problem_response(400, "Bad Request", "invalid submit request"),
    };
    if request.run_id != path_run_id || request.idempotency_key != header_idempotency_key {
        return crate::problem_response(400, "Bad Request", "invalid submit request");
    }
    match journal.replay_v2(
        &request.run_id,
        &request.idempotency_key,
        &request.content_digest,
    ) {
        Ok(Some(admission)) => return admission_response(admission),
        Ok(None) => {}
        Err(_) => return crate::journal_failure(),
    }
    match verifier.verify(&request.attempt_binding, now) {
        CurrentClaimVerification::Verified => {}
        CurrentClaimVerification::Denied => {
            return crate::problem_response(400, "Bad Request", "invalid attempt binding");
        }
        CurrentClaimVerification::Unavailable => {
            return crate::problem_response(
                503,
                "Service Unavailable",
                "current claim verification unavailable",
            );
        }
    }
    let selection = ExecutorSelection {
        schema_version: request.executor_selection.schema_version.clone(),
        executor_id: request.executor_selection.executor_id.clone(),
        executor_version: request.executor_selection.executor_version.clone(),
        capability_digest: request.executor_selection.capability_digest.clone(),
        required_capability: request.executor_selection.required_capability.clone(),
    };
    if registry.resolve(&selection).is_err() {
        return crate::problem_response(400, "Bad Request", "executor selection not allowlisted");
    }
    let acceptance = match build_initial_run_acceptance_v2(&validated, now, PRODUCER_VERSION) {
        Ok(value) => value,
        Err(_) => return crate::journal_failure(),
    };
    let mut acceptance_bytes = match canonical_bytes(&acceptance) {
        Ok(value) => value,
        Err(_) => return crate::journal_failure(),
    };
    acceptance_bytes.push(b'\n');
    let binding_json = match serde_json::to_vec(&request.attempt_binding) {
        Ok(value) => value,
        Err(_) => return crate::journal_failure(),
    };
    let selection_json = match serde_json::to_vec(&selection) {
        Ok(value) => value,
        Err(_) => return crate::journal_failure(),
    };
    let request_json = match canonical_bytes(&validated) {
        Ok(value) => value,
        Err(_) => return crate::journal_failure(),
    };
    let record = V2AdmissionRecord {
        run_id: &request.run_id,
        idempotency_key: &request.idempotency_key,
        request_digest: &request.content_digest,
        acceptance_bytes: &acceptance_bytes,
        binding_json: &binding_json,
        selection_json: &selection_json,
        request_json: &request_json,
    };
    match journal.admit_v2(record) {
        Ok(admission) => admission_response(admission),
        Err(_) => crate::journal_failure(),
    }
}

const RECONSTRUCT_V2_QUERY: &str = "SELECT
        CASE WHEN v.run_id IS NOT NULL THEN typeof(v.run_id)='text' AND v.run_id=?1 END,
        CASE WHEN a.run_id IS NOT NULL THEN typeof(a.run_id)='text' AND a.run_id=?1 END,
        CASE WHEN r.run_id IS NOT NULL THEN typeof(r.run_id)='text' AND r.run_id=?1 END,
        CASE WHEN typeof(r.admission_version)='integer' THEN r.admission_version END,
        typeof(r.executor_run_id)='text' AND r.executor_run_id=?1,
        CASE WHEN typeof(a.idempotency_key)='text' THEN
            CASE WHEN a.idempotency_key=substr(a.idempotency_key,1,64) COLLATE BINARY
                AND length(substr(a.idempotency_key,1,64)) BETWEEN 1 AND 64
                AND substr(a.idempotency_key,1,64) NOT GLOB '*[^A-Za-z0-9_-]*'
            THEN substr(a.idempotency_key,1,64) END END,
        CASE WHEN typeof(a.request_digest)='text' THEN
            CASE WHEN a.request_digest=substr(a.request_digest,1,71) COLLATE BINARY
                AND length(substr(a.request_digest,1,71))=71
                AND substr(a.request_digest,1,7)='sha256:'
                AND substr(a.request_digest,8,64) NOT GLOB '*[^0-9a-f]*'
            THEN substr(a.request_digest,1,71) END END,
        v.request_json IS NULL,
        CASE WHEN typeof(v.request_json)='blob' THEN
            CASE WHEN length(v.request_json)<=?2 THEN v.request_json END END,
        CASE WHEN typeof(a.response_json)='blob' THEN
            CASE WHEN length(a.response_json)<=?2 THEN a.response_json END END,
        CASE WHEN typeof(v.binding_json)='blob' THEN
            CASE WHEN length(v.binding_json)<=?2 THEN v.binding_json END END,
        CASE WHEN typeof(v.selection_json)='blob' THEN
            CASE WHEN length(v.selection_json)<=?2 THEN v.selection_json END END
     FROM (SELECT ?1 AS run_id) AS requested
     LEFT JOIN admission_v2_records v USING (run_id)
     LEFT JOIN accepted_requests a USING (run_id)
     LEFT JOIN runs r USING (run_id)";

impl Journal {
    /// Reconstructs historical admitted input after checking its persisted relationships.
    /// Missing runs and legacy inputs return `None`; corruption returns `InvalidJournal`.
    /// This read grants no current claim or execution authority.
    pub fn reconstruct_v2_request(
        &self,
        run_id: &str,
    ) -> Result<Option<LocalQARunRequestV2>, crate::RunError> {
        let invalid = || crate::RunError::InvalidJournal("invalid persisted v2 admission input");
        // One snapshot preserves missing rows. Lazy CASE guards BLOB type before byte
        // length; metadata returns integer facts or bounded ASCII substrings whose full
        // stored value must match, including any NUL suffix. No unbounded TEXT/BLOB
        // result or TEXT-to-BLOB cast is materialized; SQLite may still scan stored data.
        let mut statement = self
            .connection
            .prepare(RECONSTRUCT_V2_QUERY)
            .map_err(|_| invalid())?;
        let body_limit = i64::try_from(crate::MAX_SUBMIT_BODY_BYTES).map_err(|_| invalid())?;
        let mut rows = statement
            .query(rusqlite::params![run_id, body_limit])
            .map_err(|_| invalid())?;
        let row = rows.next().map_err(|_| invalid())?.ok_or_else(invalid)?;
        let text = |index| {
            row.get_ref(index)
                .map_err(|_| invalid())?
                .as_str()
                .map_err(|_| invalid())
        };
        let absent = |index| {
            row.get_ref(index)
                .map(|value| matches!(value, rusqlite::types::ValueRef::Null))
                .map_err(|_| invalid())
        };
        if absent(0)? && absent(1)? && absent(2)? {
            return Ok(None);
        }
        let matches = |index| row.get::<_, bool>(index).map_err(|_| invalid());
        let version: i64 = row.get(3).map_err(|_| invalid())?;
        if absent(0)? && version == 1 && matches(1)? && matches(2)? {
            return Ok(None);
        }
        if !matches(0)? || !matches(1)? || !matches(2)? || version != 2 || !matches(4)? {
            return Err(invalid());
        }
        if row.get::<_, bool>(7).map_err(|_| invalid())? {
            return Ok(None);
        }
        let blob = |index| {
            row.get_ref(index)
                .map_err(|_| invalid())?
                .as_blob()
                .map_err(|_| invalid())
        };
        let request_bytes = blob(8)?;
        let validated = validate_local_qa_run_request_v2(request_bytes).map_err(|_| invalid())?;
        if canonical_bytes(&validated).map_err(|_| invalid())? != request_bytes {
            return Err(invalid());
        }
        let request: LocalQARunRequestV2 =
            serde_json::from_value(validated.value().clone()).map_err(|_| invalid())?;
        if request.run_id != run_id
            || request.idempotency_key != text(5)?
            || request.content_digest != text(6)?
        {
            return Err(invalid());
        }
        let acceptance_bytes = blob(9)?.strip_suffix(b"\n").ok_or_else(invalid)?;
        let acceptance = fkst_qa_contracts::validate_run_acceptance_v2(acceptance_bytes)
            .map_err(|_| invalid())?;
        let typed_acceptance: fkst_qa_contracts::RunAcceptanceV2 =
            serde_json::from_value(acceptance.value().clone()).map_err(|_| invalid())?;
        // Rebuild at the original accepted_at, not the current clock. The existing
        // builder checks created_at <= accepted_at < the attempt deadline.
        let expected = build_initial_run_acceptance_v2(
            &validated,
            &typed_acceptance.accepted_at,
            &typed_acceptance.producer_version,
        )
        .map_err(|_| invalid())?;
        if canonical_bytes(&expected).map_err(|_| invalid())? != acceptance_bytes {
            return Err(invalid());
        }
        let binding = fkst_qa_contracts::admit_json(blob(10)?).map_err(|_| invalid())?;
        let expected_binding = fkst_qa_contracts::admit_json(
            &serde_json::to_vec(&request.attempt_binding).map_err(|_| invalid())?,
        )
        .map_err(|_| invalid())?;
        if fkst_qa_contracts::canonical_admitted_bytes(&binding).map_err(|_| invalid())?
            != fkst_qa_contracts::canonical_admitted_bytes(&expected_binding)
                .map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        let selection =
            fkst_qa_contracts::validate_executor_selection(blob(11)?).map_err(|_| invalid())?;
        if selection.value()
            != &serde_json::to_value(&request.executor_selection).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        Ok(Some(request))
    }
}

fn admission_response(admission: Admission) -> Response {
    match admission {
        Admission::Created(body) => Response::new(201, "Created", "application/json", body),
        Admission::Replay(body) => Response::new(200, "OK", "application/json", body),
        Admission::DifferentKey => crate::problem_response(
            409,
            "Conflict",
            "run_id is already accepted under a different Idempotency-Key",
        ),
        Admission::DifferentDigest => crate::problem_response(
            409,
            "Conflict",
            "run_id is already accepted with a different request digest",
        ),
        Admission::Occupied => {
            crate::problem_response(409, "Conflict", "active run slot is occupied")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde::Deserialize;

    use super::*;
    use crate::executor::FakeApiAdmissionExecutor;

    #[derive(Deserialize)]
    struct Fixture {
        expected_request_utf8: String,
    }

    struct CountingVerifier {
        calls: AtomicUsize,
        result: CurrentClaimVerification,
    }

    impl CountingVerifier {
        fn new(result: CurrentClaimVerification) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                result,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl CurrentClaimVerifier for CountingVerifier {
        fn verify(&self, _binding: &AttemptBindingV2, _now: &str) -> CurrentClaimVerification {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result
        }
    }

    fn fixture() -> Fixture {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../../packages/qa-contracts/fixtures/qa.local-run-admission/v2/happy-path.json",
        );
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn database_path(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "fkst-current-claim-{label}-{}-{nonce}.sqlite",
            std::process::id()
        ))
    }

    fn registry() -> ExecutorRegistry {
        ExecutorRegistry::new(vec![Box::new(FakeApiAdmissionExecutor::new())]).unwrap()
    }

    fn assert_tables_empty(journal: &Journal) {
        for table in [
            "accepted_requests",
            "runs",
            "events",
            "admission_v2_records",
            "active_run_slot",
        ] {
            let count: i64 = journal
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table} must remain empty");
        }
    }

    struct OwnedDirectory(std::path::PathBuf);

    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn owned_directory() -> OwnedDirectory {
        (0..128)
            .find_map(|_| {
                let path = database_path("sql-bounds");
                match fs::create_dir(&path) {
                    Ok(()) => Some(OwnedDirectory(path)),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => panic!("cannot reserve test directory: {error}"),
                }
            })
            .expect("test directory reserved atomically")
    }

    #[test]
    fn reconstruction_query_never_returns_unbounded_corrupt_storage() {
        let directory = owned_directory();
        let mut journal = Journal::open(&directory.0.join("journal.sqlite")).unwrap();
        let run_id = "00000000-0000-0000-0000-000000000002";
        let response = admit_v2(
            &mut journal,
            &registry(),
            &Mvp0DeterministicCurrentClaimVerifier,
            "2026-08-25T16:00:01Z",
            run_id,
            "idem_0002",
            fixture().expected_request_utf8.as_bytes(),
        );
        assert_eq!(response.status, 201);
        assert!(journal.reconstruct_v2_request(run_id).unwrap().is_some());
        let mut escaped = Vec::new();
        for (table, column, index, prefix) in [
            ("admission_v2_records", "request_json", 8, "{}"),
            ("accepted_requests", "response_json", 9, "{}"),
            ("admission_v2_records", "binding_json", 10, "{}"),
            ("admission_v2_records", "selection_json", 11, "{}"),
            ("accepted_requests", "idempotency_key", 5, "idem_0002"),
            ("accepted_requests", "request_digest", 6, "sha256:"),
            ("runs", "executor_run_id", 4, run_id),
            ("runs", "admission_version", 3, "2"),
        ] {
            for (kind, corrupt) in [
                (
                    "nul-text",
                    rusqlite::types::Value::Text(format!("{prefix}\0{}", "x".repeat(1_000_000))),
                ),
                (
                    "multibyte-text",
                    rusqlite::types::Value::Text("界".repeat(65_536)),
                ),
                (
                    "large-text",
                    rusqlite::types::Value::Text("x".repeat(1_000_000)),
                ),
                (
                    "large-blob",
                    rusqlite::types::Value::Blob(vec![b'x'; 1_000_000]),
                ),
            ] {
                // The CHECK on admission_version is disabled only for this damage test.
                journal.connection.execute_batch("PRAGMA ignore_check_constraints=ON; SAVEPOINT damage; DROP TRIGGER runs_executor_run_id_update").unwrap();
                journal
                    .connection
                    .execute(&format!("UPDATE {table} SET {column}=?1"), [corrupt])
                    .unwrap();
                let projected: rusqlite::types::Value = journal
                    .connection
                    .query_row(
                        RECONSTRUCT_V2_QUERY,
                        rusqlite::params![run_id, 65_536_i64],
                        |row| row.get(index),
                    )
                    .unwrap();
                if !matches!(
                    projected,
                    rusqlite::types::Value::Null | rusqlite::types::Value::Integer(0)
                ) {
                    escaped.push(format!("{table}.{column}/{kind}"));
                }
                assert!(matches!(
                    journal.reconstruct_v2_request(run_id),
                    Err(crate::RunError::InvalidJournal(
                        "invalid persisted v2 admission input"
                    ))
                ));
                journal
                    .connection
                    .execute_batch("ROLLBACK TO damage; RELEASE damage")
                    .unwrap();
            }
        }
        // Present related row IDs are projected as fixed integer facts, never text.
        for index in [0, 1, 2] {
            let projected: rusqlite::types::Value = journal
                .connection
                .query_row(
                    RECONSTRUCT_V2_QUERY,
                    rusqlite::params![run_id, 65_536_i64],
                    |row| row.get(index),
                )
                .unwrap();
            if projected != rusqlite::types::Value::Integer(1) {
                escaped.push(format!("identity projection {index}"));
            }
        }
        assert!(
            escaped.is_empty(),
            "query returned rejected storage: {escaped:?}"
        );
        assert!(journal.reconstruct_v2_request(run_id).unwrap().is_some());
    }

    #[test]
    fn reconstruction_of_existing_v1_input_is_unavailable_after_restart() {
        let directory = owned_directory();
        let path = directory.0.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000001";
        let mut journal = Journal::open(&path).unwrap();
        journal
            .seed_executable_v1(run_id, "v1-key", "v1-digest")
            .unwrap();
        let original = match journal
            .replay_v2(run_id, "v1-key", "v1-digest")
            .unwrap()
            .unwrap()
        {
            Admission::Replay(bytes) => bytes,
            _ => panic!("v1 accepted response must replay"),
        };
        drop(journal);
        let journal = Journal::open(&path).unwrap();
        let before = journal.connection.total_changes();
        assert!(journal.reconstruct_v2_request(run_id).unwrap().is_none());
        assert_eq!(journal.snapshot(run_id).unwrap().unwrap().state, "accepted");
        match journal
            .replay_v2(run_id, "v1-key", "v1-digest")
            .unwrap()
            .unwrap()
        {
            Admission::Replay(bytes) => assert_eq!(bytes, original),
            _ => panic!("v1 response replay must remain unchanged"),
        }
        assert_eq!(journal.connection.total_changes(), before);
    }

    #[test]
    fn reconstruction_preserves_cancelled_and_terminal_v2_history() {
        let directory = owned_directory();
        let path = directory.0.join("journal.sqlite");
        let run_id = "00000000-0000-0000-0000-000000000002";
        let fixture = fixture();
        let expected: LocalQARunRequestV2 =
            serde_json::from_str(&fixture.expected_request_utf8).unwrap();
        let selection: ExecutorSelection =
            serde_json::from_value(serde_json::to_value(&expected.executor_selection).unwrap())
                .unwrap();
        let mut journal = Journal::open(&path).unwrap();
        let created = admit_v2(
            &mut journal,
            &registry(),
            &Mvp0DeterministicCurrentClaimVerifier,
            "2026-08-25T16:00:01Z",
            run_id,
            "idem_0002",
            fixture.expected_request_utf8.as_bytes(),
        );
        assert_eq!(created.status, 201);
        assert!(journal.claim_next().unwrap().is_none());
        assert!(matches!(
            journal
                .cancel_with_control(
                    run_id,
                    "cancel-history",
                    &serde_json::to_string(&selection).unwrap()
                )
                .unwrap(),
            crate::journal::Cancellation::Accepted { event_sequence: 2 }
        ));
        drop(journal);
        let mut journal = Journal::open(&path).unwrap();
        let before = journal.connection.total_changes();
        assert_eq!(
            journal.reconstruct_v2_request(run_id).unwrap(),
            Some(expected.clone())
        );
        assert_eq!(journal.connection.total_changes(), before);
        assert_eq!(journal.snapshot(run_id).unwrap().unwrap().state, "accepted");
        assert!(journal.claim_next().unwrap().is_none());
        drop(journal);
        // Existing coordinator recovery settles an unstarted cancellation through
        // Journal APIs; no fabricated execution attempt or completion is inserted.
        let mut coordinator =
            crate::coordinator::CoordinatorHandle::start_versioned(&path, registry(), selection)
                .unwrap();
        coordinator.shutdown().unwrap();
        drop(coordinator);
        let mut journal = Journal::open(&path).unwrap();
        let snapshot = journal.snapshot(run_id).unwrap().unwrap();
        assert_eq!(snapshot.state, "terminal");
        assert_eq!(snapshot.execution_outcome.as_deref(), Some("cancelled"));
        let before = journal.connection.total_changes();
        assert_eq!(
            journal.reconstruct_v2_request(run_id).unwrap(),
            Some(expected)
        );
        assert_eq!(journal.connection.total_changes(), before);
        let verifier = CountingVerifier::new(CurrentClaimVerification::Unavailable);
        let replay = admit_v2(
            &mut journal,
            &registry(),
            &verifier,
            "2026-09-10T00:00:00Z",
            run_id,
            "idem_0002",
            fixture.expected_request_utf8.as_bytes(),
        );
        assert_eq!(replay.status, 200);
        assert_eq!(replay.body, created.body);
        assert_eq!(verifier.calls(), 0);
        assert!(journal.claim_next().unwrap().is_none());
        assert_eq!(
            journal
                .connection
                .query_row("SELECT COUNT(*) FROM execution_attempts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn unavailable_and_denied_authority_fail_before_admission_mutation() {
        let fixture = fixture();
        for (label, decision, expected_status) in [
            ("unavailable", CurrentClaimVerification::Unavailable, 503),
            ("denied", CurrentClaimVerification::Denied, 400),
        ] {
            let path = database_path(label);
            let mut journal = Journal::open(&path).unwrap();
            let verifier = CountingVerifier::new(decision);
            let response = admit_v2(
                &mut journal,
                &registry(),
                &verifier,
                "2026-08-25T16:00:01Z",
                "00000000-0000-0000-0000-000000000002",
                "idem_0002",
                fixture.expected_request_utf8.as_bytes(),
            );
            assert_eq!(response.status, expected_status);
            assert_eq!(verifier.calls(), 1);
            assert_tables_empty(&journal);
            drop(journal);
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn durable_exact_replay_does_not_reverify_current_claim() {
        let fixture = fixture();
        let path = database_path("replay");
        let mut journal = Journal::open(&path).unwrap();
        let verified = CountingVerifier::new(CurrentClaimVerification::Verified);
        let created = admit_v2(
            &mut journal,
            &registry(),
            &verified,
            "2026-08-25T16:00:01Z",
            "00000000-0000-0000-0000-000000000002",
            "idem_0002",
            fixture.expected_request_utf8.as_bytes(),
        );
        assert_eq!(created.status, 201);
        assert_eq!(verified.calls(), 1);

        let unavailable = CountingVerifier::new(CurrentClaimVerification::Unavailable);
        let replayed = admit_v2(
            &mut journal,
            &registry(),
            &unavailable,
            "2026-08-25T16:00:01Z",
            "00000000-0000-0000-0000-000000000002",
            "idem_0002",
            fixture.expected_request_utf8.as_bytes(),
        );
        assert_eq!(replayed.status, 200);
        assert_eq!(replayed.body, created.body);
        assert_eq!(unavailable.calls(), 0);
        drop(journal);
        let _ = fs::remove_file(path);
    }
}
