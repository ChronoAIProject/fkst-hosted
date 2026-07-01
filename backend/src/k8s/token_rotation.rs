//! In-place per-session installation-token rotation (issue #359 §5.4, PR5b).
//!
//! A Model B session pod is LONG-LIVED but its GitHub App installation token lives
//! only an hour. Rather than restart the pod, the control plane rewrites the pod's
//! mounted per-session Secret in place: the whole-volume projection propagates the
//! new `github-token` file to the running container, and the in-pod git credential
//! helper + `gh` shim read the CURRENT token on every op — so a Secret patch here
//! refreshes both with NO in-pod refresh loop.
//!
//! This loop LISTs the substrate-session pods every `pod_token_refresh_secs` (bound
//! strictly below the 1-hour token TTL by [`ReconcileConfig`]), re-mints each
//! session's least-privilege token, and server-side patches its Secret's
//! `github-token`. A deleted pod/Secret (`NotFound`) is a benign no-op; a vanished
//! installation is logged + enqueued for a reconcile (which kills the now-orphaned
//! session). The token is never logged.

use std::time::Duration;

use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::api::{Api, ListParams, Patch, PatchParams};

use crate::error::AppError;
use crate::github_app::{session_permissions, GithubAppError, GithubAppTokens};
use crate::k8s::client::KubeClient;
use crate::k8s::session_launcher::{
    session_github_token_json, session_object_name, COMPONENT_LABEL_KEY, COMPONENT_LABEL_VALUE,
    SESSION_ID_LABEL,
};
use crate::reconcile::repo::repo_key_from_pod;
use crate::reconcile::ReconcileHandle;
use crate::reconcile_config::ReconcileConfig;
use crate::session_spec::creds::GITHUB_TOKEN_FILE;

/// The rotation loop: every `pod_token_refresh_secs`, refresh every live session
/// pod's mounted installation token. Runs for the process lifetime; a sweep error
/// is logged, never fatal.
pub async fn run_token_rotation_loop(
    kube: KubeClient,
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
        if let Err(error) = rotate_once(&kube, &github, &handle).await {
            tracing::warn!(error = %error, "token rotation: sweep failed (will retry)");
        }
    }
}

/// One rotation sweep: LIST the session pods and refresh each one's Secret. Only a
/// failure to LIST the pods surfaces as `Err`; every per-pod failure is handled
/// (logged / enqueued) so one bad session never stalls the rest.
async fn rotate_once(
    kube: &KubeClient,
    github: &GithubAppTokens,
    handle: &ReconcileHandle,
) -> Result<(), AppError> {
    let pods: Api<Pod> = Api::namespaced(kube.client().clone(), kube.namespace());
    let secrets: Api<Secret> = Api::namespaced(kube.client().clone(), kube.namespace());
    let selector = format!("{COMPONENT_LABEL_KEY}={COMPONENT_LABEL_VALUE}");
    let list = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("token rotation list pods: {e}")))?;

    for pod in &list.items {
        let Some(session_id) = pod
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get(SESSION_ID_LABEL))
            .cloned()
        else {
            continue;
        };
        let Some((installation, repo)) = repo_key_from_pod(pod) else {
            continue;
        };
        rotate_one(github, &secrets, handle, &session_id, installation, &repo).await;
    }
    Ok(())
}

/// Rotate one session's token: re-mint (least-privilege, repo-scoped), then patch
/// its Secret's `github-token`. `NotFound` (deleted pod/Secret) is benign;
/// `InstallationGone` enqueues the repo so the reconciler kills the orphan.
async fn rotate_one(
    github: &GithubAppTokens,
    secrets: &Api<Secret>,
    handle: &ReconcileHandle,
    session_id: &str,
    installation: i64,
    repo: &crate::models::RepoRef,
) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    match github
        .token_with_expiry_for_repo(&owner_repo, Some(session_permissions()))
        .await
    {
        Ok((token, expires_at)) => {
            let token_json = session_github_token_json(&token, expires_at);
            let name = session_object_name(session_id);
            let patch = secret_github_token_patch(&token_json);
            match secrets
                .patch(&name, &PatchParams::default(), &Patch::Merge(patch))
                .await
            {
                Ok(_) => {
                    tracing::info!(session_id = %session_id, owner_repo = %owner_repo, "token rotation: rotated session token")
                }
                Err(kube::Error::Api(e)) if e.code == 404 => {}
                Err(error) => {
                    tracing::warn!(session_id = %session_id, error = %error, "token rotation: secret patch failed")
                }
            }
        }
        Err(GithubAppError::InstallationGone { .. }) => {
            tracing::warn!(
                session_id = %session_id,
                owner_repo = %owner_repo,
                "token rotation: installation gone; enqueueing repo for reconcile (kill)"
            );
            handle.enqueue((installation, repo.clone()));
        }
        Err(error) => {
            tracing::warn!(session_id = %session_id, owner_repo = %owner_repo, error = %error, "token rotation: token mint failed; leaving current token in place")
        }
    }
}

/// The JSON merge patch that rewrites a session Secret's `github-token` key with a
/// fresh `{token, expires_at}` value (via `stringData`, which K8s folds into the
/// Secret's data). Pure + unit-tested so the key + shape can't drift.
fn secret_github_token_patch(token_json: &str) -> serde_json::Value {
    let string_data = serde_json::Map::from_iter([(
        GITHUB_TOKEN_FILE.to_string(),
        serde_json::Value::String(token_json.to_string()),
    )]);
    serde_json::json!({ "stringData": serde_json::Value::Object(string_data) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_patch_targets_the_github_token_key_via_string_data() {
        let patch = secret_github_token_patch(
            r#"{"token":"ghs_new","expires_at":"2026-07-01T13:00:00+00:00"}"#,
        );
        let value = &patch["stringData"][GITHUB_TOKEN_FILE];
        assert_eq!(
            value.as_str().unwrap(),
            r#"{"token":"ghs_new","expires_at":"2026-07-01T13:00:00+00:00"}"#
        );
        // Nothing else is touched (a merge patch leaves other keys/fields intact).
        assert!(patch.get("data").is_none());
        assert_eq!(GITHUB_TOKEN_FILE, "github-token");
    }
}
