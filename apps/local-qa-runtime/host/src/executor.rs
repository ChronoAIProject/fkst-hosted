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
#[expect(dead_code)]
pub(crate) struct ExecutorDescriptor {
    pub schema_version: String,
    pub executor_id: String,
    pub executor_version: String,
    pub capabilities: Vec<String>,
    pub capability_digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[expect(dead_code)]
pub(crate) struct ExecutorSelection {
    pub schema_version: String,
    pub executor_id: String,
    pub executor_version: String,
    pub capability_digest: String,
    pub required_capability: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[expect(dead_code)]
pub(crate) struct ExecutorRequest {
    pub schema_version: String,
    pub run_id: String,
    pub selection: ExecutorSelection,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[expect(dead_code)]
pub(crate) struct ExecutorResult {
    pub schema_version: String,
    pub run_id: String,
    pub executor_id: String,
    pub executor_version: String,
    pub capability_digest: String,
    pub execution_outcome: String,
}

#[expect(dead_code)]
pub(crate) trait VersionedExecutor: Send + Sync {
    fn descriptor(&self) -> &ExecutorDescriptor;
    fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError>;
}

#[expect(dead_code)]
pub(crate) struct LegacyExecutorAdapter {
    descriptor: ExecutorDescriptor,
    legacy: Mutex<Box<dyn Executor>>,
}

impl LegacyExecutorAdapter {
    #[expect(dead_code)]
    pub(crate) fn new(legacy: Box<dyn Executor>, descriptor: ExecutorDescriptor) -> Self {
        Self {
            descriptor,
            legacy: Mutex::new(legacy),
        }
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

#[expect(dead_code)]
pub(crate) struct ExecutorRegistry {
    entries: BTreeMap<(String, String, String, String), Arc<dyn VersionedExecutor>>,
}

impl ExecutorRegistry {
    #[expect(dead_code)]
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

    #[expect(dead_code)]
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
#[expect(dead_code)]
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
