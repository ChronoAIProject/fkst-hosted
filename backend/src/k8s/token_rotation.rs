//! In-place per-session installation-token rotation (issue #359 §5.4, PR5b).
//!
//! A Model B session pod is LONG-LIVED but its GitHub App installation token lives
//! only an hour. Rather than restart the pod, the control plane rewrites the pod's
//! mounted per-session Secret in place: the whole-volume projection propagates the
//! new `github-token` file to the running container, and the in-pod git credential
//! helper + `gh` shim read the CURRENT token on every op — so a delivery here
//! refreshes both with NO in-pod refresh loop.
//!
//! This loop enumerates the substrate-session fleet every `pod_token_refresh_secs`
//! (bound strictly below the 1-hour token TTL by [`ReconcileConfig`]) through the
//! [`SessionBackend`], re-mints each session's least-privilege token, and delivers it
//! into the session's mounted credential file. A deleted pod/Secret
//! ([`DeliveryOutcome::SessionGone`]) is a benign no-op; a vanished installation is
//! logged + enqueued for a reconcile (which kills the now-orphaned session). The
//! token is never logged.

use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;

use crate::error::AppError;
use crate::github_app::{session_permissions, GithubAppError, GithubAppTokens};
use crate::k8s::session_launcher::session_github_token_json;
use crate::reconcile::ReconcileHandle;
use crate::reconcile_config::ReconcileConfig;
use crate::session_backend::{DeliveryOutcome, SessionBackend, SessionHandle};
use crate::session_spec::creds::GITHUB_TOKEN_FILE;

/// The rotation loop: every `pod_token_refresh_secs`, refresh every live session
/// pod's mounted installation token. Runs for the process lifetime; a sweep error
/// is logged, never fatal.
pub async fn run_token_rotation_loop(
    backend: Arc<dyn SessionBackend>,
    github: GithubAppTokens,
    cfg: ReconcileConfig,
    handle: ReconcileHandle,
) {
    // The cadence is bounded (>=1, <3600) by ReconcileConfig, so the token always
    // rotates strictly inside its TTL.
    let interval = Duration::from_secs(cfg.pod_token_refresh_secs.max(1));
    tracing::info!(?interval, "token rotation: started");
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        if let Err(error) = rotate_once(backend.as_ref(), &github, &handle).await {
            tracing::warn!(error = %error, "token rotation: sweep failed (will retry)");
        }
    }
}

/// One rotation sweep: enumerate the session fleet and refresh each one's Secret.
/// Only a failure to LIST the fleet surfaces as `Err`; every per-session failure is
/// handled (logged / enqueued) so one bad session never stalls the rest.
async fn rotate_once(
    backend: &dyn SessionBackend,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
) -> Result<(), AppError> {
    let fleet = backend
        .list_fleet()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("token rotation list pods: {e}")))?;

    for session in &fleet {
        rotate_one(backend, github, handle, session).await;
    }
    Ok(())
}

/// Rotate one session's token: re-mint (least-privilege, repo-scoped), then deliver
/// it into the session's mounted `github-token`. [`DeliveryOutcome::SessionGone`]
/// (deleted pod/Secret) is benign; `InstallationGone` enqueues the repo so the
/// reconciler kills the orphan.
async fn rotate_one(
    backend: &dyn SessionBackend,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
    session: &SessionHandle,
) {
    let owner_repo = format!("{}/{}", session.repo.owner, session.repo.name);
    // FORCED re-mint: extend the mounted token a full TTL every interval. The cached
    // path only re-mints in the last EXPIRY_BUFFER before expiry, so a rotation
    // interval longer than that buffer would leave the Secret with an expired token
    // between rotations for a session that outlives the token TTL.
    match github
        .token_with_expiry_for_repo_forced(&owner_repo, Some(session_permissions()))
        .await
    {
        Ok((token, expires_at)) => {
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
                    tracing::info!(session_id = %session.session_id, owner_repo = %owner_repo, "token rotation: rotated session token")
                }
                Ok(DeliveryOutcome::SessionGone) => {}
                Err(error) => {
                    tracing::warn!(session_id = %session.session_id, error = %error, "token rotation: secret patch failed")
                }
            }
        }
        Err(GithubAppError::InstallationGone { .. }) => {
            tracing::warn!(
                session_id = %session.session_id,
                owner_repo = %owner_repo,
                "token rotation: installation gone; enqueueing repo for reconcile (kill)"
            );
            handle.enqueue((session.installation_id, session.repo.clone()));
        }
        Err(error) => {
            tracing::warn!(session_id = %session.session_id, owner_repo = %owner_repo, error = %error, "token rotation: token mint failed; leaving current token in place")
        }
    }
}

#[cfg(test)]
#[path = "token_rotation_tests.rs"]
mod tests;
