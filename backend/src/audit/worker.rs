//! The direct-to-PostHog delivery worker: bounded queue, batching, capped
//! retries, and a bounded graceful drain.
//!
//! Shape and its reasons:
//!
//! - **Admission is a bounded, non-blocking `try_send`.** Audit backpressure must
//!   never become product latency, so a full queue drops the newest event with a
//!   metric and a warning instead of awaiting capacity.
//! - **One task owns the buffer and the HTTP client.** All state lives in the
//!   task, so there is no lock on the request path, and nothing here blocks a
//!   Tokio worker thread (every wait is an `await`).
//! - **Retries are capped, jittered, and `Retry-After`-aware.** The jitter is the
//!   shared [`crate::retry::jittered_delay`], so a fleet of replicas retrying the
//!   same PostHog outage does not re-converge on one instant.
//! - **Failure is loud.** Queue overflow, contract violations, oversize records,
//!   exhausted retries, permanent rejections, and an expired drain deadline each
//!   increment a bounded metric AND emit a structured log. Nothing is silently
//!   discarded (epic `AUD-06`).
//!
//! This is the *functional* delivery implementation. The production audit
//! guarantee (`required` mode) belongs to the durable relay, which plugs into the
//! same [`AuditSink`] boundary — so nothing here may assume PostHog is the final
//! destination.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::config::AuditConfig;
use super::event::ApiRequestCompletedV1;
use super::metrics::{AuditMetrics, DeliveryResult, DropReason};
use super::posthog::PostHogClient;
use super::projection::{CaptureEvent, EventLimits};
use super::sink::{AuditSink, DrainReport, SubmitError};
use crate::retry::jittered_delay;

/// Spread applied to every retry delay (±%), so replicas do not synchronise.
const RETRY_JITTER_PERCENT: u64 = 20;

/// Slack added to the configured drain budget before [`PostHogSink::drain`]
/// gives up waiting for the worker. Covers the in-flight HTTP request that the
/// worker may be finishing exactly as the budget expires.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Capped exponential backoff parameters for one batch.
#[derive(Clone, Copy, Debug)]
struct RetryPolicy {
    max_retries: u32,
    initial: Duration,
    max: Duration,
}

/// How a batch ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BatchOutcome {
    /// Accepted by capture (not proven query-visible).
    Accepted,
    /// Shutdown arrived mid-retry; the batch is handed back for the final drain.
    Interrupted,
    /// Still failing retryably after the retry budget was exhausted.
    Retryable,
    /// Rejected permanently (payload/auth/configuration).
    Permanent,
}

/// The queue-side handle installed on the application state.
#[derive(Debug)]
pub struct PostHogSink {
    tx: mpsc::Sender<ApiRequestCompletedV1>,
    /// Events admitted but not yet received by the worker.
    depth: Arc<AtomicU64>,
    /// Admission gate: set by [`PostHogSink::drain`] before cancellation, so no
    /// event can slip in behind the drain.
    closed: Arc<AtomicBool>,
    cancel: CancellationToken,
    finished: watch::Receiver<bool>,
    remaining: Arc<AtomicU64>,
    drain_budget: Duration,
    metrics: AuditMetrics,
}

/// Start the worker task and return its queue-side sink.
pub fn spawn(config: &AuditConfig, client: PostHogClient, metrics: AuditMetrics) -> PostHogSink {
    let (tx, rx) = mpsc::channel(config.queue_capacity);
    let depth = Arc::new(AtomicU64::new(0));
    let remaining = Arc::new(AtomicU64::new(0));
    let cancel = CancellationToken::new();
    let (finished_tx, finished_rx) = watch::channel(false);
    let drain_budget = Duration::from_secs(config.shutdown_flush_secs);

    let worker = Worker {
        rx,
        client,
        metrics: metrics.clone(),
        depth: depth.clone(),
        cancel: cancel.clone(),
        limits: EventLimits::new(config.max_event_bytes),
        batch_size: config.batch_size,
        flush_interval: Duration::from_millis(config.flush_interval_ms),
        retry: RetryPolicy {
            max_retries: config.max_retries,
            initial: Duration::from_millis(config.retry_initial_ms),
            max: Duration::from_millis(config.retry_max_ms),
        },
        drain_budget,
        buffer: Vec::with_capacity(config.batch_size),
    };
    let worker_remaining = remaining.clone();
    tokio::spawn(async move { worker.run(finished_tx, worker_remaining).await });

    PostHogSink {
        tx,
        depth,
        closed: Arc::new(AtomicBool::new(false)),
        cancel,
        finished: finished_rx,
        remaining,
        drain_budget,
        metrics,
    }
}

#[async_trait::async_trait]
impl AuditSink for PostHogSink {
    fn submit(&self, event: ApiRequestCompletedV1) -> Result<(), SubmitError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(SubmitError::ShuttingDown);
        }
        match self.tx.try_send(event) {
            Ok(()) => {
                let depth = self.depth.fetch_add(1, Ordering::Relaxed).saturating_add(1);
                self.metrics.set_queue_depth(depth);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => Err(SubmitError::QueueFull),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(SubmitError::ShuttingDown),
        }
    }

    fn queue_depth(&self) -> u64 {
        self.depth.load(Ordering::Relaxed)
    }

    fn is_delivering(&self) -> bool {
        true
    }

    async fn drain(&self) -> DrainReport {
        // Close admission BEFORE cancelling, so the worker's final sweep sees a
        // queue that can only shrink.
        self.closed.store(true, Ordering::Release);
        self.cancel.cancel();
        let mut finished = self.finished.clone();
        // `wait_for` evaluates the current value first, so a worker that already
        // finished is observed immediately rather than waited on forever.
        if tokio::time::timeout(
            self.drain_budget + DRAIN_GRACE,
            finished.wait_for(|done| *done),
        )
        .await
        .is_err()
        {
            tracing::warn!(
                budget_secs = self.drain_budget.as_secs(),
                "audit drain did not complete within its budget"
            );
        }
        let remaining = self.remaining.load(Ordering::Relaxed);
        if remaining > 0 {
            tracing::warn!(remaining, "audit drain finished with undelivered events");
        } else {
            tracing::info!("audit drain complete");
        }
        DrainReport { remaining }
    }
}

/// The delivery task. Owns everything mutable, so the request path is lock-free.
struct Worker {
    rx: mpsc::Receiver<ApiRequestCompletedV1>,
    client: PostHogClient,
    metrics: AuditMetrics,
    depth: Arc<AtomicU64>,
    cancel: CancellationToken,
    limits: EventLimits,
    batch_size: usize,
    flush_interval: Duration,
    retry: RetryPolicy,
    drain_budget: Duration,
    buffer: Vec<CaptureEvent>,
}

impl Worker {
    async fn run(mut self, finished: watch::Sender<bool>, remaining: Arc<AtomicU64>) {
        let mut ticker = tokio::time::interval(self.flush_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick is immediate; consume it so an empty queue does not
        // burn a flush cycle at startup.
        ticker.tick().await;
        tracing::info!(
            batch_size = self.batch_size,
            flush_interval_ms = self.flush_interval.as_millis(),
            "audit delivery worker started"
        );
        loop {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => break,
                received = self.rx.recv() => match received {
                    Some(event) => {
                        self.note_received();
                        self.accept(event);
                        if self.buffer.len() >= self.batch_size {
                            self.flush(None).await;
                        }
                    }
                    // Every sink handle was dropped: nothing more can arrive.
                    None => break,
                },
                _ = ticker.tick() => self.flush(None).await,
            }
        }
        let residual = self.final_drain().await;
        remaining.store(residual, Ordering::Relaxed);
        self.metrics.set_shutdown_remaining(residual);
        // Ignore a send error: it only means the sink was dropped without ever
        // calling `drain`, in which case nobody is waiting for the report.
        let _ = finished.send(true);
        tracing::info!(residual, "audit delivery worker stopped");
    }

    /// One event left the queue: keep the depth gauge honest.
    fn note_received(&self) {
        let depth = self
            .depth
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(1))
            })
            .unwrap_or(0)
            .saturating_sub(1);
        self.metrics.set_queue_depth(depth);
    }

    /// Project one record into the buffer, or drop it loudly.
    fn accept(&mut self, event: ApiRequestCompletedV1) {
        match event.to_capture_event(self.limits) {
            Ok(projected) => self.buffer.push(projected),
            Err(error) => {
                let reason = match error {
                    super::validate::EventError::TooLarge { .. } => DropReason::Oversized,
                    _ => DropReason::Invalid,
                };
                self.metrics.record_dropped(reason, 1);
                // Identifiers only: the record itself may hold data that is safe
                // for PostHog but not for a log line.
                tracing::error!(
                    reason = reason.as_str(),
                    error = %error,
                    event_id = %event.event_id,
                    operation_id = %event.operation_id,
                    "audit event rejected before delivery"
                );
            }
        }
    }

    /// Send the buffered batch. `deadline` bounds retries during the drain.
    async fn flush(&mut self, deadline: Option<Instant>) {
        if self.buffer.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.buffer);
        let count = u64::try_from(batch.len()).unwrap_or(u64::MAX);
        match self.deliver(&batch, deadline).await {
            BatchOutcome::Accepted => {
                self.metrics.record_batch(DeliveryResult::Accepted);
                tracing::debug!(events = batch.len(), "audit batch accepted by capture");
            }
            BatchOutcome::Interrupted => {
                // Shutdown arrived mid-retry: keep the batch so the bounded
                // final drain gets one more chance at it.
                self.buffer = batch;
            }
            BatchOutcome::Retryable => {
                self.metrics.record_batch(DeliveryResult::Retryable);
                self.metrics.record_dropped(DropReason::Retryable, count);
                tracing::error!(
                    events = count,
                    "audit batch abandoned after exhausting retries"
                );
            }
            BatchOutcome::Permanent => {
                self.metrics.record_batch(DeliveryResult::Permanent);
                self.metrics.record_dropped(DropReason::Permanent, count);
                tracing::error!(events = count, "audit batch permanently rejected");
            }
        }
        self.buffer.reserve(self.batch_size);
    }

    /// Attempt one batch with capped, jittered exponential backoff.
    async fn deliver(&self, batch: &[CaptureEvent], deadline: Option<Instant>) -> BatchOutcome {
        let mut attempt = 0_u32;
        let mut backoff = self.retry.initial;
        loop {
            let started = Instant::now();
            let error = match self.client.capture(batch).await {
                Ok(()) => {
                    self.metrics
                        .record_delivery_attempt(DeliveryResult::Accepted, started.elapsed());
                    return BatchOutcome::Accepted;
                }
                Err(error) => {
                    self.metrics
                        .record_delivery_attempt(error.delivery_result(), started.elapsed());
                    error
                }
            };
            if !error.is_retryable() {
                return BatchOutcome::Permanent;
            }
            if attempt >= self.retry.max_retries {
                return BatchOutcome::Retryable;
            }
            // A server-supplied `Retry-After` wins, but is capped: an hour-long
            // hint would otherwise pin the whole queue behind one batch.
            let wait = jittered_delay(
                error.retry_after().unwrap_or(backoff).min(self.retry.max),
                RETRY_JITTER_PERCENT,
            );
            if let Some(deadline) = deadline {
                if deadline.saturating_duration_since(Instant::now()) <= wait {
                    return BatchOutcome::Retryable;
                }
                tokio::time::sleep(wait).await;
            } else {
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = self.cancel.cancelled() => return BatchOutcome::Interrupted,
                }
            }
            attempt += 1;
            backoff = backoff.saturating_mul(2).min(self.retry.max);
        }
    }

    /// Drain what is still queued within the configured budget and report the
    /// residue. Admission is already closed by [`PostHogSink::drain`].
    async fn final_drain(&mut self) -> u64 {
        let deadline = Instant::now()
            .checked_add(self.drain_budget)
            .unwrap_or_else(Instant::now);
        while Instant::now() < deadline {
            match self.rx.try_recv() {
                Ok(event) => {
                    self.note_received();
                    self.accept(event);
                    if self.buffer.len() >= self.batch_size {
                        self.flush(Some(deadline)).await;
                    }
                }
                // Empty means admission is closed and the queue is exhausted;
                // disconnected means every sink handle is gone. Both are done.
                Err(mpsc::error::TryRecvError::Empty)
                | Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        self.flush(Some(deadline)).await;
        let unsent = u64::try_from(self.buffer.len()).unwrap_or(u64::MAX);
        let residual = unsent.saturating_add(self.depth.load(Ordering::Relaxed));
        if residual > 0 {
            self.metrics.record_dropped(DropReason::Shutdown, residual);
            tracing::error!(
                residual,
                "audit drain deadline elapsed with undelivered events"
            );
        }
        residual
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
