//! Session progress journaling: capture engine progress signals, record them
//! durably (MongoDB `session_progress`), and surface them to GitHub (the
//! per-logical-run progress record file) so a redo on another pod can skip
//! already-completed work.
//!
//! Key derivation (the heart of the redo contract): every raised event gets a
//! stable, content-derived `idem_key`, and every logical run a content-derived
//! `run_key`, both identical whether produced by the original session or a
//! redo on a different pod. Correctness never depends on timestamps or on the
//! engine's LOCAL `once()` marks / `with_lock` / codex-permits.
//!
//! Write discipline (CANON):
//! - MongoDB is the always-on floor: every signal is inserted immediately in
//!   [`Journaler::record`]; the unique partial index dedupes across sessions.
//! - GitHub is the cross-pod source of truth, synced by a debounced
//!   [`Journaler::flush`] running a fenced CAS-merge loop. A failed flush
//!   retains the buffer (already durable in Mongo) and never crashes a
//!   session.
//! - Fencing is strict-`<`: a writer whose token is below the highest token
//!   that ever reached GitHub for this run is a zombie and never writes;
//!   equality is the rightful holder.

pub mod comments;
pub mod config;
pub mod github;
pub mod github_http;
pub mod index;
pub mod keys;
pub mod merge;
pub mod model;
pub mod parse;
pub mod signals;
pub mod store;

use std::time::Duration;

use crate::journal::github::{FileSha, ProgressRepo, RemoteRecord};
use crate::journal::merge::{identity_projection, now_rfc3339};
use crate::journal::model::{
    sanitize_event_json, CompletedEntry, LifecycleEntry, ProgressKind, RunJournalDoc,
    RunJournalGithub, SessionProgressDoc, WriterEntry, UNVERIFIED_SHA,
};
use crate::journal::store::{InsertOutcome, ProgressStore};

// Re-exports: external import paths (`crate::journal::{...}` used by
// `sessions/service.rs`) stay unchanged after the split into sibling modules,
// and the [`Journaler`] impl below keeps referencing these items by bare name.
pub use config::{default_identity_pointers, FlushOutcome, JournalConfig, SessionCtx};
pub use keys::{idem_key, package_fingerprint, run_key};
pub use merge::merge_record;
pub use signals::{LifecycleEvent, ProgressSignal, SkipSet, Transition};

/// Journaling failures. Secret hygiene is load-bearing: no variant ever
/// carries the GitHub token (asserted by tests in [`github`]); HTTP errors
/// are reduced to status/context strings before they enter the chain.
#[derive(thiserror::Error, Debug)]
pub enum JournalError {
    /// The optimistic-concurrency loop exhausted its retry budget.
    #[error("github contents conflict after {0} retries")]
    CasExhausted(u32),
    /// One PUT lost the CAS race (409 / sha mismatch / concurrent create):
    /// the caller's CAS loop re-reads and retries.
    #[error("github contents sha conflict")]
    CasConflict,
    /// 404 on the update path (remote file deleted mid-run): the caller
    /// falls back to create.
    #[error("remote journal file missing on update")]
    RemoteMissing,
    /// Stale writer fenced off (strict `<`; equality is the rightful holder).
    #[error("stale fencing token {got} < {known}")]
    Fenced { got: i64, known: i64 },
    /// 401, or 403 without rate-limit headers: bad/expired token.
    #[error("github auth failed")]
    GithubAuth,
    /// 403 carrying rate-limit headers; the value is seconds until reset.
    #[error("github rate limited; reset in {0}s")]
    GithubRateLimited(u64),
    /// Remote progress record declares a schema we must not overwrite.
    #[error("remote progress record schema unsupported: {0}")]
    UnsupportedSchema(String),
    #[error(transparent)]
    Mongo(#[from] mongodb::error::Error),
    /// Network / 5xx / unexpected-status failures, reduced to a string that
    /// never contains credentials.
    #[error("github http error: {0}")]
    Http(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Journaler
// ---------------------------------------------------------------------------

/// Per-session journaler: owns the Mongo + GitHub sync for one logical run.
pub struct Journaler<S: ProgressStore> {
    store: S,
    github: Option<ProgressRepo>,
    cfg: JournalConfig,
    ctx: SessionCtx,
    run_key: String,
    journal_path: String,
    /// Per-session monotonic counter over BOTH signal kinds.
    seq: i64,
    /// Newly-inserted completions awaiting a GitHub flush.
    completed_buffer: Vec<CompletedEntry>,
    /// Lifecycle entries awaiting a GitHub flush.
    lifecycle_buffer: Vec<LifecycleEntry>,
    last_flush: tokio::time::Instant,
    /// Highest fencing token known to have reached GitHub for this run.
    known_max_token: i64,
    /// Set on auth failure / schema refusal / fencing: GitHub flushing is
    /// off for the remainder of the session (Mongo continues).
    github_disabled: bool,
    /// Rate-limit backoff: no GitHub calls before this instant.
    backoff_until: Option<tokio::time::Instant>,
    first_signal_at: Option<String>,
    last_signal_at: Option<String>,
    /// Counter: malformed RAISED payloads observed.
    pub malformed_raised_total: u64,
    /// Counter: oversized RAISED lines observed.
    pub oversize_raised_total: u64,
}

impl<S: ProgressStore> Journaler<S> {
    /// Resolve `run_key`, build the GitHub client (or log why it is
    /// disabled), and upsert the `run_journals` head. The caller stamps
    /// `run_key` onto the sessions doc via the sessions repo (this module
    /// never writes the `sessions` collection).
    pub async fn start(
        ctx: SessionCtx,
        cfg: JournalConfig,
        store: S,
    ) -> Result<Self, JournalError> {
        if !crate::engine::is_valid_name(&ctx.package_name) {
            return Err(JournalError::Other(anyhow::anyhow!(
                "invalid package name for journaling: must fully match [A-Za-z0-9_-]+"
            )));
        }
        let run_key = run_key(&ctx.package_name, &ctx.package_fingerprint);
        let journal_path = format!(".fkst-hosted/journal/{run_key}.json");

        let github = if !cfg.github_enabled {
            tracing::info!(
                run_key = %run_key,
                package_name = %ctx.package_name,
                "github journaling disabled by config; mongo-only"
            );
            None
        } else {
            match (&cfg.github_repo, &cfg.github_token) {
                (Some(repo), Some(token)) => Some(ProgressRepo::new(
                    &cfg.github_api_base,
                    repo,
                    &cfg.github_branch,
                    Some(token.clone()),
                )?),
                (repo, token) => {
                    tracing::warn!(
                        run_key = %run_key,
                        package_name = %ctx.package_name,
                        has_repo = repo.is_some(),
                        has_token = token.is_some(),
                        "github coordinates incomplete; journaling is mongo-only"
                    );
                    None
                }
            }
        };

        // Preserve an existing head (its max token / sha / comment ids are
        // cross-session state); create a fresh "unverified" head otherwise.
        let existing = store.get_run_journal(&run_key).await?;
        let head = match existing {
            Some(mut head) => {
                head.package_name = ctx.package_name.clone();
                head.updated_at = bson::DateTime::now();
                if let Some(repo) = github.as_ref() {
                    head.github.repo = Some(repo.repo().to_string());
                    head.github.branch = repo.branch().to_string();
                    head.github.journal_path = journal_path.clone();
                }
                head
            }
            None => RunJournalDoc {
                run_key: run_key.clone(),
                package_name: ctx.package_name.clone(),
                completed_idem_keys_count: 0,
                github: RunJournalGithub {
                    repo: github.as_ref().map(|repo| repo.repo().to_string()),
                    branch: cfg.github_branch.clone(),
                    journal_path: journal_path.clone(),
                    last_commit_sha: Some(UNVERIFIED_SHA.to_string()),
                    issue_number: None,
                    last_comment_id: None,
                },
                max_fencing_token: 0,
                updated_at: bson::DateTime::now(),
            },
        };
        let known_max_token = head.max_fencing_token;
        store.upsert_run_journal(&head).await?;

        tracing::info!(
            session_id = %ctx.session_id,
            package_name = %ctx.package_name,
            run_key = %run_key,
            pod_id = ?ctx.pod_id,
            fencing_token = ctx.fencing_token,
            github = github.is_some(),
            "journaler started"
        );

        Ok(Self {
            store,
            github,
            cfg,
            ctx,
            run_key,
            journal_path,
            seq: 0,
            completed_buffer: Vec::new(),
            lifecycle_buffer: Vec::new(),
            last_flush: tokio::time::Instant::now(),
            known_max_token,
            github_disabled: false,
            backoff_until: None,
            first_signal_at: None,
            last_signal_at: None,
            malformed_raised_total: 0,
            oversize_raised_total: 0,
        })
    }

    /// The logical-run key (the GitHub journal file basename).
    pub fn run_key(&self) -> &str {
        &self.run_key
    }

    /// The configured journaling config (parsing knobs for the caller).
    pub fn config(&self) -> &JournalConfig {
        &self.cfg
    }

    /// Completions buffered for the next GitHub flush.
    pub fn buffered(&self) -> usize {
        self.completed_buffer.len()
    }

    fn next_seq(&mut self) -> i64 {
        let seq = self.seq;
        self.seq += 1;
        seq
    }

    fn touch_signal_time(&mut self) -> String {
        let now = now_rfc3339();
        if self.first_signal_at.is_none() {
            self.first_signal_at = Some(now.clone());
        }
        self.last_signal_at = Some(now.clone());
        now
    }

    fn base_doc(&mut self, kind: ProgressKind) -> SessionProgressDoc {
        SessionProgressDoc {
            id: bson::Uuid::new().to_string(),
            session_id: self.ctx.session_id.clone(),
            package_name: self.ctx.package_name.clone(),
            run_key: self.run_key.clone(),
            kind,
            seq: self.next_seq(),
            idem_key: None,
            event_json: None,
            event_json_raw: None,
            event_json_unstorable: None,
            lifecycle: None,
            pod_id: self.ctx.pod_id.clone(),
            fencing_token: self.ctx.fencing_token,
            recorded_at: bson::DateTime::now(),
        }
    }

    /// Record one signal: insert into `session_progress` (deduped by the
    /// unique index) and buffer new completions / lifecycle entries for the
    /// next GitHub flush. Idempotent on duplicate `idem_key`. Mongo failures
    /// propagate ([`JournalError::Mongo`]); the caller decides session
    /// disposition (journaling never owns `sessions.status`).
    pub async fn record(&mut self, signal: ProgressSignal) -> Result<(), JournalError> {
        let at = self.touch_signal_time();
        match signal {
            ProgressSignal::Raised { event_json } => {
                // Identity is derived from the ORIGINAL decoded JSON, before
                // any BSON sanitization.
                let idem = idem_key(
                    &self.ctx.package_name,
                    &event_json,
                    &self.cfg.identity_pointers,
                );
                let sanitized = sanitize_event_json(&event_json);
                let mut doc = self.base_doc(ProgressKind::Raised);
                doc.idem_key = Some(idem.clone());
                doc.event_json = sanitized.event_json;
                doc.event_json_raw = sanitized.event_json_raw;
                doc.event_json_unstorable = sanitized.unstorable.then_some(true);
                let seq = doc.seq;
                match self.store.insert_progress(&doc).await? {
                    InsertOutcome::Inserted => {
                        tracing::debug!(
                            session_id = %self.ctx.session_id,
                            package_name = %self.ctx.package_name,
                            run_key = %self.run_key,
                            pod_id = ?self.ctx.pod_id,
                            fencing_token = self.ctx.fencing_token,
                            idem_key = %idem,
                            seq,
                            "raised event journaled"
                        );
                        self.completed_buffer.push(CompletedEntry {
                            idem_key: idem,
                            event: identity_projection(&event_json, &self.cfg.identity_pointers),
                            at,
                        });
                    }
                    InsertOutcome::Duplicate => {
                        tracing::debug!(
                            session_id = %self.ctx.session_id,
                            run_key = %self.run_key,
                            idem_key = %idem,
                            "raised event already journaled (duplicate no-op)"
                        );
                    }
                }
            }
            ProgressSignal::Lifecycle(event) => {
                let mut doc = self.base_doc(ProgressKind::Lifecycle);
                doc.lifecycle = Some(event.transition.to_doc());
                doc.recorded_at = event.at;
                self.store.insert_progress(&doc).await?;
                tracing::debug!(
                    session_id = %self.ctx.session_id,
                    package_name = %self.ctx.package_name,
                    run_key = %self.run_key,
                    pod_id = ?self.ctx.pod_id,
                    fencing_token = self.ctx.fencing_token,
                    transition = event.transition.name(),
                    seq = doc.seq,
                    "lifecycle journaled"
                );
                self.lifecycle_buffer.push(LifecycleEntry {
                    transition: event.transition.name().to_string(),
                    session_id: self.ctx.session_id.clone(),
                    pod_id: self.ctx.pod_id.clone(),
                    fencing_token: self.ctx.fencing_token,
                    at: event
                        .at
                        .try_to_rfc3339_string()
                        .unwrap_or_else(|_| at.clone()),
                });
            }
        }
        Ok(())
    }

    /// Read-modify-write the `run_journals` head.
    async fn update_head(
        &self,
        apply: impl FnOnce(&mut RunJournalDoc),
    ) -> Result<(), JournalError> {
        let mut head = match self.store.get_run_journal(&self.run_key).await? {
            Some(head) => head,
            None => RunJournalDoc {
                run_key: self.run_key.clone(),
                package_name: self.ctx.package_name.clone(),
                completed_idem_keys_count: 0,
                github: RunJournalGithub {
                    repo: self.github.as_ref().map(|repo| repo.repo().to_string()),
                    branch: self.cfg.github_branch.clone(),
                    journal_path: self.journal_path.clone(),
                    last_commit_sha: Some(UNVERIFIED_SHA.to_string()),
                    issue_number: None,
                    last_comment_id: None,
                },
                max_fencing_token: 0,
                updated_at: bson::DateTime::now(),
            },
        };
        apply(&mut head);
        head.updated_at = bson::DateTime::now();
        self.store.upsert_run_journal(&head).await
    }

    /// Redo bootstrap: GET the GitHub progress record, build the
    /// [`SkipSet`], and mirror `completed[]` into local `session_progress`
    /// (idempotent inserts; E11000 = benign) so local truth matches GitHub
    /// truth BEFORE any package work.
    ///
    /// Fail-open: an absent file or unreachable GitHub yields an EMPTY set
    /// (safe re-execution; engine departments are git-idempotent) plus a
    /// `warn` and the `"unverified"` sha sentinel.
    pub async fn load_skip_set(&mut self) -> Result<SkipSet, JournalError> {
        let Some(github) = self.github.as_ref() else {
            tracing::warn!(
                session_id = %self.ctx.session_id,
                run_key = %self.run_key,
                "skip-set bootstrap skipped: github journaling disabled"
            );
            return Ok(SkipSet::default());
        };

        let remote = match github.get_record(&self.journal_path).await {
            Ok(remote) => remote,
            Err(error) => {
                tracing::warn!(
                    session_id = %self.ctx.session_id,
                    package_name = %self.ctx.package_name,
                    run_key = %self.run_key,
                    pod_id = ?self.ctx.pod_id,
                    fencing_token = self.ctx.fencing_token,
                    error = %error,
                    "github unreachable at bootstrap; proceeding with an EMPTY skip-set"
                );
                if matches!(error, JournalError::GithubAuth) {
                    self.github_disabled = true;
                }
                self.update_head(|head| {
                    head.github.last_commit_sha = Some(UNVERIFIED_SHA.to_string());
                })
                .await?;
                return Ok(SkipSet::default());
            }
        };

        match remote {
            None => {
                tracing::info!(
                    session_id = %self.ctx.session_id,
                    run_key = %self.run_key,
                    "no remote progress record; fresh logical run (empty skip-set)"
                );
                Ok(SkipSet::default())
            }
            Some(RemoteRecord::Corrupt { .. }) => {
                tracing::error!(
                    session_id = %self.ctx.session_id,
                    run_key = %self.run_key,
                    "remote progress record corrupt; EMPTY skip-set, never overwriting blindly"
                );
                Ok(SkipSet::default())
            }
            Some(RemoteRecord::NewerSchema { schema, .. }) => {
                tracing::warn!(
                    session_id = %self.ctx.session_id,
                    run_key = %self.run_key,
                    schema = %schema,
                    "remote progress record schema unsupported; refusing to write github"
                );
                self.github_disabled = true;
                Ok(SkipSet::default())
            }
            Some(RemoteRecord::Valid { record, sha }) => {
                let set: SkipSet = record
                    .completed
                    .iter()
                    .map(|entry| entry.idem_key.clone())
                    .collect();
                // Mirror remote truth into local Mongo (idempotent).
                for entry in &record.completed {
                    let mut doc = self.base_doc(ProgressKind::Raised);
                    doc.idem_key = Some(entry.idem_key.clone());
                    doc.event_json = bson::to_bson(&entry.event).ok();
                    self.store.insert_progress(&doc).await?;
                }
                let remote_token = record.max_fencing_token;
                self.known_max_token = self.known_max_token.max(remote_token);
                let count = record.completed.len() as i64;
                let sha_string = sha.0.clone();
                self.update_head(move |head| {
                    head.github.last_commit_sha = Some(sha_string);
                    head.completed_idem_keys_count = head.completed_idem_keys_count.max(count);
                    head.max_fencing_token = head.max_fencing_token.max(remote_token);
                })
                .await?;
                tracing::info!(
                    session_id = %self.ctx.session_id,
                    package_name = %self.ctx.package_name,
                    run_key = %self.run_key,
                    pod_id = ?self.ctx.pod_id,
                    fencing_token = self.ctx.fencing_token,
                    skip_set_size = set.len(),
                    "skip-set loaded from github truth"
                );
                Ok(set)
            }
        }
    }

    /// Debounced (or forced) flush of buffered completions + lifecycle to
    /// GitHub via the fenced CAS-merge loop. A failure RETAINS the buffer
    /// (already durable in Mongo) for the next tick; auth failures disable
    /// GitHub for the session; fencing returns `fenced: true` with no write.
    pub async fn flush(&mut self, force: bool) -> Result<FlushOutcome, JournalError> {
        let pending = self.completed_buffer.len() + self.lifecycle_buffer.len();
        if pending == 0 {
            return Ok(FlushOutcome::skipped());
        }
        if !force
            && self.completed_buffer.len() < self.cfg.flush_max_batch
            && self.last_flush.elapsed() < self.cfg.flush_interval
        {
            tracing::debug!(
                run_key = %self.run_key,
                buffered = self.completed_buffer.len(),
                "flush deferred (debounce)"
            );
            return Ok(FlushOutcome::skipped());
        }
        if self.github.is_none() || self.github_disabled {
            // Mongo-only mode: keep the head count honest and drop the
            // GitHub buffers (every entry is already durable in Mongo).
            let drained = self.completed_buffer.len() as i64;
            self.completed_buffer.clear();
            self.lifecycle_buffer.clear();
            self.last_flush = tokio::time::Instant::now();
            self.update_head(|head| head.completed_idem_keys_count += drained)
                .await?;
            return Ok(FlushOutcome::skipped());
        }
        if let Some(until) = self.backoff_until {
            if tokio::time::Instant::now() < until {
                tracing::debug!(run_key = %self.run_key, "flush deferred (rate-limit backoff)");
                return Ok(FlushOutcome::skipped());
            }
            self.backoff_until = None;
        }

        let mut attempts: u32 = 0;
        loop {
            if attempts >= self.cfg.cas_max_retries {
                tracing::error!(
                    session_id = %self.ctx.session_id,
                    package_name = %self.ctx.package_name,
                    run_key = %self.run_key,
                    pod_id = ?self.ctx.pod_id,
                    fencing_token = self.ctx.fencing_token,
                    retries = attempts,
                    "github CAS retries exhausted; buffer retained for the next tick"
                );
                return Err(JournalError::CasExhausted(attempts));
            }

            let get_result = match self.github.as_ref() {
                Some(github) => github.get_record(&self.journal_path).await,
                None => return Ok(FlushOutcome::skipped()),
            };
            let remote = match get_result {
                Ok(remote) => remote,
                Err(error) => match self.handle_github_error(error, &mut attempts).await? {
                    Some(outcome) => return Ok(outcome),
                    None => continue,
                },
            };

            let (base, prev_sha) = match remote {
                None => (None, None),
                Some(RemoteRecord::Valid { record, sha }) => (Some(record), Some(sha)),
                Some(RemoteRecord::Corrupt { .. }) => {
                    tracing::error!(
                        run_key = %self.run_key,
                        "remote record corrupt; refusing to overwrite (buffer retained)"
                    );
                    return Ok(FlushOutcome::skipped());
                }
                Some(RemoteRecord::NewerSchema { schema, .. }) => {
                    tracing::warn!(
                        run_key = %self.run_key,
                        schema = %schema,
                        "remote schema unsupported; github flushing disabled"
                    );
                    self.github_disabled = true;
                    return Err(JournalError::UnsupportedSchema(schema));
                }
            };

            // Fencing (CANON: the engine has ZERO cross-host fencing — this
            // is it). Strict `<` is the tie-break: equality is the rightful
            // current holder, never a zombie.
            let remote_token = base.as_ref().map(|r| r.max_fencing_token).unwrap_or(0);
            let known = self.known_max_token.max(remote_token);
            let got = self.ctx.fencing_token;
            if got < known {
                tracing::warn!(
                    session_id = %self.ctx.session_id,
                    package_name = %self.ctx.package_name,
                    run_key = %self.run_key,
                    pod_id = ?self.ctx.pod_id,
                    got,
                    known,
                    "stale writer fenced off; no github write"
                );
                self.github_disabled = true;
                return Ok(FlushOutcome {
                    committed: 0,
                    commit_sha: None,
                    fenced: true,
                });
            }

            let writer = WriterEntry {
                session_id: self.ctx.session_id.clone(),
                pod_id: self.ctx.pod_id.clone(),
                fencing_token: got,
                first_at: self.first_signal_at.clone().unwrap_or_else(now_rfc3339),
                last_at: self.last_signal_at.clone().unwrap_or_else(now_rfc3339),
            };
            let merged = merge_record(
                base.as_ref(),
                &self.run_key,
                &self.ctx.package_name,
                &self.ctx.package_fingerprint,
                &self.completed_buffer,
                &self.lifecycle_buffer,
                Some(&writer),
                known.max(got),
                now_rfc3339(),
            );

            let message = format!(
                "chore(fkst-hosted): journal progress for {} ({} completed)",
                self.ctx.package_name,
                merged.completed.len()
            );
            let put_result = match self.github.as_ref() {
                Some(github) => {
                    github
                        .put_record(&self.journal_path, &merged, prev_sha.as_ref(), &message)
                        .await
                }
                None => return Ok(FlushOutcome::skipped()),
            };
            match put_result {
                Ok(FileSha(sha)) => {
                    let committed = self.completed_buffer.len();
                    self.completed_buffer.clear();
                    self.lifecycle_buffer.clear();
                    self.last_flush = tokio::time::Instant::now();
                    self.known_max_token = merged.max_fencing_token;
                    let count = merged.completed.len() as i64;
                    let token = merged.max_fencing_token;
                    let sha_for_head = sha.clone();
                    self.update_head(move |head| {
                        head.github.last_commit_sha = Some(sha_for_head);
                        head.completed_idem_keys_count = count;
                        head.max_fencing_token = head.max_fencing_token.max(token);
                    })
                    .await?;
                    tracing::info!(
                        session_id = %self.ctx.session_id,
                        package_name = %self.ctx.package_name,
                        run_key = %self.run_key,
                        pod_id = ?self.ctx.pod_id,
                        fencing_token = got,
                        committed,
                        commit_sha = %sha,
                        "github journal flush succeeded"
                    );
                    return Ok(FlushOutcome {
                        committed,
                        commit_sha: Some(sha),
                        fenced: false,
                    });
                }
                Err(JournalError::CasConflict) | Err(JournalError::RemoteMissing) => {
                    // Expected concurrent-writer path (or deleted-mid-run:
                    // the re-read sees 404 and the next PUT creates).
                    attempts += 1;
                    tracing::debug!(
                        run_key = %self.run_key,
                        attempts,
                        "github CAS conflict; re-reading and retrying"
                    );
                    continue;
                }
                Err(error) => match self.handle_github_error(error, &mut attempts).await? {
                    Some(outcome) => return Ok(outcome),
                    None => continue,
                },
            }
        }
    }

    /// Shared disposition for auth / rate-limit / transient GitHub errors.
    /// `Err(_)` propagates fatal errors; `Ok(None)` means "retry the loop";
    /// `Ok(Some(outcome))` is unreachable today but keeps the shape uniform.
    async fn handle_github_error(
        &mut self,
        error: JournalError,
        attempts: &mut u32,
    ) -> Result<Option<FlushOutcome>, JournalError> {
        match error {
            JournalError::GithubAuth => {
                tracing::error!(
                    session_id = %self.ctx.session_id,
                    package_name = %self.ctx.package_name,
                    run_key = %self.run_key,
                    pod_id = ?self.ctx.pod_id,
                    fencing_token = self.ctx.fencing_token,
                    "github auth failed; disabling github flushing for this session"
                );
                self.github_disabled = true;
                Err(JournalError::GithubAuth)
            }
            JournalError::GithubRateLimited(reset_secs) => {
                tracing::warn!(
                    run_key = %self.run_key,
                    reset_secs,
                    "github rate limited; backing off (flushing stays enabled)"
                );
                self.backoff_until = Some(
                    tokio::time::Instant::now() + Duration::from_secs(reset_secs.clamp(1, 3600)),
                );
                Err(JournalError::GithubRateLimited(reset_secs))
            }
            transient => {
                *attempts += 1;
                let backoff = Duration::from_millis(
                    50u64.saturating_mul(1 << (*attempts).min(5)) + u64::from(*attempts % 7) * 13,
                );
                tracing::warn!(
                    run_key = %self.run_key,
                    attempts = *attempts,
                    error = %transient,
                    backoff_ms = backoff.as_millis() as u64,
                    "transient github error; backing off and retrying"
                );
                if *attempts >= self.cfg.cas_max_retries {
                    return Err(transient);
                }
                tokio::time::sleep(backoff).await;
                Ok(None)
            }
        }
    }

    /// Terminal flush: record the terminal lifecycle, force-flush, and
    /// (dormant; gated on config + a known issue number) mirror the summary
    /// into a GitHub issue comment.
    pub async fn finish(&mut self, terminal: LifecycleEvent) -> Result<(), JournalError> {
        let terminal_name = terminal.transition.name();
        self.record(ProgressSignal::Lifecycle(terminal)).await?;
        let outcome = self.flush(true).await?;
        tracing::info!(
            session_id = %self.ctx.session_id,
            package_name = %self.ctx.package_name,
            run_key = %self.run_key,
            pod_id = ?self.ctx.pod_id,
            fencing_token = self.ctx.fencing_token,
            terminal = terminal_name,
            committed = outcome.committed,
            "journaler finished"
        );

        if self.cfg.issue_comments && !self.github_disabled {
            if let Some(github) = self.github.as_ref() {
                let head = self.store.get_run_journal(&self.run_key).await?;
                if let Some(head) = head {
                    if let Some(issue) = head.github.issue_number {
                        let body = format!(
                            "fkst-hosted progress for `{}` (run `{}`): {} completed event(s); \
                             terminal state `{terminal_name}`.",
                            self.ctx.package_name, self.run_key, head.completed_idem_keys_count,
                        );
                        let comment_id = head.github.last_comment_id.map(|id| id as u64);
                        let new_id = github
                            .upsert_issue_comment(issue as u64, comment_id, &body)
                            .await?;
                        self.update_head(|h| h.github.last_comment_id = Some(new_id as i64))
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use secrecy::SecretString;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::journal::model::ProgressRecord;

    fn pointers() -> Vec<String> {
        default_identity_pointers()
    }

    // ---- in-memory store -----------------------------------------------------

    #[derive(Default)]
    struct MemInner {
        progress: Vec<SessionProgressDoc>,
        journals: HashMap<String, RunJournalDoc>,
    }

    /// In-memory [`ProgressStore`] mirroring the unique-partial-index
    /// semantics of `sp_run_idem_uniq`.
    #[derive(Clone, Default)]
    struct MemStore {
        inner: Arc<Mutex<MemInner>>,
    }

    impl MemStore {
        fn progress_len(&self) -> usize {
            self.inner.lock().unwrap().progress.len()
        }

        fn head(&self, run_key: &str) -> Option<RunJournalDoc> {
            self.inner.lock().unwrap().journals.get(run_key).cloned()
        }
    }

    impl ProgressStore for MemStore {
        async fn insert_progress(
            &self,
            doc: &SessionProgressDoc,
        ) -> Result<InsertOutcome, JournalError> {
            let mut inner = self.inner.lock().unwrap();
            if let Some(key) = &doc.idem_key {
                let duplicate = inner
                    .progress
                    .iter()
                    .any(|d| d.run_key == doc.run_key && d.idem_key.as_ref() == Some(key));
                if duplicate {
                    return Ok(InsertOutcome::Duplicate);
                }
            }
            inner.progress.push(doc.clone());
            Ok(InsertOutcome::Inserted)
        }

        async fn get_run_journal(
            &self,
            run_key: &str,
        ) -> Result<Option<RunJournalDoc>, JournalError> {
            Ok(self.inner.lock().unwrap().journals.get(run_key).cloned())
        }

        async fn upsert_run_journal(&self, doc: &RunJournalDoc) -> Result<(), JournalError> {
            self.inner
                .lock()
                .unwrap()
                .journals
                .insert(doc.run_key.clone(), doc.clone());
            Ok(())
        }
    }

    fn ctx(token: i64) -> SessionCtx {
        SessionCtx {
            session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            package_name: "demo".to_string(),
            package_fingerprint: "fp".to_string(),
            pod_id: Some("pod-0".to_string()),
            fencing_token: token,
        }
    }

    fn github_cfg(server_uri: &str) -> JournalConfig {
        JournalConfig {
            github_repo: Some("owner/name".to_string()),
            github_api_base: server_uri.to_string(),
            github_token: Some(SecretString::from("test-token".to_string())),
            cas_max_retries: 3,
            ..JournalConfig::default()
        }
    }

    fn mongo_only_cfg() -> JournalConfig {
        JournalConfig {
            github_enabled: false,
            ..JournalConfig::default()
        }
    }

    fn raised(department: &str, name: &str) -> ProgressSignal {
        ProgressSignal::Raised {
            event_json: json!({
                "department": department, "source": "raiser", "name": name, "corr": "c-1"
            }),
        }
    }

    /// Contents-API GET body for a record.
    fn contents_body(record: &ProgressRecord, sha: &str) -> serde_json::Value {
        json!({
            "content": STANDARD.encode(serde_json::to_vec(record).expect("json")),
            "sha": sha,
            "encoding": "base64"
        })
    }

    fn completed(idem: &str, at: &str) -> CompletedEntry {
        CompletedEntry {
            idem_key: idem.to_string(),
            event: json!({"department": "d"}),
            at: at.to_string(),
        }
    }

    // ---- journaler: record ---------------------------------------------------------

    #[tokio::test]
    async fn record_dedupes_identical_raised_events() {
        let store = MemStore::default();
        let mut journaler = Journaler::start(ctx(1), mongo_only_cfg(), store.clone())
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("first");
        journaler
            .record(raised("d", "e1"))
            .await
            .expect("duplicate is a benign no-op");
        journaler.record(raised("d", "e2")).await.expect("second");

        assert_eq!(store.progress_len(), 2, "duplicate creates no second doc");
        assert_eq!(journaler.buffered(), 2, "only NEW completions buffered");
    }

    #[tokio::test]
    async fn record_assigns_a_monotonic_seq_across_both_kinds() {
        let store = MemStore::default();
        let mut journaler = Journaler::start(ctx(1), mongo_only_cfg(), store.clone())
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("raised");
        journaler
            .record(ProgressSignal::Lifecycle(LifecycleEvent::now(
                Transition::Running,
            )))
            .await
            .expect("lifecycle");
        journaler.record(raised("d", "e2")).await.expect("raised 2");

        let seqs: Vec<i64> = store
            .inner
            .lock()
            .unwrap()
            .progress
            .iter()
            .map(|d| d.seq)
            .collect();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn lifecycle_docs_omit_idem_key_and_carry_the_transition() {
        let store = MemStore::default();
        let mut journaler = Journaler::start(ctx(1), mongo_only_cfg(), store.clone())
            .await
            .expect("start");
        journaler
            .record(ProgressSignal::Lifecycle(LifecycleEvent::now(
                Transition::Spawned { pid: 4242 },
            )))
            .await
            .expect("spawned");
        let docs = store.inner.lock().unwrap().progress.clone();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].kind, ProgressKind::Lifecycle);
        assert!(docs[0].idem_key.is_none());
        let lifecycle = docs[0].lifecycle.as_ref().expect("lifecycle");
        assert_eq!(lifecycle.transition, "spawned");
        assert_eq!(lifecycle.pid, Some(4242));
    }

    #[tokio::test]
    async fn start_rejects_an_invalid_package_name() {
        let bad = SessionCtx {
            package_name: "../escape".to_string(),
            ..ctx(1)
        };
        let err = match Journaler::start(bad, mongo_only_cfg(), MemStore::default()).await {
            Err(err) => err,
            Ok(_) => panic!("invalid package name must be rejected"),
        };
        assert!(matches!(err, JournalError::Other(_)));
    }

    // ---- journaler: flush --------------------------------------------------------------

    #[tokio::test]
    async fn flush_is_debounced_and_force_creates_the_remote_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "sha-1" } })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let store = MemStore::default();
        let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()), store.clone())
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");

        // Below the batch size and inside the interval: deferred.
        let deferred = journaler.flush(false).await.expect("deferred flush");
        assert_eq!(deferred, FlushOutcome::skipped());
        assert_eq!(journaler.buffered(), 1, "buffer retained");

        let outcome = journaler.flush(true).await.expect("forced flush");
        assert_eq!(outcome.committed, 1);
        assert_eq!(outcome.commit_sha.as_deref(), Some("sha-1"));
        assert!(!outcome.fenced);
        assert_eq!(journaler.buffered(), 0);

        let head = store.head(journaler.run_key()).expect("head");
        assert_eq!(head.github.last_commit_sha.as_deref(), Some("sha-1"));
        assert_eq!(head.completed_idem_keys_count, 1);
    }

    #[tokio::test]
    async fn flush_merges_with_the_remote_record_and_sends_the_prior_sha() {
        let server = MockServer::start().await;
        let mut remote = ProgressRecord::new("ignored", "demo", "fp", "t0".to_string());
        remote.completed = vec![completed("remote-key", "2026-06-09T00:00:00Z")];
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&remote, "prev")))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(body_partial_json(json!({ "sha": "prev" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "content": { "sha": "next" } })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let store = MemStore::default();
        let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()), store.clone())
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");
        let outcome = journaler.flush(true).await.expect("flush");
        assert_eq!(outcome.commit_sha.as_deref(), Some("next"));
        // Head count reflects the union (remote + ours).
        assert_eq!(
            store
                .head(journaler.run_key())
                .expect("head")
                .completed_idem_keys_count,
            2
        );
    }

    #[tokio::test]
    async fn flush_retries_on_cas_conflict_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        // First PUT loses the race; the re-read + second PUT wins.
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(409))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "sha-2" } })),
            )
            .with_priority(5)
            .mount(&server)
            .await;

        let mut journaler =
            Journaler::start(ctx(1), github_cfg(&server.uri()), MemStore::default())
                .await
                .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");
        let outcome = journaler.flush(true).await.expect("flush must converge");
        assert_eq!(outcome.commit_sha.as_deref(), Some("sha-2"));
    }

    #[tokio::test]
    async fn flush_exhausts_cas_retries_keeps_the_buffer_and_recovers_next_tick() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(409))
            .mount(&server)
            .await;

        let mut journaler =
            Journaler::start(ctx(1), github_cfg(&server.uri()), MemStore::default())
                .await
                .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");
        let err = journaler.flush(true).await.expect_err("must exhaust");
        assert!(matches!(err, JournalError::CasExhausted(_)), "got {err:?}");
        assert_eq!(
            journaler.buffered(),
            1,
            "buffer retained (durable in mongo)"
        );

        // The conflict clears (e.g. the competing writer finished): the next
        // forced flush commits the retained buffer.
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "sha-3" } })),
            )
            .mount(&server)
            .await;
        let outcome = journaler.flush(true).await.expect("retry tick");
        assert_eq!(outcome.committed, 1);
    }

    #[tokio::test]
    async fn auth_failure_disables_github_while_mongo_journaling_continues() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let store = MemStore::default();
        let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()), store.clone())
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");
        let err = journaler.flush(true).await.expect_err("auth must fail");
        assert!(matches!(err, JournalError::GithubAuth), "got {err:?}");

        // GitHub now disabled: recording + flushing keep working Mongo-only,
        // with NO further GitHub calls (the server would 404 the GET and
        // surface an Http error if one were made).
        server.reset().await;
        journaler.record(raised("d", "e2")).await.expect("record 2");
        let outcome = journaler
            .flush(true)
            .await
            .expect("mongo-only flush must succeed");
        assert_eq!(outcome, FlushOutcome::skipped());
        assert_eq!(store.progress_len(), 2);
        assert_eq!(
            journaler.buffered(),
            0,
            "buffers drained in mongo-only mode"
        );
    }

    // ---- journaler: fencing ----------------------------------------------------------

    #[tokio::test]
    async fn stale_writer_is_fenced_off_and_never_puts() {
        let server = MockServer::start().await;
        let mut remote = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
        remote.max_fencing_token = 5;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&remote, "s1")))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0) // the load-bearing assertion: NO write happens
            .mount(&server)
            .await;

        let store = MemStore::default();
        let mut journaler = Journaler::start(ctx(3), github_cfg(&server.uri()), store.clone())
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");
        let outcome = journaler.flush(true).await.expect("fenced is not an error");
        assert!(outcome.fenced);
        assert_eq!(outcome.committed, 0);
        assert!(outcome.commit_sha.is_none());
        // Local Mongo journaling for the stale writer is still allowed.
        assert_eq!(store.progress_len(), 1);
    }

    #[tokio::test]
    async fn equal_token_proceeds_and_greater_token_bumps_known() {
        for (token, expected_max) in [(5i64, 5i64), (7, 7)] {
            let server = MockServer::start().await;
            let mut remote = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
            remote.max_fencing_token = 5;
            Mock::given(method("GET"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(contents_body(&remote, "s1")),
                )
                .mount(&server)
                .await;
            Mock::given(method("PUT"))
                .and(body_partial_json(json!({ "sha": "s1" })))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({ "content": { "sha": "s2" } })),
                )
                .expect(1)
                .mount(&server)
                .await;

            let store = MemStore::default();
            let mut journaler =
                Journaler::start(ctx(token), github_cfg(&server.uri()), store.clone())
                    .await
                    .expect("start");
            journaler.record(raised("d", "e1")).await.expect("record");
            let outcome = journaler.flush(true).await.expect("must proceed");
            assert!(!outcome.fenced, "token {token} must not be fenced");
            assert_eq!(outcome.committed, 1);
            assert_eq!(
                store
                    .head(journaler.run_key())
                    .expect("head")
                    .max_fencing_token,
                expected_max
            );
        }
    }

    // ---- journaler: skip-set bootstrap ---------------------------------------------------

    #[tokio::test]
    async fn load_skip_set_mirrors_remote_truth_and_makes_reemission_a_no_op() {
        let server = MockServer::start().await;
        let event1 = json!({"department":"d","source":"raiser","name":"e1","corr":"c-1"});
        let event2 = json!({"department":"d","source":"raiser","name":"e2","corr":"c-1"});
        let mut remote = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
        remote.completed = vec![
            CompletedEntry {
                idem_key: idem_key("demo", &event1, &pointers()),
                event: event1.clone(),
                at: "t1".to_string(),
            },
            CompletedEntry {
                idem_key: idem_key("demo", &event2, &pointers()),
                event: event2.clone(),
                at: "t2".to_string(),
            },
        ];
        remote.max_fencing_token = 1;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&remote, "s9")))
            .mount(&server)
            .await;

        let store = MemStore::default();
        // The redo writer carries a HIGHER fencing token (a fresh lease).
        let mut journaler = Journaler::start(ctx(2), github_cfg(&server.uri()), store.clone())
            .await
            .expect("start");
        let skip = journaler.load_skip_set().await.expect("bootstrap");
        assert_eq!(skip.len(), 2);
        assert!(skip.contains(&idem_key("demo", &event1, &pointers())));
        assert_eq!(store.progress_len(), 2, "remote truth mirrored locally");
        assert_eq!(
            store
                .head(journaler.run_key())
                .expect("head")
                .github
                .last_commit_sha
                .as_deref(),
            Some("s9")
        );

        // Re-emitting the mirrored events produces ZERO new docs.
        journaler
            .record(raised("d", "e1"))
            .await
            .expect("re-emit 1");
        journaler
            .record(raised("d", "e2"))
            .await
            .expect("re-emit 2");
        assert_eq!(store.progress_len(), 2, "idempotent redo");
        assert_eq!(journaler.buffered(), 0, "nothing newly completed");
    }

    #[tokio::test]
    async fn unreachable_github_at_bootstrap_fails_open_with_unverified_sha() {
        let cfg = JournalConfig {
            github_api_base: "http://127.0.0.1:1".to_string(),
            ..github_cfg("http://127.0.0.1:1")
        };
        let store = MemStore::default();
        let mut journaler = Journaler::start(ctx(1), cfg, store.clone())
            .await
            .expect("start");
        let skip = journaler.load_skip_set().await.expect("fail-open");
        assert!(skip.is_empty());
        assert_eq!(
            store
                .head(journaler.run_key())
                .expect("head")
                .github
                .last_commit_sha
                .as_deref(),
            Some(UNVERIFIED_SHA)
        );
        // The session still proceeds: Mongo journaling works.
        journaler.record(raised("d", "e1")).await.expect("record");
        assert_eq!(store.progress_len(), 1);
    }

    #[tokio::test]
    async fn corrupt_and_newer_schema_remotes_yield_safe_empty_skip_sets() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": STANDARD.encode(b"not json"), "sha": "c1"
            })))
            .mount(&server)
            .await;
        let mut journaler =
            Journaler::start(ctx(1), github_cfg(&server.uri()), MemStore::default())
                .await
                .expect("start");
        assert!(journaler.load_skip_set().await.expect("corrupt").is_empty());

        let server2 = MockServer::start().await;
        let mut newer = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
        newer.schema = "fkst-hosted/progress-record@9".to_string();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&newer, "n1")))
            .mount(&server2)
            .await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server2)
            .await;
        let mut journaler2 =
            Journaler::start(ctx(1), github_cfg(&server2.uri()), MemStore::default())
                .await
                .expect("start");
        assert!(journaler2.load_skip_set().await.expect("newer").is_empty());
        // Forward-compat guard: it must now refuse to write.
        journaler2.record(raised("d", "e1")).await.expect("record");
        let outcome = journaler2.flush(true).await.expect("mongo-only");
        assert_eq!(outcome, FlushOutcome::skipped());
    }

    // ---- journaler: finish ------------------------------------------------------------------

    #[tokio::test]
    async fn finish_records_the_terminal_lifecycle_and_force_flushes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "fin" } })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let store = MemStore::default();
        let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()), store.clone())
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");
        journaler
            .finish(LifecycleEvent::now(Transition::Stopped {
                exit_code: Some(0),
            }))
            .await
            .expect("finish");

        let docs = store.inner.lock().unwrap().progress.clone();
        assert_eq!(docs.len(), 2);
        let terminal = docs[1].lifecycle.as_ref().expect("terminal lifecycle");
        assert_eq!(terminal.transition, "stopped");
        assert_eq!(terminal.exit_code, Some(0));
        assert_eq!(
            store
                .head(journaler.run_key())
                .expect("head")
                .github
                .last_commit_sha
                .as_deref(),
            Some("fin")
        );
    }

    // ---- secret hygiene: tracing capture --------------------------------------------------------

    /// A shared, in-memory sink that captures every byte a tracing subscriber
    /// writes. Hand-rolled so this stays dependency-free (no `tracing-test`).
    #[derive(Clone, Default)]
    struct CaptureBuffer(Arc<Mutex<Vec<u8>>>);

    impl CaptureBuffer {
        fn contents(&self) -> Vec<u8> {
            self.0.lock().expect("capture lock poisoned").clone()
        }
    }

    impl std::io::Write for CaptureBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("capture lock poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl tracing_subscriber::fmt::MakeWriter<'_> for CaptureBuffer {
        type Writer = CaptureBuffer;
        fn make_writer(&self) -> Self::Writer {
            self.clone()
        }
    }

    /// Drive a flush against one GitHub error path with a canary token; the
    /// caller wraps this in a capturing subscriber. `status = None` forces a
    /// raw reqwest network error (a closed port).
    async fn flush_canary(status: Option<u16>, token: &str) {
        let api_base = match status {
            Some(code) => {
                let server = MockServer::start().await;
                Mock::given(method("GET"))
                    .respond_with(ResponseTemplate::new(404))
                    .mount(&server)
                    .await;
                let put = if code == 403 {
                    // 403 WITH rate-limit headers => GithubRateLimited.
                    ResponseTemplate::new(403)
                        .insert_header("x-ratelimit-remaining", "0")
                        .insert_header("retry-after", "30")
                } else {
                    ResponseTemplate::new(code)
                };
                Mock::given(method("PUT"))
                    .respond_with(put)
                    .mount(&server)
                    .await;
                let uri = server.uri();
                // Keep the mock alive across the flush; the test process is
                // short-lived so leaking it is harmless and avoids a borrow.
                std::mem::forget(server);
                uri
            }
            None => "http://127.0.0.1:1".to_string(),
        };
        let cfg = JournalConfig {
            github_repo: Some("owner/name".to_string()),
            github_api_base: api_base,
            github_token: Some(SecretString::from(token.to_string())),
            cas_max_retries: 2,
            ..JournalConfig::default()
        };
        let mut journaler = Journaler::start(ctx(1), cfg, MemStore::default())
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");
        // Every arm is an error/fenced path; we only care that whatever it
        // logs is token-free, so swallow the outcome.
        let _ = journaler.flush(true).await;
    }

    /// Spec §Testing: "tracing capture for a flush never contains the token".
    /// Capture ALL tracing output at TRACE level while flushes traverse every
    /// GitHub error path (auth 401, rate-limit 403, a transient 500 with
    /// retries, and a raw network failure) with a canary token installed, and
    /// assert the captured bytes contain neither the token value nor the
    /// `Bearer`/`Authorization` markers an accidental header-or-token
    /// interpolation would leak. Deterministic: wiremock + a closed port, no
    /// real network. A plain `#[test]` with its own runtime so the subscriber
    /// can be thread-scoped via `with_default`.
    #[test]
    fn tracing_capture_for_a_flush_never_contains_the_token() {
        const CANARY: &str = "ghp_tracing_canary_value";

        let capture = CaptureBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(async {
                flush_canary(Some(401), CANARY).await; // auth
                flush_canary(Some(403), CANARY).await; // rate limit
                flush_canary(Some(500), CANARY).await; // transient 5xx + retries
                flush_canary(None, CANARY).await; // raw network error
            });
        });

        let bytes = capture.contents();
        assert!(
            !bytes.is_empty(),
            "the flush error paths must have logged SOMETHING"
        );
        let text = String::from_utf8_lossy(&bytes);
        for needle in [CANARY, "Bearer", "Authorization", "authorization"] {
            assert!(
                !text.contains(needle),
                "tracing output leaked {needle:?}:\n{text}"
            );
        }
    }
}
