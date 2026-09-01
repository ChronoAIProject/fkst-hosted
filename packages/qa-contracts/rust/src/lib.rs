#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path};

use base64::Engine as _;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

const CONTRACT_REGISTRY: &str = include_str!("../../contracts/registry.json");
const FOUNDATION_SCHEMA: &str =
    include_str!("../../contracts/qa.contract-foundation/v1/schema.json");
const FOUNDATION_SCHEMA_PATH: &str = "contracts/qa.contract-foundation/v1/schema.json";
const LOCAL_LIFECYCLE_SCHEMA: &str =
    include_str!("../../contracts/qa.local-lifecycle/v1/schema.json");
const LOCAL_LIFECYCLE_SCHEMA_PATH: &str = "contracts/qa.local-lifecycle/v1/schema.json";
const LOCAL_EVIDENCE_SCHEMA: &str =
    include_str!("../../contracts/qa.local-evidence/v1/schema.json");
const LOCAL_EVIDENCE_SCHEMA_PATH: &str = "contracts/qa.local-evidence/v1/schema.json";
const LOCAL_WORKER_SCHEMA: &str =
    include_str!("../../contracts/qa.local-worker-protocol/v1/schema.json");
const LOCAL_WORKER_SCHEMA_PATH: &str = "contracts/qa.local-worker-protocol/v1/schema.json";
const LOCAL_RUN_ADMISSION_SCHEMA: &str =
    include_str!("../../contracts/qa.local-run-admission/v1/schema.json");
const LOCAL_RUN_ADMISSION_SCHEMA_PATH: &str = "contracts/qa.local-run-admission/v1/schema.json";
const LOCAL_RUN_ADMISSION_V2_SCHEMA: &str =
    include_str!("../../contracts/qa.local-run-admission/v2/schema.json");
const LOCAL_RUN_ADMISSION_V2_SCHEMA_PATH: &str = "contracts/qa.local-run-admission/v2/schema.json";
const LOCAL_EXECUTOR_SCHEMA: &str =
    include_str!("../../contracts/qa.local-executor/v1/schema.json");
const LOCAL_EXECUTOR_SCHEMA_PATH: &str = "contracts/qa.local-executor/v1/schema.json";
const LOCAL_CANCELLATION_SCHEMA: &str =
    include_str!("../../contracts/qa.local-cancellation/v1/schema.json");
const LOCAL_CANCELLATION_SCHEMA_PATH: &str = "contracts/qa.local-cancellation/v1/schema.json";
const LOCAL_WORKER_CONTROL_SCHEMA: &str =
    include_str!("../../contracts/qa.local-worker-control/v1/schema.json");
const LOCAL_WORKER_CONTROL_SCHEMA_PATH: &str = "contracts/qa.local-worker-control/v1/schema.json";
const LOCAL_EXECUTOR_CONTROL_SCHEMA: &str =
    include_str!("../../contracts/qa.local-executor-control/v1/schema.json");
const LOCAL_EXECUTOR_CONTROL_SCHEMA_PATH: &str =
    "contracts/qa.local-executor-control/v1/schema.json";
const EMBEDDED_SCHEMAS: &[(&str, &str)] = &[
    (FOUNDATION_SCHEMA_PATH, FOUNDATION_SCHEMA),
    (LOCAL_LIFECYCLE_SCHEMA_PATH, LOCAL_LIFECYCLE_SCHEMA),
    (LOCAL_EVIDENCE_SCHEMA_PATH, LOCAL_EVIDENCE_SCHEMA),
    (LOCAL_WORKER_SCHEMA_PATH, LOCAL_WORKER_SCHEMA),
    (LOCAL_RUN_ADMISSION_SCHEMA_PATH, LOCAL_RUN_ADMISSION_SCHEMA),
    (
        LOCAL_RUN_ADMISSION_V2_SCHEMA_PATH,
        LOCAL_RUN_ADMISSION_V2_SCHEMA,
    ),
    (LOCAL_EXECUTOR_SCHEMA_PATH, LOCAL_EXECUTOR_SCHEMA),
    (LOCAL_CANCELLATION_SCHEMA_PATH, LOCAL_CANCELLATION_SCHEMA),
    (
        LOCAL_WORKER_CONTROL_SCHEMA_PATH,
        LOCAL_WORKER_CONTROL_SCHEMA,
    ),
    (
        LOCAL_EXECUTOR_CONTROL_SCHEMA_PATH,
        LOCAL_EXECUTOR_CONTROL_SCHEMA,
    ),
];
const LOCAL_STATE_TYPE_NAME: &str = "LocalState";
const EXECUTION_OUTCOME_TYPE_NAME: &str = "ExecutionOutcome";
const CANCEL_DISPOSITION_TYPE_NAME: &str = "CancelDisposition";
const EVENT_SEQUENCE_TYPE_NAME: &str = "EventSequence";
const EVENT_CURSOR_TYPE_NAME: &str = "EventCursor";
const LIFECYCLE_TYPE_NAMES: [&str; 5] = [
    LOCAL_STATE_TYPE_NAME,
    EXECUTION_OUTCOME_TYPE_NAME,
    CANCEL_DISPOSITION_TYPE_NAME,
    EVENT_SEQUENCE_TYPE_NAME,
    EVENT_CURSOR_TYPE_NAME,
];
const LOCAL_SANITIZED_OBSERVATION_TYPE_NAME: &str = "LocalSanitizedObservation";
const LOCAL_EVIDENCE_OBJECT_TYPE_NAME: &str = "LocalEvidenceObject";
const LOCAL_SANITIZED_OBSERVATION_REF_TYPE_NAME: &str = "LocalSanitizedObservationRef";
const LOCAL_EVIDENCE_OBJECT_REF_TYPE_NAME: &str = "LocalEvidenceObjectRef";
const LOCAL_EVIDENCE_TYPE_NAMES: [&str; 4] = [
    LOCAL_SANITIZED_OBSERVATION_TYPE_NAME,
    LOCAL_EVIDENCE_OBJECT_TYPE_NAME,
    LOCAL_SANITIZED_OBSERVATION_REF_TYPE_NAME,
    LOCAL_EVIDENCE_OBJECT_REF_TYPE_NAME,
];
const SUPPORTED_SCHEMA_MAJORS: [u64; 2] = [1, 2];
pub const LOCAL_WORKER_MAX_FRAME_BYTES: usize = 65_536;
const LOCAL_WORKER_TYPE_NAMES: [&str; 6] = [
    "LocalWorkerFrame",
    "LocalWorkerInvocation",
    "LocalWorkerCapabilityRequest",
    "LocalWorkerCapabilityResult",
    "LocalWorkerTerminalResult",
    "LocalWorkerProtocolFailure",
];
const LOCAL_QA_RUN_REQUEST_TYPE_NAME: &str = "LocalQARunRequest";
const RUN_ACCEPTANCE_TYPE_NAME: &str = "RunAcceptance";
const LOCAL_QA_RUN_REQUEST_V2_TYPE_NAME: &str = "LocalQARunRequestV2";
const RUN_ACCEPTANCE_V2_TYPE_NAME: &str = "RunAcceptanceV2";
const LOCAL_RUN_ADMISSION_TYPE_NAMES: [&str; 4] = [
    LOCAL_QA_RUN_REQUEST_TYPE_NAME,
    RUN_ACCEPTANCE_TYPE_NAME,
    LOCAL_QA_RUN_REQUEST_V2_TYPE_NAME,
    RUN_ACCEPTANCE_V2_TYPE_NAME,
];
const LOCAL_EXECUTOR_TYPE_NAMES: [&str; 4] = [
    "ExecutorDescriptor",
    "ExecutorSelection",
    "ExecutorRequest",
    "ExecutorResult",
];
const LOCAL_CANCELLATION_TYPE_NAMES: [&str; 6] = [
    "ControlStatus",
    "EffectDisposition",
    "IndependentOutcome",
    "CleanupOutcome",
    "CleanupReceipt",
    "SanitizedResidual",
];
const LOCAL_WORKER_CONTROL_TYPE_NAMES: [&str; 4] = [
    "LocalWorkerControlFrame",
    "LocalWorkerAbort",
    "LocalWorkerCancelAck",
    "LocalWorkerControlFailure",
];
const LOCAL_EXECUTOR_CONTROL_TYPE_NAMES: [&str; 2] =
    ["ExecutorControlRequest", "ExecutorControlReport"];
const MAX_DEPTH: usize = 128;
const MAX_SAFE_INTEGER_TEXT: &str = "9007199254740991";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Rejection {
    pub category: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    pub reason: String,
    pub path: String,
}

impl Rejection {
    fn canonical(code: &'static str, reason: &'static str) -> Self {
        Self {
            category: "canonicalization",
            code: Some(code),
            reason: reason.into(),
            path: "/".into(),
        }
    }

    fn contract(code: &'static str, reason: &'static str, path: impl Into<String>) -> Self {
        Self {
            category: "contract",
            code: Some(code),
            reason: reason.into(),
            path: path.into(),
        }
    }

    fn validation(reason: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            category: "validation",
            code: None,
            reason: reason.into(),
            path: path.into(),
        }
    }
}

#[derive(Debug, Error)]
#[error("{0:?}")]
pub struct ContractError(pub Rejection);

#[derive(Clone, Debug)]
pub struct AdmittedJson(Value);

impl AdmittedJson {
    pub fn value(&self) -> &Value {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct ValidatedValue(Value);

impl ValidatedValue {
    pub fn value(&self) -> &Value {
        &self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptBindingV2 {
    pub qa_task_id: String,
    pub qa_attempt_id: String,
    pub machine_id: String,
    pub worker_id: String,
    pub installation_id: String,
    pub generation: u64,
    pub fence_token: String,
    pub deadline: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DigestBoundReferenceV2 {
    pub kind: String,
    pub id: String,
    pub schema_version: String,
    pub content_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdmissionPolicyV2 {
    pub allow_network: bool,
    pub retain_workspace: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdmissionBudgetV2 {
    pub max_cases: u64,
    pub max_duration_ms: u64,
    pub max_evidence_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutorSelection {
    pub schema_version: String,
    pub executor_id: String,
    pub executor_version: String,
    pub capability_digest: String,
    pub required_capability: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalQARunRequestV2 {
    pub schema_version: String,
    pub content_digest: String,
    pub run_id: String,
    pub created_at: String,
    pub producer_version: String,
    pub profile: String,
    pub idempotency_key: String,
    pub nonce: String,
    pub attempt_binding: AttemptBindingV2,
    pub source: DigestBoundReferenceV2,
    pub test_case_set: DigestBoundReferenceV2,
    pub structured_plan: DigestBoundReferenceV2,
    pub package_manifest: DigestBoundReferenceV2,
    pub environment: DigestBoundReferenceV2,
    pub executor_selection: ExecutorSelection,
    pub policy: AdmissionPolicyV2,
    pub budget: AdmissionBudgetV2,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunAcceptanceV2 {
    pub schema_version: String,
    pub content_digest: String,
    pub run_id: String,
    pub created_at: String,
    pub producer_version: String,
    pub request_digest: String,
    pub idempotency_key: String,
    pub profile: String,
    pub state: String,
    pub accepted_at: String,
    pub event_sequence: u64,
    pub executor_selection: ExecutorSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum FoundationType {
    ContractMeta,
    HostScopedMeta,
    ResourceRef,
    ActorRef,
    DigestBoundRef,
    SignatureBlock,
    ProjectionSpecimen,
    StrictUnionSpecimen,
}

impl FoundationType {
    pub const ALL: [Self; 8] = [
        Self::ContractMeta,
        Self::HostScopedMeta,
        Self::ResourceRef,
        Self::ActorRef,
        Self::DigestBoundRef,
        Self::SignatureBlock,
        Self::ProjectionSpecimen,
        Self::StrictUnionSpecimen,
    ];

    pub const fn definition(self) -> &'static str {
        match self {
            Self::ContractMeta => "ContractMeta",
            Self::HostScopedMeta => "HostScopedMeta",
            Self::ResourceRef => "ResourceRef",
            Self::ActorRef => "ActorRef",
            Self::DigestBoundRef => "DigestBoundRef",
            Self::SignatureBlock => "SignatureBlock",
            Self::ProjectionSpecimen => "ProjectionSpecimen",
            Self::StrictUnionSpecimen => "StrictUnionSpecimen",
        }
    }

    const fn fixture_only(self) -> bool {
        matches!(self, Self::ProjectionSpecimen | Self::StrictUnionSpecimen)
    }
}

#[derive(Debug, Deserialize)]
struct Registry {
    registry_version: String,
    profile: String,
    schemas: BTreeMap<String, RegistrySchemaEntry>,
    types: BTreeMap<String, RegistryTypeEntry>,
}

#[derive(Debug, Deserialize)]
struct RegistrySchemaEntry {
    path: String,
    id: String,
    major: u64,
}

#[derive(Debug, Deserialize)]
struct RegistryTypeEntry {
    schema: String,
    pointer: String,
    #[serde(default)]
    fixture_only: bool,
}

pub fn contract_registry() -> Result<Value, ContractError> {
    validate_registry()?;
    serde_json::from_str(CONTRACT_REGISTRY)
        .map_err(|_| ContractError(Rejection::validation("invalid_embedded_registry", "/")))
}

pub fn admit_json(raw: &[u8]) -> Result<AdmittedJson, ContractError> {
    let text = std::str::from_utf8(raw).map_err(|_| {
        ContractError(Rejection::canonical(
            "canonicalization.invalid_utf8",
            "invalid_utf8",
        ))
    })?;
    if text.chars().next().is_some_and(char::is_whitespace)
        || text.chars().next_back().is_some_and(char::is_whitespace)
    {
        return Err(ContractError(Rejection::validation("invalid_json", "/")));
    }
    preflight_depth(text)?;
    preflight_numbers(text)?;
    validate_json_syntax(text)?;
    let mut deserializer = serde_json::Deserializer::from_str(text);
    deserializer.disable_recursion_limit();
    let value = StrictValue::deserialize(&mut deserializer)
        .map_err(|error| classify_strict_json_error(error, text))?
        .0;
    deserializer.end().map_err(classify_json_error)?;
    ensure_depth(&value, 0)?;
    Ok(AdmittedJson(value))
}

pub fn validate_foundation(
    raw: &[u8],
    foundation_type: FoundationType,
) -> Result<ValidatedValue, ContractError> {
    validate_value(admit_json(raw)?, foundation_type)
}

pub fn validate_local_state(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, LOCAL_STATE_TYPE_NAME)
}

pub fn validate_execution_outcome(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, EXECUTION_OUTCOME_TYPE_NAME)
}

pub fn validate_cancel_disposition(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, CANCEL_DISPOSITION_TYPE_NAME)
}

pub fn validate_event_sequence(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, EVENT_SEQUENCE_TYPE_NAME)
}

pub fn validate_event_cursor(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, EVENT_CURSOR_TYPE_NAME)
}

pub fn validate_local_sanitized_observation(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    let validated =
        validate_registered_value(admit_json(raw)?, LOCAL_SANITIZED_OBSERVATION_TYPE_NAME)?;
    validate_local_sanitized_observation_rules(validated.value())?;
    Ok(validated)
}

pub fn validate_local_evidence_object(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, LOCAL_EVIDENCE_OBJECT_TYPE_NAME)
}

pub fn validate_local_sanitized_observation_ref(
    raw: &[u8],
) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, LOCAL_SANITIZED_OBSERVATION_REF_TYPE_NAME)
}

pub fn validate_local_evidence_object_ref(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, LOCAL_EVIDENCE_OBJECT_REF_TYPE_NAME)
}

pub fn validate_local_worker_frame(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    let validated = validate_registered_value(admit_json(raw)?, "LocalWorkerFrame")?;
    validate_local_worker_rules(validated.value())?;
    Ok(validated)
}

pub fn validate_local_worker_invocation(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    let validated = validate_registered_value(admit_json(raw)?, "LocalWorkerInvocation")?;
    validate_local_worker_rules(validated.value())?;
    Ok(validated)
}

pub fn validate_local_worker_capability_request(
    raw: &[u8],
) -> Result<ValidatedValue, ContractError> {
    let validated = validate_registered_value(admit_json(raw)?, "LocalWorkerCapabilityRequest")?;
    validate_local_worker_rules(validated.value())?;
    Ok(validated)
}

pub fn validate_local_worker_capability_result(
    raw: &[u8],
) -> Result<ValidatedValue, ContractError> {
    let validated = validate_registered_value(admit_json(raw)?, "LocalWorkerCapabilityResult")?;
    validate_local_worker_rules(validated.value())?;
    Ok(validated)
}

pub fn validate_local_worker_terminal_result(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    let validated = validate_registered_value(admit_json(raw)?, "LocalWorkerTerminalResult")?;
    validate_local_worker_rules(validated.value())?;
    Ok(validated)
}

pub fn validate_local_worker_protocol_failure(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "LocalWorkerProtocolFailure")
}

pub fn validate_local_qa_run_request(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_local_qa_run_request_value(admit_json(raw)?)
}

pub fn validate_run_acceptance(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_run_acceptance_value(admit_json(raw)?)
}

pub fn validate_local_qa_run_request_v2(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    let admitted = admit_json(raw)?;
    let nonce_has_invalid_encoding = admitted
        .0
        .get("nonce")
        .and_then(Value::as_str)
        .is_some_and(|nonce| !validate_base64url_no_pad(nonce));
    let fence_token_has_invalid_encoding = admitted
        .0
        .pointer("/attempt_binding/fence_token")
        .and_then(Value::as_str)
        .is_some_and(|fence_token| !validate_base64url_no_pad(fence_token));
    let validated = match validate_registered_value(admitted, LOCAL_QA_RUN_REQUEST_V2_TYPE_NAME) {
        Ok(validated) => validated,
        Err(error) if error.0.reason == "schema_violation" && nonce_has_invalid_encoding => {
            return Err(ContractError(Rejection::contract(
                "contract.invalid_encoding",
                "invalid_encoding",
                "/nonce",
            )));
        }
        Err(error) if error.0.reason == "schema_violation" && fence_token_has_invalid_encoding => {
            return Err(ContractError(Rejection::contract(
                "contract.invalid_encoding",
                "invalid_encoding",
                "/attempt_binding/fence_token",
            )));
        }
        Err(error) => return Err(error),
    };
    let request: LocalQARunRequestV2 = serde_json::from_value(validated.0.clone())
        .map_err(|_| ContractError(Rejection::validation("schema_violation", "/")))?;
    if !validate_base64url_no_pad(&request.nonce) {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_encoding",
            "invalid_encoding",
            "/nonce",
        )));
    }
    if !validate_base64url_no_pad(&request.attempt_binding.fence_token) {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_encoding",
            "invalid_encoding",
            "/attempt_binding/fence_token",
        )));
    }
    if request.producer_version.len() > 128 {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/producer_version",
        )));
    }
    if !validate_iso8601(&request.created_at)
        || !validate_iso8601(&request.attempt_binding.deadline)
    {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/created_at",
        )));
    }
    if !compare_iso8601(&request.created_at, &request.attempt_binding.deadline).is_lt() {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "invalid_attempt_window",
            "/attempt_binding/deadline",
        )));
    }
    verify_contract_content_digest(&validated)?;
    Ok(validated)
}

pub fn validate_run_acceptance_v2(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    let validated = validate_registered_value(admit_json(raw)?, RUN_ACCEPTANCE_V2_TYPE_NAME)?;
    let acceptance: RunAcceptanceV2 = serde_json::from_value(validated.0.clone())
        .map_err(|_| ContractError(Rejection::validation("schema_violation", "/")))?;
    if !validate_iso8601(&acceptance.created_at) || !validate_iso8601(&acceptance.accepted_at) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/created_at",
        )));
    }
    if acceptance.created_at != acceptance.accepted_at {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "accepted_at_mismatch",
            "/created_at",
        )));
    }
    if acceptance.producer_version.len() > 128 {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/producer_version",
        )));
    }
    verify_contract_content_digest(&validated)?;
    Ok(validated)
}

pub fn build_initial_run_acceptance_v2(
    request: &ValidatedValue,
    accepted_at: &str,
    producer_version: &str,
) -> Result<ValidatedValue, ContractError> {
    let request_bytes = canonical_bytes(request)?;
    let validated = validate_local_qa_run_request_v2(&request_bytes)?;
    let request: LocalQARunRequestV2 = serde_json::from_value(validated.0)
        .map_err(|_| ContractError(Rejection::validation("schema_violation", "/")))?;
    if !validate_iso8601(accepted_at) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/accepted_at",
        )));
    }
    if producer_version.is_empty() || producer_version.len() > 128 {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/producer_version",
        )));
    }
    if compare_iso8601(accepted_at, &request.created_at).is_lt()
        || !compare_iso8601(accepted_at, &request.attempt_binding.deadline).is_lt()
    {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "accepted_at_out_of_window",
            "/accepted_at",
        )));
    }
    let mut value = serde_json::json!({
        "schema_version": "qa.local-run-admission/v2",
        "run_id": request.run_id,
        "created_at": accepted_at,
        "producer_version": producer_version,
        "request_digest": request.content_digest,
        "idempotency_key": request.idempotency_key,
        "profile": "local_qa_agent_mvp",
        "state": "accepted",
        "accepted_at": accepted_at,
        "event_sequence": 1,
        "executor_selection": request.executor_selection,
    });
    let digest = contract_content_digest(&ValidatedValue(value.clone()))?;
    value
        .as_object_mut()
        .expect("acceptance object")
        .insert("content_digest".into(), Value::String(digest));
    validate_run_acceptance_v2(
        &serde_json::to_vec(&value)
            .map_err(|_| ContractError(Rejection::validation("schema_violation", "/")))?,
    )
}

pub fn validate_executor_descriptor(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    let validated = validate_registered_value(admit_json(raw)?, "ExecutorDescriptor")?;
    validate_executor_descriptor_rules(validated.value())?;
    Ok(validated)
}

pub fn validate_executor_selection(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "ExecutorSelection")
}

pub fn validate_executor_request(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "ExecutorRequest")
}

pub fn validate_executor_result(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "ExecutorResult")
}

pub fn validate_local_worker_control_frame(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "LocalWorkerControlFrame")
}

pub fn validate_local_worker_abort(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "LocalWorkerAbort")
}

pub fn validate_local_worker_cancel_ack(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "LocalWorkerCancelAck")
}

pub fn validate_local_worker_control_failure(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "LocalWorkerControlFailure")
}

pub fn validate_executor_control_request(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "ExecutorControlRequest")
}

pub fn validate_executor_control_report(raw: &[u8]) -> Result<ValidatedValue, ContractError> {
    validate_registered_value(admit_json(raw)?, "ExecutorControlReport")
}

pub fn build_initial_run_acceptance(
    request: &ValidatedValue,
    accepted_at: &str,
    producer_version: &str,
) -> Result<ValidatedValue, ContractError> {
    let validated_request = validate_local_qa_run_request_value(AdmittedJson(request.0.clone()))?;
    let request_value = validated_request
        .0
        .as_object()
        .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/")))?;
    if !validate_iso8601(accepted_at) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/accepted_at",
        )));
    }
    if producer_version.is_empty() {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/producer_version",
        )));
    }
    let created_at = required_string(request_value, "created_at")?;
    let expires_at = required_string(request_value, "expires_at")?;
    if compare_iso8601(accepted_at, created_at).is_lt()
        || !compare_iso8601(accepted_at, expires_at).is_lt()
    {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "accepted_at_out_of_window",
            "/accepted_at",
        )));
    }
    let mut acceptance = Map::new();
    acceptance.insert(
        "schema_version".into(),
        Value::String("qa.local-run-admission/v1".into()),
    );
    acceptance.insert(
        "run_id".into(),
        Value::String(required_string(request_value, "run_id")?.into()),
    );
    acceptance.insert("created_at".into(), Value::String(accepted_at.into()));
    acceptance.insert(
        "producer_version".into(),
        Value::String(producer_version.into()),
    );
    acceptance.insert(
        "request_digest".into(),
        Value::String(required_string(request_value, "content_digest")?.into()),
    );
    acceptance.insert(
        "idempotency_key".into(),
        Value::String(required_string(request_value, "idempotency_key")?.into()),
    );
    acceptance.insert("state".into(), Value::String("accepted".into()));
    acceptance.insert("accepted_at".into(), Value::String(accepted_at.into()));
    let projected = ValidatedValue(Value::Object(acceptance.clone()));
    acceptance.insert(
        "content_digest".into(),
        Value::String(contract_content_digest(&projected)?),
    );
    validate_run_acceptance(&canonicalize(&Value::Object(acceptance))?)
}

pub fn encode_local_worker_frame(value: &ValidatedValue) -> Result<Vec<u8>, ContractError> {
    let payload = canonical_bytes(value)?;
    if payload.is_empty() || payload.len() > LOCAL_WORKER_MAX_FRAME_BYTES {
        return Err(ContractError(Rejection::validation(
            "frame_length_out_of_range",
            "/",
        )));
    }
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

const LOCAL_WORKER_CAPABILITY_SEQUENCE: [&str; 7] = [
    "clock.now/v1",
    "clock.monotonic-ms/v1",
    "browser-session.run/v1",
    "evidence.stage-fixed-runner-log/v1",
    "browser-session.close/v1",
    "clock.now/v1",
    "clock.monotonic-ms/v1",
];

#[derive(Default)]
pub struct LocalWorkerInputSequence {
    invocation_id: Option<String>,
    next_frame: usize,
}

impl LocalWorkerInputSequence {
    pub fn accept(&mut self, frame: &ValidatedValue) -> Result<(), ContractError> {
        let object = frame
            .value()
            .as_object()
            .ok_or_else(|| ContractError(Rejection::validation("invalid_sequence", "/")))?;
        if self.next_frame == 0 {
            if object.get("kind").and_then(Value::as_str) != Some("invocation") {
                return Err(ContractError(Rejection::validation(
                    "invalid_sequence",
                    "/kind",
                )));
            }
            self.invocation_id = object
                .get("invocation_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            self.next_frame = 1;
            return Ok(());
        }
        if self.next_frame > LOCAL_WORKER_CAPABILITY_SEQUENCE.len() {
            return Err(ContractError(Rejection::validation("trailing_input", "/")));
        }
        if object.get("kind").and_then(Value::as_str) != Some("capability_result") {
            return Err(ContractError(Rejection::validation(
                "invalid_sequence",
                "/kind",
            )));
        }
        let index = self.next_frame - 1;
        if object.get("invocation_id").and_then(Value::as_str) != self.invocation_id.as_deref()
            || object.get("request_id").and_then(Value::as_str)
                != Some(format!("capability/{index}").as_str())
            || object.get("capability").and_then(Value::as_str)
                != Some(LOCAL_WORKER_CAPABILITY_SEQUENCE[index])
        {
            return Err(ContractError(Rejection::contract(
                "contract.invalid_relation",
                "capability_mismatch",
                "/request_id",
            )));
        }
        self.next_frame += 1;
        Ok(())
    }

    pub fn finish(&self) -> Result<(), ContractError> {
        if self.next_frame == LOCAL_WORKER_CAPABILITY_SEQUENCE.len() + 1 {
            Ok(())
        } else {
            Err(ContractError(Rejection::validation("unexpected_eof", "/")))
        }
    }
}

#[derive(Default)]
pub struct LocalWorkerFrameDecoder {
    buffer: Vec<u8>,
}

impl LocalWorkerFrameDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ValidatedValue>, ContractError> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        let mut offset = 0;
        while self.buffer.len().saturating_sub(offset) >= 4 {
            let length = u32::from_be_bytes(
                self.buffer[offset..offset + 4]
                    .try_into()
                    .expect("four-byte prefix"),
            ) as usize;
            if length == 0 || length > LOCAL_WORKER_MAX_FRAME_BYTES {
                return Err(ContractError(Rejection::validation(
                    "frame_length_out_of_range",
                    "/",
                )));
            }
            if self.buffer.len() - offset - 4 < length {
                break;
            }
            frames.push(validate_local_worker_frame(
                &self.buffer[offset + 4..offset + 4 + length],
            )?);
            offset += 4 + length;
        }
        if offset > 0 {
            self.buffer.drain(..offset);
        }
        Ok(frames)
    }

    pub fn finish(&self) -> Result<(), ContractError> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(ContractError(Rejection::validation("truncated_frame", "/")))
        }
    }
}

pub fn validate_value(
    admitted: AdmittedJson,
    foundation_type: FoundationType,
) -> Result<ValidatedValue, ContractError> {
    let value = admitted.0;
    validate_special_rules(&value, foundation_type)?;
    validate_registered_value(AdmittedJson(value), foundation_type.definition())
}

fn validate_registered_value(
    admitted: AdmittedJson,
    type_name: &str,
) -> Result<ValidatedValue, ContractError> {
    let value = admitted.0;
    let schema = schema_for_type(type_name)?;
    let validator = jsonschema::draft202012::new(&schema)
        .map_err(|_| ContractError(Rejection::validation("invalid_embedded_schema", "/")))?;
    if let Err(error) = validator.validate(&value) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            pointer_or_root(error.instance_path().as_str()),
        )));
    }
    Ok(ValidatedValue(value))
}

fn validate_local_qa_run_request_value(
    admitted: AdmittedJson,
) -> Result<ValidatedValue, ContractError> {
    let nonce_has_invalid_encoding = admitted
        .0
        .get("nonce")
        .and_then(Value::as_str)
        .is_some_and(|nonce| !validate_base64url_no_pad(nonce));
    let validated = match validate_registered_value(admitted, LOCAL_QA_RUN_REQUEST_TYPE_NAME) {
        Ok(validated) => validated,
        Err(error) if error.0.reason == "schema_violation" && nonce_has_invalid_encoding => {
            return Err(ContractError(Rejection::contract(
                "contract.invalid_encoding",
                "invalid_encoding",
                "/nonce",
            )));
        }
        Err(error) => return Err(error),
    };
    let value = validated
        .0
        .as_object()
        .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/")))?;
    if !required_string(value, "created_at").is_ok_and(validate_iso8601) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/created_at",
        )));
    }
    if !required_string(value, "expires_at").is_ok_and(validate_iso8601) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/expires_at",
        )));
    }
    if !required_string(value, "nonce").is_ok_and(validate_base64url_no_pad) {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_encoding",
            "invalid_encoding",
            "/nonce",
        )));
    }
    let created_at = required_string(value, "created_at")?;
    let expires_at = required_string(value, "expires_at")?;
    if !compare_iso8601(created_at, expires_at).is_lt() {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "invalid_request_window",
            "/expires_at",
        )));
    }
    verify_contract_content_digest(&validated)?;
    Ok(validated)
}

fn validate_run_acceptance_value(admitted: AdmittedJson) -> Result<ValidatedValue, ContractError> {
    let validated = validate_registered_value(admitted, RUN_ACCEPTANCE_TYPE_NAME)?;
    let value = validated
        .0
        .as_object()
        .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/")))?;
    let created_at = required_string(value, "created_at")?;
    let accepted_at = required_string(value, "accepted_at")?;
    if !validate_iso8601(created_at) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/created_at",
        )));
    }
    if !validate_iso8601(accepted_at) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/accepted_at",
        )));
    }
    if created_at != accepted_at {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "accepted_at_mismatch",
            "/created_at",
        )));
    }
    verify_contract_content_digest(&validated)?;
    Ok(validated)
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, ContractError> {
    object.get(field).and_then(Value::as_str).ok_or_else(|| {
        ContractError(Rejection::validation(
            "schema_violation",
            format!("/{}", pointer_token(field)),
        ))
    })
}

pub fn validate_scalar(name: &str, value: &str) -> Result<(), ContractError> {
    let valid = match name {
        "ISO8601" => validate_iso8601(value),
        "Sha256" => validate_sha256(value),
        "Base64UrlNoPad" => validate_base64url_no_pad(value),
        "UUID" => validate_uuid(value),
        "SchemaVersion" => parse_schema_major(value).is_some(),
        _ => return Err(ContractError(Rejection::validation("unknown_scalar", "/"))),
    };
    if valid {
        Ok(())
    } else if name == "Base64UrlNoPad" {
        Err(ContractError(Rejection::contract(
            "contract.invalid_encoding",
            "invalid_encoding",
            "/",
        )))
    } else {
        Err(ContractError(Rejection::validation("invalid_scalar", "/")))
    }
}

pub fn canonical_bytes(value: &ValidatedValue) -> Result<Vec<u8>, ContractError> {
    canonicalize(&value.0)
}

pub fn canonical_admitted_bytes(value: &AdmittedJson) -> Result<Vec<u8>, ContractError> {
    canonicalize(&value.0)
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

pub fn contract_content_projection(value: &ValidatedValue) -> Result<Vec<u8>, ContractError> {
    let mut projected = value.0.clone();
    let root = projected
        .as_object_mut()
        .ok_or_else(|| ContractError(Rejection::validation("projection_requires_object", "/")))?;
    root.remove("content_digest");
    root.remove("signature");
    canonicalize(&projected)
}

pub fn contract_content_digest(value: &ValidatedValue) -> Result<String, ContractError> {
    Ok(sha256_digest(&contract_content_projection(value)?))
}

pub fn verify_contract_content_digest(value: &ValidatedValue) -> Result<(), ContractError> {
    let observed = value
        .0
        .get("content_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ContractError(Rejection::validation(
                "missing_content_digest",
                "/content_digest",
            ))
        })?;
    let expected = contract_content_digest(value)?;
    if observed == expected {
        Ok(())
    } else {
        Err(ContractError(Rejection::contract(
            "contract.digest_mismatch",
            "digest_mismatch",
            "/content_digest",
        )))
    }
}

fn validate_registry() -> Result<Registry, ContractError> {
    let registry: Registry = serde_json::from_str(CONTRACT_REGISTRY)
        .map_err(|_| ContractError(Rejection::validation("invalid_embedded_registry", "/")))?;
    validate_registry_value(&registry)?;
    Ok(registry)
}

fn validate_registry_value(registry: &Registry) -> Result<(), ContractError> {
    if registry.registry_version != "qa.contract-registry/v1"
        || registry.profile != "local_qa_host_mvp"
    {
        return Err(ContractError(Rejection::validation(
            "invalid_embedded_registry",
            "/",
        )));
    }
    for foundation_type in FoundationType::ALL {
        schema_for_registered_type(registry, foundation_type.definition(), embedded_schema)?;
        let entry = registry
            .types
            .get(foundation_type.definition())
            .expect("registered type was resolved above");
        if entry.fixture_only != foundation_type.fixture_only() {
            return Err(ContractError(Rejection::validation(
                "invalid_embedded_registry",
                format!("/types/{}", foundation_type.definition()),
            )));
        }
    }
    for type_name in LIFECYCLE_TYPE_NAMES {
        schema_for_registered_type(registry, type_name, embedded_schema)?;
        let entry = registry
            .types
            .get(type_name)
            .expect("registered type was resolved above");
        if entry.fixture_only {
            return Err(ContractError(Rejection::validation(
                "invalid_embedded_registry",
                format!("/types/{type_name}"),
            )));
        }
    }
    for type_name in LOCAL_EVIDENCE_TYPE_NAMES {
        schema_for_registered_type(registry, type_name, embedded_schema)?;
        let entry = registry
            .types
            .get(type_name)
            .expect("registered type was resolved above");
        if entry.fixture_only {
            return Err(ContractError(Rejection::validation(
                "invalid_embedded_registry",
                format!("/types/{type_name}"),
            )));
        }
    }
    for type_name in LOCAL_WORKER_TYPE_NAMES {
        schema_for_registered_type(registry, type_name, embedded_schema)?;
        let entry = registry
            .types
            .get(type_name)
            .expect("registered type was resolved above");
        if entry.fixture_only {
            return Err(ContractError(Rejection::validation(
                "invalid_embedded_registry",
                format!("/types/{type_name}"),
            )));
        }
    }
    for type_name in LOCAL_RUN_ADMISSION_TYPE_NAMES {
        schema_for_registered_type(registry, type_name, embedded_schema)?;
        let entry = registry
            .types
            .get(type_name)
            .expect("registered type was resolved above");
        if entry.fixture_only {
            return Err(ContractError(Rejection::validation(
                "invalid_embedded_registry",
                format!("/types/{type_name}"),
            )));
        }
    }
    for type_name in LOCAL_EXECUTOR_TYPE_NAMES {
        schema_for_registered_type(registry, type_name, embedded_schema)?;
        let entry = registry
            .types
            .get(type_name)
            .expect("registered type was resolved above");
        if entry.fixture_only {
            return Err(ContractError(Rejection::validation(
                "invalid_embedded_registry",
                format!("/types/{type_name}"),
            )));
        }
    }
    for type_name in LOCAL_CANCELLATION_TYPE_NAMES
        .into_iter()
        .chain(LOCAL_WORKER_CONTROL_TYPE_NAMES)
        .chain(LOCAL_EXECUTOR_CONTROL_TYPE_NAMES)
    {
        schema_for_registered_type(registry, type_name, embedded_schema)?;
        let entry = registry
            .types
            .get(type_name)
            .expect("registered type was resolved above");
        if entry.fixture_only {
            return Err(ContractError(Rejection::validation(
                "invalid_embedded_registry",
                format!("/types/{type_name}"),
            )));
        }
    }
    Ok(())
}

fn validate_executor_descriptor_rules(value: &Value) -> Result<(), ContractError> {
    let object = value
        .as_object()
        .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/")))?;
    let capabilities = object
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/capabilities")))?;
    if capabilities
        .windows(2)
        .any(|pair| pair[0].as_str() >= pair[1].as_str())
    {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "capabilities_not_sorted",
            "/capabilities",
        )));
    }
    let mut projection = Map::new();
    for key in [
        "capabilities",
        "executor_id",
        "executor_version",
        "schema_version",
    ] {
        projection.insert(
            key.into(),
            object
                .get(key)
                .cloned()
                .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/")))?,
        );
    }
    let digest = sha256_digest(&canonicalize(&Value::Object(projection))?);
    if object.get("capability_digest").and_then(Value::as_str) != Some(digest.as_str()) {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "capability_digest_mismatch",
            "/capability_digest",
        )));
    }
    Ok(())
}

fn validate_local_worker_rules(value: &Value) -> Result<(), ContractError> {
    validate_local_worker_urls(value)?;
    let Some(object) = value.as_object() else {
        return Ok(());
    };

    if object.get("kind").and_then(Value::as_str) == Some("capability_result") {
        let capability = object.get("capability").and_then(Value::as_str);
        let output = object
            .get("output")
            .and_then(Value::as_object)
            .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/output")))?;
        if capability == Some("clock.now/v1") {
            let timestamp = output.get("value").and_then(Value::as_str).ok_or_else(|| {
                ContractError(Rejection::validation("invalid_timestamp", "/output/value"))
            })?;
            if !validate_worker_timestamp(timestamp) {
                return Err(ContractError(Rejection::validation(
                    "invalid_timestamp",
                    "/output/value",
                )));
            }
        }
        if capability == Some("browser-session.run/v1") {
            validate_expected_reference(
                output.get("sanitizedObservationRef"),
                "observation/0",
                "/output/sanitizedObservationRef/id",
            )?;
            validate_expected_reference(
                output.get("screenshotEvidenceRef"),
                "evidence/0",
                "/output/screenshotEvidenceRef/id",
            )?;
        }
        if capability == Some("evidence.stage-fixed-runner-log/v1") {
            validate_expected_reference(
                output.get("runnerLogEvidenceRef"),
                "evidence/1",
                "/output/runnerLogEvidenceRef/id",
            )?;
        }
    }

    if object.get("kind").and_then(Value::as_str) == Some("terminal_result") {
        let result = object
            .get("result")
            .and_then(Value::as_object)
            .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/result")))?;
        validate_terminal_result_relations(result)?;
    }
    Ok(())
}

fn validate_expected_reference(
    value: Option<&Value>,
    expected_id: &str,
    path: &str,
) -> Result<(), ContractError> {
    if value
        .and_then(Value::as_object)
        .and_then(|reference| reference.get("id"))
        .and_then(Value::as_str)
        == Some(expected_id)
    {
        Ok(())
    } else {
        Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "reference_id_mismatch",
            path,
        )))
    }
}

fn validate_terminal_result_relations(result: &Map<String, Value>) -> Result<(), ContractError> {
    let started_at = result
        .get("startedAt")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ContractError(Rejection::validation(
                "invalid_timestamp",
                "/result/startedAt",
            ))
        })?;
    let finished_at = result
        .get("finishedAt")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ContractError(Rejection::validation(
                "invalid_timestamp",
                "/result/finishedAt",
            ))
        })?;
    let started_ms = worker_timestamp_millis(started_at).ok_or_else(|| {
        ContractError(Rejection::validation(
            "invalid_timestamp",
            "/result/startedAt",
        ))
    })?;
    let finished_ms = worker_timestamp_millis(finished_at).ok_or_else(|| {
        ContractError(Rejection::validation(
            "invalid_timestamp",
            "/result/finishedAt",
        ))
    })?;
    if started_ms > finished_ms {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "finished_before_started",
            "/result/finishedAt",
        )));
    }
    let duration_ms = result
        .get("durationMs")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ContractError(Rejection::validation(
                "schema_violation",
                "/result/durationMs",
            ))
        })?;
    if finished_ms - started_ms != duration_ms {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "duration_mismatch",
            "/result/durationMs",
        )));
    }
    let observation = result
        .get("observation")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ContractError(Rejection::validation(
                "schema_violation",
                "/result/observation",
            ))
        })?;
    validate_expected_reference(
        observation.get("sanitizedObservationRef"),
        "observation/0",
        "/result/observation/sanitizedObservationRef/id",
    )?;
    let evidence = result
        .get("evidence")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ContractError(Rejection::validation(
                "schema_violation",
                "/result/evidence",
            ))
        })?;
    for (index, expected_id) in ["evidence/0", "evidence/1"].into_iter().enumerate() {
        let entry = evidence[index].as_object().ok_or_else(|| {
            ContractError(Rejection::validation(
                "schema_violation",
                format!("/result/evidence/{index}"),
            ))
        })?;
        if entry.get("objectId").and_then(Value::as_str) != Some(expected_id) {
            return Err(ContractError(Rejection::contract(
                "contract.invalid_relation",
                "object_id_mismatch",
                format!("/result/evidence/{index}/objectId"),
            )));
        }
        validate_expected_reference(
            entry.get("artifactRef"),
            expected_id,
            &format!("/result/evidence/{index}/artifactRef/id"),
        )?;
    }
    Ok(())
}

fn validate_worker_timestamp(value: &str) -> bool {
    worker_timestamp_millis(value).is_some()
}

fn worker_timestamp_millis(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() != 24
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
        || !bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 23) || byte.is_ascii_digit()
        })
    {
        return None;
    }
    let year = u64::from(decimal_field(bytes, 0, 4)?);
    let month = u64::from(decimal_field(bytes, 5, 7)?);
    let day = u64::from(decimal_field(bytes, 8, 10)?);
    let hour = u64::from(decimal_field(bytes, 11, 13)?);
    let minute = u64::from(decimal_field(bytes, 14, 16)?);
    let second = u64::from(decimal_field(bytes, 17, 19)?);
    let millisecond = u64::from(decimal_field(bytes, 20, 23)?);
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = [
        31,
        if leap_year { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day == 0 || day > days_in_month[(month - 1) as usize] {
        return None;
    }
    let days_before_year = 365 * (year - 1) + (year - 1) / 4 - (year - 1) / 100 + (year - 1) / 400;
    let days_before_month: u64 = days_in_month[..(month - 1) as usize].iter().copied().sum();
    let days = days_before_year + days_before_month + day - 1;
    Some((((days * 24 + hour) * 60 + minute) * 60 + second) * 1000 + millisecond)
}

fn validate_local_worker_urls(value: &Value) -> Result<(), ContractError> {
    match value {
        Value::Array(items) => {
            for item in items {
                validate_local_worker_urls(item)?;
            }
        }
        Value::Object(object) => {
            for (key, child) in object {
                if matches!(key.as_str(), "fixtureUrl" | "finalUrl")
                    && !child.as_str().is_some_and(is_fixed_fixture_url)
                {
                    return Err(ContractError(Rejection::validation(
                        "schema_violation",
                        format!("/{key}"),
                    )));
                }
                validate_local_worker_urls(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_local_sanitized_observation_rules(value: &Value) -> Result<(), ContractError> {
    let object = value
        .as_object()
        .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/")))?;
    let fixture_url = object
        .get("fixture_url")
        .and_then(Value::as_str)
        .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/fixture_url")))?;
    let final_url = object
        .get("final_url")
        .and_then(Value::as_str)
        .ok_or_else(|| ContractError(Rejection::validation("schema_violation", "/final_url")))?;
    if !is_fixed_fixture_url(fixture_url) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/fixture_url",
        )));
    }
    if !is_fixed_fixture_url(final_url) {
        return Err(ContractError(Rejection::validation(
            "schema_violation",
            "/final_url",
        )));
    }
    if final_url != fixture_url {
        return Err(ContractError(Rejection::contract(
            "contract.invalid_relation",
            "fixture_url_mismatch",
            "/final_url",
        )));
    }
    Ok(())
}

fn is_fixed_fixture_url(value: &str) -> bool {
    let Some(port) = value
        .strip_prefix("http://127.0.0.1:")
        .and_then(|remainder| remainder.strip_suffix("/fixed-page.html"))
    else {
        return false;
    };
    !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port
            .parse::<u32>()
            .is_ok_and(|port_number| (1..=65_535).contains(&port_number))
}

fn schema_for_type(type_name: &str) -> Result<Value, ContractError> {
    let registry = validate_registry()?;
    schema_for_registered_type(&registry, type_name, embedded_schema)
}

fn schema_for_registered_type(
    registry: &Registry,
    type_name: &str,
    load_schema: fn(&str) -> Option<&'static str>,
) -> Result<Value, ContractError> {
    let type_entry = registry
        .types
        .get(type_name)
        .ok_or_else(|| ContractError(Rejection::validation("unknown_registered_type", "/types")))?;
    let schema_entry = registry.schemas.get(&type_entry.schema).ok_or_else(|| {
        ContractError(Rejection::validation(
            "unknown_registered_schema",
            "/schemas",
        ))
    })?;
    let registered_major = type_entry
        .schema
        .rsplit_once("/v")
        .and_then(|(_, major)| major.parse::<u64>().ok());
    if !SUPPORTED_SCHEMA_MAJORS.contains(&schema_entry.major)
        || registered_major != Some(schema_entry.major)
    {
        return Err(ContractError(Rejection::validation(
            "unsupported_schema_major",
            format!("/schemas/{}/major", pointer_token(&type_entry.schema)),
        )));
    }
    validate_package_path(&schema_entry.path)?;
    let schema_source = load_schema(&schema_entry.path).ok_or_else(|| {
        ContractError(Rejection::validation(
            "invalid_embedded_schema_path",
            format!("/schemas/{}/path", pointer_token(&type_entry.schema)),
        ))
    })?;
    let schema: Value = serde_json::from_str(schema_source)
        .map_err(|_| ContractError(Rejection::validation("invalid_embedded_schema", "/")))?;
    if schema.get("$id").and_then(Value::as_str) != Some(schema_entry.id.as_str()) {
        return Err(ContractError(Rejection::validation(
            "invalid_embedded_schema",
            "/$id",
        )));
    }
    let mut schema = resolve_registered_references(schema, registry, load_schema)?;
    let pointer = type_entry.pointer.strip_prefix('#').ok_or_else(|| {
        ContractError(Rejection::validation(
            "unresolved_registered_pointer",
            "/types",
        ))
    })?;
    if !valid_json_pointer(pointer) || schema.pointer(pointer).is_none() {
        return Err(ContractError(Rejection::validation(
            "unresolved_registered_pointer",
            "/types",
        )));
    }
    let schema_object = schema
        .as_object_mut()
        .ok_or_else(|| ContractError(Rejection::validation("invalid_embedded_schema", "/")))?;
    schema_object.remove("$id");
    schema_object.insert("$ref".into(), Value::String(type_entry.pointer.clone()));
    Ok(schema)
}

fn embedded_schema(path: &str) -> Option<&'static str> {
    EMBEDDED_SCHEMAS
        .iter()
        .find_map(|(registered_path, schema)| (*registered_path == path).then_some(*schema))
}

fn validate_package_path(path: &str) -> Result<(), ContractError> {
    if path.is_empty()
        || path.contains(':')
        || Path::new(path).is_absolute()
        || Path::new(path)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ContractError(Rejection::validation(
            "invalid_embedded_schema_path",
            "/schemas",
        )));
    }
    Ok(())
}

fn resolve_registered_references(
    value: Value,
    registry: &Registry,
    load_schema: fn(&str) -> Option<&'static str>,
) -> Result<Value, ContractError> {
    let mut active_references = BTreeSet::new();
    let mut resolved_reference_count = 0;
    resolve_registered_reference_value(
        value,
        registry,
        load_schema,
        &mut active_references,
        &mut resolved_reference_count,
    )
}

fn resolve_registered_reference_value(
    value: Value,
    registry: &Registry,
    load_schema: fn(&str) -> Option<&'static str>,
    active_references: &mut BTreeSet<String>,
    resolved_reference_count: &mut usize,
) -> Result<Value, ContractError> {
    match value {
        Value::Array(items) => Ok(Value::Array(
            items
                .into_iter()
                .map(|item| {
                    resolve_registered_reference_value(
                        item,
                        registry,
                        load_schema,
                        active_references,
                        resolved_reference_count,
                    )
                })
                .collect::<Result<_, _>>()?,
        )),
        Value::Object(mut object) => {
            let external_reference = object
                .get("$ref")
                .and_then(Value::as_str)
                .filter(|reference| !reference.starts_with('#'))
                .map(str::to_owned);
            if let Some(reference) = external_reference {
                object.remove("$ref");
                let imported = import_registered_reference(
                    &reference,
                    registry,
                    load_schema,
                    active_references,
                    resolved_reference_count,
                )?;
                if object.is_empty() {
                    return Ok(imported);
                }
                let siblings = resolve_registered_reference_value(
                    Value::Object(object),
                    registry,
                    load_schema,
                    active_references,
                    resolved_reference_count,
                )?;
                return Ok(serde_json::json!({ "allOf": [imported, siblings] }));
            }
            for child in object.values_mut() {
                *child = resolve_registered_reference_value(
                    child.take(),
                    registry,
                    load_schema,
                    active_references,
                    resolved_reference_count,
                )?;
            }
            Ok(Value::Object(object))
        }
        scalar => Ok(scalar),
    }
}

fn import_registered_reference(
    reference: &str,
    registry: &Registry,
    load_schema: fn(&str) -> Option<&'static str>,
    active_references: &mut BTreeSet<String>,
    resolved_reference_count: &mut usize,
) -> Result<Value, ContractError> {
    if *resolved_reference_count >= MAX_DEPTH || !active_references.insert(reference.to_owned()) {
        return Err(ContractError(Rejection::validation(
            "invalid_embedded_schema",
            "/$ref",
        )));
    }
    *resolved_reference_count += 1;
    let result = import_registered_reference_inner(
        reference,
        registry,
        load_schema,
        active_references,
        resolved_reference_count,
    );
    active_references.remove(reference);
    result
}

fn import_registered_reference_inner(
    reference: &str,
    registry: &Registry,
    load_schema: fn(&str) -> Option<&'static str>,
    active_references: &mut BTreeSet<String>,
    resolved_reference_count: &mut usize,
) -> Result<Value, ContractError> {
    let (_, schema_entry) = registry
        .schemas
        .iter()
        .find(|(_, entry)| {
            reference == entry.id || reference.starts_with(&format!("{}#", entry.id))
        })
        .ok_or_else(|| {
            ContractError(Rejection::validation("external_schema_reference", "/$ref"))
        })?;
    validate_package_path(&schema_entry.path)?;
    let schema_source = load_schema(&schema_entry.path).ok_or_else(|| {
        ContractError(Rejection::validation(
            "invalid_embedded_schema_path",
            "/schemas",
        ))
    })?;
    let schema: Value = serde_json::from_str(schema_source)
        .map_err(|_| ContractError(Rejection::validation("invalid_embedded_schema", "/")))?;
    if schema.get("$id").and_then(Value::as_str) != Some(schema_entry.id.as_str()) {
        return Err(ContractError(Rejection::validation(
            "invalid_embedded_schema",
            "/$id",
        )));
    }
    let fragment = &reference[schema_entry.id.len()..];
    let target = if fragment.is_empty() {
        &schema
    } else {
        let pointer = fragment.strip_prefix('#').ok_or_else(|| {
            ContractError(Rejection::validation("invalid_embedded_schema", "/$ref"))
        })?;
        if !valid_json_pointer(pointer) {
            return Err(ContractError(Rejection::validation(
                "invalid_embedded_schema",
                "/$ref",
            )));
        }
        schema.pointer(pointer).ok_or_else(|| {
            ContractError(Rejection::validation("invalid_embedded_schema", "/$ref"))
        })?
    };
    resolve_imported_references(
        target.clone(),
        schema_entry.id.as_str(),
        registry,
        load_schema,
        active_references,
        resolved_reference_count,
    )
}

fn resolve_imported_references(
    value: Value,
    source_schema_id: &str,
    registry: &Registry,
    load_schema: fn(&str) -> Option<&'static str>,
    active_references: &mut BTreeSet<String>,
    resolved_reference_count: &mut usize,
) -> Result<Value, ContractError> {
    match value {
        Value::Array(items) => Ok(Value::Array(
            items
                .into_iter()
                .map(|item| {
                    resolve_imported_references(
                        item,
                        source_schema_id,
                        registry,
                        load_schema,
                        active_references,
                        resolved_reference_count,
                    )
                })
                .collect::<Result<_, _>>()?,
        )),
        Value::Object(mut object) => {
            let reference = object
                .get("$ref")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if let Some(reference) = reference {
                object.remove("$ref");
                let absolute_reference = if reference.starts_with('#') {
                    format!("{source_schema_id}{reference}")
                } else {
                    reference
                };
                let imported = import_registered_reference(
                    &absolute_reference,
                    registry,
                    load_schema,
                    active_references,
                    resolved_reference_count,
                )?;
                if object.is_empty() {
                    return Ok(imported);
                }
                let siblings = resolve_imported_references(
                    Value::Object(object),
                    source_schema_id,
                    registry,
                    load_schema,
                    active_references,
                    resolved_reference_count,
                )?;
                return Ok(serde_json::json!({ "allOf": [imported, siblings] }));
            }
            for child in object.values_mut() {
                *child = resolve_imported_references(
                    child.take(),
                    source_schema_id,
                    registry,
                    load_schema,
                    active_references,
                    resolved_reference_count,
                )?;
            }
            Ok(Value::Object(object))
        }
        scalar => Ok(scalar),
    }
}

fn pointer_token(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn valid_json_pointer(pointer: &str) -> bool {
    if !pointer.starts_with('/') {
        return false;
    }
    pointer.split('/').skip(1).all(|token| {
        let mut characters = token.chars();
        while let Some(character) = characters.next() {
            if character == '~' && !matches!(characters.next(), Some('0' | '1')) {
                return false;
            }
        }
        true
    })
}

fn canonicalize(value: &Value) -> Result<Vec<u8>, ContractError> {
    serde_json_canonicalizer::to_vec(value)
        .map_err(|_| ContractError(Rejection::validation("canonicalization_failed", "/")))
}

fn validate_special_rules(
    value: &Value,
    foundation_type: FoundationType,
) -> Result<(), ContractError> {
    let object = value
        .as_object()
        .ok_or_else(|| ContractError(Rejection::validation("expected_object", "/")))?;
    let allowed = allowed_fields(foundation_type, object)?;
    for key in object.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(ContractError(Rejection::contract(
                "contract.forbidden_field",
                "unknown_field",
                json_pointer(key),
            )));
        }
    }

    if let Some(version) = object.get("schema_version").and_then(Value::as_str) {
        let major = parse_schema_major(version).ok_or_else(|| {
            ContractError(Rejection::validation(
                "invalid_schema_version",
                "/schema_version",
            ))
        })?;
        if major != "1" {
            return Err(ContractError(Rejection::contract(
                "contract.unsupported_version",
                "unsupported_version",
                "/schema_version",
            )));
        }
    }

    match foundation_type {
        FoundationType::ContractMeta => validate_meta_scalars(object, true)?,
        FoundationType::HostScopedMeta => validate_meta_scalars(object, false)?,
        FoundationType::ActorRef => {
            validate_closed_enum(object, "type", &["user", "service", "device", "module"], "")?
        }
        FoundationType::SignatureBlock => validate_signature_block(object, "")?,
        FoundationType::DigestBoundRef => {
            validate_optional_scalar(object, "content_digest", "Sha256", "/content_digest")?
        }
        FoundationType::ResourceRef => {
            validate_optional_scalar(object, "digest", "Sha256", "/digest")?
        }
        FoundationType::ProjectionSpecimen => {
            validate_optional_scalar(object, "content_digest", "Sha256", "/content_digest")?;
            if let Some(signature) = object.get("signature").and_then(Value::as_object) {
                validate_signature_block(signature, "/signature")?;
            }
        }
        FoundationType::StrictUnionSpecimen => validate_strict_union(object)?,
    }
    Ok(())
}

fn validate_optional_scalar(
    object: &Map<String, Value>,
    field: &str,
    scalar: &str,
    path: &str,
) -> Result<(), ContractError> {
    if let Some(value) = object.get(field).and_then(Value::as_str) {
        validate_scalar(scalar, value).map_err(|mut error| {
            error.0.path = path.into();
            error
        })?;
    }
    Ok(())
}

fn validate_signature_block(
    object: &Map<String, Value>,
    path_prefix: &str,
) -> Result<(), ContractError> {
    let allowed = ["algorithm", "key_id", "value"];
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ContractError(Rejection::contract(
                "contract.forbidden_field",
                "unknown_field",
                format!("{path_prefix}{}", json_pointer(key)),
            )));
        }
    }
    validate_closed_enum(object, "algorithm", &["ed25519", "es256"], path_prefix)?;
    if let Some(value) = object.get("value").and_then(Value::as_str) {
        validate_scalar("Base64UrlNoPad", value).map_err(|mut error| {
            error.0.path = format!("{path_prefix}/value");
            error
        })?;
    }
    Ok(())
}

fn allowed_fields(
    foundation_type: FoundationType,
    object: &Map<String, Value>,
) -> Result<BTreeSet<&'static str>, ContractError> {
    let fields: &[&str] = match foundation_type {
        FoundationType::ContractMeta => &[
            "schema_version",
            "content_digest",
            "run_id",
            "created_at",
            "producer_version",
            "correlation_id",
        ],
        FoundationType::HostScopedMeta => &[
            "schema_version",
            "content_digest",
            "host_instance_id",
            "created_at",
            "producer_version",
            "correlation_id",
        ],
        FoundationType::ResourceRef => &["kind", "id", "digest", "version"],
        FoundationType::ActorRef => &["type", "id", "display_name"],
        FoundationType::DigestBoundRef => {
            &["kind", "id", "schema_version", "content_digest", "version"]
        }
        FoundationType::SignatureBlock => &["algorithm", "key_id", "value"],
        FoundationType::ProjectionSpecimen => {
            &["schema_version", "content_digest", "signature", "payload"]
        }
        FoundationType::StrictUnionSpecimen => return strict_union_allowed_fields(object),
    };
    Ok(fields.iter().copied().collect())
}

fn strict_union_allowed_fields(
    object: &Map<String, Value>,
) -> Result<BTreeSet<&'static str>, ContractError> {
    let kind = object.get("kind").and_then(Value::as_str).ok_or_else(|| {
        ContractError(Rejection::contract(
            "contract.invalid_variant",
            "missing_required_field",
            "/kind",
        ))
    })?;
    let fields: &[&str] = match kind {
        "alpha" => &["kind", "common", "alpha_value"],
        "beta" => &["kind", "common", "beta_count"],
        _ => {
            return Err(ContractError(Rejection::contract(
                "contract.invalid_variant",
                "unknown_discriminator",
                "/kind",
            )))
        }
    };
    let other = if kind == "alpha" {
        "beta_count"
    } else {
        "alpha_value"
    };
    if object.contains_key(other) {
        return Err(ContractError(Rejection::contract(
            "contract.forbidden_field",
            "mixed_variant_fields",
            json_pointer(other),
        )));
    }
    Ok(fields.iter().copied().collect())
}

fn validate_strict_union(object: &Map<String, Value>) -> Result<(), ContractError> {
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let required = if kind == "alpha" {
        ["common", "alpha_value"]
    } else {
        ["common", "beta_count"]
    };
    for field in required {
        if !object.contains_key(field) {
            return Err(ContractError(Rejection::contract(
                "contract.invalid_variant",
                "missing_required_field",
                json_pointer(field),
            )));
        }
    }
    Ok(())
}

fn validate_meta_scalars(
    object: &Map<String, Value>,
    run_scoped: bool,
) -> Result<(), ContractError> {
    for (field, scalar) in [
        ("schema_version", "SchemaVersion"),
        ("content_digest", "Sha256"),
        ("created_at", "ISO8601"),
    ] {
        validate_optional_scalar(object, field, scalar, &json_pointer(field))?;
    }
    if run_scoped {
        validate_optional_scalar(object, "run_id", "UUID", "/run_id")?;
    }
    Ok(())
}

fn validate_closed_enum(
    object: &Map<String, Value>,
    field: &str,
    allowed: &[&str],
    path_prefix: &str,
) -> Result<(), ContractError> {
    if let Some(value) = object.get(field).and_then(Value::as_str) {
        if !allowed.contains(&value) {
            return Err(ContractError(Rejection::contract(
                "contract.unsupported_enum",
                "unsupported_enum",
                format!("{path_prefix}{}", json_pointer(field)),
            )));
        }
    }
    Ok(())
}

fn validate_iso8601(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || *bytes.last().expect("length checked") != b'Z'
    {
        return false;
    }
    let fixed_digits = bytes[..19]
        .iter()
        .enumerate()
        .filter(|(index, _)| !matches!(index, 4 | 7 | 10 | 13 | 16))
        .all(|(_, byte)| byte.is_ascii_digit());
    if !fixed_digits {
        return false;
    }
    match bytes.get(19) {
        Some(b'Z') if bytes.len() == 20 => {}
        Some(b'.') if bytes.len() > 21 => {
            let fraction = &bytes[20..bytes.len() - 1];
            if !fraction.iter().all(u8::is_ascii_digit) || fraction.last() == Some(&b'0') {
                return false;
            }
        }
        _ => return false,
    }

    let Some(year) = decimal_field(bytes, 0, 4) else {
        return false;
    };
    let Some(month) = decimal_field(bytes, 5, 7) else {
        return false;
    };
    let Some(day) = decimal_field(bytes, 8, 10) else {
        return false;
    };
    let Some(hour) = decimal_field(bytes, 11, 13) else {
        return false;
    };
    let Some(minute) = decimal_field(bytes, 14, 16) else {
        return false;
    };
    let Some(second) = decimal_field(bytes, 17, 19) else {
        return false;
    };

    if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = [
        31,
        if leap_year { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    day >= 1 && day <= days_in_month[(month - 1) as usize]
}

fn compare_iso8601(left: &str, right: &str) -> Ordering {
    match left[..19].cmp(&right[..19]) {
        Ordering::Equal => {}
        ordering => return ordering,
    }
    let left_fraction = iso8601_fraction(left).as_bytes();
    let right_fraction = iso8601_fraction(right).as_bytes();
    for index in 0..left_fraction.len().max(right_fraction.len()) {
        let left_digit = left_fraction.get(index).copied().unwrap_or(b'0');
        let right_digit = right_fraction.get(index).copied().unwrap_or(b'0');
        match left_digit.cmp(&right_digit) {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

fn iso8601_fraction(value: &str) -> &str {
    if value.as_bytes().get(19) == Some(&b'.') {
        &value[20..value.len() - 1]
    } else {
        ""
    }
}

fn decimal_field(bytes: &[u8], start: usize, end: usize) -> Option<u32> {
    bytes.get(start..end)?.iter().try_fold(0, |value, byte| {
        byte.is_ascii_digit()
            .then_some(value * 10 + u32::from(byte - b'0'))
    })
}

fn validate_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_base64url_no_pad(value: &str) -> bool {
    !value.contains('=')
        && base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(value)
            .map(|bytes| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes) == value)
            .unwrap_or(false)
}

fn validate_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

fn parse_schema_major(value: &str) -> Option<&str> {
    let (domain, major) = value.rsplit_once("/v")?;
    if !domain.starts_with("qa.")
        || major.starts_with('0')
        || major.is_empty()
        || !major.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let name = &domain[3..];
    if name.is_empty()
        || !name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
    {
        return None;
    }
    Some(major)
}

fn validate_json_syntax(text: &str) -> Result<(), ContractError> {
    let probe = syntax_probe_text(text);
    let mut deserializer = serde_json::Deserializer::from_str(&probe);
    deserializer.disable_recursion_limit();
    Value::deserialize(&mut deserializer)
        .map_err(|_| ContractError(Rejection::validation("invalid_json", "/")))?;
    deserializer
        .end()
        .map_err(|_| ContractError(Rejection::validation("invalid_json", "/")))
}

fn syntax_probe_text(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let mut index = 0;
    let mut in_string = false;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                in_string = !in_string;
                index += 1;
            }
            b'\\' if in_string => {
                if bytes.get(index + 1) == Some(&b'u') && unicode_escape(&bytes, index).is_some() {
                    bytes[index + 2..index + 6].fill(b'0');
                    index += 6;
                } else {
                    index += 2;
                }
            }
            _ => index += 1,
        }
    }
    String::from_utf8(bytes).expect("syntax probe preserves UTF-8")
}

fn preflight_unicode_scalars(text: &str) -> Result<(), ContractError> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                in_string = !in_string;
                index += 1;
            }
            b'\\' if in_string => {
                if bytes.get(index + 1) != Some(&b'u') {
                    index += 2;
                    continue;
                }
                let Some(code_unit) = unicode_escape(bytes, index) else {
                    index += 2;
                    continue;
                };
                if (0xd800..=0xdbff).contains(&code_unit) {
                    let Some(low_surrogate) = unicode_escape(bytes, index + 6) else {
                        return Err(invalid_unicode_scalar());
                    };
                    if !(0xdc00..=0xdfff).contains(&low_surrogate) {
                        return Err(invalid_unicode_scalar());
                    }
                    index += 12;
                } else if (0xdc00..=0xdfff).contains(&code_unit) {
                    return Err(invalid_unicode_scalar());
                } else {
                    index += 6;
                }
            }
            _ => index += 1,
        }
    }
    Ok(())
}

fn unicode_escape(bytes: &[u8], index: usize) -> Option<u16> {
    if bytes.get(index..index + 2)? != b"\\u" {
        return None;
    }
    bytes
        .get(index + 2..index + 6)?
        .iter()
        .try_fold(0, |value, byte| {
            let digit = match byte {
                b'0'..=b'9' => u16::from(byte - b'0'),
                b'a'..=b'f' => u16::from(byte - b'a') + 10,
                b'A'..=b'F' => u16::from(byte - b'A') + 10,
                _ => return None,
            };
            Some(value * 16 + digit)
        })
}

fn invalid_unicode_scalar() -> ContractError {
    ContractError(Rejection::canonical(
        "canonicalization.invalid_unicode_scalar",
        "invalid_unicode_scalar",
    ))
}

fn preflight_depth(text: &str) -> Result<(), ContractError> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
        } else if matches!(byte, b'{' | b'[') {
            depth += 1;
            if depth > MAX_DEPTH {
                return Err(ContractError(Rejection::validation("depth_overflow", "/")));
            }
        } else if matches!(byte, b'}' | b']') {
            depth = depth.saturating_sub(1);
        }
    }
    Ok(())
}

fn ensure_depth(value: &Value, depth: usize) -> Result<(), ContractError> {
    if depth > MAX_DEPTH {
        return Err(ContractError(Rejection::validation("depth_overflow", "/")));
    }
    match value {
        Value::Array(values) => values
            .iter()
            .try_for_each(|value| ensure_depth(value, depth + 1)),
        Value::Object(values) => values
            .values()
            .try_for_each(|value| ensure_depth(value, depth + 1)),
        _ => Ok(()),
    }
}

fn preflight_numbers(text: &str) -> Result<(), ContractError> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte == b'-' || byte.is_ascii_digit() || matches!(byte, b'+' | b'N' | b'I') {
            let start = index;
            while index < bytes.len()
                && !matches!(
                    bytes[index],
                    b' ' | b'\n' | b'\r' | b'\t' | b',' | b']' | b'}' | b':'
                )
            {
                index += 1;
            }
            check_number_token(&text[start..index])?;
            continue;
        }
        index += 1;
    }
    Ok(())
}

fn check_number_token(token: &str) -> Result<(), ContractError> {
    if !valid_json_number(token) {
        return Err(ContractError(Rejection::canonical(
            "canonicalization.invalid_json_number",
            "invalid_json_number",
        )));
    }
    let number = token.parse::<f64>().map_err(|_| {
        ContractError(Rejection::canonical(
            "canonicalization.invalid_json_number",
            "invalid_json_number",
        ))
    })?;
    if !number.is_finite() {
        return Err(ContractError(Rejection::canonical(
            "canonicalization.invalid_json_number",
            "invalid_json_number",
        )));
    }
    let plain_integer_token = !token.contains(['.', 'e', 'E']);
    let renders_as_plain_integer = number.abs() < 1e21;
    if (plain_integer_token || renders_as_plain_integer) && exact_integer_exceeds_safe(token) {
        return Err(ContractError(Rejection::canonical(
            "canonicalization.unsafe_integer",
            "unsafe_integer",
        )));
    }
    Ok(())
}

fn exact_integer_exceeds_safe(token: &str) -> bool {
    let unsigned = token.strip_prefix('-').unwrap_or(token);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(index) => (
            &unsigned[..index],
            unsigned[index + 1..].parse::<i64>().ok(),
        ),
        None => (unsigned, Some(0)),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits = format!("{integer}{fraction}");
    let trimmed = digits.trim_start_matches('0').len();
    if trimmed == 0 {
        return false;
    }
    digits.drain(..digits.len() - trimmed);
    let Some(exponent) = exponent else {
        return false;
    };
    let scale = fraction.len() as i128 - i128::from(exponent);
    let integer_digits = if scale <= 0 {
        let zero_count = -scale;
        if digits.len() as i128 + zero_count > MAX_SAFE_INTEGER_TEXT.len() as i128 {
            return true;
        }
        digits.extend(std::iter::repeat_n('0', zero_count as usize));
        digits.as_str()
    } else {
        if scale >= digits.len() as i128 {
            return false;
        }
        let split = digits.len() - scale as usize;
        if !digits[split..].bytes().all(|byte| byte == b'0') {
            return false;
        }
        &digits[..split]
    };
    integer_digits.len() > MAX_SAFE_INTEGER_TEXT.len()
        || (integer_digits.len() == MAX_SAFE_INTEGER_TEXT.len()
            && integer_digits > MAX_SAFE_INTEGER_TEXT)
}

fn valid_json_number(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut index = 0;
    if bytes.get(index) == Some(&b'-') {
        index += 1;
    }
    match bytes.get(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => {
            index += 1;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    index == bytes.len()
}

fn classify_strict_json_error(error: serde_json::Error, text: &str) -> ContractError {
    if error.to_string().contains("duplicate member") {
        return ContractError(Rejection::canonical(
            "canonicalization.duplicate_member",
            "duplicate_member",
        ));
    }
    if let Err(error) = preflight_unicode_scalars(text) {
        return error;
    }
    classify_json_error(error)
}

fn classify_json_error(error: serde_json::Error) -> ContractError {
    let message = error.to_string();
    if message.contains("duplicate member") {
        ContractError(Rejection::canonical(
            "canonicalization.duplicate_member",
            "duplicate_member",
        ))
    } else if message.contains("surrogate") || message.contains("unicode") {
        ContractError(Rejection::canonical(
            "canonicalization.invalid_unicode_scalar",
            "invalid_unicode_scalar",
        ))
    } else if message.contains("number") {
        ContractError(Rejection::canonical(
            "canonicalization.invalid_json_number",
            "invalid_json_number",
        ))
    } else {
        ContractError(Rejection::validation("invalid_json", "/"))
    }
}

fn pointer_or_root(path: &str) -> String {
    if path.is_empty() {
        "/".into()
    } else {
        path.into()
    }
}

fn json_pointer(field: &str) -> String {
    format!("/{}", field.replace('~', "~0").replace('/', "~1"))
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("I-JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(|number| StrictValue(Value::Number(number)))
            .ok_or_else(|| E::custom("invalid number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom("duplicate member"));
            }
            let value = map.next_value::<StrictValue>()?;
            values.insert(key, value.0);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn executor_capability_digests_match_shared_vectors() {
        for (descriptor, expected_digest) in [
            (
                serde_json::json!({
                    "capabilities": ["browser.observe"],
                    "executor_id": "fake.browser",
                    "executor_version": "1.0.0",
                    "schema_version": "qa.local-executor/v1"
                }),
                "sha256:0f447361154fd5aa70f1b6c830547ae0401a3b185174177a123d9dbce1dc41b1",
            ),
            (
                serde_json::json!({
                    "capabilities": ["api.request"],
                    "executor_id": "fake.api",
                    "executor_version": "1.0.0",
                    "schema_version": "qa.local-executor/v1"
                }),
                "sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335",
            ),
        ] {
            assert_eq!(
                sha256_digest(&canonicalize(&descriptor).expect("canonical descriptor")),
                expected_digest
            );
        }
    }

    #[test]
    fn admission_v2_import_resolves_executor_v1_local_references() {
        let registry: Registry =
            serde_json::from_str(CONTRACT_REGISTRY).expect("parse embedded registry");
        let schema: Value =
            serde_json::from_str(LOCAL_RUN_ADMISSION_V2_SCHEMA).expect("parse admission v2 schema");
        let resolved = resolve_registered_references(schema, &registry, embedded_schema)
            .expect("resolve admission v2 references");
        let selection = resolved
            .pointer("/$defs/LocalQARunRequestV2/properties/executor_selection")
            .expect("resolved executor selection");

        assert_eq!(
            selection.pointer("/properties/executor_id/type"),
            Some(&Value::String("string".into()))
        );
        assert_eq!(
            selection.pointer("/properties/executor_version/pattern"),
            Some(&Value::String(
                "^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$".into()
            ))
        );
        assert_eq!(
            selection.pointer("/properties/capability_digest/pattern"),
            Some(&Value::String("^sha256:[0-9a-f]{64}$".into()))
        );
    }

    #[test]
    fn nested_registered_references_fail_closed() {
        let registry = reference_test_registry();
        for (reference, reason) in [
            (
                "urn:example:missing:v1#/$defs/Value",
                "external_schema_reference",
            ),
            (
                "urn:example:reference-tests:v1#/$defs/Malformed",
                "invalid_embedded_schema",
            ),
            (
                "urn:example:reference-tests:v1#/$defs/Unresolved",
                "invalid_embedded_schema",
            ),
            (
                "urn:example:reference-tests:v1#/$defs/CycleA",
                "invalid_embedded_schema",
            ),
        ] {
            let error = resolve_registered_references(
                serde_json::json!({ "$ref": reference }),
                &registry,
                reference_test_schema,
            )
            .expect_err("reference must fail closed");
            assert_eq!(error.0.reason, reason, "reference {reference}");
        }
    }

    #[test]
    fn registry_resolution_fails_closed() {
        assert_registry_error(
            |registry| {
                registry
                    .schemas
                    .get_mut("qa.local-lifecycle/v1")
                    .expect("lifecycle schema")
                    .id = "urn:example:mismatch".into();
            },
            "invalid_embedded_schema",
        );
        assert_registry_error(
            |registry| {
                registry
                    .schemas
                    .get_mut("qa.local-lifecycle/v1")
                    .expect("lifecycle schema")
                    .major = 2;
            },
            "unsupported_schema_major",
        );
        assert_registry_error(
            |registry| {
                registry.types.remove(EXECUTION_OUTCOME_TYPE_NAME);
            },
            "unknown_registered_type",
        );
        assert_registry_error(
            |registry| {
                registry
                    .types
                    .get_mut(EXECUTION_OUTCOME_TYPE_NAME)
                    .expect("ExecutionOutcome type")
                    .pointer = "#/$defs/Missing".into();
            },
            "unresolved_registered_pointer",
        );
        assert_registry_error(
            |registry| {
                registry.types.remove(CANCEL_DISPOSITION_TYPE_NAME);
            },
            "unknown_registered_type",
        );
        assert_registry_error(
            |registry| {
                registry
                    .types
                    .get_mut(CANCEL_DISPOSITION_TYPE_NAME)
                    .expect("CancelDisposition type")
                    .pointer = "#/$defs/Missing".into();
            },
            "unresolved_registered_pointer",
        );
        assert_registry_error(
            |registry| {
                registry
                    .types
                    .get_mut(CANCEL_DISPOSITION_TYPE_NAME)
                    .expect("CancelDisposition type")
                    .fixture_only = true;
            },
            "invalid_embedded_registry",
        );
        assert_registry_error(
            |registry| {
                registry.types.remove(EVENT_SEQUENCE_TYPE_NAME);
            },
            "unknown_registered_type",
        );
        assert_registry_error(
            |registry| {
                registry
                    .types
                    .get_mut(EVENT_SEQUENCE_TYPE_NAME)
                    .expect("EventSequence type")
                    .pointer = "#/$defs/Missing".into();
            },
            "unresolved_registered_pointer",
        );
        assert_registry_error(
            |registry| {
                registry
                    .types
                    .get_mut(EVENT_SEQUENCE_TYPE_NAME)
                    .expect("EventSequence type")
                    .fixture_only = true;
            },
            "invalid_embedded_registry",
        );
        assert_registry_error(
            |registry| {
                registry.types.remove(EVENT_CURSOR_TYPE_NAME);
            },
            "unknown_registered_type",
        );
        assert_registry_error(
            |registry| {
                registry
                    .types
                    .get_mut(EVENT_CURSOR_TYPE_NAME)
                    .expect("EventCursor type")
                    .pointer = "#/$defs/Missing".into();
            },
            "unresolved_registered_pointer",
        );
        assert_registry_error(
            |registry| {
                registry
                    .types
                    .get_mut(EVENT_CURSOR_TYPE_NAME)
                    .expect("EventCursor type")
                    .fixture_only = true;
            },
            "invalid_embedded_registry",
        );
        assert_registry_error(
            |registry| {
                registry
                    .schemas
                    .get_mut("qa.local-lifecycle/v1")
                    .expect("lifecycle schema")
                    .path = "../schema.json".into();
            },
            "invalid_embedded_schema_path",
        );
    }

    fn reference_test_registry() -> Registry {
        Registry {
            registry_version: "qa.contract-registry/v1".into(),
            profile: "local_qa_host_mvp".into(),
            schemas: BTreeMap::from([(
                "reference-tests/v1".into(),
                RegistrySchemaEntry {
                    path: "contracts/reference-tests/v1/schema.json".into(),
                    id: "urn:example:reference-tests:v1".into(),
                    major: 1,
                },
            )]),
            types: BTreeMap::new(),
        }
    }

    fn reference_test_schema(path: &str) -> Option<&'static str> {
        (path == "contracts/reference-tests/v1/schema.json").then_some(
            r##"{
                "$id": "urn:example:reference-tests:v1",
                "$defs": {
                    "Malformed": { "$ref": "#not-a-pointer" },
                    "Unresolved": { "$ref": "#/$defs/Missing" },
                    "CycleA": { "$ref": "#/$defs/CycleB" },
                    "CycleB": { "$ref": "#/$defs/CycleA" }
                }
            }"##,
        )
    }

    fn assert_registry_error(mutate: impl FnOnce(&mut Registry), reason: &str) {
        let mut registry: Registry =
            serde_json::from_str(CONTRACT_REGISTRY).expect("parse embedded registry");
        mutate(&mut registry);
        let error = validate_registry_value(&registry).expect_err("registry must fail closed");
        assert_eq!(error.0.reason, reason);
    }
}
