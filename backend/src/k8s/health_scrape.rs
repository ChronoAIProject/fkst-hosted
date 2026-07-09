//! The package-AGNOSTIC session-health scrape loop (session health, PR).
//!
//! Mirrors [`crate::k8s::token_rotation`]'s `run_*_loop` / `*_once` / `*_one`
//! shape: every `health_scrape_secs` it enumerates the substrate-session fleet
//! (through the [`SessionBackend`]) and, for each, reads its status + a bounded
//! window of its OWN framework logs, runs the PURE evaluator
//! ([`crate::k8s::health_eval::evaluate_health`]), and FLAGs or CLEARs the
//! [`SUBSTRATE_DEGRADED_LABEL`] on the session's trigger issue.
//!
//! What it does NOT do: interpret any package's output. "Degraded" is derived only
//! from the two signals every fkst package shares (pod status + framework log
//! severity), and the comment it posts RELAYS the framework's own line verbatim —
//! it never asserts a diagnosis. This keeps fkst-hosted package-agnostic.
//!
//! Discipline (identical to the rotation loop): best-effort + non-failing — only a
//! failure to LIST the fleet surfaces as `Err`; every per-session failure is logged
//! and swallowed so one bad session never stalls the rest. No token/secret is ever
//! logged, and a vanished installation enqueues the repo for a reconcile (kill).

use std::sync::Arc;
use std::time::Duration;

use crate::error::AppError;
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::k8s::health_eval::{evaluate_health, HealthVerdict, PodStatusSummary};
use crate::reconcile::{ReconcileHandle, SUBSTRATE_DEGRADED_LABEL};
use crate::reconcile_config::ReconcileConfig;
use crate::session_backend::{RuntimeStatus, SessionBackend, SessionHandle};

/// The scrape loop: every `health_scrape_secs`, evaluate every live session's
/// health and flag/clear its trigger issue. Runs for the process lifetime; a sweep
/// error is logged, never fatal.
pub async fn run_health_scrape_loop(
    backend: Arc<dyn SessionBackend>,
    github: GithubAppTokens,
    cfg: ReconcileConfig,
    handle: ReconcileHandle,
) {
    let interval = Duration::from_secs(cfg.health_scrape_secs.max(1));
    tracing::info!(?interval, "session health scrape: started");
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        if let Err(error) = scrape_once(backend.as_ref(), &github, &handle).await {
            tracing::warn!(error = %error, "session health scrape: sweep failed (will retry)");
        }
    }
}

/// One scrape sweep: enumerate the fleet and evaluate each session. Only a failure
/// to LIST the fleet surfaces as `Err`; every per-session failure is handled (logged
/// / enqueued) so one bad session never stalls the rest.
async fn scrape_once(
    backend: &dyn SessionBackend,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
) -> Result<(), AppError> {
    let fleet = backend
        .list_fleet()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("health scrape list pods: {e}")))?;

    for session in &fleet {
        scrape_one(backend, github, handle, session).await;
    }
    Ok(())
}

/// Evaluate ONE session and reconcile the degraded flag on its trigger issue. Skips
/// a session with no trigger issue. A vanished installation enqueues the repo for a
/// reconcile (kill), mirroring the rotation loop; every other failure is logged and
/// swallowed.
async fn scrape_one(
    backend: &dyn SessionBackend,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
    session: &SessionHandle,
) {
    let Some(issue) = session.trigger_issue else {
        return;
    };
    let owner_repo = format!("{}/{}", session.repo.owner, session.repo.name);
    let session_id = session.session_id.as_str();

    // Read the trigger issue's current labels first: it is both the dedupe signal
    // (already-flagged?) and the cheapest call, so an auth/installation failure here
    // short-circuits before we pay for a log read.
    let labels = match github.get_issue_labels(&owner_repo, issue).await {
        Ok(labels) => labels,
        Err(GithubAppError::InstallationGone { .. }) => {
            tracing::warn!(
                session_id = %session_id,
                owner_repo = %owner_repo,
                "session health: installation gone; enqueueing repo for reconcile (kill)"
            );
            handle.enqueue((session.installation_id, session.repo.clone()));
            return;
        }
        Err(error) => {
            tracing::warn!(session_id = %session_id, owner_repo = %owner_repo, error = %error, "session health: issue label read failed; skipping pod");
            return;
        }
    };

    // Read the runtime status through the backend. A read error (non-404) skips the
    // session this cycle rather than risk a wrong verdict; a gone runtime reads as an
    // empty status (nothing to see).
    let status = match backend.status_summary(session_id).await {
        Ok(status) => runtime_status_to_summary(&status),
        Err(error) => {
            tracing::warn!(session_id = %session_id, owner_repo = %owner_repo, error = %error, "session health: pod status read failed; skipping pod");
            return;
        }
    };
    // `None` = the logs could not be read (a real transport error), distinct from an
    // empty window — so a Healthy verdict on unreadable logs never CLEARS a flag.
    let logs = backend.recent_output(session_id).await;
    let parsed = logs
        .as_deref()
        .map(crate::k8s::health_eval::parse_severity_lines)
        .unwrap_or_default();
    let verdict = evaluate_health(&status, &parsed);

    apply_verdict(
        github,
        &owner_repo,
        issue,
        &verdict,
        &labels,
        logs.is_some(),
    )
    .await;
}

/// Adapt the kube-free [`RuntimeStatus`] back into the pure evaluator's
/// [`PodStatusSummary`]. The `Option<u32>` restart count round-trips to the signed
/// count the evaluator reads OUTSIDE the pure health logic (a real count is never
/// negative, so the conversion never loses information).
fn runtime_status_to_summary(status: &RuntimeStatus) -> PodStatusSummary {
    PodStatusSummary {
        phase: status.phase.clone(),
        restart_count: status
            .restart_count
            .map(|r| i32::try_from(r).unwrap_or(i32::MAX))
            .unwrap_or(0),
        waiting_reason: status.stall_reason.clone(),
    }
}

/// Reconcile the degraded flag on a trigger issue for a single verdict. Best-effort:
/// every GitHub effect is logged (never propagated). Posts a comment only on a
/// TRANSITION (degraded when not already flagged; recovered when flagged) so the
/// scrape does not re-comment every cycle.
///
/// `logs_readable` gates the CLEAR: a Healthy verdict computed while the logs could
/// NOT be read is inconclusive, so the flag is left in place rather than cleared on
/// a transient log-read failure.
async fn apply_verdict(
    github: &GithubAppTokens,
    owner_repo: &str,
    issue: u64,
    verdict: &HealthVerdict,
    labels: &[String],
    logs_readable: bool,
) {
    let has_flag = labels.iter().any(|l| l == SUBSTRATE_DEGRADED_LABEL);
    match verdict {
        HealthVerdict::Degraded {
            reason_verbatim,
            detail,
        } => {
            if has_flag {
                return; // already warned — no re-comment
            }
            let body = degraded_comment(reason_verbatim, detail);
            post_comment_best_effort(github, owner_repo, issue, &body).await;
            if let Err(error) = github
                .add_issue_labels(owner_repo, issue, &[SUBSTRATE_DEGRADED_LABEL.to_string()])
                .await
            {
                tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "session health: latch degraded label failed");
            } else {
                tracing::info!(owner_repo = %owner_repo, issue, "session health: flagged degraded");
            }
        }
        HealthVerdict::Healthy => {
            if !has_flag {
                return; // nothing to clear
            }
            if !logs_readable {
                tracing::debug!(owner_repo = %owner_repo, issue, "session health: healthy but logs unreadable; withholding clear");
                return;
            }
            post_comment_best_effort(github, owner_repo, issue, &recovered_comment()).await;
            if let Err(error) = github
                .remove_issue_label(owner_repo, issue, SUBSTRATE_DEGRADED_LABEL)
                .await
            {
                tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "session health: clear degraded label failed");
            } else {
                tracing::info!(owner_repo = %owner_repo, issue, "session health: cleared degraded flag (recovered)");
            }
        }
    }
}

/// Post a comment, logging (never propagating) any failure.
async fn post_comment_best_effort(
    github: &GithubAppTokens,
    owner_repo: &str,
    issue: u64,
    body: &str,
) {
    if let Err(error) = github.post_issue_comment(owner_repo, issue, body).await {
        tracing::warn!(owner_repo = %owner_repo, issue, error = %error, "session health: issue comment failed");
    }
}

/// The VERBATIM degraded-health comment. The offending line is quoted in a fenced
/// block untouched; the detail relays recurrence/context; the closing line makes the
/// package-agnostic stance explicit (this is the session's own signal, not a
/// fkst-hosted judgment of the work).
fn degraded_comment(reason_verbatim: &str, detail: &str) -> String {
    format!(
        "⚠️ **Session health: degraded**\n\n\
         The session's own framework (or its pod) is reporting a problem, relayed here \
         verbatim:\n\n\
         ```\n{reason_verbatim}\n```\n\n\
         {detail}\n\n\
         The pod is up but may not be doing useful work — this is the session's own log, \
         not a fkst-hosted judgment."
    )
}

/// The recovered-health comment posted when a previously-flagged session reads
/// healthy again.
fn recovered_comment() -> String {
    "✅ **Session health: recovered**\n\n\
     The session is no longer emitting the health signal that was flagged; its pod \
     looks healthy again."
        .to_string()
}

#[cfg(test)]
#[path = "health_scrape_tests.rs"]
mod tests;
