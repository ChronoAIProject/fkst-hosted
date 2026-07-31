//! The verification pass: proving an accepted event is actually query-visible.
//!
//! Capture acceptance and query visibility are different facts, and this module
//! exists so the second one has to be EARNED. It reads a batch of accepted event
//! ids back out of PostHog with a fixed query
//! ([`super::posthog::build_verification_query`]) and marks only what it saw.
//!
//! ## Absence is a delay until it is not
//!
//! PostHog's ingestion lag is real, so an event that is absent seconds after
//! acceptance is normal. Only once a record has been accepted for longer than
//! `FKST_AUDIT_RELAY_VERIFICATION_MAX_AGE_SECS` is absence treated as loss —
//! and even then the answer is to re-capture with the SAME uuid (PostHog
//! deduplicates on it) plus a loud alert, never to mark it delivered and never
//! to drop it.
//!
//! ## What is never done here
//!
//! No record is verified because it was accepted. No batch is assumed visible
//! because the query failed — a failure is counted as `failed` and the records
//! stay accepted, so a broken verification credential produces a growing
//! unverified backlog an operator can see, not a silently "verified" trail.

use std::collections::HashSet;

use k8s_openapi::chrono::{DateTime, Duration, Utc};

use super::db::delivery;
use super::db::row::StoredRecord;
use super::metrics::VerificationResult;
use super::record::RecordState;
use super::worker::RelayWorker;

impl RelayWorker {
    /// Verify one batch of accepted records.
    pub(super) async fn verify_accepted(&self, now: DateTime<Utc>) {
        let Some(client) = self.verifier.as_ref() else {
            return;
        };
        let delay = Duration::seconds(
            i64::try_from(self.config.verification_delay_secs).unwrap_or(i64::MAX),
        );
        let accepted_before = now - delay;
        let batch_size = self.config.verification_batch_size;
        let batch = match self
            .db
            .read(move |connection| {
                delivery::claim_unverified(connection, accepted_before, batch_size)
            })
            .await
        {
            Ok(batch) => batch,
            Err(error) => {
                tracing::warn!(
                    reason = error.as_str(),
                    "audit relay: could not claim records awaiting verification"
                );
                return;
            }
        };
        if batch.is_empty() {
            return;
        }

        let ids: Vec<String> = batch.iter().map(|record| record.event_id.clone()).collect();
        let window_start = window_start(&batch, now, self.config.verification_max_age_secs);
        let visible = match super::posthog::verify_visible(client, &ids, window_start).await {
            Ok(visible) => visible,
            Err(error) => {
                self.metrics
                    .record_verification(VerificationResult::Failed, ids.len() as u64);
                tracing::warn!(
                    reason = error.kind(),
                    records = ids.len(),
                    "audit relay: the verification query could not run"
                );
                return;
            }
        };

        self.mark_visible(&ids, &visible, now).await;
        self.recapture_absent(&batch, &visible, now).await;
    }

    /// Promote everything the query actually returned.
    async fn mark_visible(&self, ids: &[String], visible: &HashSet<String>, now: DateTime<Utc>) {
        let verified: Vec<String> = ids
            .iter()
            .filter(|id| visible.contains(*id))
            .cloned()
            .collect();
        if verified.is_empty() {
            return;
        }
        let count = verified.len() as u64;
        match self
            .db
            .write(move |transaction| delivery::mark_verified(transaction, &verified, now))
            .await
        {
            Ok(_) => self
                .metrics
                .record_verification(VerificationResult::Verified, count),
            Err(error) => tracing::error!(
                reason = error.as_str(),
                "audit relay: could not record verification"
            ),
        }
    }

    /// Send back for re-capture anything accepted longer ago than the lag
    /// threshold that PostHog still cannot see.
    async fn recapture_absent(
        &self,
        batch: &[StoredRecord],
        visible: &HashSet<String>,
        now: DateTime<Utc>,
    ) {
        let stale_cutoff = now
            - Duration::seconds(
                i64::try_from(self.config.verification_max_age_secs).unwrap_or(i64::MAX),
            );
        let absent: Vec<&StoredRecord> = batch
            .iter()
            .filter(|record| !visible.contains(&record.event_id))
            .collect();
        if absent.is_empty() {
            return;
        }
        self.metrics
            .record_verification(VerificationResult::Absent, absent.len() as u64);

        let mut recaptured = 0u64;
        for record in absent {
            if !accepted_before(record, stale_cutoff) {
                // Still inside the tolerated ingestion lag: leave it accepted and
                // look again next sweep.
                continue;
            }
            let event_id = record.event_id.clone();
            let restore = restore_state(record);
            if let Err(error) = self
                .db
                .write(move |transaction| {
                    delivery::requeue_for_recapture(transaction, &event_id, restore, now)
                })
                .await
            {
                tracing::error!(
                    reason = error.as_str(),
                    "audit relay: could not requeue an absent record for re-capture"
                );
                continue;
            }
            recaptured += 1;
        }
        if recaptured > 0 {
            self.metrics.record_recapture(recaptured);
            // An accepted event that PostHog still cannot see past the lag
            // threshold is the alert condition: capture said yes and the store
            // disagrees.
            tracing::error!(
                records = recaptured,
                max_age_secs = self.config.verification_max_age_secs,
                "audit relay: accepted events are still not query-visible; re-capturing with the \
                 same event ids"
            );
        }
    }
}

/// Whether this record was accepted at or before `cutoff`.
fn accepted_before(record: &StoredRecord, cutoff: DateTime<Utc>) -> bool {
    record
        .posthog_accepted_at
        .as_deref()
        .and_then(|raw| DateTime::parse_from_rfc3339(raw.trim()).ok())
        .map(|accepted| accepted.with_timezone(&Utc) <= cutoff)
        .unwrap_or(false)
}

/// Which terminal state a re-captured record returns to.
///
/// An incomplete record must go back to `incomplete`, not `complete`: the state
/// is what the read API's delivery projection reports, and a synthesized record
/// that came back as `complete` would claim a completion that never happened.
fn restore_state(record: &StoredRecord) -> RecordState {
    let Some(terminal) = record.terminal_json.as_deref() else {
        return RecordState::Complete;
    };
    let outcome = serde_json::from_slice::<serde_json::Value>(terminal)
        .ok()
        .and_then(|value| {
            value
                .get("outcome")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
    if outcome.as_deref() == Some(crate::audit::event::AuditOutcome::Incomplete.as_str()) {
        RecordState::Incomplete
    } else {
        RecordState::Complete
    }
}

/// The scan floor for the verification query: the oldest terminal instant in the
/// batch, or the lag threshold when none parses.
fn window_start(batch: &[StoredRecord], now: DateTime<Utc>, max_age_secs: u64) -> DateTime<Utc> {
    let fallback = now - Duration::seconds(i64::try_from(max_age_secs).unwrap_or(i64::MAX));
    batch
        .iter()
        .filter_map(|record| record.terminal_at.as_deref())
        .filter_map(|raw| DateTime::parse_from_rfc3339(raw.trim()).ok())
        .map(|parsed| parsed.with_timezone(&Utc))
        .min()
        // One minute of slack absorbs the millisecond rounding between the
        // recorded terminal instant and PostHog's own event timestamp.
        .map(|oldest| oldest - Duration::minutes(1))
        .unwrap_or(fallback)
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
