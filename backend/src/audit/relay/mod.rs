//! The control plane's half of the durable relay: configuration, client,
//! telemetry, and the delivery policy the outer middleware applies.
//!
//! ```text
//! audited request
//!   -> AuditDelivery::register_start   required: MUST be durable before the handler
//!   -> inner service
//!   -> AuditDelivery::complete         required: MUST be durable before the response
//! ```
//!
//! [`AuditDelivery`] is the ONE place the three modes are interpreted, so the
//! middleware never branches on configuration and no other module can invent a
//! fourth behaviour. It is cheap to clone and holds no per-request state.

pub mod bootstrap;
pub mod client;
pub mod config;
pub mod lifecycle;
pub mod metrics;

use std::sync::Arc;

use k8s_openapi::chrono::{DateTime, Duration, Utc};

pub use client::{AuditRelayClient, RelayClientError};
pub use config::{AuditDeliveryConfig, AuditDeliveryMode};
pub use lifecycle::LifecycleRelayQueue;
pub use metrics::{
    RelayCallResult, RelayClientMetrics, RelayClientMetricsSnapshot, RelayPhase, RequiredRejection,
};

use crate::audit::event::{ApiRequestCompletedV1, RequestIdentity};
use crate::audit_relay::protocol::{
    format_instant, RequestCompletionV1, RequestStartV1, PROTOCOL_SCHEMA_VERSION,
};
use crate::error::AppError;

/// The delivery policy carried by the outer audit middleware.
#[derive(Clone, Debug, Default)]
pub struct AuditDelivery {
    mode: AuditDeliveryMode,
    client: Option<Arc<AuditRelayClient>>,
    /// Added to a request's start instant to produce its `completion_deadline_at`.
    grace_secs: u64,
    metrics: RelayClientMetrics,
}

impl AuditDelivery {
    /// The disabled policy: no relay, no behaviour change.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Build the policy from resolved configuration.
    ///
    /// A mode that uses the relay with no usable write half is a CONFIGURATION
    /// error, not a silent downgrade: `required` that quietly became
    /// `best_effort` would make the deployment's central claim false.
    pub fn from_config(
        config: &AuditDeliveryConfig,
        metrics: RelayClientMetrics,
    ) -> Result<Self, AppError> {
        if !config.mode.uses_relay() {
            return Ok(Self {
                mode: config.mode,
                client: None,
                grace_secs: config.incomplete_grace_secs,
                metrics,
            });
        }
        if !config.write_configured() {
            return Err(AppError::Config(format!(
                "FKST_AUDIT_DELIVERY_MODE={} needs FKST_AUDIT_RELAY_URL and \
                 FKST_AUDIT_RELAY_WRITE_TOKEN",
                config.mode.as_str()
            )));
        }
        let client = AuditRelayClient::from_config(config, metrics.clone())?;
        tracing::info!(
            mode = config.mode.as_str(),
            start_timeout_ms = config.start_timeout_ms,
            completion_timeout_ms = config.completion_timeout_ms,
            incomplete_grace_secs = config.incomplete_grace_secs,
            "audit delivery: relay configured"
        );
        Ok(Self {
            mode: config.mode,
            client: Some(Arc::new(client)),
            grace_secs: config.incomplete_grace_secs,
            metrics,
        })
    }

    /// Build a policy around an already-constructed client (tests, and any
    /// caller that shares one client between the middleware and the queue).
    pub fn with_client(
        mode: AuditDeliveryMode,
        client: Arc<AuditRelayClient>,
        grace_secs: u64,
        metrics: RelayClientMetrics,
    ) -> Self {
        Self {
            mode,
            client: Some(client),
            grace_secs,
            metrics,
        }
    }

    /// The configured mode.
    pub fn mode(&self) -> AuditDeliveryMode {
        self.mode
    }

    /// The shared client, when one is configured.
    pub fn client(&self) -> Option<Arc<AuditRelayClient>> {
        self.client.clone()
    }

    /// The bounded telemetry handle.
    pub fn metrics(&self) -> &RelayClientMetrics {
        &self.metrics
    }

    /// Whether the terminal event must be handed to the LOCAL sink.
    ///
    /// `required` says no: the relay owns delivery, and double-sending would put
    /// the same uuid on two independent paths for no benefit. Every other mode
    /// says yes, so a deployment without a relay — or with one that is down —
    /// still captures.
    pub fn use_local_sink(&self) -> bool {
        self.mode != AuditDeliveryMode::Required
    }

    /// Register a request start.
    ///
    /// `Ok(())` in `disabled` mode and in `best_effort` mode even when the relay
    /// refused: only `required` turns a failure into a caller-visible outcome.
    pub async fn register_start(
        &self,
        identity: &RequestIdentity,
        event_id: uuid::Uuid,
        started_at: DateTime<Utc>,
        service_version: &str,
        environment: &str,
    ) -> Result<(), RelayClientError> {
        let Some(client) = self.client.as_ref() else {
            return Ok(());
        };
        let start = self.build_start(identity, event_id, started_at, service_version, environment);
        match client.register_start(&start).await {
            Ok(_) => Ok(()),
            Err(error) => {
                // A 409 on the START path is NOT "already recorded": the relay
                // answers an exact replay with `200`, so a conflict means the
                // stored start describes a DIFFERENT invocation (two replicas at
                // different versions deriving the same id, or a reused request
                // id). Running the handler anyway would leave that invocation
                // with no durable start at all, which is the one thing this mode
                // exists to prevent.
                if error == RelayClientError::Conflict {
                    tracing::error!(
                        request_id = %identity.request_id,
                        operation_id = %identity.operation_id,
                        "audit delivery: the relay already holds a DIFFERENT start for this event \
                         id; this invocation is not the one that is durable"
                    );
                }
                self.on_failure(error, "start")
            }
        }
    }

    /// Commit the terminal event.
    pub async fn complete(&self, event: &ApiRequestCompletedV1) -> Result<(), RelayClientError> {
        let Some(client) = self.client.as_ref() else {
            return Ok(());
        };
        let completion = RequestCompletionV1::from_domain(event);
        match client.complete(&completion).await {
            Ok(_) => Ok(()),
            Err(error) => {
                // A 409 on the COMPLETION path means the relay already holds a
                // different terminal projection for this id — in practice the
                // `incomplete` one it synthesized after the deadline. History is
                // intact and must not be rewritten, but this process therefore
                // has PROOF that the status it is holding was not recorded, so
                // `required` mode must refuse to hand that status back.
                if error == RelayClientError::Conflict {
                    tracing::error!(
                        request_id = %event.request_id,
                        operation_id = %event.operation_id,
                        "audit delivery: the relay already holds a different terminal event for \
                         this event id; the handler's status was NOT durably recorded"
                    );
                }
                self.on_failure(error, "completion")
            }
        }
    }

    /// The one place a relay failure becomes (or does not become) a caller-visible
    /// outcome. `required` surfaces every failure — including a conflict, which is
    /// positive evidence that this process's record is not the durable one;
    /// every other mode logs and continues, because those modes promise nothing.
    fn on_failure(
        &self,
        error: RelayClientError,
        phase: &'static str,
    ) -> Result<(), RelayClientError> {
        if self.mode == AuditDeliveryMode::Required {
            return Err(error);
        }
        tracing::warn!(
            phase,
            reason = error.kind(),
            "audit delivery: best-effort call was not durably recorded"
        );
        Ok(())
    }

    /// The wire body for a request start.
    fn build_start(
        &self,
        identity: &RequestIdentity,
        event_id: uuid::Uuid,
        started_at: DateTime<Utc>,
        service_version: &str,
        environment: &str,
    ) -> RequestStartV1 {
        let deadline =
            started_at + Duration::seconds(i64::try_from(self.grace_secs).unwrap_or(i64::MAX));
        RequestStartV1 {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: event_id.to_string(),
            request_id: identity.request_id.clone(),
            started_at: format_instant(started_at),
            method: identity.method.clone(),
            route_template: identity.route_template.clone(),
            operation_id: identity.operation_id.clone(),
            service_version: service_version.to_string(),
            deployment_environment: environment.to_string(),
            completion_deadline_at: format_instant(deadline),
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
