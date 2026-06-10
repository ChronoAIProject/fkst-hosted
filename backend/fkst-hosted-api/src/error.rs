//! Unified application error type.

/// Unified error type used across the fkst-hosted API.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Configuration could not be loaded or parsed. Renders as 500.
    #[error("configuration error: {0}")]
    Config(String),
    /// The request payload or parameters are invalid. Renders as 400.
    #[error("invalid request: {0}")]
    Validation(String),
    /// The requested resource does not exist. Renders as 404.
    #[error("not found: {0}")]
    NotFound(String),
    /// The request conflicts with the current state. Renders as 409.
    #[error("conflict: {0}")]
    Conflict(String),
    /// Any unexpected internal failure. Renders as 500.
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}
