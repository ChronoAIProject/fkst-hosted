use std::collections::BTreeMap;
use std::sync::Arc;

use fkst_qa_contracts::{
    validate_execution_outcome, validate_executor_control_report,
    validate_executor_control_request, validate_executor_descriptor, validate_executor_request,
    validate_executor_result,
};
use serde::{Deserialize, Serialize};

use crate::RunError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionOutcome(String);

impl ExecutionOutcome {
    pub(crate) fn validated(value: &str) -> Result<Self, RunError> {
        let encoded = serde_json::to_vec(value)
            .map_err(|_| RunError::Contract("ExecutionOutcome serialization failed"))?;
        validate_execution_outcome(&encoded)
            .map_err(|_| RunError::Contract("invalid ExecutionOutcome"))?;
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct ExecutorDescriptor {
    pub schema_version: String,
    pub executor_id: String,
    pub executor_version: String,
    pub capabilities: Vec<String>,
    pub capability_digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct ExecutorSelection {
    pub schema_version: String,
    pub executor_id: String,
    pub executor_version: String,
    pub capability_digest: String,
    pub required_capability: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct ExecutorRequest {
    pub schema_version: String,
    pub run_id: String,
    pub selection: ExecutorSelection,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct ExecutorResult {
    pub schema_version: String,
    pub run_id: String,
    pub executor_id: String,
    pub executor_version: String,
    pub capability_digest: String,
    pub execution_outcome: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct ExecutorControlRequest {
    pub schema_version: String,
    pub control_id: String,
    pub run_id: String,
    pub executor_run_id: String,
    pub selection: ExecutorSelection,
    pub deadline_utc: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct CleanupReceipt {
    pub receipt_id: String,
    pub no_resources_remain: bool,
    pub resource_handles: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct SanitizedResidual {
    pub code: String,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct ExecutorControlReport {
    pub schema_version: String,
    pub control_id: String,
    pub run_id: String,
    pub executor_run_id: String,
    pub executor_id: String,
    pub executor_version: String,
    pub capability_digest: String,
    pub status: String,
    pub control_acknowledged: bool,
    pub worker_stop_observed: bool,
    pub effect_disposition: String,
    pub execution_outcome: String,
    pub evidence_outcome: String,
    pub upload_outcome: String,
    pub cleanup_outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_receipt: Option<CleanupReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub residual: Option<SanitizedResidual>,
}

pub(crate) trait VersionedExecutor: Send + Sync {
    fn descriptor(&self) -> &ExecutorDescriptor;
    fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError>;
    fn control(
        &self,
        request: &ExecutorControlRequest,
    ) -> Result<ExecutorControlReport, RunError> {
        let descriptor = self.descriptor();
        Ok(ExecutorControlReport {
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
        })
    }
}

pub(crate) struct InertExecutor {
    descriptor: ExecutorDescriptor,
}

impl InertExecutor {
    pub(crate) fn new() -> Self {
        Self {
            descriptor: inert_executor_descriptor(),
        }
    }
}

impl Default for InertExecutor {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn inert_executor_descriptor() -> ExecutorDescriptor {
    ExecutorDescriptor {
        schema_version: "qa.local-executor/v1".to_owned(),
        executor_id: "local.inert".to_owned(),
        executor_version: "1.0.0".to_owned(),
        capabilities: vec!["runtime.inert".to_owned()],
        capability_digest:
            "sha256:2778ff138818dfa4d505611593b746df69ffb092dee08e2f708b5df3ca8bf4e8".to_owned(),
    }
}

pub(crate) fn inert_executor_selection() -> ExecutorSelection {
    ExecutorSelection {
        schema_version: "qa.local-executor/v1".to_owned(),
        executor_id: "local.inert".to_owned(),
        executor_version: "1.0.0".to_owned(),
        capability_digest:
            "sha256:2778ff138818dfa4d505611593b746df69ffb092dee08e2f708b5df3ca8bf4e8".to_owned(),
        required_capability: "runtime.inert".to_owned(),
    }
}

impl VersionedExecutor for InertExecutor {
    fn descriptor(&self) -> &ExecutorDescriptor {
        &self.descriptor
    }

    fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError> {
        Ok(ExecutorResult {
            schema_version: "qa.local-executor/v1".into(),
            run_id: request.run_id.clone(),
            executor_id: self.descriptor.executor_id.clone(),
            executor_version: self.descriptor.executor_version.clone(),
            capability_digest: self.descriptor.capability_digest.clone(),
            execution_outcome: "blocked".into(),
        })
    }
}

#[derive(Clone)]
pub(crate) struct ExecutorRegistry {
    entries: BTreeMap<(String, String, String, String), Arc<dyn VersionedExecutor>>,
}

impl ExecutorRegistry {
    pub(crate) fn new(executors: Vec<Box<dyn VersionedExecutor>>) -> Result<Self, RunError> {
        let mut entries = BTreeMap::new();
        for executor in executors {
            let executor: Arc<dyn VersionedExecutor> = executor.into();
            let descriptor = executor.descriptor();
            let bytes = serde_json::to_vec(descriptor)
                .map_err(|_| RunError::Contract("executor descriptor serialization failed"))?;
            validate_executor_descriptor(&bytes)
                .map_err(|_| RunError::Contract("invalid executor descriptor"))?;
            for capability in &descriptor.capabilities {
                let key = (
                    descriptor.executor_id.clone(),
                    descriptor.executor_version.clone(),
                    descriptor.capability_digest.clone(),
                    capability.clone(),
                );
                if entries.insert(key, Arc::clone(&executor)).is_some() {
                    return Err(RunError::Contract("duplicate executor selection"));
                }
            }
        }
        Ok(Self { entries })
    }

    pub(crate) fn execute(&self, request: &ExecutorRequest) -> Result<ExecutionOutcome, RunError> {
        let request_bytes = serde_json::to_vec(request)
            .map_err(|_| RunError::Contract("executor request serialization failed"))?;
        validate_executor_request(&request_bytes)
            .map_err(|_| RunError::Contract("invalid executor request"))?;
        let selection = &request.selection;
        let key = (
            selection.executor_id.clone(),
            selection.executor_version.clone(),
            selection.capability_digest.clone(),
            selection.required_capability.clone(),
        );
        let executor = self
            .entries
            .get(&key)
            .ok_or(RunError::Contract("executor selection not allowlisted"))?;
        let descriptor = executor.descriptor();
        if !descriptor
            .capabilities
            .iter()
            .any(|capability| capability == &selection.required_capability)
            || descriptor.capability_digest != selection.capability_digest
        {
            return Err(RunError::Contract("executor selection relation failed"));
        }
        let result = executor.execute(request)?;
        let result_bytes = serde_json::to_vec(&result)
            .map_err(|_| RunError::Contract("executor result serialization failed"))?;
        validate_executor_result(&result_bytes)
            .map_err(|_| RunError::Contract("invalid executor result"))?;
        if result.run_id != request.run_id
            || result.executor_id != descriptor.executor_id
            || result.executor_version != descriptor.executor_version
            || result.capability_digest != descriptor.capability_digest
        {
            return Err(RunError::Contract("executor result relation failed"));
        }
        ExecutionOutcome::validated(&result.execution_outcome)
    }

    pub(crate) fn control(
        &self,
        request: &ExecutorControlRequest,
    ) -> Result<ExecutorControlReport, RunError> {
        let request_bytes = serde_json::to_vec(request)
            .map_err(|_| RunError::Contract("executor control request serialization failed"))?;
        validate_executor_control_request(&request_bytes)
            .map_err(|_| RunError::Contract("invalid executor control request"))?;
        let selection = &request.selection;
        let key = (
            selection.executor_id.clone(),
            selection.executor_version.clone(),
            selection.capability_digest.clone(),
            selection.required_capability.clone(),
        );
        let executor = self
            .entries
            .get(&key)
            .ok_or(RunError::Contract("executor selection not allowlisted"))?;
        let report = executor.control(request)?;
        let report_bytes = serde_json::to_vec(&report)
            .map_err(|_| RunError::Contract("executor control report serialization failed"))?;
        validate_executor_control_report(&report_bytes)
            .map_err(|_| RunError::Contract("invalid executor control report"))?;
        let descriptor = executor.descriptor();
        if report.control_id != request.control_id
            || report.run_id != request.run_id
            || report.executor_run_id != request.executor_run_id
            || report.executor_id != descriptor.executor_id
            || report.executor_version != descriptor.executor_version
            || report.capability_digest != descriptor.capability_digest
        {
            return Err(RunError::Contract("executor control report relation failed"));
        }
        Ok(report)
    }

    pub(crate) fn resolve(
        &self,
        selection: &ExecutorSelection,
    ) -> Result<ExecutorSelection, RunError> {
        let key = (
            selection.executor_id.clone(),
            selection.executor_version.clone(),
            selection.capability_digest.clone(),
            selection.required_capability.clone(),
        );
        self.entries
            .contains_key(&key)
            .then(|| selection.clone())
            .ok_or(RunError::Contract("executor selection not allowlisted"))
    }
}

pub(crate) struct FakeApiAdmissionExecutor {
    descriptor: ExecutorDescriptor,
}

impl FakeApiAdmissionExecutor {
    pub(crate) fn new() -> Self {
        Self {
            descriptor: ExecutorDescriptor {
                schema_version: "qa.local-executor/v1".into(),
                executor_id: "fake.api".into(),
                executor_version: "1.0.0".into(),
                capabilities: vec!["api.request".into()],
                capability_digest:
                    "sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335".into(),
            },
        }
    }
}

impl VersionedExecutor for FakeApiAdmissionExecutor {
    fn descriptor(&self) -> &ExecutorDescriptor {
        &self.descriptor
    }

    fn execute(&self, _request: &ExecutorRequest) -> Result<ExecutorResult, RunError> {
        panic!("fake API admission executor must not execute")
    }
}

#[cfg(test)]
pub(crate) struct DeterministicExecutor {
    descriptor: ExecutorDescriptor,
    outcome: &'static str,
}

#[cfg(test)]
impl DeterministicExecutor {
    pub(crate) fn browser() -> Self {
        Self {
            descriptor: ExecutorDescriptor {
                schema_version: "qa.local-executor/v1".into(),
                executor_id: "fake.browser".into(),
                executor_version: "1.0.0".into(),
                capabilities: vec!["browser.observe".into()],
                capability_digest:
                    "sha256:0f447361154fd5aa70f1b6c830547ae0401a3b185174177a123d9dbce1dc41b1".into(),
            },
            outcome: "blocked",
        }
    }

    pub(crate) fn api() -> Self {
        Self {
            descriptor: ExecutorDescriptor {
                schema_version: "qa.local-executor/v1".into(),
                executor_id: "fake.api".into(),
                executor_version: "1.0.0".into(),
                capabilities: vec!["api.request".into()],
                capability_digest:
                    "sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335".into(),
            },
            outcome: "passed",
        }
    }
}

#[cfg(test)]
impl VersionedExecutor for DeterministicExecutor {
    fn descriptor(&self) -> &ExecutorDescriptor {
        &self.descriptor
    }

    fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError> {
        Ok(ExecutorResult {
            schema_version: "qa.local-executor/v1".into(),
            run_id: request.run_id.clone(),
            executor_id: self.descriptor.executor_id.clone(),
            executor_version: self.descriptor.executor_version.clone(),
            capability_digest: self.descriptor.capability_digest.clone(),
            execution_outcome: self.outcome.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        inert_executor_descriptor, inert_executor_selection, DeterministicExecutor,
        ExecutorDescriptor, ExecutorRegistry, ExecutorRequest, ExecutorResult, ExecutorSelection,
        InertExecutor, VersionedExecutor,
    };
    use crate::RunError;

    struct PanicOnCallExecutor {
        descriptor: ExecutorDescriptor,
    }

    impl VersionedExecutor for PanicOnCallExecutor {
        fn descriptor(&self) -> &ExecutorDescriptor {
            &self.descriptor
        }

        fn execute(&self, _request: &ExecutorRequest) -> Result<ExecutorResult, RunError> {
            panic!("selection failure must occur before executor invocation")
        }
    }

    #[test]
    fn unknown_selection_tuple_members_fail_before_invocation() {
        let cases = [
            ("executor_id", "fake.unknown"),
            ("executor_version", "2.0.0"),
            (
                "capability_digest",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            ),
            ("required_capability", "api.unknown"),
        ];

        for (field, value) in cases {
            let descriptor = DeterministicExecutor::api().descriptor().clone();
            let registry =
                ExecutorRegistry::new(vec![Box::new(PanicOnCallExecutor { descriptor })])
                    .expect("registry must be valid");
            let mut selection = ExecutorSelection {
                schema_version: "qa.local-executor/v1".to_owned(),
                executor_id: "fake.api".to_owned(),
                executor_version: "1.0.0".to_owned(),
                capability_digest:
                    "sha256:37c748fcbb32a9c03fd27f345427fc0062a8c875147732e0653794cd1b164335"
                        .to_owned(),
                required_capability: "api.request".to_owned(),
            };
            match field {
                "executor_id" => selection.executor_id = value.to_owned(),
                "executor_version" => selection.executor_version = value.to_owned(),
                "capability_digest" => selection.capability_digest = value.to_owned(),
                "required_capability" => selection.required_capability = value.to_owned(),
                _ => unreachable!(),
            }
            let request = ExecutorRequest {
                schema_version: "qa.local-executor/v1".to_owned(),
                run_id: "00000000-0000-0000-0000-000000000001".to_owned(),
                selection,
            };
            assert!(registry.execute(&request).is_err(), "field {field}");
        }
    }

    #[test]
    fn production_inert_tuple_is_exact() {
        assert_eq!(
            inert_executor_descriptor(),
            ExecutorDescriptor {
                schema_version: "qa.local-executor/v1".to_owned(),
                executor_id: "local.inert".to_owned(),
                executor_version: "1.0.0".to_owned(),
                capabilities: vec!["runtime.inert".to_owned()],
                capability_digest:
                    "sha256:2778ff138818dfa4d505611593b746df69ffb092dee08e2f708b5df3ca8bf4e8"
                        .to_owned(),
            }
        );
        assert_eq!(
            inert_executor_selection(),
            ExecutorSelection {
                schema_version: "qa.local-executor/v1".to_owned(),
                executor_id: "local.inert".to_owned(),
                executor_version: "1.0.0".to_owned(),
                capability_digest:
                    "sha256:2778ff138818dfa4d505611593b746df69ffb092dee08e2f708b5df3ca8bf4e8"
                        .to_owned(),
                required_capability: "runtime.inert".to_owned(),
            }
        );
        assert_eq!(
            DeterministicExecutor::browser().descriptor().executor_id,
            "fake.browser"
        );
    }

    #[test]
    fn production_inert_executor_returns_exact_blocked_result() {
        let executor = InertExecutor::new();
        let request = ExecutorRequest {
            schema_version: "qa.local-executor/v1".to_owned(),
            run_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            selection: inert_executor_selection(),
        };

        assert_eq!(executor.descriptor(), &inert_executor_descriptor());
        assert_eq!(
            executor
                .execute(&request)
                .expect("inert execution succeeds"),
            ExecutorResult {
                schema_version: "qa.local-executor/v1".to_owned(),
                run_id: request.run_id,
                executor_id: "local.inert".to_owned(),
                executor_version: "1.0.0".to_owned(),
                capability_digest:
                    "sha256:2778ff138818dfa4d505611593b746df69ffb092dee08e2f708b5df3ca8bf4e8"
                        .to_owned(),
                execution_outcome: "blocked".to_owned(),
            }
        );
    }
}
