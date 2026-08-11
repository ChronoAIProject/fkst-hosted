use fkst_qa_contracts::validate_execution_outcome;

use crate::RunError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionOutcome(String);

impl ExecutionOutcome {
    pub(crate) fn passed() -> Result<Self, RunError> {
        Self::validated("passed")
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

pub(crate) struct PassingExecutor;

impl Executor for PassingExecutor {
    fn execute(&mut self, _run_id: &str) -> Result<ExecutionOutcome, RunError> {
        ExecutionOutcome::passed()
    }
}
