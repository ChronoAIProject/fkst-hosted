//! Session progress journaling: capture engine progress signals and surface
//! them to GitHub (the per-logical-run progress record file) so a redo on
//! another pod can skip already-completed work.
//!
//! Single source of truth (#139): the committed GitHub progress-record file is
//! the SOLE machine-truth for a logical run. The two Mongo journaling
//! collections were removed; the run-head pointers (`issue_number`,
//! `last_comment_id`, `last_commit_sha`, `completed_count`) moved into that
//! file, and the in-RAM `completed[]` skip-set replaces the unique partial
//! index that used to dedupe across sessions.
//!
//! Key derivation (the heart of the redo contract): every raised event gets a
//! stable, content-derived `idem_key`, and every logical run a content-derived
//! `run_key`, both identical whether produced by the original session or a
//! redo on a different pod. Correctness never depends on timestamps or on the
//! engine's LOCAL `once()` marks / `with_lock` / codex-permits.
//!
//! Write discipline (CANON):
//! - The in-RAM buffer is the ONLY pre-flush store; a crash before flush
//!   re-executes that work on redo (engine git-idempotency). Within a session,
//!   the in-RAM skip-set (seeded by `load_skip_set` + the controller's
//!   single-writer-per-run guarantee, #135) dedupes completions.
//! - GitHub is the cross-pod source of truth, synced by a debounced
//!   [`Journaler::flush`] running a fenced CAS-merge loop. A failed flush
//!   retains the buffer and never crashes a session.
//! - Fencing is strict-`<`: a writer whose token is below the highest token
//!   that ever reached GitHub for this run is a zombie and never writes;
//!   equality is the rightful holder.

pub mod activity;
pub mod comments;
pub mod config;
pub mod flush;
pub mod github;
pub mod github_http;
pub mod keys;
pub mod merge;
pub mod model;
pub mod parse;
pub mod signals;

#[cfg(test)]
pub(crate) mod test_support;

use crate::journal::github::ProgressRepo;
use crate::journal::merge::{identity_projection, now_rfc3339};
use crate::journal::model::{CompletedEntry, LifecycleEntry};

// Re-exports: external import paths (`crate::journal::{...}` used by
// `sessions/service.rs`) stay unchanged after the split into sibling modules,
// and the [`Journaler`] impl blocks keep referencing these items by bare name.
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

/// Per-session journaler: owns the GitHub sync for one logical run. The
/// committed progress-record file is the sole machine-truth (#139), so the
/// journaler is store-free.
pub struct Journaler {
    pub(crate) github: Option<ProgressRepo>,
    pub(crate) cfg: JournalConfig,
    pub(crate) ctx: SessionCtx,
    pub(crate) run_key: String,
    pub(crate) journal_path: String,
    /// Per-session monotonic counter over BOTH signal kinds.
    pub(crate) seq: i64,
    /// In-memory redo skip-set: the `idem_key`s already completed for this run.
    /// Seeded by [`Journaler::load_skip_set`] from the committed `completed[]`
    /// and grown as this session buffers new completions.
    pub(crate) skip_set: std::collections::HashSet<String>,
    /// Newly-observed completions awaiting a GitHub flush.
    pub(crate) completed_buffer: Vec<CompletedEntry>,
    /// Lifecycle entries awaiting a GitHub flush.
    pub(crate) lifecycle_buffer: Vec<LifecycleEntry>,
    pub(crate) last_flush: tokio::time::Instant,
    /// Highest fencing token known to have reached GitHub for this run.
    pub(crate) known_max_token: i64,
    /// Set on auth failure / schema refusal / fencing: GitHub flushing is
    /// off for the remainder of the session.
    pub(crate) github_disabled: bool,
    /// Rate-limit backoff: no GitHub calls before this instant.
    pub(crate) backoff_until: Option<tokio::time::Instant>,
    pub(crate) first_signal_at: Option<String>,
    pub(crate) last_signal_at: Option<String>,
    /// Issue this run mirrors its activity comment onto; hydrated from the
    /// committed file at bootstrap and folded into the next `merge_record`.
    pub(crate) issue_number: Option<i64>,
    /// Rolling activity comment id; hydrated from the committed file at
    /// bootstrap, updated after each successful activity-comment upsert, and
    /// folded into the next `merge_record` so a cold worker recovers it.
    pub(crate) last_comment_id: Option<i64>,
    /// Counter: malformed RAISED payloads observed.
    pub malformed_raised_total: u64,
    /// Counter: oversized RAISED lines observed.
    pub oversize_raised_total: u64,
}

impl Journaler {
    /// Resolve `run_key` and build the GitHub client (or log why it is
    /// disabled). The run head now lives only in the committed file (read on
    /// [`Journaler::load_skip_set`]); there is no datastore bootstrap. The
    /// caller stamps `run_key` onto the sessions doc via the sessions repo.
    pub async fn start(ctx: SessionCtx, cfg: JournalConfig) -> Result<Self, JournalError> {
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
                "github journaling disabled by config; no durable journal floor"
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
                        "github coordinates incomplete; no durable journal floor"
                    );
                    None
                }
            }
        };

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
            github,
            cfg,
            ctx,
            run_key,
            journal_path,
            seq: 0,
            skip_set: std::collections::HashSet::new(),
            completed_buffer: Vec::new(),
            lifecycle_buffer: Vec::new(),
            last_flush: tokio::time::Instant::now(),
            known_max_token: 0,
            github_disabled: false,
            backoff_until: None,
            first_signal_at: None,
            last_signal_at: None,
            issue_number: None,
            last_comment_id: None,
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

    /// Record one signal: dedupe completions against the in-RAM skip-set and
    /// buffer new completions / lifecycle entries for the next GitHub flush.
    /// Idempotent on a duplicate `idem_key` (a benign no-op).
    pub async fn record(&mut self, signal: ProgressSignal) -> Result<(), JournalError> {
        let at = self.touch_signal_time();
        let seq = self.next_seq();
        match signal {
            ProgressSignal::Raised { event_json } => {
                // Identity is derived from the ORIGINAL decoded JSON.
                let idem = idem_key(
                    &self.ctx.package_name,
                    &event_json,
                    &self.cfg.identity_pointers,
                );
                // why: the Mongo unique partial index is replaced by the
                // controller single-writer-per-run guarantee (#135) plus this
                // in-file completed[] skip-set. A completion is a benign no-op
                // when the skip-set already knows it OR it is already buffered
                // for this flush (the same event re-emitted within a session).
                let already_buffered = self
                    .completed_buffer
                    .iter()
                    .any(|entry| entry.idem_key == idem);
                if self.skip_set.contains(&idem) || already_buffered {
                    tracing::debug!(
                        session_id = %self.ctx.session_id,
                        run_key = %self.run_key,
                        idem_key = %idem,
                        "raised event already journaled (duplicate no-op)"
                    );
                    return Ok(());
                }
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
                    idem_key: idem.clone(),
                    event: identity_projection(&event_json, &self.cfg.identity_pointers),
                    at,
                });
                self.skip_set.insert(idem);
            }
            ProgressSignal::Lifecycle(event) => {
                tracing::debug!(
                    session_id = %self.ctx.session_id,
                    package_name = %self.ctx.package_name,
                    run_key = %self.run_key,
                    pod_id = ?self.ctx.pod_id,
                    fencing_token = self.ctx.fencing_token,
                    transition = event.transition.name(),
                    seq,
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

    /// Terminal flush: record the terminal lifecycle and force-flush. The
    /// activity comment (when enabled) rides the flush itself (#139).
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::journal::test_support::{ctx, github_cfg, mongo_only_cfg, raised};

    // ---- journaler: record ---------------------------------------------------------

    #[tokio::test]
    async fn record_dedupes_within_session() {
        let mut journaler = Journaler::start(ctx(1), mongo_only_cfg())
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("first");
        journaler
            .record(raised("d", "e1"))
            .await
            .expect("duplicate is a benign no-op");
        journaler.record(raised("d", "e2")).await.expect("second");

        assert_eq!(journaler.buffered(), 2, "only NEW completions buffered");
    }

    #[tokio::test]
    async fn record_assigns_a_monotonic_seq_across_both_kinds() {
        let mut journaler = Journaler::start(ctx(1), mongo_only_cfg())
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
        // The seq counter advances over both kinds (3 signals => next is 3).
        assert_eq!(journaler.seq, 3);
    }

    #[tokio::test]
    async fn lifecycle_record_buffers_the_transition() {
        let mut journaler = Journaler::start(ctx(1), mongo_only_cfg())
            .await
            .expect("start");
        journaler
            .record(ProgressSignal::Lifecycle(LifecycleEvent::now(
                Transition::Spawned { pid: 4242 },
            )))
            .await
            .expect("spawned");
        assert_eq!(journaler.lifecycle_buffer.len(), 1);
        assert_eq!(journaler.lifecycle_buffer[0].transition, "spawned");
        assert_eq!(journaler.buffered(), 0, "lifecycle is not a completion");
    }

    #[tokio::test]
    async fn start_rejects_an_invalid_package_name() {
        let bad = SessionCtx {
            package_name: "../escape".to_string(),
            ..ctx(1)
        };
        let err = match Journaler::start(bad, mongo_only_cfg()).await {
            Err(err) => err,
            Ok(_) => panic!("invalid package name must be rejected"),
        };
        assert!(matches!(err, JournalError::Other(_)));
    }

    // ---- journaler: finish ------------------------------------------------------------------

    #[tokio::test]
    async fn finish_records_the_terminal_lifecycle_and_force_flushes() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

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

        let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()))
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");
        journaler
            .finish(LifecycleEvent::now(Transition::Stopped {
                exit_code: Some(0),
            }))
            .await
            .expect("finish");
        // The terminal lifecycle + the completion were both committed (buffers
        // drained on the successful flush).
        assert_eq!(journaler.buffered(), 0);
    }
}
