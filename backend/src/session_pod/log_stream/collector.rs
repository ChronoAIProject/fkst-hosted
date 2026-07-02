//! The IN-POD log collector: capture → redact → commit+push, wired together.
//!
//! Spawned from the `run-substrate` driver ONLY when `FKST_LOG_STREAMING=1`, the
//! collector runs on its OWN OS thread (not a tokio task) so its synchronous git
//! subprocesses never block the async runtime that drives the engine. Records arrive
//! two ways: the tee forwards the supervise child + driver lines over a bounded
//! channel, and a timer tails the framework-child + codex log files incrementally.
//! EVERY record is passed through the fail-closed [`Redactor`] before it touches
//! disk, then buffered per tree-class and flushed (write → git add/commit/push) on a
//! size/time cadence plus a final flush at shutdown.
//!
//! Fail-safe by construction: the whole thing is best-effort. A redaction, file, or
//! git error is logged (redacted) and swallowed — it can NEVER crash the session or
//! block `supervise`; the engine keeps running even if streaming fails outright.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use k8s_openapi::chrono::{DateTime, Utc};

use crate::session_spec::creds::CredsLayout;

use super::super::creds_helper::GitConfigEntry;
use super::classify::{discover_sources, LogClass, TreeAnchors};
use super::gitbranch::{LogBranch, RealGitRunner};
use super::instance::{compute_instance_id, readme_markdown, InstanceMeta};
use super::redact::Redactor;
use super::seed::{read_github_token, seed_secrets, LABEL_GITHUB_TOKEN};
use super::tail::TailTracker;
use super::{
    log_branch_for_issue, DEFAULT_FLUSH_BYTES, DEFAULT_FLUSH_SECS, ENV_CONFIG_HASH,
    ENV_FLUSH_BYTES, ENV_FLUSH_SECS, ENV_LOG_BRANCH, ENV_LOG_STREAMING, ENV_POD_NAME, ENV_POD_UID,
    ENV_TRIGGER_ISSUE, LOG_STREAMING_ENABLED,
};

/// One captured, not-yet-redacted log line + the tree class it routes to. The tee
/// tags supervise/driver lines; tailed files are tagged by their source dir.
pub type CollectorRecord = (LogClass, String);

/// How often the collector wakes to tail files + check flush cadence.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// How often the mounted GitHub token is re-read to pick up a control-plane rotation.
const TOKEN_REREAD_SECS: u64 = 30;
/// Upper bound on how long shutdown waits for the final flush (a stuck push must not
/// wedge pod termination).
const SHUTDOWN_SECS: u64 = 30;

/// Everything the collector thread needs. All fields are owned + `Send` so the whole
/// struct moves onto the collector thread. Non-secret except the token FILE PATH
/// (the value is read on the thread), so it is safe to hold.
pub struct CollectorConfig {
    pub instance_id: String,
    pub branch: String,
    pub trigger_issue: i64,
    /// `owner/name` of the target repo (also the log branch's repo).
    pub repo: String,
    pub engine_ref: String,
    pub config_hash: String,
    pub pod_uid: String,
    pub start_time: DateTime<Utc>,
    pub runtime_root: PathBuf,
    pub codex_home: PathBuf,
    pub creds_dir: PathBuf,
    pub git_entries: Vec<GitConfigEntry>,
    pub token_file: PathBuf,
    /// The isolated git worktree the log branch is checked out into.
    pub worktree_dir: PathBuf,
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
    /// and exits; the join is time-boxed so a stuck git push cannot wedge shutdown.
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

/// Assemble a [`CollectorConfig`] from the pod-injected `FKST_LOG_*` env plus the
/// driver-derived inputs, returning `None` when log streaming is not enabled. Keeps
/// the env plumbing out of the driver's I/O shell. The `worktree_dir` is an isolated
/// `logstream/` under the runtime root so the log repo never collides with the
/// workspace/project clones.
#[allow(clippy::too_many_arguments)]
pub fn collector_config_from_env(
    repo: String,
    engine_ref: String,
    runtime_root: PathBuf,
    codex_home: PathBuf,
    creds_dir: PathBuf,
    git_entries: Vec<GitConfigEntry>,
    token_file: PathBuf,
) -> Option<CollectorConfig> {
    let enabled = std::env::var(ENV_LOG_STREAMING).ok();
    if enabled.as_deref().map(str::trim) != Some(LOG_STREAMING_ENABLED) {
        return None;
    }
    let trigger_issue = std::env::var(ENV_TRIGGER_ISSUE)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let branch = std::env::var(ENV_LOG_BRANCH)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| log_branch_for_issue(trigger_issue));
    let pod_uid = std::env::var(ENV_POD_UID).unwrap_or_default();
    let pod_name = std::env::var(ENV_POD_NAME).unwrap_or_default();
    let start_time = Utc::now();
    let instance_id = compute_instance_id(start_time, &pod_uid, &pod_name);
    let worktree_dir = runtime_root.join("logstream");

    Some(CollectorConfig {
        instance_id,
        branch,
        trigger_issue,
        repo,
        engine_ref,
        config_hash: std::env::var(ENV_CONFIG_HASH).unwrap_or_default(),
        pod_uid,
        start_time,
        runtime_root,
        codex_home,
        creds_dir,
        git_entries,
        token_file,
        worktree_dir,
        flush_secs: parse_env_u64(ENV_FLUSH_SECS, DEFAULT_FLUSH_SECS),
        flush_bytes: parse_env_usize(ENV_FLUSH_BYTES, DEFAULT_FLUSH_BYTES),
        channel_capacity: CHANNEL_CAPACITY,
    })
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

/// The collector thread body. Seeds the redactor, bootstraps the branch, then loops
/// draining the channel + tailing files until the channel disconnects, with a final
/// flush on exit. Every fallible step is best-effort.
fn run_collector(config: CollectorConfig, rx: Receiver<CollectorRecord>) {
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
        "log-stream: collector started"
    );

    let remote_url = format!("https://github.com/{}.git", config.repo);
    let readme = readme_markdown(&config.repo, config.trigger_issue, &config.branch);
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

    let runner = RealGitRunner::new(
        config.worktree_dir.clone(),
        &config.git_entries,
        &config.token_file,
    );
    let mut branch = LogBranch::new(
        runner,
        config.worktree_dir.clone(),
        config.branch.clone(),
        config.instance_id.clone(),
    );
    if let Err(error) = branch.bootstrap(&remote_url, &readme, &meta_json) {
        log_redacted(
            &redactor,
            &format!("log-stream: branch bootstrap failed: {error}"),
        );
    }

    let mut tree = TreeWriter::new(branch.instance_dir());
    let anchors = TreeAnchors::new(&config.runtime_root, &config.codex_home);
    let mut tails: HashMap<PathBuf, (LogClass, TailTracker)> = HashMap::new();

    let flush_interval = Duration::from_secs(config.flush_secs.max(1));
    let mut seq = 0u64;
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
            do_flush(
                &mut branch,
                &mut tree,
                &mut seq,
                &redactor,
                &remote_url,
                &readme,
                &meta_json,
            );
            last_flush = now;
        }
    }

    // Final drain: last tail read, flush the unterminated tails, then a final push.
    tail_sources(&anchors, &mut tails, &redactor, &mut tree);
    finish_tails(&mut tails, &redactor, &mut tree);
    do_flush(
        &mut branch,
        &mut tree,
        &mut seq,
        &redactor,
        &remote_url,
        &readme,
        &meta_json,
    );
    tracing::info!(instance = %config.instance_id, flushes = seq, "log-stream: collector stopped");
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

/// One flush cycle: persist buffered redacted lines to the local instance dir
/// (bounding memory), lazily (re)bootstrap the branch, then commit + push. Every
/// step is best-effort — a failure is logged (redacted) and the next flush retries.
#[allow(clippy::too_many_arguments)]
fn do_flush(
    branch: &mut LogBranch<RealGitRunner>,
    tree: &mut TreeWriter,
    seq: &mut u64,
    redactor: &Redactor,
    remote_url: &str,
    readme: &str,
    meta_json: &str,
) {
    // 1. Always land the buffered lines on disk first so memory stays bounded even if
    //    git is unavailable; the files are picked up by a later push.
    if let Err(error) = tree.flush_pending() {
        log_redacted(
            redactor,
            &format!("log-stream: writing redacted files failed: {error}"),
        );
        return;
    }
    // 2. Ensure the branch is ready (retry a failed bootstrap).
    if !branch.is_bootstrapped() {
        if let Err(error) = branch.bootstrap(remote_url, readme, meta_json) {
            log_redacted(
                redactor,
                &format!("log-stream: bootstrap retry failed: {error}"),
            );
            return;
        }
    }
    // 3. Commit + push this instance's dir.
    match branch.flush(*seq) {
        Ok(true) => *seq += 1,
        Ok(false) => {}
        Err(error) => log_redacted(redactor, &format!("log-stream: git flush failed: {error}")),
    }
}

/// Log a collector diagnostic AFTER redacting it — an error string may echo a git
/// stderr line, so it must pass through the redactor before it is ever emitted.
fn log_redacted(redactor: &Redactor, message: &str) {
    tracing::warn!(detail = %redactor.redact_line(message));
}

/// Buffers redacted lines per tree-class and appends them to the on-branch files.
/// One class == one file under the instance dir; flushing appends (never rewrites),
/// so a growing log accretes across flushes.
pub(crate) struct TreeWriter {
    instance_dir: PathBuf,
    buffers: HashMap<LogClass, String>,
    pending_bytes: usize,
}

impl TreeWriter {
    pub(crate) fn new(instance_dir: PathBuf) -> Self {
        Self {
            instance_dir,
            buffers: HashMap::new(),
            pending_bytes: 0,
        }
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
            let path = self.instance_dir.join(class.relative_path());
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
