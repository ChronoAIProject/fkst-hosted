//! LLM gateway seam — the ONLY LLM abstraction in fkst-hosted. Business logic
//! (the NyxID-backed gateway) plugs into the `LlmGateway` trait; callers depend
//! only on the trait, so the gateway is swappable and mockable in tests.
pub mod gateway;
use std::time::Duration;

use async_trait::async_trait;
pub use gateway::NyxLlmGateway;

#[async_trait]
pub trait LlmGateway: Send + Sync {
    /// One completion call; returns the raw model text.
    async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError>;
}

/// LLM gateway errors. No variant carries secrets or credentials.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("llm gateway not configured")]
    Unconfigured,
    #[error("llm gateway timed out")]
    Timeout,
    #[error("llm gateway authentication failed")]
    Auth,
    #[error("llm gateway http error: {0}")]
    Http(String),
    #[error("llm gateway response malformed: {0}")]
    Malformed(String),
}

/// Wiring for the NyxID-backed gateway. `gateway_url` is the operator-set base
/// (NyxID's `{base}/api/v1/llm/gateway/v1`); `model` is the routing key.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub gateway_url: String,
    pub model: String,
    pub timeout: Duration,
    pub max_output_bytes: usize,
}
