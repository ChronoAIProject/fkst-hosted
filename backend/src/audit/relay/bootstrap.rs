//! Process startup: build the relay client once, and attach it to the consumers
//! that are not the request middleware.
//!
//! ```text
//! bootstrap::client(...)  ──> LifecycleRelayQueue     (background effects)
//!                         └─> RelayActivitySource     (the operations read)
//! ```
//!
//! The outer request middleware is deliberately NOT wired here. It resolves its
//! own [`super::AuditDelivery`] inside [`crate::router::build_router`] from the
//! same configuration, because the delivery policy is the middleware's only
//! consumer and putting it on [`crate::state::AppState`] would hand every route
//! a handle to something none of them may use. The cost is one extra HTTP
//! connection pool for one internal service, which is the cheaper half of that
//! trade.
//!
//! It lives in the library rather than in `main` so both halves are unit-testable
//! and so the binary's startup path does not keep growing.

use std::sync::Arc;

use crate::audit::AuditHandle;
use crate::error::AppError;

use super::{AuditDeliveryConfig, AuditRelayClient, LifecycleRelayQueue, RelayClientMetrics};

/// Build the relay client this process shares, or `None` when neither half of
/// the relay is configured.
///
/// A configured-but-unbuildable client is an error, never a silent `None`: a
/// `required` deployment that quietly ran without a relay would make its central
/// durability claim false.
pub fn client(
    config: &AuditDeliveryConfig,
    metrics: RelayClientMetrics,
) -> Result<Option<Arc<AuditRelayClient>>, AppError> {
    if !config.write_configured() && !config.read_configured() {
        tracing::debug!("audit delivery: no relay half is configured; no client is built");
        return Ok(None);
    }
    Ok(Some(Arc::new(AuditRelayClient::from_config(
        config, metrics,
    )?)))
}

/// Route sandbox lifecycle events through the relay when the mode uses it.
///
/// Returns `audit` unchanged in `disabled` mode, or when no client exists, so the
/// caller never has to branch on configuration twice.
pub fn with_lifecycle_relay(
    audit: AuditHandle,
    client: Option<&Arc<AuditRelayClient>>,
    config: &AuditDeliveryConfig,
) -> AuditHandle {
    match client {
        Some(client) if config.mode.uses_relay() => {
            tracing::info!(
                mode = config.mode.as_str(),
                "sandbox lifecycle events route through the durable audit relay"
            );
            audit.with_lifecycle_relay(LifecycleRelayQueue::spawn(client.clone()))
        }
        _ => audit,
    }
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
