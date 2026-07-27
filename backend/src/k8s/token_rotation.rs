//! In-place per-session installation-token rotation (issue #359 §5.4, PR5b).
//!
//! A Model B session pod is LONG-LIVED but its GitHub App installation token lives
//! only an hour. Rather than restart the pod, the control plane rewrites the pod's
//! mounted per-session Secret in place: the whole-volume projection propagates the
//! new `github-token` file to the running container, and the in-pod git credential
//! helper + `gh` shim read the CURRENT token on every op — so a delivery here
//! refreshes both with NO in-pod refresh loop.
//!
//! This loop enumerates the substrate-session fleet through the [`SessionBackend`],
//! re-mints each session's least-privilege token, and delivers it into the session's
//! mounted credential file. The token is never logged.
//!
//! ## Cadence and retry (#3822)
//!
//! A session's token is minted at a FULL ~1 h TTL — at delivery (#3410) and at every
//! rotation — and [`ReconcileConfig`] bounds `pod_token_refresh_secs` strictly below
//! that TTL, so a session's token always outlives the wait for the sweep that replaces
//! it. The load-bearing word is *that* sweep: the invariant assumes the next rotation
//! both HAPPENS and SUCCEEDS.
//!
//! It does not always. A fleet list, a mint, or a delivery can fail transiently, and
//! falling through to the next periodic tick costs a full interval — longer than the
//! margin the TTL leaves (900 s at the 2700 s default), so ONE dropped sweep expired
//! the token and left `gh`/`git` returning `Bad credentials` for ~30 minutes.
//!
//! So a failed or PARTIAL pass retries with bounded backoff ([`RetryBackoff`]) instead
//! of waiting out the cadence, and four properties keep that honest:
//!
//! - **A repair pass re-rotates only the sessions that failed**, never the whole
//!   fleet, so retrying costs one mint per still-broken session — not per session.
//! - **Each session backs off on its OWN failure count**, so a session that never
//!   recovers cannot ratchet a shared counter to its ceiling and make every other
//!   session's first retry wait a full interval — which would be this very bug again.
//! - **The backoff ceiling is the periodic interval itself**, so a persistently
//!   failing session degrades to exactly the un-retried cadence and never to a slower
//!   one, nor to an unbounded mint rate.
//! - **A full sweep still runs every interval regardless**, so a session that fails
//!   every repair pass cannot starve the fleet it shares the loop with.
//! - **Permanent failures are not retried at all.** A vanished, suspended, or
//!   under-permissioned installation cannot be fixed by trying again; it is logged and
//!   enqueued for a reconcile (which kills the now-orphaned session) rather than
//!   consuming the App's API budget forever.
//!
//! [`ReconcileConfig`]: crate::reconcile_config::ReconcileConfig

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;
// tokio's Instant, not std's: the loop-level tests drive a PAUSED clock, and only this
// one follows it.
use tokio::time::Instant;

use crate::error::AppError;
use crate::github_app::{session_permissions, GithubAppError, GithubAppTokens};
use crate::k8s::session_launcher::session_github_token_json;
use crate::reconcile::ReconcileHandle;
use crate::reconcile_config::ReconcileConfig;
use crate::retry::{jittered_delay, RetryBackoff};
use crate::session_backend::{DeliveryOutcome, SessionBackend, SessionHandle};
use crate::session_spec::creds::GITHUB_TOKEN_FILE;

/// Shortest delay before re-attempting a failed or partial sweep. Small enough that a
/// blip costs seconds rather than a rotation interval; the backoff doubles from here.
const RETRY_INITIAL_SECS: u64 = 15;

/// Spread applied to every retry delay so a fleet of control planes retrying the same
/// unavailable dependency does not re-converge on one instant.
const RETRY_JITTER_PERCENT: u64 = 20;

/// What rotating ONE session achieved.
#[derive(Debug, PartialEq, Eq)]
enum RotationOutcome {
    /// The session now holds a freshly minted, full-TTL token.
    Refreshed,
    /// The runtime is gone; it needs no token. Not a failure.
    Gone,
    /// A transient failure — worth another attempt shortly.
    Retryable,
    /// A failure no retry can fix (the installation vanished, was suspended, or no
    /// longer grants the permissions the mint needs). Enqueued for a reconcile.
    Permanent,
}

/// What one pass achieved, in the shape the coordinator needs to update its backlog.
#[derive(Debug, Default)]
struct SweepSummary {
    /// Every session this pass tried, by id. A FULL sweep's list is the authoritative
    /// fleet, so anything in the backlog that is absent from it has gone away.
    attempted: Vec<String>,
    /// Sessions a transient failure left holding their old token.
    retryable: Vec<SessionHandle>,
    permanent: usize,
}

/// A session awaiting another rotation attempt, carrying ITS OWN backoff.
///
/// Per-session, not fleet-global. With one shared counter, a session that never
/// recovers ratchets the delay to its ceiling and every OTHER session's FIRST retry
/// then inherits it — a ~45-minute wait, which is exactly the dead window this loop
/// exists to close. Each session's cadence must depend only on its own failures.
struct PendingRotation {
    session: SessionHandle,
    backoff: RetryBackoff,
    /// Earliest instant this session may be retried.
    due: Instant,
}

/// The rotation loop: keep every live session's mounted installation token fresh.
///
/// Runs for the process lifetime (one leader generation). The first sweep fires
/// IMMEDIATELY, which is what makes a control-plane restart or a leadership handover
/// re-mint the whole fleet at once instead of leaving it to age until the first tick.
pub async fn run_token_rotation_loop(
    backend: Arc<dyn SessionBackend>,
    github: GithubAppTokens,
    cfg: ReconcileConfig,
    handle: ReconcileHandle,
) {
    let periodic_interval = Duration::from_secs(cfg.pod_token_refresh_secs.max(1));
    tracing::info!(
        ?periodic_interval,
        retry_initial_secs = RETRY_INITIAL_SECS,
        "token rotation: started"
    );

    // Backlog of sessions awaiting a retry, each with its OWN backoff and due time.
    let mut pending: HashMap<String, PendingRotation> = HashMap::new();
    // Backoff for repeated failures to LIST the fleet. This one IS fleet-global,
    // because the failure is: nothing was rotated at all.
    let mut fleet_retry = RetryBackoff::new(RETRY_INITIAL_SECS, periodic_interval.as_secs());
    // When the fleet must be re-enumerated no matter what. Without this deadline a
    // session that fails every repair pass would keep the backlog non-empty forever and
    // no full sweep would ever run again — every OTHER session's token would then
    // expire, which is worse than the gap this loop exists to close. Re-listing also
    // drops backlog entries for sessions that have since gone away.
    let mut next_full_sweep = Instant::now();

    loop {
        let now = Instant::now();
        if now >= next_full_sweep {
            next_full_sweep = now + periodic_interval;
            match rotate_once(backend.as_ref(), &github, &handle).await {
                Ok(summary) => {
                    fleet_retry.reset();
                    // The fleet list is authoritative: forget any backlog entry for a
                    // session that no longer exists.
                    pending.retain(|id, _| summary.attempted.contains(id));
                    absorb(&mut pending, &summary, periodic_interval);
                    tracing::debug!(
                        sessions = summary.attempted.len(),
                        unrefreshed = summary.retryable.len(),
                        permanent_failures = summary.permanent,
                        "token rotation: full sweep done"
                    );
                }
                Err(error) => {
                    // Nothing was rotated. Bring the whole sweep forward rather than
                    // leaving every session to age out for a full interval.
                    let delay = jittered_delay(fleet_retry.next_delay(), RETRY_JITTER_PERCENT);
                    next_full_sweep = now + delay;
                    tracing::warn!(
                        error = %error,
                        retry_delay_secs = delay.as_secs(),
                        "token rotation: fleet list failed; retrying the whole sweep"
                    );
                }
            }
        } else {
            // Repair pass: exactly the backlog entries whose own delay has elapsed.
            let due: Vec<SessionHandle> = pending
                .values()
                .filter(|p| p.due <= now)
                .map(|p| p.session.clone())
                .collect();
            if !due.is_empty() {
                let summary = rotate_sessions(backend.as_ref(), &github, &handle, due).await;
                // Anything attempted and no longer retryable has been repaired (or has
                // gone away, or failed permanently) — either way it leaves the backlog.
                let repaired = summary.attempted.len() - summary.retryable.len();
                pending.retain(|id, _| {
                    !summary.attempted.contains(id)
                        || summary.retryable.iter().any(|s| &s.session_id == id)
                });
                absorb(&mut pending, &summary, periodic_interval);
                tracing::info!(
                    attempted = summary.attempted.len(),
                    repaired,
                    still_unrefreshed = summary.retryable.len(),
                    "token rotation: repair pass done"
                );
            }
        }

        // Wake for whichever comes first: the next full sweep, or the soonest backlog
        // entry. Never later than the deadline, so a long per-session backoff can never
        // push the full sweep out past the token TTL it must stay inside.
        let wake = pending
            .values()
            .map(|p| p.due)
            .min()
            .unwrap_or(next_full_sweep)
            .min(next_full_sweep);
        tokio::time::sleep(wake.saturating_duration_since(Instant::now())).await;
    }
}

/// Fold a pass's still-failing sessions into the backlog, advancing each one's OWN
/// backoff and scheduling its next attempt. A session failing for the first time starts
/// at the floor no matter how long another session has been broken.
fn absorb(
    pending: &mut HashMap<String, PendingRotation>,
    summary: &SweepSummary,
    periodic_interval: Duration,
) {
    let now = Instant::now();
    for session in &summary.retryable {
        let entry = pending
            .entry(session.session_id.clone())
            .or_insert_with(|| PendingRotation {
                session: session.clone(),
                backoff: RetryBackoff::new(RETRY_INITIAL_SECS, periodic_interval.as_secs()),
                due: now,
            });
        // Refresh the handle: a full sweep may have re-listed it with newer metadata.
        entry.session = session.clone();
        entry.due = now + jittered_delay(entry.backoff.next_delay(), RETRY_JITTER_PERCENT);
    }
}

/// One full sweep: enumerate the session fleet and refresh each one. Only a failure to
/// LIST the fleet surfaces as `Err` (nothing was rotated, so the whole pass repeats);
/// every per-session failure is classified into the summary, so one bad session never
/// stalls the rest.
async fn rotate_once(
    backend: &dyn SessionBackend,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
) -> Result<SweepSummary, AppError> {
    let fleet = backend
        .list_fleet()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("token rotation list pods: {e}")))?;

    Ok(rotate_sessions(backend, github, handle, fleet).await)
}

/// Rotate exactly these sessions, classifying each outcome. Drives both a full sweep
/// and a repair pass, so the two can never diverge in how they treat a failure.
async fn rotate_sessions(
    backend: &dyn SessionBackend,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
    sessions: Vec<SessionHandle>,
) -> SweepSummary {
    let mut summary = SweepSummary {
        attempted: sessions.iter().map(|s| s.session_id.clone()).collect(),
        ..SweepSummary::default()
    };
    // Deliberately NOT wrapped in an outer timeout. Every step already carries its own
    // budget (the mint's HTTP client, each execd verb, the Secret patch), and an outer
    // `tokio::time::timeout` would introduce a cancellation point into delivery code
    // that is not cancel-safe — the OpenSandbox re-push path takes the cached bundle
    // OUT of its map and puts it back after the upload, so a cancelled future would
    // destroy it permanently.
    for session in sessions {
        match rotate_one(backend, github, handle, &session).await {
            RotationOutcome::Refreshed | RotationOutcome::Gone => {}
            RotationOutcome::Retryable => summary.retryable.push(session),
            RotationOutcome::Permanent => summary.permanent += 1,
        }
    }
    summary
}

/// Rotate one session's token: re-mint (least-privilege, repo-scoped), then deliver it
/// into the session's mounted `github-token`.
async fn rotate_one(
    backend: &dyn SessionBackend,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
    session: &SessionHandle,
) -> RotationOutcome {
    let owner_repo = format!("{}/{}", session.repo.owner, session.repo.name);
    // FORCED re-mint: extend the mounted token a full TTL every interval. The cached
    // path only re-mints in the last EXPIRY_BUFFER before expiry, so a rotation
    // interval longer than that buffer would leave the credential file with an expired
    // token between rotations for a session that outlives the token TTL.
    let (token, expires_at) = match github
        .token_with_expiry_for_repo_forced(&owner_repo, Some(session_permissions()))
        .await
    {
        Ok(pair) => pair,
        Err(error) if is_permanent_mint_failure(&error) => {
            // No amount of retrying mints a token the App is no longer entitled to.
            // Hand the repo to the reconciler, which kills the orphaned session.
            tracing::error!(
                session_id = %session.session_id,
                owner_repo = %owner_repo,
                error = %error,
                "token rotation: installation can no longer mint a session token; \
                 enqueueing repo for reconcile (kill)"
            );
            handle.enqueue((session.installation_id, session.repo.clone()));
            return RotationOutcome::Permanent;
        }
        Err(error) => {
            tracing::warn!(
                session_id = %session.session_id,
                owner_repo = %owner_repo,
                error = %error,
                "token rotation: token mint failed; keeping the current token and retrying"
            );
            return RotationOutcome::Retryable;
        }
    };

    let token_json = session_github_token_json(&token, expires_at);
    match backend
        .deliver_credential(
            &session.session_id,
            GITHUB_TOKEN_FILE,
            SecretString::from(token_json),
        )
        .await
    {
        Ok(DeliveryOutcome::Delivered) => {
            tracing::info!(
                session_id = %session.session_id,
                owner_repo = %owner_repo,
                "token rotation: rotated session token"
            );
            RotationOutcome::Refreshed
        }
        // The runtime vanished between the list and the delivery — it needs no token.
        // This is the ONLY benign case: the backend contract reserves `SessionGone` for
        // it, so a bare `BackendError::NotFound` is a 404 from some sub-request of a
        // delivery already in flight, i.e. an ordinary failure, and falls through below.
        Ok(DeliveryOutcome::SessionGone) => RotationOutcome::Gone,
        Err(error) => {
            tracing::warn!(
                session_id = %session.session_id,
                error = %error,
                "token rotation: credential delivery failed; retrying"
            );
            RotationOutcome::Retryable
        }
    }
}

/// Whether a mint failure is one that retrying cannot fix.
///
/// Fail-SOFT by design: an unrecognised error is treated as transient, so a
/// misclassification costs bounded extra mints (the backoff ceiling is the rotation
/// interval) rather than a silently abandoned session. The listed variants all mean the
/// App has lost its entitlement to mint for this repo, which only an operator or a
/// reconcile can resolve.
fn is_permanent_mint_failure(error: &GithubAppError) -> bool {
    matches!(
        error,
        GithubAppError::InstallationGone { .. }
            | GithubAppError::NotInstalled { .. }
            | GithubAppError::AppAuth
            | GithubAppError::InvalidKey
            | GithubAppError::InvalidRepoRef
            | GithubAppError::TokenRequestRejected(_)
    )
}

#[cfg(test)]
#[path = "token_rotation_tests.rs"]
mod tests;
