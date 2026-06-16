//! The worker-side credential-refresh servicer (issue #151, increment 5).
//!
//! The worker NEVER mints (it holds no GitHub-App key): every fresh installation
//! token comes from the controller over the `/internal/v1/credential-refresh`
//! RPC. This mirrors the control-plane driver's `service_mint_request` /
//! `refresh_goal_token` / `reactive_refresh_goal_token` trio
//! (`fkst-control-plane/src/sessions/service.rs`), MINUS the App-key mint — here
//! the mint is a fence-stamped RPC. Three triggers feed one [`RefreshState`], all
//! gated by the SAME escalating cooldown (tighten near expiry; never hammer it):
//! JIT ([`RefreshReason::Jit`], the helper's nonce-bearing `<token>.request`
//! file), Periodic ([`RefreshReason::Periodic`], ~55min / ~T-10min pre-expiry),
//! and Reactive ([`RefreshReason::Reactive`], a stdout 401 flag).
//!
//! Fence echoing is load-bearing: every refresh carries the claim's `fencing_id`.
//! A `credentials: None` response means a STALE fence — the worker self-fences
//! (stops driving, never mints again, never writes a token); `gone: true` fails
//! the session loudly. The worker is the SOLE writer of the token file. Secrets
//! are never logged.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;

use fkst_engine::{
    read_nonce_file, verify_mint_nonce, write_token_file, MINT_REQUEST_SUFFIX, TOKEN_FILE_NAME,
};
use fkst_shared::protocol::{CredentialRefreshResponse, RefreshReason};

use crate::agent::WorkerAgent;

/// Proactive refresh interval: replace the token ~55 min after the last mint
/// (installation tokens expire after ~60 min, a 5-min buffer). Mirrors the
/// driver's `TOKEN_REFRESH_INTERVAL`.
const TOKEN_REFRESH_INTERVAL: Duration = Duration::from_secs(55 * 60);

/// Minimum cooldown between refresh attempts (never hammer the controller more
/// than once per minute). Mirrors the driver's `TOKEN_REFRESH_COOLDOWN`.
const TOKEN_REFRESH_COOLDOWN: Duration = Duration::from_secs(60);

/// Tightened cooldown once the on-disk token is within
/// [`TOKEN_EXPIRY_URGENT_MARGIN`] of expiry: a flaky refresh near the deadline is
/// retried every 10s instead of every 60s. Mirrors the driver's
/// `TOKEN_REFRESH_COOLDOWN_URGENT`.
const TOKEN_REFRESH_COOLDOWN_URGENT: Duration = Duration::from_secs(10);

/// Margin before expiry at which the urgent cooldown engages. Also the margin at
/// which the periodic timer pre-mints (mint at ~T-10min). Mirrors the driver's
/// `TOKEN_EXPIRY_URGENT_MARGIN`.
const TOKEN_EXPIRY_URGENT_MARGIN: Duration = Duration::from_secs(600);

/// Consecutive refresh failures, WITH the token already past expiry, that fail
/// the session instead of letting the engine hit a silent 401. Mirrors the
/// driver's `MAX_CONSECUTIVE_MINT_FAILURES`.
const MAX_CONSECUTIVE_REFRESH_FAILURES: u32 = 5;

/// What a refresh attempt resolved to, surfaced to the supervise loop. The loop
/// acts on the terminal variants (self-fence / fatal); the rest keep supervising.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// A fresh token was minted by the controller and written to the token file.
    Refreshed,
    /// Nothing attempted (cooldown not elapsed, not yet armed, or no trigger).
    Skipped,
    /// The attempt failed (transport / transient controller error) but the
    /// current token is still valid — keep supervising; the next tick retries.
    TransientFailure,
    /// The controller refused the refresh (`credentials: None`): this worker is a
    /// STALE fence. The loop stops driving the engine (self-fence), never mints
    /// again.
    StaleFence,
    /// The App installation is gone (`gone: true`) OR the token is past expiry
    /// after persistent failures: the session must fail loudly.
    Fatal { reason: String },
}

/// Confirms a (possibly adopted) session's CURRENT fencing id with the
/// controller before its refresh servicer may mint. A FRESH dispatch arrives
/// already-confirmed (the controller just placed it); an ADOPTED engine inherits
/// a `.mint-nonce` from a DEAD worker and a claim the controller may have
/// moved/restarted, so it MUST re-establish a live fence before it mints — never
/// mint on a stale fence.
#[async_trait]
pub trait FenceConfirmer: Send + Sync {
    /// The live fencing id for `session_id`, or `None` when not (yet) confirmed.
    /// Polled on the mint tick until it answers `Some`; while `None` the refresh
    /// servicer is PARKED (no mint, no token write).
    async fn confirm(&self, session_id: &str) -> Option<i64>;
}

/// A fence that is already confirmed at the supplied id — the FRESH-dispatch
/// case (the controller dispatched with a live `fencing_id`, so the servicer is
/// armed from the first tick).
pub struct AlreadyConfirmed(pub i64);

#[async_trait]
impl FenceConfirmer for AlreadyConfirmed {
    async fn confirm(&self, _session_id: &str) -> Option<i64> {
        Some(self.0)
    }
}

/// A fence that is NEVER confirmed — the DEFAULT seam for an adopted session
/// until the controller re-issues a live fencing id (TODO(#151 activation)).
/// While this is the confirmer, the adopted session's refresh servicer stays
/// parked and the engine never mints on the dead worker's stale fence.
pub struct NeverConfirmed;

#[async_trait]
impl FenceConfirmer for NeverConfirmed {
    async fn confirm(&self, _session_id: &str) -> Option<i64> {
        None
    }
}

/// Per-session refresh state threaded through the supervise loop's mint tick.
/// Mirrors the driver's `GoalDrive` token-lifecycle fields, minus the App-key
/// mint machinery (the worker mints over RPC).
pub struct RefreshState {
    agent: Arc<WorkerAgent>,
    session_id: String,
    repo_ref: String,
    /// `<runtime_dir>/github-token`: the token file the worker is sole writer of.
    token_path: PathBuf,
    /// `<runtime_dir>`: the nonce file (`.mint-nonce`) and the JIT request file
    /// (`<token_path>.request`) live under it.
    runtime_dir: PathBuf,
    /// The fence-confirmer; once it answers `Some`, `fencing_id` is set and the
    /// servicer is armed.
    confirmer: Arc<dyn FenceConfirmer>,
    /// The CONFIRMED live fencing id, set once [`Self::confirmer`] answers. While
    /// `None` the servicer is parked (an adopted session never mints on a stale
    /// fence; a fresh dispatch confirms on the first tick).
    fencing_id: Option<i64>,
    /// When the current on-disk token was minted; drives the periodic interval.
    minted_at: Instant,
    /// When the last refresh attempt ran; drives the escalating cooldown.
    last_attempt: Instant,
    /// Expiry of the token currently on disk; drives the cooldown tightening and
    /// the periodic pre-mint margin.
    token_expires_at: SystemTime,
    /// Consecutive refresh-failure count, reset on every success.
    consecutive_failures: u32,
    /// Set when a GitHub 401 is seen on stdout; drained on the next mint tick.
    reactive_pending: bool,
}

impl RefreshState {
    /// Build the refresh state for a driven session. `runtime_dir` is the
    /// engine's runtime dir (known once it has started / been adopted);
    /// `token_expires_at` is the dispatch's `github_token_expires_at_unix_ms`.
    pub fn new(
        agent: Arc<WorkerAgent>,
        session_id: String,
        repo_ref: String,
        runtime_dir: PathBuf,
        token_expires_at: SystemTime,
        confirmer: Arc<dyn FenceConfirmer>,
    ) -> Self {
        let token_path = runtime_dir.join(TOKEN_FILE_NAME);
        let now = Instant::now();
        Self {
            agent,
            session_id,
            repo_ref,
            token_path,
            runtime_dir,
            confirmer,
            fencing_id: None,
            minted_at: now,
            last_attempt: now,
            token_expires_at,
            consecutive_failures: 0,
            reactive_pending: false,
        }
    }

    /// The confirmed live fencing id once the servicer is armed (for the
    /// supervise loop's status reports). `None` until the fence is confirmed.
    pub fn armed_fencing_id(&self) -> Option<i64> {
        self.fencing_id
    }

    /// Move the cooldown stamps into the past so the next attempt is NOT gated by
    /// the escalating cooldown — the test-only seam the supervise suite uses to
    /// drive a refresh deterministically (mirrors the driver's
    /// `goal_drive_with(last_attempt_ago)`; prod legitimately suppresses an
    /// immediate re-mint right after the startup mint).
    #[cfg(test)]
    pub(crate) fn clear_cooldown_for_test(&mut self) {
        let past = Instant::now() - Duration::from_secs(3600);
        self.last_attempt = past;
        self.minted_at = past;
    }

    /// Flag a reactive refresh: a GitHub 401 was seen on the engine's stdout.
    /// Drained (with the cooldown) on the next mint tick.
    pub fn flag_reactive_401(&mut self) {
        self.reactive_pending = true;
    }

    /// Ensure the servicer is armed: ask the confirmer for the live fencing id.
    /// Returns `true` once armed. Idempotent — a re-confirm only refreshes the
    /// id (the controller is authoritative for the live fence).
    async fn ensure_armed(&mut self) -> bool {
        match self.confirmer.confirm(&self.session_id).await {
            Some(fencing_id) => {
                if self.fencing_id != Some(fencing_id) {
                    tracing::info!(
                        session_id = %self.session_id,
                        "refresh servicer armed (fence confirmed)"
                    );
                }
                self.fencing_id = Some(fencing_id);
                true
            }
            None => false,
        }
    }

    /// One mint-tick of the servicer (the driver's `mint_tick` arm): service any
    /// pending JIT request, then the reactive / periodic triggers. Returns the
    /// MOST SIGNIFICANT outcome of the tick — a terminal variant (StaleFence /
    /// Fatal) short-circuits and stops the supervise loop; otherwise a
    /// `Refreshed` from ANY sub-step is preserved over a later `Skipped` so the
    /// caller sees that a fresh token landed.
    ///
    /// Parked (returns [`RefreshOutcome::Skipped`]) until the fence is confirmed.
    pub async fn service_tick(&mut self) -> RefreshOutcome {
        if !self.ensure_armed().await {
            // Adopted + unconfirmed: never mint on a stale fence.
            return RefreshOutcome::Skipped;
        }

        // JIT first (a waiting git operation is the most latency-sensitive), then
        // the reactive / periodic triggers (each gated by the cooldown inside
        // `refresh`). The first terminal outcome short-circuits; non-terminal
        // outcomes fold into `best` (Refreshed > TransientFailure > Skipped).
        let mut best = RefreshOutcome::Skipped;

        let jit = self.service_jit_request().await;
        if is_terminal(&jit) {
            return jit;
        }
        best = fold_outcome(best, jit);

        if self.reactive_pending {
            self.reactive_pending = false;
            tracing::warn!(
                session_id = %self.session_id,
                "github auth failure flagged; requesting a reactive refresh"
            );
            let reactive = self.refresh(RefreshReason::Reactive).await;
            if is_terminal(&reactive) {
                return reactive;
            }
            best = fold_outcome(best, reactive);
        }

        if self.minted_at.elapsed() >= self.periodic_interval() {
            let periodic = self.refresh(RefreshReason::Periodic).await;
            if is_terminal(&periodic) {
                return periodic;
            }
            best = fold_outcome(best, periodic);
        }
        best
    }

    /// The proactive interval: the standard 55-min cadence, but pulled in so a
    /// refresh fires by ~T-10min if the dispatch's expiry is sooner than that
    /// (mint at ~T-10min from `github_token_expires_at_unix_ms`).
    fn periodic_interval(&self) -> Duration {
        let to_urgent = self
            .token_expires_at
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
            .saturating_sub(TOKEN_EXPIRY_URGENT_MARGIN);
        TOKEN_REFRESH_INTERVAL.min(to_urgent.max(Duration::ZERO))
    }

    /// Service a pending JIT mint request from the engine's credential helper.
    /// The helper drops `<token_path>.request` carrying the per-session nonce;
    /// verify the nonce (shared engine helper), refresh over RPC, then DELETE the
    /// request file (ALWAYS, even on failure, so the helper stops waiting and
    /// falls back to the current token). A non-matching / absent nonce or no
    /// request file is a no-op.
    async fn service_jit_request(&mut self) -> RefreshOutcome {
        let request_path = mint_request_path(&self.token_path);
        let Ok(contents) = tokio::fs::read_to_string(&request_path).await else {
            return RefreshOutcome::Skipped; // no pending request
        };
        // Authenticate against the per-session nonce file (0600). Only this
        // session's own engine child knows the nonce, so a mismatch is a
        // stray/forged file — drop it without minting.
        let expected = match read_nonce_file(&self.runtime_dir) {
            Ok(expected) => expected,
            Err(error) => {
                tracing::warn!(
                    session_id = %self.session_id,
                    error = %error,
                    "mint-request nonce file unreadable; clearing the request"
                );
                let _ = tokio::fs::remove_file(&request_path).await;
                return RefreshOutcome::Skipped;
            }
        };
        if !verify_mint_nonce(expected.as_deref(), &contents) {
            tracing::warn!(
                session_id = %self.session_id,
                "mint-request nonce mismatch; ignoring and clearing the request file"
            );
            let _ = tokio::fs::remove_file(&request_path).await;
            return RefreshOutcome::Skipped;
        }

        let outcome = self.refresh(RefreshReason::Jit).await;
        // Signal completion to the waiting helper by deleting the request file —
        // ALWAYS, even on a transient failure (the helper falls back to the
        // current token rather than blocking the whole patience window).
        let _ = tokio::fs::remove_file(&request_path).await;
        outcome
    }

    /// Shared refresh-and-rewrite core for all three triggers. Respects the
    /// escalating per-attempt cooldown, asks the controller to mint over the
    /// fence-stamped RPC, and atomically rewrites the 0600 token file. The token
    /// value is never logged.
    async fn refresh(&mut self, reason: RefreshReason) -> RefreshOutcome {
        // Escalating cooldown gate (applies to ALL triggers): never hammer the
        // controller more than once per window, tightening near expiry.
        if self.last_attempt.elapsed() < effective_cooldown(self.token_expires_at) {
            return RefreshOutcome::Skipped;
        }
        let Some(fencing_id) = self.fencing_id else {
            return RefreshOutcome::Skipped; // not armed
        };
        self.last_attempt = Instant::now();

        match self
            .agent
            .refresh_credential(&self.session_id, fencing_id, &self.repo_ref, reason)
            .await
        {
            Ok(CredentialRefreshResponse { gone: true, .. }) => RefreshOutcome::Fatal {
                reason: "github app installation removed for the session repo".to_string(),
            },
            Ok(CredentialRefreshResponse {
                credentials: None, ..
            }) => {
                // Stale fence: the controller refused. Self-fence — never mint
                // again; the worker stops driving this session.
                tracing::warn!(
                    session_id = %self.session_id,
                    reason = ?reason,
                    "credential-refresh refused (stale fence); self-fencing"
                );
                RefreshOutcome::StaleFence
            }
            Ok(CredentialRefreshResponse {
                credentials: Some(refreshed),
                ..
            }) => {
                let expires_at = unix_ms_to_system_time(refreshed.expires_at_unix_ms);
                // The worker is the SOLE writer of the token file; the shared
                // engine writer keeps the 0600 + atomic-rename format identical
                // to the startup write. The token is exposed ONLY here.
                match write_token_file(&self.token_path, &refreshed.token, expires_at) {
                    Ok(()) => {
                        self.minted_at = Instant::now();
                        self.token_expires_at = expires_at;
                        self.consecutive_failures = 0;
                        tracing::info!(
                            session_id = %self.session_id,
                            reason = ?reason,
                            "github token refreshed over the controller RPC"
                        );
                        RefreshOutcome::Refreshed
                    }
                    Err(error) => {
                        tracing::warn!(
                            session_id = %self.session_id,
                            error = %error,
                            "token file rewrite failed; retrying next tick"
                        );
                        self.classify_failure()
                    }
                }
            }
            Err(error) => {
                // Transport / transient controller error: keep the current token
                // and retry, unless it has already expired after persistent
                // failures (then fail loudly rather than silently 401). The
                // `AgentError` never carries a secret, so logging it is safe.
                tracing::warn!(
                    session_id = %self.session_id,
                    reason = ?reason,
                    error = %error,
                    "credential-refresh failed; engine keeps previous token"
                );
                self.classify_failure()
            }
        }
    }

    /// Bump the failure counter and decide whether the failure is fatal: fatal
    /// only once the on-disk token is genuinely past expiry AND the failures have
    /// persisted, so a transient blip while the token is still valid never kills
    /// a healthy session. Mirrors the driver's `classify_mint_failure`.
    fn classify_failure(&mut self) -> RefreshOutcome {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let expired = self.token_expires_at <= SystemTime::now();
        if expired && self.consecutive_failures >= MAX_CONSECUTIVE_REFRESH_FAILURES {
            RefreshOutcome::Fatal {
                reason: format!(
                    "github token refresh failed {} times and the token has expired",
                    self.consecutive_failures
                ),
            }
        } else {
            RefreshOutcome::TransientFailure
        }
    }
}

/// Whether an outcome stops the supervise loop (self-fence or fatal).
fn is_terminal(outcome: &RefreshOutcome) -> bool {
    matches!(
        outcome,
        RefreshOutcome::StaleFence | RefreshOutcome::Fatal { .. }
    )
}

/// Fold a sub-step's non-terminal outcome into the tick's running best:
/// `Refreshed` (a token landed) wins over `TransientFailure` (something tried but
/// failed) which wins over `Skipped` (nothing happened). Terminal outcomes are
/// never folded — the caller short-circuits on them.
fn fold_outcome(current: RefreshOutcome, next: RefreshOutcome) -> RefreshOutcome {
    fn rank(o: &RefreshOutcome) -> u8 {
        match o {
            RefreshOutcome::Refreshed => 3,
            RefreshOutcome::TransientFailure => 2,
            RefreshOutcome::Skipped => 1,
            // Terminal variants are short-circuited before folding; rank low so a
            // stray fold can never mask a real Refreshed.
            RefreshOutcome::StaleFence | RefreshOutcome::Fatal { .. } => 0,
        }
    }
    if rank(&next) >= rank(&current) {
        next
    } else {
        current
    }
}

/// Effective cooldown for this attempt: tightened to the urgent window once the
/// token is within [`TOKEN_EXPIRY_URGENT_MARGIN`] of expiry (driver-mirrored).
fn effective_cooldown(token_expires_at: SystemTime) -> Duration {
    let remaining = token_expires_at
        .duration_since(SystemTime::now())
        .unwrap_or(Duration::ZERO);
    if remaining <= TOKEN_EXPIRY_URGENT_MARGIN {
        TOKEN_REFRESH_COOLDOWN_URGENT
    } else {
        TOKEN_REFRESH_COOLDOWN
    }
}

/// Path of the credential-helper's JIT mint-request file for a token file.
fn mint_request_path(token_path: &Path) -> PathBuf {
    let mut p = token_path.as_os_str().to_owned();
    p.push(MINT_REQUEST_SUFFIX);
    PathBuf::from(p)
}

/// Convert a non-negative unix-ms timestamp to a `SystemTime` (saturating at the
/// epoch for a non-positive value, so a clock-skewed response never panics).
fn unix_ms_to_system_time(unix_ms: i64) -> SystemTime {
    if unix_ms <= 0 {
        return SystemTime::UNIX_EPOCH;
    }
    SystemTime::UNIX_EPOCH + Duration::from_millis(unix_ms as u64)
}

/// The token-file path for a runtime dir (the worker is its sole writer).
/// Test-only: the supervise suite reads it to assert the token rewrite.
#[cfg(test)]
pub(crate) fn token_file_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(TOKEN_FILE_NAME)
}

/// The credential-helper's JIT mint-request file path for a runtime dir — what a
/// helper drops and the servicer consumes. Test-only: the supervise suite drops
/// a request file here to drive the JIT path.
#[cfg(test)]
pub(crate) fn request_file_path(runtime_dir: &Path) -> PathBuf {
    mint_request_path(&token_file_path(runtime_dir))
}
