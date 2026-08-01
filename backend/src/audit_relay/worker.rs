//! The relay's background sweep: close, capture, verify, purge, publish.
//!
//! ```text
//! every FKST_AUDIT_RELAY_WORKER_INTERVAL_SECS:
//!   1. close_overdue_starts   [closer]   deadline + grace -> incomplete
//!   2. capture_due            [capture]  FIFO batch -> PostHog capture
//!   3. verify_accepted        [verify]   batched fixed query -> verified
//!   4. purge_verified         [closer]   verified retention only
//!   5. refresh_gauges         [here]     bounded {state} gauges + capacity flag
//! ```
//!
//! The order is not arbitrary. Closing first means an expired start becomes a
//! deliverable record in the SAME sweep rather than the next one, so a crashed
//! request's incomplete row is not held back a full interval. Capturing before
//! verifying means a record can never be verified in the sweep that accepted it,
//! which is exactly right: ingestion lag is real, and a verification that ran
//! immediately would report `absent` for healthy events.
//!
//! Every pass is a separate `async fn` with no shared mutable state, so a failure
//! in one cannot skip the others: a PostHog outage must not stop incomplete
//! synthesis, and a verification outage must not stop capture.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use k8s_openapi::chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use crate::audit::posthog::PostHogClient;
use crate::audit::projection::EventLimits;
use crate::audit::AuditConfig;
use crate::error::AppError;
use crate::operations::posthog::PosthogQueryClient;

use super::config::RelayConfig;
use super::db::{ingest, Database};
use super::http::RelayState;
use super::metrics::{RelayMetrics, StorageGauges};
use super::record::RecordState;

/// The background worker. Cloneable so a test can drive one sweep directly.
#[derive(Clone)]
pub struct RelayWorker {
    pub(super) db: Database,
    pub(super) metrics: RelayMetrics,
    pub(super) config: Arc<RelayConfig>,
    pub(super) at_capacity: Arc<AtomicBool>,
    pub(super) capture: Option<PostHogClient>,
    pub(super) verifier: Option<PosthogQueryClient>,
    pub(super) limits: EventLimits,
}

impl std::fmt::Debug for RelayWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayWorker")
            .field("capture_configured", &self.capture.is_some())
            .field("verification_configured", &self.verifier.is_some())
            .finish()
    }
}

impl RelayWorker {
    /// Build the worker for a configured relay.
    ///
    /// A missing PostHog host or query key is NOT an error: the relay is then a
    /// pure durable outbox, records accumulate, the backlog gauges make that
    /// visible, and readiness stays true — which is the whole point of having an
    /// outbox in front of an external system.
    pub fn new(state: &RelayState) -> Result<Self, AppError> {
        let config = state.config.clone();
        let capture = if config.capture_configured() {
            Some(PostHogClient::from_config(&capture_config(&config))?)
        } else {
            tracing::warn!(
                "audit relay: PostHog capture is not configured; records will accumulate durably"
            );
            None
        };
        let verifier = match (config.verification_configured(), config.query_url()) {
            (true, Some(url)) => Some(PosthogQueryClient::new(
                url,
                config.posthog_query_api_key.clone(),
                std::time::Duration::from_secs(30),
            )?),
            _ => {
                tracing::warn!(
                    "audit relay: PostHog query verification is not configured; records will \
                     remain `posthog_accepted` and never be renamed verified"
                );
                None
            }
        };
        // Published before the first sweep so the headroom alert has a
        // denominator from process start, including on a relay that is failing
        // to make progress.
        state.metrics.set_max_records(config.max_records);
        Ok(Self {
            db: state.db.clone(),
            metrics: state.metrics.clone(),
            at_capacity: state.at_capacity.clone(),
            capture,
            verifier,
            limits: EventLimits::new(capture_config(&config).max_event_bytes),
            config,
        })
    }

    /// Run until cancelled. Every sweep is independent; a failing pass logs and
    /// the loop continues, because the alternative is a relay that stops closing
    /// records because a remote system is down.
    pub async fn run(self, cancel: CancellationToken) {
        let interval = std::time::Duration::from_secs(self.config.worker_interval_secs.max(1));
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tracing::info!(
            interval_secs = self.config.worker_interval_secs,
            capture = self.capture.is_some(),
            verification = self.verifier.is_some(),
            "audit relay: delivery worker started"
        );
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("audit relay: delivery worker stopping");
                    return;
                }
                _ = ticker.tick() => {
                    self.sweep(Utc::now()).await;
                }
            }
        }
    }

    /// One full pass. Exposed so tests drive it deterministically rather than
    /// waiting on wall-clock ticks.
    pub async fn sweep(&self, now: DateTime<Utc>) {
        self.close_overdue_starts(now).await;
        self.capture_due(now).await;
        self.verify_accepted(now).await;
        self.purge_expired(now).await;
        self.refresh_gauges(now).await;
    }

    /// Publish the bounded `{state}` gauges and the capacity flag.
    async fn refresh_gauges(&self, now: DateTime<Utc>) {
        let snapshot = self
            .db
            .read(move |connection| {
                let counts = ingest::state_counts(connection)?;
                let oldest = ingest::oldest_per_state(connection)?;
                let total = ingest::record_count(connection)?;
                Ok((counts, oldest, total))
            })
            .await;
        let (counts, oldest, total) = match snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(
                    reason = error.as_str(),
                    "audit relay: could not refresh storage gauges"
                );
                return;
            }
        };

        let mut gauges = StorageGauges {
            db_bytes: database_bytes(&self.config.db_path),
            ..StorageGauges::default()
        };
        for (index, state) in RecordState::ALL.into_iter().enumerate() {
            gauges.records[index] = counts
                .iter()
                .find(|(name, _)| name == state.as_str())
                .map(|(_, count)| *count)
                .unwrap_or(0);
            gauges.oldest_age_secs[index] = oldest
                .iter()
                .find(|(name, _)| name == state.as_str())
                .and_then(|(_, created_at)| age_secs(created_at, now))
                .unwrap_or(0);
        }
        self.metrics.publish(gauges);
        self.metrics
            .set_writer_queue_depth(self.db.writer_queue_depth());

        // Fail closed: at the ceiling, ingress is refused with a bounded error
        // rather than filling the volume and losing what is already stored.
        let at_capacity = total >= self.config.max_records;
        if self.at_capacity.swap(at_capacity, Ordering::Relaxed) != at_capacity {
            if at_capacity {
                tracing::error!(
                    max_records = self.config.max_records,
                    "audit relay: reached the configured record capacity; refusing ingress"
                );
            } else {
                tracing::info!("audit relay: below the record capacity again; accepting ingress");
            }
        }
    }
}

/// The capture-side [`AuditConfig`] the relay drives
/// [`crate::audit::posthog::PostHogClient`] with.
///
/// Reusing that client means reusing its retry classification, its `uuid`
/// deduplication, and its secret hygiene; only the host/token/environment differ
/// from the defaults.
pub(super) fn capture_config(config: &RelayConfig) -> AuditConfig {
    AuditConfig {
        enabled: true,
        host: config.posthog_host.clone(),
        project_token: config.posthog_project_token.clone(),
        environment: config.environment.clone(),
        batch_size: config.capture_batch_size,
        ..AuditConfig::default()
    }
}

/// Size of the database plus its WAL sidecar. A missing file is `0` rather than
/// an error: the gauge is operational colour, not a correctness signal.
fn database_bytes(path: &std::path::Path) -> u64 {
    let wal = path.with_extension(format!(
        "{}-wal",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
    ));
    [path.to_path_buf(), wal]
        .iter()
        .filter_map(|candidate| std::fs::metadata(candidate).ok())
        .map(|metadata| metadata.len())
        .sum()
}

/// Seconds between a stored RFC3339 instant and `now`, or `None` when unparseable.
fn age_secs(stored: &str, now: DateTime<Utc>) -> Option<u64> {
    let parsed = DateTime::parse_from_rfc3339(stored.trim()).ok()?;
    u64::try_from((now - parsed.with_timezone(&Utc)).num_seconds().max(0)).ok()
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
