//! The capture pass: FIFO batches to PostHog, with a poison record able to die
//! alone.
//!
//! ## Fairness
//!
//! [`super::db::delivery::claim_due`] orders by `(terminal_at, event_id)`, so the
//! oldest terminal record is always attempted first — a backlog drains in the
//! order it was produced. What must not happen is the other half of FIFO: one
//! permanently-rejected record parking every record behind it forever. Two
//! mechanisms prevent that, and both are needed:
//!
//! - a **retryable** batch failure schedules each record individually with its
//!   own attempt counter and capped backoff, so a record that keeps failing backs
//!   off further and further while healthy records keep flowing;
//! - a **permanent** batch failure is retried record-by-record in the same pass.
//!   PostHog rejects a whole batch for one malformed payload, so isolating is the
//!   only way to learn WHICH record is poison. The offender dead-letters; its
//!   batch-mates are accepted.
//!
//! ## Acceptance is not delivery
//!
//! A `2xx` sets `posthog_accepted_at` and moves the row to `posthog_accepted`.
//! Nothing in this module ever sets `posthog_verified_at`; that belongs to
//! [`super::verify`], which has actually read the event back.

use k8s_openapi::chrono::{DateTime, Duration, Utc};

use crate::audit::posthog::CaptureError;
use crate::audit::projection::CaptureEvent;

use super::db::delivery;
use super::db::row::StoredRecord;
use super::metrics::{CaptureResult, DeadLetterReason};
use super::projection;
use super::worker::RelayWorker;

impl RelayWorker {
    /// Attempt one batch of due records.
    pub(super) async fn capture_due(&self, now: DateTime<Utc>) {
        let Some(client) = self.capture.as_ref() else {
            return;
        };
        let batch_size = self.config.capture_batch_size;
        let due = match self
            .db
            .read(move |connection| delivery::claim_due(connection, now, batch_size))
            .await
        {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(
                    reason = error.as_str(),
                    "audit relay: could not claim due records"
                );
                return;
            }
        };
        if due.is_empty() {
            return;
        }

        let mut events = Vec::with_capacity(due.len());
        let mut records = Vec::with_capacity(due.len());
        for record in due {
            match projection::capture_event(&record, self.limits) {
                Ok(event) => {
                    events.push(event);
                    records.push(record);
                }
                // A stored row this build cannot project will never become
                // projectable by waiting. It dead-letters with a bounded reason
                // and is RETAINED for an operator.
                Err(error) => {
                    tracing::error!(
                        reason = error.as_str(),
                        "audit relay: dead-lettering a record that cannot be projected"
                    );
                    self.dead_letter(&record, DeadLetterReason::Invalid, error.as_str(), now)
                        .await;
                }
            }
        }
        if events.is_empty() {
            return;
        }

        match client.capture(&events).await {
            Ok(()) => self.accept(&records, now).await,
            Err(error) if error.is_retryable() => {
                self.metrics
                    .record_capture(CaptureResult::Retryable, records.len() as u64);
                tracing::warn!(
                    records = records.len(),
                    "audit relay: capture batch failed retryably"
                );
                for record in &records {
                    self.reschedule(record, "retryable", now).await;
                }
            }
            Err(error) => {
                self.isolate_permanent(client, &records, events, error, now)
                    .await
            }
        }
    }

    /// Mark a whole batch accepted.
    async fn accept(&self, records: &[StoredRecord], now: DateTime<Utc>) {
        let ids: Vec<String> = records.iter().map(|r| r.event_id.clone()).collect();
        let count = ids.len() as u64;
        match self
            .db
            .write(move |transaction| delivery::mark_accepted(transaction, &ids, now))
            .await
        {
            Ok(_) => self.metrics.record_capture(CaptureResult::Accepted, count),
            Err(error) => tracing::error!(
                reason = error.as_str(),
                "audit relay: could not record capture acceptance"
            ),
        }
    }

    /// A permanent batch rejection: retry each record alone so only the poison
    /// one dies. A single-record batch is already isolated.
    async fn isolate_permanent(
        &self,
        client: &crate::audit::posthog::PostHogClient,
        records: &[StoredRecord],
        events: Vec<CaptureEvent>,
        error: CaptureError,
        now: DateTime<Utc>,
    ) {
        if records.len() == 1 {
            self.metrics.record_capture(CaptureResult::Permanent, 1);
            tracing::error!(
                retryable = error.is_retryable(),
                "audit relay: capture rejected a record permanently"
            );
            self.dead_letter(&records[0], DeadLetterReason::Permanent, "permanent", now)
                .await;
            return;
        }
        tracing::warn!(
            records = records.len(),
            "audit relay: capture rejected the batch permanently; isolating records"
        );
        for (record, event) in records.iter().zip(events) {
            match client.capture(std::slice::from_ref(&event)).await {
                Ok(()) => self.accept(std::slice::from_ref(record), now).await,
                Err(inner) if inner.is_retryable() => {
                    self.metrics.record_capture(CaptureResult::Retryable, 1);
                    self.reschedule(record, "retryable", now).await;
                }
                Err(_) => {
                    self.metrics.record_capture(CaptureResult::Permanent, 1);
                    self.dead_letter(record, DeadLetterReason::Permanent, "permanent", now)
                        .await;
                }
            }
        }
    }

    /// Schedule the next attempt, or give up when the budget is spent.
    async fn reschedule(&self, record: &StoredRecord, code: &'static str, now: DateTime<Utc>) {
        if record.capture_attempts + 1 >= self.config.max_capture_attempts {
            tracing::error!(
                attempts = record.capture_attempts + 1,
                "audit relay: dead-lettering a record after exhausting its capture attempts"
            );
            self.dead_letter(record, DeadLetterReason::AttemptsExhausted, code, now)
                .await;
            return;
        }
        let delay = self.backoff(record.capture_attempts);
        let event_id = record.event_id.clone();
        let next = now + delay;
        if let Err(error) = self
            .db
            .write(move |transaction| delivery::mark_retry(transaction, &event_id, next, code, now))
            .await
        {
            tracing::error!(
                reason = error.as_str(),
                "audit relay: could not schedule a capture retry"
            );
        }
    }

    /// Abandon one record permanently. It stays in the database.
    pub(super) async fn dead_letter(
        &self,
        record: &StoredRecord,
        reason: DeadLetterReason,
        code: &'static str,
        now: DateTime<Utc>,
    ) {
        let event_id = record.event_id.clone();
        match self
            .db
            .write(move |transaction| delivery::mark_dead_letter(transaction, &event_id, code, now))
            .await
        {
            Ok(()) => self.metrics.record_dead_letter(reason),
            Err(error) => tracing::error!(
                reason = error.as_str(),
                "audit relay: could not record a dead letter"
            ),
        }
    }

    /// Exponential backoff capped at the configured ceiling.
    fn backoff(&self, attempts: u32) -> Duration {
        let factor = 1u64.checked_shl(attempts.min(20)).unwrap_or(u64::MAX);
        let seconds = self
            .config
            .retry_initial_secs
            .saturating_mul(factor)
            .min(self.config.retry_max_secs);
        Duration::seconds(i64::try_from(seconds).unwrap_or(i64::MAX))
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
