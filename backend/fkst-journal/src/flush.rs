//! The flushing half of the [`Journaler`]: the debounced fenced CAS-merge to
//! the committed GitHub progress-record file, the redo skip-set bootstrap, the
//! shared GitHub-error disposition, and the rolling activity comment.
//!
//! Split out of `mod.rs` (#139) to keep every `journal/*.rs` under 500 lines.
//! This is a second inherent `impl Journaler` block (same crate, same struct),
//! so it sees the struct's crate-visible fields.

use std::time::Duration;

use crate::activity::render_activity;
use crate::github::{FileSha, RemoteRecord};
use crate::merge::{merge_record, now_rfc3339, HeadPointers};
use crate::model::{ProgressRecord, WriterEntry};
use crate::{FlushOutcome, JournalError, Journaler, SkipSet};

impl Journaler {
    /// Redo bootstrap: GET the committed progress record and build the
    /// [`SkipSet`] from its `completed[]`. The run-head pointers
    /// (`issue_number`/`last_comment_id`) are hydrated from the file so a cold
    /// worker recovers them with no datastore.
    ///
    /// Eventual consistency: a 404 right after a fresh redo may mean the
    /// just-committed file is not yet visible, so the read is retried up to
    /// `cfg.bootstrap_read_retries` with a small jittered backoff BEFORE
    /// concluding "fresh run". Corrupt / auth / rate-limit are NOT retried.
    ///
    /// Fail-open: an absent file (after retries) or unreachable GitHub yields
    /// an EMPTY set (safe re-execution; engine departments are git-idempotent)
    /// plus a `warn`.
    pub async fn load_skip_set(&mut self) -> Result<SkipSet, JournalError> {
        let Some(github) = self.github.as_ref() else {
            tracing::warn!(
                session_id = %self.ctx.session_id,
                run_key = %self.run_key,
                "skip-set bootstrap skipped: github journaling disabled"
            );
            return Ok(SkipSet::default());
        };

        let max_attempts = self.cfg.bootstrap_read_retries.max(1);
        let mut attempt: u32 = 0;
        loop {
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
                    // Corrupt/auth/rate-limit/transient: do NOT retry the 404
                    // eventual-consistency loop; fail open immediately.
                    return Ok(SkipSet::default());
                }
            };

            match remote {
                None => {
                    attempt += 1;
                    if attempt < max_attempts {
                        // why: a fresh redo may race the just-committed file's
                        // visibility on GitHub; retry before declaring a fresh
                        // run. Jitter is derived from the run_key bytes since
                        // Date.now/rand are unavailable in this layer.
                        let backoff = self.bootstrap_backoff(attempt);
                        tracing::debug!(
                            session_id = %self.ctx.session_id,
                            run_key = %self.run_key,
                            attempt,
                            max_attempts,
                            backoff_ms = backoff.as_millis() as u64,
                            "no remote progress record yet; retrying bootstrap read"
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    tracing::info!(
                        session_id = %self.ctx.session_id,
                        run_key = %self.run_key,
                        attempts = attempt,
                        "no remote progress record after retries; fresh logical run (empty skip-set)"
                    );
                    return Ok(SkipSet::default());
                }
                Some(RemoteRecord::Corrupt { .. }) => {
                    tracing::error!(
                        session_id = %self.ctx.session_id,
                        run_key = %self.run_key,
                        "remote progress record corrupt; EMPTY skip-set, never overwriting blindly"
                    );
                    return Ok(SkipSet::default());
                }
                Some(RemoteRecord::NewerSchema { schema, .. }) => {
                    tracing::warn!(
                        session_id = %self.ctx.session_id,
                        run_key = %self.run_key,
                        schema = %schema,
                        "remote progress record schema unsupported; refusing to write github"
                    );
                    self.github_disabled = true;
                    return Ok(SkipSet::default());
                }
                Some(RemoteRecord::Valid { record, .. }) => {
                    let set: SkipSet = record
                        .completed
                        .iter()
                        .map(|entry| entry.idem_key.clone())
                        .collect();
                    self.skip_set = record
                        .completed
                        .iter()
                        .map(|entry| entry.idem_key.clone())
                        .collect();
                    self.known_max_token = self.known_max_token.max(record.max_fencing_token);
                    // Hydrate the run-head pointers from the committed truth.
                    if record.issue_number.is_some() {
                        self.issue_number = record.issue_number;
                    }
                    if record.last_comment_id.is_some() {
                        self.last_comment_id = record.last_comment_id;
                    }
                    tracing::info!(
                        session_id = %self.ctx.session_id,
                        package_name = %self.ctx.package_name,
                        run_key = %self.run_key,
                        pod_id = ?self.ctx.pod_id,
                        fencing_token = self.ctx.fencing_token,
                        skip_set_size = set.len(),
                        "skip-set loaded from github truth"
                    );
                    return Ok(set);
                }
            }
        }
    }

    /// Deterministic small bootstrap backoff: `200ms * attempt` plus a jitter
    /// derived from the run_key bytes (no wall clock / RNG in this layer).
    fn bootstrap_backoff(&self, attempt: u32) -> Duration {
        let bytes = self.run_key.as_bytes();
        let jitter = if bytes.is_empty() {
            0
        } else {
            u64::from(bytes[(attempt as usize) % bytes.len()]) % 200
        };
        Duration::from_millis(200u64.saturating_mul(u64::from(attempt)) + jitter)
    }

    /// Debounced (or forced) flush of buffered completions + lifecycle to the
    /// committed GitHub file via the fenced CAS-merge loop. A failure RETAINS
    /// the buffer for the next tick; auth failures disable GitHub for the
    /// session; fencing returns `fenced: true` with no write.
    ///
    /// The in-RAM buffer is the ONLY pre-flush store; a crash before flush
    /// re-executes that work on redo (engine git-idempotency).
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
            // No durable floor now: there is nowhere to persist. Drop the
            // buffers (the work is re-derivable on redo) and warn loudly.
            let dropped = self.completed_buffer.len();
            self.completed_buffer.clear();
            self.lifecycle_buffer.clear();
            self.last_flush = tokio::time::Instant::now();
            tracing::warn!(
                session_id = %self.ctx.session_id,
                package_name = %self.ctx.package_name,
                run_key = %self.run_key,
                dropped,
                "github journaling unavailable; completions dropped from the durable record \
                 (re-derivable on redo via engine git-idempotency)"
            );
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
                Some(RemoteRecord::Valid { record, sha }) => (Some(*record), Some(sha)),
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
                HeadPointers {
                    issue_number: self.issue_number,
                    last_comment_id: self.last_comment_id,
                },
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
                    // why: the machine-truth file is committed FIRST; the
                    // activity comment rides after. A new last_comment_id is
                    // folded into the NEXT flush (idempotent upsert — worst
                    // case one extra create the next flush reconciles).
                    self.upsert_activity_comment(&merged).await;
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

    /// Rolling activity comment on the committed-flush cadence (#139). Runs AT
    /// MOST ONCE per committed flush and NEVER from `record`. Failure is
    /// swallowed (the machine-truth file is already committed).
    async fn upsert_activity_comment(&mut self, merged: &ProgressRecord) {
        if !self.cfg.github_enabled || !self.cfg.activity_comment_enabled {
            return;
        }
        let Some(github) = self.github.as_ref() else {
            return;
        };
        let Some(issue) = merged.issue_number else {
            tracing::debug!(
                run_key = %self.run_key,
                "no issue_number; skipping activity comment"
            );
            return;
        };
        let body = render_activity(merged);
        match github
            .upsert_issue_comment(
                issue as u64,
                self.last_comment_id.map(|id| id as u64),
                &body,
            )
            .await
        {
            Ok(new_id) => {
                // Store the id so the NEXT merge_record folds it into the file
                // (a cold worker then recovers it).
                self.last_comment_id = Some(new_id as i64);
                tracing::debug!(
                    run_key = %self.run_key,
                    issue,
                    comment_id = new_id,
                    "activity comment upserted"
                );
            }
            Err(error) => {
                tracing::warn!(
                    run_key = %self.run_key,
                    issue,
                    error = %error,
                    "activity comment upsert failed; flush succeeds anyway (file committed)"
                );
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
}

// The journaler tests are split across sibling files (declared via `#[path]`)
// so every `journal/*.rs` stays under 500 lines (#139). All three share the
// `crate::test_support` fixtures and see the crate-internal
// `Journaler` fields.
#[cfg(test)]
#[path = "flush_mech_tests.rs"]
mod flush_mech_tests;

#[cfg(test)]
#[path = "flush_bootstrap_tests.rs"]
mod flush_bootstrap_tests;
