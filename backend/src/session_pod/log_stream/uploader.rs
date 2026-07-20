//! The bundle UPLOADER: the collector's write side to chrono-storage.
//!
//! The collector's capture loop produces a redacted `tar.gz`; this module owns WHERE
//! it goes. It is split out of [`super::collector`] so the per-run write policy — the
//! stable machinery of issue #568 — lives in one small, independently-tested place and
//! the capture loop stays focused on capture.
//!
//! Per-run separation, without regressing the legacy download path:
//!
//! - Every flush PUTs the bundle to BOTH the authoritative `latest_key`
//!   (`logs/<sid>/latest.tar.gz`, byte-for-byte the object the existing download path
//!   already reads — zero regression) AND this run's immutable `run_key`
//!   (`logs/<sid>/runs/<run_id>.tar.gz`), so a revived pod never clobbers a prior
//!   run's logs.
//! - The run is registered in the session's `index_key` (`logs/<sid>/runs.json`) via a
//!   read-modify-write: [`Uploader::add_run_to_index`] on the first upload, then
//!   [`Uploader::finalize_run_in_index`] at shutdown. The RMW is race-free because a
//!   session has exactly ONE live pod at a time (idle-reap → auto-revive is strictly
//!   sequential), so two collectors never contend for the same index object.
//!
//! The collector thread is a plain OS thread (not a tokio worker), so the uploader
//! carries its OWN current-thread runtime to `block_on` the async `put`/`get` without
//! ever touching the engine's async runtime. Every method is best-effort: it returns
//! its error for the caller to log-swallow, and NEVER leaks a credential (the errors
//! are the leak-free [`SinkError`] rendering).

use axum::body::Bytes;
use k8s_openapi::chrono::{DateTime, Utc};

use crate::session_spec::creds::CredsLayout;

use super::bundle_key;
use super::collector::CollectorConfig;
use super::runs::{self, LogRun};
use super::sink::{ChronoStorageSink, LogSink, SinkError};

/// The bundle uploader: a [`LogSink`] + the current-thread runtime it is driven on +
/// the object keys it writes (latest, this run, the run index).
pub(super) struct Uploader {
    sink: Box<dyn LogSink>,
    runtime: tokio::runtime::Runtime,
    /// The authoritative whole-session object (overwritten by every run).
    latest_key: String,
    /// This run's immutable per-incarnation object.
    run_key: String,
    /// The session's run-index object.
    index_key: String,
    /// This run's id (== the collector instance id).
    run_id: String,
    /// When this run's pod started (for the index entry).
    started_at: DateTime<Utc>,
}

impl Uploader {
    /// Build an uploader for `session_id`'s `run_id`, deriving the three object keys
    /// from the run model so the layout lives in exactly one place ([`super::runs`]).
    pub(super) fn new(
        sink: Box<dyn LogSink>,
        runtime: tokio::runtime::Runtime,
        session_id: &str,
        run_id: String,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            sink,
            runtime,
            latest_key: bundle_key(session_id),
            run_key: runs::run_bundle_key(session_id, &run_id),
            index_key: runs::runs_index_key(session_id),
            run_id,
            started_at,
        }
    }

    /// Upload `gz` to BOTH the authoritative `latest` object (first, so it is always
    /// correct) and this run's per-incarnation object, blocking the collector thread
    /// for each PUT (bounded by the storage client's request timeout). Returns the
    /// latest result AND-ed with the run result, so a per-run failure leaves the flush
    /// un-acked (retried next cadence) while `latest` — already written — stays
    /// authoritative regardless.
    pub(super) fn upload(&self, gz: Bytes) -> Result<(), SinkError> {
        let latest_result = self
            .runtime
            .block_on(self.sink.put(&self.latest_key, gz.clone()));
        let run_result = self.runtime.block_on(self.sink.put(&self.run_key, gz));
        latest_result.and(run_result)
    }

    /// Register this run in the session's run index (read-modify-write): `get` the
    /// current index, [`runs::upsert_run`] this run in (idempotent), `put` it back.
    /// Best-effort — returns the error for the caller to log-swallow.
    pub(super) fn add_run_to_index(&self) -> Result<(), SinkError> {
        let existing = self.runtime.block_on(self.sink.get(&self.index_key))?;
        let updated = runs::upsert_run(
            existing.as_deref(),
            &LogRun {
                run_id: self.run_id.clone(),
                started_at: runs::rfc3339(self.started_at),
                ended_at: None,
            },
        );
        self.runtime
            .block_on(self.sink.put(&self.index_key, Bytes::from(updated)))
    }

    /// Stamp this run's end time into the session's run index (read-modify-write).
    /// Best-effort — returns the error for the caller to log-swallow.
    pub(super) fn finalize_run_in_index(&self, ended_at: DateTime<Utc>) -> Result<(), SinkError> {
        let existing = self.runtime.block_on(self.sink.get(&self.index_key))?;
        let updated =
            runs::finalize_run(existing.as_deref(), &self.run_id, &runs::rfc3339(ended_at));
        self.runtime
            .block_on(self.sink.put(&self.index_key, Bytes::from(updated)))
    }
}

/// Build the uploader from the mounted storage SA creds, or `None` when they are
/// not configured / a runtime cannot be built — the fail-closed path: the collector
/// still captures + redacts to disk, it just uploads nothing.
pub(super) fn build_uploader(creds: &CredsLayout, config: &CollectorConfig) -> Option<Uploader> {
    let Some(sink) = ChronoStorageSink::from_creds(creds) else {
        tracing::warn!("log-stream: storage SA creds not mounted; capturing without upload");
        return None;
    };
    let runtime = build_upload_runtime()?;
    Some(Uploader::new(
        Box::new(sink),
        runtime,
        &config.session_id,
        config.instance_id.clone(),
        config.start_time,
    ))
}

/// Build the dedicated current-thread runtime the uploader blocks on. A build
/// failure disables uploads (a warning) rather than crashing the collector.
fn build_upload_runtime() -> Option<tokio::runtime::Runtime> {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            tracing::warn!(error = %error, "log-stream: could not build upload runtime; no bundle upload");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::session_pod::log_stream::sink::FakeSink;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    fn uploader(fake: FakeSink) -> Uploader {
        Uploader::new(
            Box::new(fake),
            runtime(),
            "sess-1",
            "run-1".to_string(),
            Utc::now(),
        )
    }

    #[test]
    fn upload_writes_both_the_latest_and_the_per_run_object() {
        let fake = FakeSink::default();
        uploader(fake.clone())
            .upload(Bytes::from_static(b"gz-bytes"))
            .expect("upload ok");

        // Both objects carry the identical bundle bytes.
        assert_eq!(
            fake.stored("logs/sess-1/latest.tar.gz").as_deref(),
            Some(&b"gz-bytes"[..])
        );
        assert_eq!(
            fake.stored("logs/sess-1/runs/run-1.tar.gz").as_deref(),
            Some(&b"gz-bytes"[..])
        );
        // Latest is written FIRST (authoritative), then the run object.
        let calls = fake.calls();
        assert_eq!(calls[0].0, "logs/sess-1/latest.tar.gz");
        assert_eq!(calls[1].0, "logs/sess-1/runs/run-1.tar.gz");
    }

    #[test]
    fn upload_propagates_a_sink_failure_so_the_flush_is_retried() {
        let fake = FakeSink {
            fail: true,
            ..Default::default()
        };
        let err = uploader(fake)
            .upload(Bytes::from_static(b"gz"))
            .expect_err("a failing sink surfaces the error");
        assert!(matches!(err, SinkError::Upload(_)));
    }

    #[test]
    fn add_then_finalize_records_the_run_with_an_end_time() {
        let fake = FakeSink::default();
        let up = uploader(fake.clone());

        up.add_run_to_index().expect("add ok");
        // The live run is in the index with a start time and no end time.
        let mid = runs::parse_runs(&fake.stored("logs/sess-1/runs.json").expect("index"));
        assert_eq!(mid.len(), 1);
        assert_eq!(mid[0].run_id, "run-1");
        assert!(mid[0].ended_at.is_none());

        up.finalize_run_in_index(Utc::now()).expect("finalize ok");
        let end = runs::parse_runs(&fake.stored("logs/sess-1/runs.json").expect("index"));
        assert_eq!(end.len(), 1, "finalize does not duplicate the run");
        assert!(
            end[0].ended_at.is_some(),
            "the run was stamped with an end time"
        );
    }

    #[test]
    fn add_run_to_index_is_idempotent_across_flushes() {
        let fake = FakeSink::default();
        let up = uploader(fake.clone());
        up.add_run_to_index().expect("add 1");
        up.add_run_to_index().expect("add 2");
        let runs = runs::parse_runs(&fake.stored("logs/sess-1/runs.json").expect("index"));
        assert_eq!(runs.len(), 1, "the same run is never duplicated");
    }
}
