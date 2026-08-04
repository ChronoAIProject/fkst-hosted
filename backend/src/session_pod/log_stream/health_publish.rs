//! Publishing copied health reports to chrono-storage as small, individually
//! addressable objects plus a compact index.
//!
//! The bundle already makes reports durable, but reaching one means fetching and
//! decompressing the whole `latest.tar.gz`. The dashboard asks "what is this session's
//! health?" on every card render and the staleness watchdog asks it on every read, so
//! both need a kilobyte read. The object layout and every pure transform live in
//! [`crate::session_health::index`], shared with the read API so the writer and the
//! reader can never disagree about the shape; this module is only the in-pod glue.
//!
//! # Why a queue
//!
//! A `put` failure must be retried, but the copy tracker deliberately will not re-copy
//! an unchanged file, so a failed publish would otherwise be lost forever — and with
//! it that tick's entry in the index, permanently. Pending publishes are therefore
//! held here and retried on the FLUSH cadence (~20 s), not the 500 ms poll cadence: a
//! storage outage must not turn into a tight retry loop.
//!
//! # Fail-safe
//!
//! Best-effort throughout, matching the collector. No storage configured → the queue
//! drains to nothing and no error surfaces. An unparseable report is never published
//! (it still reached the bundle, so nothing is silently lost) and is not retried,
//! because it will never parse. Nothing here can crash or block `supervise`.

use crate::session_health::{index_entry, parse_report, parse_report_filename};

use super::copied::CopiedFile;
use super::redact::Redactor;
use super::uploader::Uploader;

/// Most reports held awaiting publication.
///
/// Matches the index cap: holding more than the index can ever list is pure waste,
/// and this is also the backstop against a storage outage growing the queue without
/// bound for the life of the pod.
const MAX_PENDING: usize = crate::session_health::index::MAX_INDEX_ENTRIES;

/// Reports copied into the bundle that still owe chrono-storage a publish.
#[derive(Debug, Default)]
pub(super) struct HealthPublishQueue {
    pending: Vec<CopiedFile>,
}

impl HealthPublishQueue {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Take newly copied reports. Oldest are dropped once the queue is full — the
    /// bundle still carries them, so the loss is confined to the index.
    pub(super) fn enqueue(&mut self, copied: Vec<CopiedFile>, redactor: &Redactor) {
        if copied.is_empty() {
            return;
        }
        self.pending.extend(copied);
        if self.pending.len() > MAX_PENDING {
            let dropped = self.pending.len() - MAX_PENDING;
            self.pending.drain(..dropped);
            log_redacted(
                redactor,
                &format!(
                    "log-stream: dropped {dropped} unpublished health report(s) over the queue cap"
                ),
            );
        }
    }

    /// Attempt every pending publish, keeping the ones that failed for the next cycle.
    ///
    /// With no uploader (no chrono-storage configured) the queue is cleared rather
    /// than accumulating for the pod's whole life: there is nothing to publish to, and
    /// the reports are already in the bundle.
    pub(super) fn publish_pending(
        &mut self,
        uploader: Option<&Uploader>,
        session_id: &str,
        redactor: &Redactor,
    ) {
        if self.pending.is_empty() {
            return;
        }
        let Some(uploader) = uploader else {
            self.pending.clear();
            return;
        };
        let mut retry = Vec::new();
        for file in std::mem::take(&mut self.pending) {
            if publish_one(uploader, session_id, &file, redactor) == Outcome::Retry {
                retry.push(file);
            }
        }
        self.pending = retry;
    }

    /// Whether anything is still owed, so the shutdown drain can say so.
    #[cfg(test)]
    pub(super) fn pending(&self) -> usize {
        self.pending.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Published, or permanently unpublishable — either way, stop carrying it.
    Done,
    /// A transient transport failure; try again next cycle.
    Retry,
}

fn publish_one(
    uploader: &Uploader,
    session_id: &str,
    file: &CopiedFile,
    redactor: &Redactor,
) -> Outcome {
    // The filename must be a contract name: its stem becomes the report's public id
    // and an object-key segment, and `parse_report_filename` is the traversal guard
    // that makes both safe. A `.md` that is not a report still rode into the bundle.
    let Some(name) = parse_report_filename(&file.file_name) else {
        log_redacted(
            redactor,
            &format!(
                "log-stream: {} is not a v1 health report filename; bundled but not published",
                file.file_name
            ),
        );
        return Outcome::Done;
    };

    let report = match parse_report(&file.redacted) {
        Ok(report) => report,
        Err(error) => {
            // Redaction matters here: the parse error can quote the offending input,
            // which came out of a session pod.
            log_redacted(
                redactor,
                &format!(
                    "log-stream: health report {} did not parse; bundled but not published: {error}",
                    file.file_name
                ),
            );
            return Outcome::Done;
        }
    };

    let entry = index_entry(session_id, &name, &report);
    match uploader.publish_health_report(session_id, entry, &file.redacted) {
        Ok(()) => Outcome::Done,
        Err(error) => {
            log_redacted(
                redactor,
                &format!(
                    "log-stream: publishing health report {} failed (will retry): {error}",
                    file.file_name
                ),
            );
            Outcome::Retry
        }
    }
}

fn log_redacted(redactor: &Redactor, message: &str) {
    tracing::warn!(detail = %redactor.redact_line(message));
}

#[cfg(test)]
#[path = "health_publish_tests.rs"]
mod tests;
