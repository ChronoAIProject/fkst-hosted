//! The package-AGNOSTIC session-health scrape loop (session health, PR).
//!
//! Mirrors [`crate::k8s::token_rotation`]'s `run_*_loop` / `*_once` / `*_one`
//! shape: every `health_scrape_secs` it LISTs the substrate-session pods (the same
//! `COMPONENT_LABEL` selector the reconciler uses) and, for each, reads its status +
//! a bounded window of its OWN framework logs, runs the PURE evaluator
//! ([`crate::k8s::health_eval::evaluate_health`]), and FLAGs or CLEARs the
//! [`SUBSTRATE_DEGRADED_LABEL`] on the pod's trigger issue.
//!
//! What it does NOT do: interpret any package's output. "Degraded" is derived only
//! from the two signals every fkst package shares (pod status + framework log
//! severity), and the comment it posts RELAYS the framework's own line verbatim —
//! it never asserts a diagnosis. This keeps fkst-hosted package-agnostic.
//!
//! Discipline (identical to the rotation loop): best-effort + non-failing — only a
//! failure to LIST the pods surfaces as `Err`; every per-pod failure is logged and
//! swallowed so one bad session never stalls the rest. No token/secret is ever
//! logged, and a vanished installation enqueues the repo for a reconcile (kill).

use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};

use crate::error::AppError;
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::k8s::client::KubeClient;
use crate::k8s::health_eval::{evaluate_health, summarize_pod_status, HealthVerdict};
use crate::k8s::pod_logs::pod_recent_logs;
use crate::k8s::session_launcher::{
    ANNOTATION_TRIGGER_ISSUE, COMPONENT_LABEL_KEY, COMPONENT_LABEL_VALUE, SESSION_ID_LABEL,
};
use crate::reconcile::{ReconcileHandle, SUBSTRATE_DEGRADED_LABEL};
use crate::reconcile_config::ReconcileConfig;
use crate::session_backend::k8s::repo_key_from_pod;

/// The scrape loop: every `health_scrape_secs`, evaluate every live session pod's
/// health and flag/clear its trigger issue. Runs for the process lifetime; a sweep
/// error is logged, never fatal.
pub async fn run_health_scrape_loop(
    kube: KubeClient,
    github: GithubAppTokens,
    cfg: ReconcileConfig,
    handle: ReconcileHandle,
) {
    let interval = Duration::from_secs(cfg.health_scrape_secs.max(1));
    tracing::info!(?interval, "session health scrape: started");
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        if let Err(error) = scrape_once(&kube, &github, &handle).await {
            tracing::warn!(error = %error, "session health scrape: sweep failed (will retry)");
        }
    }
}

/// One scrape sweep: LIST the session pods and evaluate each. Only a failure to LIST
/// the pods surfaces as `Err`; every per-pod failure is handled (logged / enqueued)
/// so one bad session never stalls the rest.
async fn scrape_once(
    kube: &KubeClient,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
) -> Result<(), AppError> {
    let pods: Api<Pod> = Api::namespaced(kube.client().clone(), kube.namespace());
    let selector = format!("{COMPONENT_LABEL_KEY}={COMPONENT_LABEL_VALUE}");
    let list = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("health scrape list pods: {e}")))?;

    for pod in &list.items {
        scrape_one(kube, github, handle, pod).await;
    }
    Ok(())
}

/// Evaluate ONE pod and reconcile the degraded flag on its trigger issue. Skips a
/// pod that is not fully one of ours (missing repo key / trigger issue / name). A
/// vanished installation enqueues the repo for a reconcile (kill), mirroring the
/// rotation loop; every other failure is logged and swallowed.
async fn scrape_one(
    kube: &KubeClient,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
    pod: &Pod,
) {
    let Some((installation, repo)) = repo_key_from_pod(pod) else {
        return;
    };
    let Some(issue) = trigger_issue_from_pod(pod) else {
        return;
    };
    let Some(name) = pod.metadata.name.as_deref() else {
        return;
    };
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let session_id = pod
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get(SESSION_ID_LABEL))
        .map(String::as_str)
        .unwrap_or("");

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
            handle.enqueue((installation, repo));
            return;
        }
        Err(error) => {
            tracing::warn!(session_id = %session_id, owner_repo = %owner_repo, error = %error, "session health: issue label read failed; skipping pod");
            return;
        }
    };

    let status = summarize_pod_status(pod);
    // `None` = the logs could not be read (a real transport error), distinct from an
    // empty window — so a Healthy verdict on unreadable logs never CLEARS a flag.
    let logs = pod_recent_logs(kube, name).await;
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

/// Read the trigger-issue number a pod was stamped with (the same
/// [`ANNOTATION_TRIGGER_ISSUE`] the session launcher writes + the reconciler reads).
/// `None` when the annotation is missing / unparseable / zero (the sentinel the
/// live-pod projection uses for "unknown").
fn trigger_issue_from_pod(pod: &Pod) -> Option<u64> {
    let raw = pod
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(ANNOTATION_TRIGGER_ISSUE))?;
    let number = raw.parse::<u64>().ok()?;
    (number != 0).then_some(number)
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
