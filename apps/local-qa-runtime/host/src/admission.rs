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
    let record = V2AdmissionRecord {
        run_id: &request.run_id,
        idempotency_key: &request.idempotency_key,
        request_digest: &request.content_digest,
        acceptance_bytes: &acceptance_bytes,
        binding_json: &binding_json,
        selection_json: &selection_json,
    };
    match journal.admit_v2(record) {
        Ok(admission) => admission_response(admission),
        Err(_) => crate::journal_failure(),
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
