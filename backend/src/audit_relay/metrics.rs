//! The relay's bounded telemetry and its Prometheus exposition.
//!
//! Every label is a CLOSED Rust enum, so the series count is finite by
//! construction (epic `OPS-04`). Event ids, request ids, actor ids, session ids,
//! repositories, and route templates are structured-log fields at most — never
//! labels and never values. The one non-enum dimension, `state`, is
//! [`super::record::RecordState`], which has six variants and always will.
//!
//! Counters are `Arc`ed atomics cloned into the HTTP handlers and the worker;
//! the gauges are refreshed by the worker's sweep and read atomically, so a
//! scrape never touches SQLite and can never perturb delivery.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::record::RecordState;

/// Which ingress endpoint was called.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressKind {
    RequestStart,
    RequestCompletion,
    LifecycleEvent,
    RecordsRead,
}

impl IngressKind {
    pub const ALL: [IngressKind; 4] = [
        IngressKind::RequestStart,
        IngressKind::RequestCompletion,
        IngressKind::LifecycleEvent,
        IngressKind::RecordsRead,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IngressKind::RequestStart => "request_start",
            IngressKind::RequestCompletion => "request_completion",
            IngressKind::LifecycleEvent => "lifecycle_event",
            IngressKind::RecordsRead => "records_read",
        }
    }

    fn index(self) -> usize {
        match self {
            IngressKind::RequestStart => 0,
            IngressKind::RequestCompletion => 1,
            IngressKind::LifecycleEvent => 2,
            IngressKind::RecordsRead => 3,
        }
    }
}

/// How an ingress call ended.
///
/// `created` and `replayed` describe what happened in STORAGE, not what HTTP
/// status was returned — a completion always answers `200` whether it committed
/// the terminal projection or acknowledged a byte-identical retry, and an
/// operator watching for retry storms needs to tell those apart. `served` exists
/// so the read endpoint never borrows a write label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressResult {
    /// A new durable record was committed.
    Created,
    /// An exact idempotent replay.
    Replayed,
    /// A scoped read answered a page. Nothing was committed.
    Served,
    /// The event id exists with different immutable content.
    Conflict,
    /// The body failed schema/contract validation.
    Rejected,
    /// Credentials missing or refused.
    Unauthorized,
    /// Storage could not answer.
    Unavailable,
}

impl IngressResult {
    pub const ALL: [IngressResult; 7] = [
        IngressResult::Created,
        IngressResult::Replayed,
        IngressResult::Served,
        IngressResult::Conflict,
        IngressResult::Rejected,
        IngressResult::Unauthorized,
        IngressResult::Unavailable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IngressResult::Created => "created",
            IngressResult::Replayed => "replayed",
            IngressResult::Served => "served",
            IngressResult::Conflict => "conflict",
            IngressResult::Rejected => "rejected",
            IngressResult::Unauthorized => "unauthorized",
            IngressResult::Unavailable => "unavailable",
        }
    }

    fn index(self) -> usize {
        match self {
            IngressResult::Created => 0,
            IngressResult::Replayed => 1,
            IngressResult::Served => 2,
            IngressResult::Conflict => 3,
            IngressResult::Rejected => 4,
            IngressResult::Unauthorized => 5,
            IngressResult::Unavailable => 6,
        }
    }
}

/// The outcome of one capture attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureResult {
    /// PostHog capture answered `2xx`. Accepted, NOT proven query-visible.
    Accepted,
    Retryable,
    Permanent,
}

impl CaptureResult {
    pub const ALL: [CaptureResult; 3] = [
        CaptureResult::Accepted,
        CaptureResult::Retryable,
        CaptureResult::Permanent,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CaptureResult::Accepted => "accepted",
            CaptureResult::Retryable => "retryable",
            CaptureResult::Permanent => "permanent",
        }
    }

    fn index(self) -> usize {
        match self {
            CaptureResult::Accepted => 0,
            CaptureResult::Retryable => 1,
            CaptureResult::Permanent => 2,
        }
    }
}

/// The outcome of one verification read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationResult {
    /// The event id was read back: genuinely query-visible.
    Verified,
    /// Accepted, but still absent past the configured lag threshold.
    Absent,
    /// The verification query itself could not run.
    Failed,
}

impl VerificationResult {
    pub const ALL: [VerificationResult; 3] = [
        VerificationResult::Verified,
        VerificationResult::Absent,
        VerificationResult::Failed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            VerificationResult::Verified => "verified",
            VerificationResult::Absent => "absent",
            VerificationResult::Failed => "failed",
        }
    }

    fn index(self) -> usize {
        match self {
            VerificationResult::Verified => 0,
            VerificationResult::Absent => 1,
            VerificationResult::Failed => 2,
        }
    }
}

/// Why a record was abandoned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeadLetterReason {
    /// PostHog refused the payload permanently (auth, schema).
    Permanent,
    /// Retries were exhausted while the failure was still retryable.
    AttemptsExhausted,
    /// The stored body could not be projected onto the capture wire format.
    Invalid,
}

impl DeadLetterReason {
    pub const ALL: [DeadLetterReason; 3] = [
        DeadLetterReason::Permanent,
        DeadLetterReason::AttemptsExhausted,
        DeadLetterReason::Invalid,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DeadLetterReason::Permanent => "permanent",
            DeadLetterReason::AttemptsExhausted => "attempts_exhausted",
            DeadLetterReason::Invalid => "invalid",
        }
    }

    fn index(self) -> usize {
        match self {
            DeadLetterReason::Permanent => 0,
            DeadLetterReason::AttemptsExhausted => 1,
            DeadLetterReason::Invalid => 2,
        }
    }
}

/// The gauge block one sweep publishes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StorageGauges {
    /// Row count per [`RecordState`], in `RecordState::ALL` order.
    pub records: [u64; RecordState::ALL.len()],
    /// Age in seconds of the oldest row in each state, same order.
    pub oldest_age_secs: [u64; RecordState::ALL.len()],
    /// On-disk size of the database file plus its WAL.
    pub db_bytes: u64,
}

/// Process-local counters plus the last published gauges.
#[derive(Debug, Default)]
struct Counters {
    ingress: [[AtomicU64; IngressResult::ALL.len()]; IngressKind::ALL.len()],
    capture: [AtomicU64; CaptureResult::ALL.len()],
    verification: [AtomicU64; VerificationResult::ALL.len()],
    dead_letters: [AtomicU64; DeadLetterReason::ALL.len()],
    incomplete: AtomicU64,
    recaptures: AtomicU64,
    purged: AtomicU64,
    writer_queue_depth: AtomicU64,
    /// `FKST_AUDIT_RELAY_MAX_RECORDS`, published so an alert can express
    /// headroom as a RATIO. Set once at startup rather than per sweep, because
    /// an alert that only became expressible after the first sweep would be
    /// absent exactly when a relay is failing to start.
    max_records: AtomicU64,
    gauges: Mutex<StorageGauges>,
}

/// The cloneable metrics handle.
#[derive(Clone, Debug, Default)]
pub struct RelayMetrics {
    counters: Arc<Counters>,
}

impl RelayMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_ingress(&self, kind: IngressKind, result: IngressResult) {
        self.counters.ingress[kind.index()][result.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_capture(&self, result: CaptureResult, count: u64) {
        self.counters.capture[result.index()].fetch_add(count, Ordering::Relaxed);
    }

    pub fn record_verification(&self, result: VerificationResult, count: u64) {
        self.counters.verification[result.index()].fetch_add(count, Ordering::Relaxed);
    }

    pub fn record_dead_letter(&self, reason: DeadLetterReason) {
        self.counters.dead_letters[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_incomplete(&self, count: u64) {
        self.counters.incomplete.fetch_add(count, Ordering::Relaxed);
    }

    pub fn record_recapture(&self, count: u64) {
        self.counters.recaptures.fetch_add(count, Ordering::Relaxed);
    }

    pub fn record_purged(&self, count: u64) {
        self.counters.purged.fetch_add(count, Ordering::Relaxed);
    }

    pub fn set_writer_queue_depth(&self, depth: u64) {
        self.counters
            .writer_queue_depth
            .store(depth, Ordering::Relaxed);
    }

    /// Publish the configured capacity guard so headroom alerting is a ratio
    /// against the deployment's own limit rather than a number copied into a
    /// Prometheus rule and left behind when the claim is resized.
    pub fn set_max_records(&self, max_records: u64) {
        self.counters
            .max_records
            .store(max_records, Ordering::Relaxed);
    }

    /// Publish the sweep's gauge block.
    pub fn publish(&self, gauges: StorageGauges) {
        *self
            .counters
            .gauges
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = gauges;
    }

    /// A consistent read of the gauge block.
    pub fn gauges(&self) -> StorageGauges {
        self.counters
            .gauges
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// One counter's value (tests and the exposition).
    pub fn ingress_count(&self, kind: IngressKind, result: IngressResult) -> u64 {
        self.counters.ingress[kind.index()][result.index()].load(Ordering::Relaxed)
    }

    pub fn capture_count(&self, result: CaptureResult) -> u64 {
        self.counters.capture[result.index()].load(Ordering::Relaxed)
    }

    pub fn verification_count(&self, result: VerificationResult) -> u64 {
        self.counters.verification[result.index()].load(Ordering::Relaxed)
    }

    pub fn dead_letter_count(&self, reason: DeadLetterReason) -> u64 {
        self.counters.dead_letters[reason.index()].load(Ordering::Relaxed)
    }

    pub fn incomplete_count(&self) -> u64 {
        self.counters.incomplete.load(Ordering::Relaxed)
    }

    /// Render the Prometheus exposition body.
    ///
    /// Every label tuple is rendered even when its counter is zero, so a quiet
    /// series is visibly quiet rather than silently absent.
    pub fn render(&self, ingress_ready: bool) -> String {
        let gauges = self.gauges();
        let mut body = format!(
            "# HELP fkst_audit_relay_up 1 when the relay is serving.\n\
             # TYPE fkst_audit_relay_up gauge\n\
             fkst_audit_relay_up 1\n\
             # HELP fkst_audit_relay_ingress_ready 1 when durable ingress can still be promised.\n\
             # TYPE fkst_audit_relay_ingress_ready gauge\n\
             fkst_audit_relay_ingress_ready {}\n\
             # HELP fkst_audit_relay_db_bytes On-disk size of the outbox database and its WAL.\n\
             # TYPE fkst_audit_relay_db_bytes gauge\n\
             fkst_audit_relay_db_bytes {}\n\
             # HELP fkst_audit_relay_writer_queue_depth Records queued for the single writer.\n\
             # TYPE fkst_audit_relay_writer_queue_depth gauge\n\
             fkst_audit_relay_writer_queue_depth {}\n\
             # HELP fkst_audit_relay_max_records Configured record capacity guard; ingress is refused at it.\n\
             # TYPE fkst_audit_relay_max_records gauge\n\
             fkst_audit_relay_max_records {}\n\
             # HELP fkst_audit_relay_records Stored records by bounded delivery state.\n\
             # TYPE fkst_audit_relay_records gauge\n",
            u8::from(ingress_ready),
            gauges.db_bytes,
            self.counters.writer_queue_depth.load(Ordering::Relaxed),
            self.counters.max_records.load(Ordering::Relaxed),
        );
        for (index, state) in RecordState::ALL.into_iter().enumerate() {
            body.push_str(&format!(
                "fkst_audit_relay_records{{state=\"{}\"}} {}\n",
                state.as_str(),
                gauges.records[index]
            ));
        }
        body.push_str(
            "# HELP fkst_audit_relay_oldest_record_age_seconds Age of the oldest record in each state.\n\
             # TYPE fkst_audit_relay_oldest_record_age_seconds gauge\n",
        );
        for (index, state) in RecordState::ALL.into_iter().enumerate() {
            body.push_str(&format!(
                "fkst_audit_relay_oldest_record_age_seconds{{state=\"{}\"}} {}\n",
                state.as_str(),
                gauges.oldest_age_secs[index]
            ));
        }
        body.push_str(
            "# HELP fkst_audit_relay_ingress_total Internal-protocol calls by bounded kind and result.\n\
             # TYPE fkst_audit_relay_ingress_total counter\n",
        );
        for kind in IngressKind::ALL {
            for result in IngressResult::ALL {
                body.push_str(&format!(
                    "fkst_audit_relay_ingress_total{{kind=\"{}\",result=\"{}\"}} {}\n",
                    kind.as_str(),
                    result.as_str(),
                    self.ingress_count(kind, result)
                ));
            }
        }
        body.push_str(
            "# HELP fkst_audit_relay_capture_total PostHog capture attempts by bounded result (accepted is not verified).\n\
             # TYPE fkst_audit_relay_capture_total counter\n",
        );
        for result in CaptureResult::ALL {
            body.push_str(&format!(
                "fkst_audit_relay_capture_total{{result=\"{}\"}} {}\n",
                result.as_str(),
                self.capture_count(result)
            ));
        }
        body.push_str(
            "# HELP fkst_audit_relay_verification_total Query-visibility checks by bounded result.\n\
             # TYPE fkst_audit_relay_verification_total counter\n",
        );
        for result in VerificationResult::ALL {
            body.push_str(&format!(
                "fkst_audit_relay_verification_total{{result=\"{}\"}} {}\n",
                result.as_str(),
                self.verification_count(result)
            ));
        }
        body.push_str(
            "# HELP fkst_audit_relay_dead_letters_total Records abandoned permanently, by bounded reason.\n\
             # TYPE fkst_audit_relay_dead_letters_total counter\n",
        );
        for reason in DeadLetterReason::ALL {
            body.push_str(&format!(
                "fkst_audit_relay_dead_letters_total{{reason=\"{}\"}} {}\n",
                reason.as_str(),
                self.dead_letter_count(reason)
            ));
        }
        body.push_str(&format!(
            "# HELP fkst_audit_relay_incomplete_total Requests closed as incomplete past their deadline.\n\
             # TYPE fkst_audit_relay_incomplete_total counter\n\
             fkst_audit_relay_incomplete_total {}\n\
             # HELP fkst_audit_relay_recaptures_total Accepted events re-captured because they stayed absent.\n\
             # TYPE fkst_audit_relay_recaptures_total counter\n\
             fkst_audit_relay_recaptures_total {}\n\
             # HELP fkst_audit_relay_purged_total Verified records removed after the retention window.\n\
             # TYPE fkst_audit_relay_purged_total counter\n\
             fkst_audit_relay_purged_total {}\n",
            self.incomplete_count(),
            self.counters.recaptures.load(Ordering::Relaxed),
            self.counters.purged.load(Ordering::Relaxed),
        ));
        body
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
