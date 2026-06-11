//! Lease-pool domain errors.

/// Errors surfaced by the lease coordination layer.
///
/// Lease *contention* is never an error: contended acquires, lost renews,
/// and not-held releases are expressed through the outcome enums. `PoolError`
/// covers only unexpected driver failures and invalid configuration.
///
/// Downstream mapping intent (documented only; the conversion into
/// [`crate::error::AppError`] is wired by the issue that surfaces leases via
/// the HTTP edge): `Config` -> `AppError::Config`, `Mongo` ->
/// `AppError::Mongo`.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// Unexpected MongoDB driver failure. The driver text may carry
    /// host/connection detail; log it server-side, never echo it to clients.
    #[error("mongodb error: {0}")]
    Mongo(#[from] mongodb::error::Error),
    /// Invalid lease configuration (a bad environment variable value). The
    /// message names the offending variable so startup failures are
    /// actionable.
    #[error("invalid lease config: {0}")]
    Config(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_display_is_stable_and_carries_the_reason() {
        let err = PoolError::Config("FKST_LEASE_TTL_SECS must be at least 1 second".to_string());
        assert_eq!(
            err.to_string(),
            "invalid lease config: FKST_LEASE_TTL_SECS must be at least 1 second"
        );
    }

    #[test]
    fn mongo_errors_convert_via_from_and_keep_the_source() {
        let io = std::io::Error::other("connection refused");
        let err: PoolError = mongodb::error::Error::from(io).into();
        assert!(matches!(err, PoolError::Mongo(_)));
        let source = std::error::Error::source(&err).expect("source preserved");
        assert!(source.to_string().contains("connection refused"));
    }
}
