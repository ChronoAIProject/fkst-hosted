//! The IN-POD log collector: capture → redact → tar.gz → chrono-storage upload.
//!
//! Spawned from the `run-substrate` driver on EVERY session, the collector runs on
//! its OWN OS thread (not a tokio task) so its blocking work never stalls the async
//! runtime that drives the engine. Records arrive two ways: the tee forwards the
//! supervise child + driver lines over a bounded channel, and a timer tails the
//! framework-child + codex log files incrementally. EVERY record is passed through
//! the fail-closed [`Redactor`] before it touches disk; the redacted tree is folded
//! into a single `tar.gz` and uploaded to `logs/<session_id>/latest.tar.gz` through
//! the [`LogSink`] on a size/time cadence plus a final flush at shutdown.
//!
//! Fail-safe by construction: the whole thing is best-effort. A redaction, file,
//! bundle, or upload error is logged (redacted) and swallowed — it can NEVER crash
//! the session or block `supervise`; the engine keeps running even if streaming
//! fails outright, and a session whose control plane configured no write-only SA
//! simply produces no bundle (the uploader is not spawned).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use k8s_openapi::chrono::{DateTime, Utc};

use crate::session_spec::creds::CredsLayout;

use super::bundle::tar_gz_dir;
use super::classify::{discover_sources, LogClass, TreeAnchors};
use super::instance::{compute_instance_id, readme_markdown, InstanceMeta};
use super::redact::Redactor;
use super::seed::{read_github_token, seed_secrets, LABEL_GITHUB_TOKEN};
use super::sink::{ChronoStorageSink, LogSink, SinkError};
use super::tail::TailTracker;
use super::{
    bundle_key, DEFAULT_FLUSH_BYTES, DEFAULT_FLUSH_SECS, ENV_CONFIG_HASH, ENV_FLUSH_BYTES,
    ENV_FLUSH_SECS, ENV_POD_NAME, ENV_POD_UID, ENV_SESSION_ID, ENV_TRIGGER_ISSUE,
};

/// One captured, not-yet-redacted log line + the tree class it routes to. The tee
/// tags supervise/driver lines; tailed files are tagged by their source dir.
pub type CollectorRecord = (LogClass, String);

/// How often the collector wakes to tail files + check flush cadence.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// How often the mounted GitHub token is re-read to pick up a control-plane rotation.
const TOKEN_REREAD_SECS: u64 = 30;
/// Upper bound on how long shutdown waits for the final flush (a stuck upload must
/// not wedge pod termination).
const SHUTDOWN_SECS: u64 = 30;

/// Everything the collector thread needs. All fields are owned + `Send` so the whole
/// struct moves onto the collector thread. Non-secret (only the creds DIR path — the
/// values are read on the thread), so it is safe to hold.
pub struct CollectorConfig {
    pub instance_id: String,
    /// The deterministic session id; the bundle uploads to `logs/<session_id>/…`.
    pub session_id: String,
    pub trigger_issue: i64,
    /// `owner/name` of the target repo (recorded in `meta.json`/`README`).
    pub repo: String,
    pub engine_ref: String,
    pub config_hash: String,
    pub pod_uid: String,
    pub start_time: DateTime<Utc>,
    pub runtime_root: PathBuf,
    pub codex_home: PathBuf,
    pub creds_dir: PathBuf,
    /// The local staging dir the redacted tree is written into (then tar.gz'd).
    pub tree_dir: PathBuf,
    pub flush_secs: u64,
    pub flush_bytes: usize,
    pub channel_capacity: usize,
}

/// The driver's handle to a running collector: a cloneable sender (for the tee +
/// driver records) and the join handle for an ordered shutdown.
pub struct LogStreamHandle {
    tx: SyncSender<CollectorRecord>,
    join: Option<JoinHandle<()>>,
}

impl LogStreamHandle {
    /// A sender clone to hand to the tee tasks.
    pub fn sender(&self) -> SyncSender<CollectorRecord> {
        self.tx.clone()
    }

    /// Forward one of the driver's OWN records into `fkst-hosted/driver.log`.
    /// Best-effort (drop on a full/closed channel); the line is never logged raw.
    pub fn emit_driver(&self, line: impl Into<String>) {
        let _ = self.tx.try_send((LogClass::HostedDriver, line.into()));
    }

    /// Signal end-of-stream and wait (bounded) for the final flush. Dropping the
    /// sender disconnects the channel, so the collector drains, does a final flush,
    /// and exits; the join is time-boxed so a stuck upload cannot wedge shutdown.
    pub async fn shutdown(self) {
        let LogStreamHandle { tx, join } = self;
        drop(tx);
        if let Some(join) = join {
            let _ = tokio::time::timeout(
                Duration::from_secs(SHUTDOWN_SECS),
                tokio::task::spawn_blocking(move || {
                    let _ = join.join();
                }),
            )
            .await;
        }
    }
}

/// Spawn the collector on its own thread and return the driver's handle. A thread
/// spawn failure degrades gracefully: the handle still exists but every send drops
/// (the engine is unaffected), so streaming simply does nothing.
pub fn spawn_collector(config: CollectorConfig) -> LogStreamHandle {
    let (tx, rx) = sync_channel::<CollectorRecord>(config.channel_capacity.max(1));
    let join = std::thread::Builder::new()
        .name("fkst-logstream".to_string())
        .spawn(move || run_collector(config, rx))
        .map_err(|error| {
            tracing::warn!(error = %error, "log-stream: could not spawn collector thread; streaming disabled");
            error
        })
        .ok();
    LogStreamHandle { tx, join }
}

/// Fixed number of the in-flight log lines the bounded channel buffers before the
/// tee starts dropping (drop-on-full keeps the engine non-blocking).
const CHANNEL_CAPACITY: usize = 16_384;

/// Assemble a [`CollectorConfig`] from the pod-injected `FKST_*` env plus the
/// driver-derived inputs. Streaming is unconditional, so this always yields a
/// config; the `tree_dir` is an isolated `logstream/` under the runtime root so the
/// staging tree never collides with the workspace/project clones.
pub fn collector_config_from_env(
    repo: String,
    engine_ref: String,
    runtime_root: PathBuf,
    codex_home: PathBuf,
    creds_dir: PathBuf,
) -> CollectorConfig {
    let session_id = std::env::var(ENV_SESSION_ID).unwrap_or_default();
    let trigger_issue = std::env::var(ENV_TRIGGER_ISSUE)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let pod_uid = std::env::var(ENV_POD_UID).unwrap_or_default();
    let pod_name = std::env::var(ENV_POD_NAME).unwrap_or_default();
    let start_time = Utc::now();
    let instance_id = compute_instance_id(start_time, &pod_uid, &pod_name);
    let tree_dir = runtime_root.join("logstream");

    CollectorConfig {
        instance_id,
        session_id,
        trigger_issue,
        repo,
        engine_ref,
        config_hash: std::env::var(ENV_CONFIG_HASH).unwrap_or_default(),
        pod_uid,
        start_time,
        runtime_root,
        codex_home,
        creds_dir,
        tree_dir,
        flush_secs: parse_env_u64(ENV_FLUSH_SECS, DEFAULT_FLUSH_SECS),
        flush_bytes: parse_env_usize(ENV_FLUSH_BYTES, DEFAULT_FLUSH_BYTES),
        channel_capacity: CHANNEL_CAPACITY,
    }
}

/// Parse a `u64` from `key`, falling back to `default` on absence/blank/garbage.
fn parse_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

/// Parse a `usize` from `key`, falling back to `default` on absence/blank/garbage.
fn parse_env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

/// The bundle uploader: a [`LogSink`] + the current-thread runtime it is driven on +
/// the session's stable object key. The collector thread is a plain OS thread (not a
/// tokio worker), so a dedicated current-thread runtime lets it `block_on` the async
/// `put` without ever touching the engine's async runtime.
struct Uploader {
    sink: Box<dyn LogSink>,
    runtime: tokio::runtime::Runtime,
    key: String,
}

impl Uploader {
    fn new(sink: Box<dyn LogSink>, runtime: tokio::runtime::Runtime, key: String) -> Self {
        Self { sink, runtime, key }
    }

    /// Upload `gz` under the session's stable key, blocking the collector thread for
    /// the PUT (bounded by the storage client's request timeout).
    fn upload(&self, gz: Bytes) -> Result<(), SinkError> {
        self.runtime.block_on(self.sink.put(&self.key, gz))
    }
}

/// Build the uploader from the mounted write-only SA creds, or `None` when the SA is
/// not configured / a runtime cannot be built — the fail-closed path: the collector
/// still captures + redacts to disk, it just uploads nothing.
fn build_uploader(creds: &CredsLayout, config: &CollectorConfig) -> Option<Uploader> {
    let Some(sink) = ChronoStorageSink::from_creds(creds) else {
        tracing::warn!("log-stream: write-only storage SA not mounted; capturing without upload");
        return None;
    };
    let runtime = build_upload_runtime()?;
    Some(Uploader::new(
        Box::new(sink),
        runtime,
        bundle_key(&config.session_id),
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

/// The collector thread body: build the uploader, then run the capture loop.
fn run_collector(config: CollectorConfig, rx: Receiver<CollectorRecord>) {
    let creds = CredsLayout::new(&config.creds_dir);
    let uploader = build_uploader(&creds, &config);
    collect(config, rx, uploader);
}

/// The capture loop: seed the redactor + the tree root, then drain the channel +
/// tail files until the channel disconnects, flushing on the size/time cadence with
/// a final flush on exit. Every fallible step is best-effort. Separated from
/// [`run_collector`] so a test can inject a fake uploader.
fn collect(config: CollectorConfig, rx: Receiver<CollectorRecord>, uploader: Option<Uploader>) {
    let creds = CredsLayout::new(&config.creds_dir);
    let seeds = seed_secrets(&creds);
    let seed_refs: Vec<(&str, &str)> = seeds
        .iter()
        .map(|(l, v)| (l.as_str(), v.as_str()))
        .collect();
    let mut redactor = Redactor::new(&seed_refs);
    let mut current_token = read_github_token(&creds.github_token());
    tracing::info!(
        instance = %config.instance_id,
        seeded_secrets = seeds.len(),
        uploads_enabled = uploader.is_some(),
        "log-stream: collector started"
    );

    // Seed the tree root with the redacted-notice README + the per-instance meta so
    // even a session with no captured output still uploads a self-describing bundle.
    let meta_json = InstanceMeta::new(
        config.instance_id.clone(),
        config.start_time,
        config.pod_uid.clone(),
        config.engine_ref.clone(),
        config.config_hash.clone(),
        config.trigger_issue,
        config.repo.clone(),
    )
    .to_json();
    let readme = readme_markdown(&config.repo, config.trigger_issue);
    seed_tree_root(&config.tree_dir, &readme, &meta_json, &redactor);

    let mut tree = TreeWriter::new(config.tree_dir.clone());
    let anchors = TreeAnchors::new(&config.runtime_root, &config.codex_home);
    let mut tails: HashMap<PathBuf, (LogClass, TailTracker)> = HashMap::new();

    let flush_interval = Duration::from_secs(config.flush_secs.max(1));
    let mut uploaded_once = false;
    let mut last_flush = Instant::now();
    let mut last_tick = Instant::now();
    let mut last_token = Instant::now();

    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok((class, line)) => append_line(&mut tree, &redactor, class, &line),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let now = Instant::now();
        if now.duration_since(last_tick) >= POLL_INTERVAL {
            tail_sources(&anchors, &mut tails, &redactor, &mut tree);
            last_tick = now;
        }
        if now.duration_since(last_token) >= Duration::from_secs(TOKEN_REREAD_SECS) {
            reread_token(&creds, &mut current_token, &mut redactor);
            last_token = now;
        }
        if now.duration_since(last_flush) >= flush_interval
            || tree.pending_bytes() >= config.flush_bytes
        {
            do_flush(&mut tree, uploader.as_ref(), &redactor, &mut uploaded_once);
            last_flush = now;
        }
    }

    // Final drain: last tail read, flush the unterminated tails, then a final upload.
    tail_sources(&anchors, &mut tails, &redactor, &mut tree);
    finish_tails(&mut tails, &redactor, &mut tree);
    do_flush(&mut tree, uploader.as_ref(), &redactor, &mut uploaded_once);
    tracing::info!(instance = %config.instance_id, "log-stream: collector stopped");
}

/// Redact a captured line and append it to its tree-class buffer. The single choke
/// point guaranteeing no record reaches disk un-redacted.
pub(crate) fn append_line(tree: &mut TreeWriter, redactor: &Redactor, class: LogClass, line: &str) {
    let redacted = redactor.redact_line(line);
    tree.append(class, &redacted);
}

/// Poll every discovered source file, appending its new (redacted) lines. A source
/// keeps its tail cursor across polls so only fresh content is read.
fn tail_sources(
    anchors: &TreeAnchors,
    tails: &mut HashMap<PathBuf, (LogClass, TailTracker)>,
    redactor: &Redactor,
    tree: &mut TreeWriter,
) {
    for (path, class) in discover_sources(anchors) {
        let entry = tails
            .entry(path.clone())
            .or_insert_with(|| (class, TailTracker::new()));
        for line in entry.1.poll(&path) {
            append_line(tree, redactor, entry.0, &line);
        }
    }
}

/// Emit each tail's final unterminated line at shutdown so a last partial line is
/// not lost.
fn finish_tails(
    tails: &mut HashMap<PathBuf, (LogClass, TailTracker)>,
    redactor: &Redactor,
    tree: &mut TreeWriter,
) {
    for (class, tracker) in tails.values_mut() {
        if let Some(line) = tracker.finish() {
            append_line(tree, redactor, *class, &line);
        }
    }
}

/// Re-read the mounted GitHub token; on a rotation, teach the redactor the NEW value
/// (the old one is retained — [`Redactor::add_secret`] never drops a prior secret).
fn reread_token(creds: &CredsLayout, current: &mut Option<String>, redactor: &mut Redactor) {
    if let Some(token) = read_github_token(&creds.github_token()) {
        if current.as_deref() != Some(token.as_str()) {
            redactor.add_secret(LABEL_GITHUB_TOKEN, &token);
            *current = Some(token);
            tracing::info!("log-stream: picked up a rotated github token");
        }
    }
}

/// Seed the tree root: create it, then drop the non-secret `README.md` + `meta.json`
/// so a first flush uploads a self-describing baseline. Best-effort.
fn seed_tree_root(root: &Path, readme: &str, meta_json: &str, redactor: &Redactor) {
    if let Err(error) = std::fs::create_dir_all(root) {
        log_redacted(
            redactor,
            &format!("log-stream: create tree dir failed: {error}"),
        );
        return;
    }
    if let Err(error) = std::fs::write(root.join("README.md"), readme) {
        log_redacted(
            redactor,
            &format!("log-stream: write README failed: {error}"),
        );
    }
    if let Err(error) = std::fs::write(root.join("meta.json"), meta_json) {
        log_redacted(
            redactor,
            &format!("log-stream: write meta.json failed: {error}"),
        );
    }
}

/// One flush cycle: persist buffered redacted lines to disk (bounding memory), then
/// — when new content landed and an uploader exists — fold the whole tree into a
/// `tar.gz` and upload it, overwriting the session's `latest.tar.gz`. Every step is
/// best-effort; a failure is logged (redacted) and the next flush retries. An
/// unchanged tree is not re-uploaded (the baseline uploads exactly once).
fn do_flush(
    tree: &mut TreeWriter,
    uploader: Option<&Uploader>,
    redactor: &Redactor,
    uploaded_once: &mut bool,
) {
    let had_pending = tree.pending_bytes() > 0;
    if let Err(error) = tree.flush_pending() {
        log_redacted(
            redactor,
            &format!("log-stream: writing redacted files failed: {error}"),
        );
        return;
    }
    let Some(uploader) = uploader else {
        return;
    };
    if !had_pending && *uploaded_once {
        return;
    }
    let gz = match tar_gz_dir(tree.root()) {
        Ok(gz) => gz,
        Err(error) => {
            log_redacted(
                redactor,
                &format!("log-stream: bundle assembly failed: {error}"),
            );
            return;
        }
    };
    match uploader.upload(Bytes::from(gz)) {
        Ok(()) => *uploaded_once = true,
        Err(error) => log_redacted(
            redactor,
            &format!("log-stream: bundle upload failed: {error}"),
        ),
    }
}

/// Log a collector diagnostic AFTER redacting it — an error string may echo a
/// filesystem path or a wrapped detail, so it passes through the redactor before it
/// is ever emitted.
fn log_redacted(redactor: &Redactor, message: &str) {
    tracing::warn!(detail = %redactor.redact_line(message));
}

/// Buffers redacted lines per tree-class and appends them to the on-disk tree files.
/// One class == one file under the tree root; flushing appends (never rewrites), so
/// a growing log accretes across flushes.
pub(crate) struct TreeWriter {
    root: PathBuf,
    buffers: HashMap<LogClass, String>,
    pending_bytes: usize,
}

impl TreeWriter {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            buffers: HashMap::new(),
            pending_bytes: 0,
        }
    }

    /// The tree root the bundle is assembled from.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Append one already-redacted line (a newline is added) to its class buffer.
    pub(crate) fn append(&mut self, class: LogClass, redacted_line: &str) {
        let buffer = self.buffers.entry(class).or_default();
        buffer.push_str(redacted_line);
        buffer.push('\n');
        self.pending_bytes += redacted_line.len() + 1;
    }

    /// Bytes buffered since the last flush (drives the size-based flush trigger).
    pub(crate) fn pending_bytes(&self) -> usize {
        self.pending_bytes
    }

    /// Append every non-empty class buffer to its tree file (creating parent dirs),
    /// then clear the buffers. A per-class write error propagates but leaves the
    /// unwritten buffer intact for the next attempt.
    pub(crate) fn flush_pending(&mut self) -> std::io::Result<()> {
        for (class, buffer) in self.buffers.iter_mut() {
            if buffer.is_empty() {
                continue;
            }
            let path = self.root.join(class.relative_path());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            file.write_all(buffer.as_bytes())?;
            buffer.clear();
        }
        self.pending_bytes = 0;
        Ok(())
    }
}

#[cfg(test)]
#[path = "collector_tests.rs"]
mod tests;
