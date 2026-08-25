use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use fkst_qa_contracts::{
    validate_execution_outcome, validate_executor_descriptor, validate_executor_request,
    validate_executor_result,
};
use serde::{Deserialize, Serialize};

use crate::RunError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionOutcome(String);

impl ExecutionOutcome {
    #[cfg(test)]
    pub(crate) fn passed() -> Result<Self, RunError> {
        Self::validated("passed")
    }

    pub(crate) fn blocked() -> Result<Self, RunError> {
        Self::validated("blocked")
    }

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

pub(crate) trait Executor: Send + 'static {
    fn execute(&mut self, run_id: &str) -> Result<ExecutionOutcome, RunError>;
}

pub(crate) struct InertExecutor;

impl Executor for InertExecutor {
    fn execute(&mut self, _run_id: &str) -> Result<ExecutionOutcome, RunError> {
        ExecutionOutcome::blocked()
    }
}

#[cfg(test)]
pub(crate) struct PassingExecutor;

#[cfg(test)]
impl Executor for PassingExecutor {
    fn execute(&mut self, _run_id: &str) -> Result<ExecutionOutcome, RunError> {
        ExecutionOutcome::passed()
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

pub(crate) trait VersionedExecutor: Send + Sync {
    fn descriptor(&self) -> &ExecutorDescriptor;
    fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError>;
}

pub(crate) struct LegacyExecutorAdapter {
    descriptor: ExecutorDescriptor,
    legacy: Mutex<Box<dyn Executor>>,
}

impl LegacyExecutorAdapter {
    pub(crate) fn new(legacy: Box<dyn Executor>, descriptor: ExecutorDescriptor) -> Self {
        Self {
            descriptor,
            legacy: Mutex::new(legacy),
        }
    }
}

pub(crate) fn legacy_executor_descriptor() -> ExecutorDescriptor {
    ExecutorDescriptor {
        schema_version: "qa.local-executor/v1".to_owned(),
        executor_id: "legacy.executor".to_owned(),
        executor_version: "1.0.0".to_owned(),
        capabilities: vec!["legacy.execute".to_owned()],
        capability_digest:
            "sha256:e4760210c40c509504bf4cbf529835fc895e1b7d8e6cc3313fa673658e56a787"
                .to_owned(),
    }
}

pub(crate) fn legacy_executor_selection() -> ExecutorSelection {
    ExecutorSelection {
        schema_version: "qa.local-executor/v1".to_owned(),
        executor_id: "legacy.executor".to_owned(),
        executor_version: "1.0.0".to_owned(),
        capability_digest:
            "sha256:e4760210c40c509504bf4cbf529835fc895e1b7d8e6cc3313fa673658e56a787"
                .to_owned(),
        required_capability: "legacy.execute".to_owned(),
    }
}

pub(crate) fn inert_executor_descriptor() -> ExecutorDescriptor {
    ExecutorDescriptor {
        schema_version: "qa.local-executor/v1".to_owned(),
        executor_id: "local.inert".to_owned(),
        executor_version: "1.0.0".to_owned(),
        capabilities: vec!["runtime.inert".to_owned()],
        capability_digest:
            "sha256:2778ff138818dfa4d505611593b746df69ffb092dee08e2f708b5df3ca8bf4e8"
                .to_owned(),
    }
}

pub(crate) fn inert_executor_selection() -> ExecutorSelection {
    ExecutorSelection {
        schema_version: "qa.local-executor/v1".to_owned(),
        executor_id: "local.inert".to_owned(),
        executor_version: "1.0.0".to_owned(),
        capability_digest:
            "sha256:2778ff138818dfa4d505611593b746df69ffb092dee08e2f708b5df3ca8bf4e8"
                .to_owned(),
        required_capability: "runtime.inert".to_owned(),
    }
}

impl VersionedExecutor for LegacyExecutorAdapter {
    fn descriptor(&self) -> &ExecutorDescriptor {
        &self.descriptor
    }

    fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError> {
        let outcome = self
            .legacy
            .lock()
            .map_err(|_| RunError::Contract("legacy executor bridge poisoned"))?
            .execute(&request.run_id)?;
        Ok(ExecutorResult {
            schema_version: "qa.local-executor/v1".into(),
            run_id: request.run_id.clone(),
            executor_id: self.descriptor.executor_id.clone(),
            executor_version: self.descriptor.executor_version.clone(),
            capability_digest: self.descriptor.capability_digest.clone(),
            execution_outcome: outcome.as_str().into(),
        })
    }
}

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
        inert_executor_descriptor, inert_executor_selection, legacy_executor_descriptor,
        legacy_executor_selection, DeterministicExecutor, ExecutorDescriptor, ExecutorRegistry,
        ExecutorRequest, ExecutorResult, ExecutorSelection, VersionedExecutor,
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
            let registry = ExecutorRegistry::new(vec![Box::new(PanicOnCallExecutor {
                descriptor,
            })])
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
    fn legacy_compatibility_tuple_is_exact() {
        assert_eq!(
            legacy_executor_descriptor(),
            ExecutorDescriptor {
                schema_version: "qa.local-executor/v1".to_owned(),
                executor_id: "legacy.executor".to_owned(),
                executor_version: "1.0.0".to_owned(),
                capabilities: vec!["legacy.execute".to_owned()],
                capability_digest:
                    "sha256:e4760210c40c509504bf4cbf529835fc895e1b7d8e6cc3313fa673658e56a787"
                        .to_owned(),
            }
        );
        assert_eq!(
            legacy_executor_selection(),
            ExecutorSelection {
                schema_version: "qa.local-executor/v1".to_owned(),
                executor_id: "legacy.executor".to_owned(),
                executor_version: "1.0.0".to_owned(),
                capability_digest:
                    "sha256:e4760210c40c509504bf4cbf529835fc895e1b7d8e6cc3313fa673658e56a787"
                        .to_owned(),
                required_capability: "legacy.execute".to_owned(),
            }
        );
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
}
