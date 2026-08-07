//! The credentials watch loop (issue #5927).
//!
//! Credential state is POD-scoped: the bundle lives in the session container's
//! filesystem and dies with it. The decision to (re)deliver it is SANDBOX-scoped:
//! `ensure_session` sees `sandbox already exists` and takes the slow recovery path
//! instead of the inline path a brand-new sandbox gets. So when a pod is REPLACED
//! under a surviving runtime — the cluster autoscaler scaling down its node is the
//! common cause — the replacement starts with an empty creds dir and a ticking
//! abort deadline, while re-delivery only happens as a planned action inside the
//! per-repo reconcile pass (observed spacing 51s-297s in prod).
//!
//! This loop closes that gap WITHOUT adding GitHub load: it runs the CHEAP backend
//! probe (`credential_recovery_needed`, an execd file check through the sandbox
//! proxy — no GitHub call) on a tight cadence, and only when a live session is
//! genuinely missing its credentials does it enqueue that repo, letting the normal
//! reconcile pass plan and execute the recovery. A healthy fleet therefore costs
//! exactly one backend probe per session per tick and nothing else.
//!
//! Discipline mirrors [`crate::k8s::health_scrape`]: best-effort and non-failing —
//! only a failure to LIST the fleet surfaces as `Err`; every per-session failure is
//! logged and swallowed so one bad session never stalls the rest.

use std::sync::Arc;
use std::time::Duration;

use crate::error::AppError;
use crate::reconcile::ReconcileHandle;
use crate::reconcile_config::ReconcileConfig;
use crate::session_backend::SessionBackend;

/// The watch loop: every `creds_watch_secs`, probe every live session for a missing
/// credential bundle and enqueue its repo when one is found. Runs for the process
/// lifetime; a sweep error is logged, never fatal.
pub async fn run_creds_watch_loop(
    backend: Arc<dyn SessionBackend>,
    cfg: ReconcileConfig,
    handle: ReconcileHandle,
) {
    let interval = Duration::from_secs(cfg.creds_watch_secs.max(1));
    tracing::info!(?interval, "credentials watch: started");
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        if let Err(error) = watch_once(backend.as_ref(), &handle).await {
            tracing::warn!(error = %error, "credentials watch: sweep failed (will retry)");
        }
    }
}

/// One watch sweep. Only a failure to LIST the fleet surfaces as `Err`.
async fn watch_once(
    backend: &dyn SessionBackend,
    handle: &ReconcileHandle,
) -> Result<(), AppError> {
    let fleet = backend
        .list_fleet()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("creds watch list fleet: {e}")))?;
    for session in &fleet {
        match backend
            .credential_recovery_needed(&session.session_id)
            .await
        {
            // The overwhelmingly common case: credentials are present, so this
            // costs one probe and stops here. No GitHub call, no enqueue.
            Ok(false) => {}
            Ok(true) => {
                tracing::info!(
                    session_id = %session.session_id,
                    owner_repo = %format!("{}/{}", session.repo.owner, session.repo.name),
                    "credentials watch: live runtime is missing its credentials; enqueueing repo for recovery"
                );
                handle.enqueue((session.installation_id, session.repo.clone()));
            }
            // A probe failure is EXPECTED while a replacement pod is still coming
            // up (its execd is not listening yet, so the proxy cannot connect).
            // Swallow it: the next tick re-probes, and the pod's own bounded wait
            // is what ultimately bounds this.
            Err(error) => {
                tracing::debug!(
                    session_id = %session.session_id,
                    error = %error,
                    "credentials watch: probe failed; retrying next tick"
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "creds_watch_tests.rs"]
mod tests;
