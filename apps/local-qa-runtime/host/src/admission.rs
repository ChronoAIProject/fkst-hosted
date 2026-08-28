use fkst_qa_contracts::{
    build_initial_run_acceptance_v2, canonical_bytes, validate_local_qa_run_request_v2,
    AttemptBindingV2, LocalQARunRequestV2,
};

use crate::executor::{ExecutorRegistry, ExecutorSelection};
use crate::journal::{Admission, Journal, V2AdmissionRecord};
use crate::Response;

const PRODUCER_VERSION: &str = "fkst-local-qa-host/0.1.0";

pub(crate) trait AttemptBindingVerifier {
    fn verify(&self, binding: &AttemptBindingV2, now: &str) -> Result<(), ()>;
}

pub(crate) struct Mvp0DeterministicAttemptBindingVerifier;

impl AttemptBindingVerifier for Mvp0DeterministicAttemptBindingVerifier {
    fn verify(&self, binding: &AttemptBindingV2, now: &str) -> Result<(), ()> {
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
        expected.then_some(()).ok_or(())
    }
}

pub(crate) fn admit_v2(
    journal: &mut Journal,
    registry: &ExecutorRegistry,
    verifier: &dyn AttemptBindingVerifier,
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
    if verifier.verify(&request.attempt_binding, now).is_err() {
        return crate::problem_response(400, "Bad Request", "invalid attempt binding");
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
        Ok(Admission::Created(body)) => Response::new(201, "Created", "application/json", body),
        Ok(Admission::Replay(body)) => Response::new(200, "OK", "application/json", body),
        Ok(Admission::DifferentKey) => crate::problem_response(
            409,
            "Conflict",
            "run_id is already accepted under a different Idempotency-Key",
        ),
        Ok(Admission::DifferentDigest) => crate::problem_response(
            409,
            "Conflict",
            "run_id is already accepted with a different request digest",
        ),
        Ok(Admission::Occupied) => {
            crate::problem_response(409, "Conflict", "active run slot is occupied")
        }
        Err(_) => crate::journal_failure(),
    }
}
