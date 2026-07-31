//! Closing expired starts, and the one deletion the relay performs.
//!
//! ## Closing is atomic against a late completion
//!
//! [`super::db::delivery::synthesize_incomplete`] guards on `state = 'started'`,
//! so if a real completion commits in the same instant the synthesis changes
//! nothing and the real terminal wins. That ordering matters: an incomplete
//! record that overwrote a real one would report "no response was ever produced"
//! about a request that returned `200`.
//!
//! ## Retention deletes verified rows and nothing else
//!
//! `posthog_verified` rows exist only as a dedup/query overlap window; once
//! PostHog can answer for them, the relay copy is redundant and is purged after
//! `FKST_AUDIT_RELAY_VERIFIED_RETENTION_DAYS`.
//!
//! Unverified, incomplete, and dead-letter rows are NEVER auto-deleted. They are
//! precisely the records whose delivery could not be proven, which makes them the
//! last thing an audit trail may discard on a timer;
//! `FKST_AUDIT_RELAY_AUDIT_RETENTION_DAYS` is the documented floor an operator
//! keeps them for before remediating deliberately, and the relay validates it
//! against the verified window at startup so the two cannot be inverted.

use k8s_openapi::chrono::{DateTime, Duration, Utc};

use super::db::delivery;
use super::incomplete;
use super::protocol::RequestStartV1;
use super::worker::RelayWorker;

impl RelayWorker {
    /// Close every start past its own deadline plus the configured grace.
    pub(super) async fn close_overdue_starts(&self, now: DateTime<Utc>) {
        let grace = self.config.incomplete_grace_secs;
        let batch_size = self.config.capture_batch_size;
        let overdue = match self
            .db
            .read(move |connection| {
                delivery::claim_overdue_starts(connection, now, grace, batch_size)
            })
            .await
        {
            Ok(overdue) => overdue,
            Err(error) => {
                tracing::warn!(
                    reason = error.as_str(),
                    "audit relay: could not claim overdue request starts"
                );
                return;
            }
        };
        if overdue.is_empty() {
            return;
        }

        let mut closed = 0u64;
        for start in overdue {
            let Ok(registered) = serde_json::from_slice::<RequestStartV1>(&start.start_json) else {
                // The start was validated before it was stored, so this can only
                // mean storage-level damage. It is left alone rather than closed
                // with invented content, and the warning names the stage.
                tracing::error!("audit relay: a stored request start is not readable; leaving it");
                continue;
            };
            let (terminal, terminal_at) = match incomplete::synthesize(&registered) {
                Ok(synthesized) => synthesized,
                Err(error) => {
                    tracing::error!(
                        reason = %error,
                        "audit relay: could not synthesize an incomplete projection"
                    );
                    continue;
                }
            };
            let Ok(encoded) = serde_json::to_vec(&terminal) else {
                tracing::error!("audit relay: could not encode an incomplete projection");
                continue;
            };
            let event_id = start.event_id.clone();
            match self
                .db
                .write(move |transaction| {
                    delivery::synthesize_incomplete(
                        transaction,
                        &event_id,
                        &encoded,
                        terminal_at,
                        now,
                    )
                })
                .await
            {
                // `false` means a real completion won the race — the right
                // outcome, and not an error.
                Ok(true) => closed += 1,
                Ok(false) => {}
                Err(error) => tracing::error!(
                    reason = error.as_str(),
                    "audit relay: could not commit an incomplete projection"
                ),
            }
        }
        if closed > 0 {
            self.metrics.record_incomplete(closed);
            tracing::warn!(
                records = closed,
                grace_secs = self.config.incomplete_grace_secs,
                "audit relay: closed request starts that never completed"
            );
        }
    }

    /// Purge verified rows past the dedup/query overlap window.
    pub(super) async fn purge_expired(&self, now: DateTime<Utc>) {
        let verified_before = now
            - Duration::days(
                i64::try_from(self.config.verified_retention_days).unwrap_or(i64::MAX),
            );
        match self
            .db
            .write(move |transaction| delivery::purge_verified(transaction, verified_before))
            .await
        {
            Ok(0) => {}
            Ok(purged) => {
                self.metrics.record_purged(purged as u64);
                tracing::info!(
                    records = purged,
                    retention_days = self.config.verified_retention_days,
                    "audit relay: purged verified records past the retention window"
                );
            }
            Err(error) => tracing::warn!(
                reason = error.as_str(),
                "audit relay: retention purge failed"
            ),
        }
    }
}

#[cfg(test)]
#[path = "closer_tests.rs"]
mod tests;
