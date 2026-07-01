//! The reconciler's long-lived loops (issue #359 §4.2/§5.4, PR5b): the queue
//! consumer, the periodic pod sweep, and the periodic full resync.
//!
//! One `run_reconcile_loop` CONSUMER drains the queue and reconciles each repo
//! serially (deduping a burst of enqueues for the same repo into one pass); two
//! PRODUCERS keep the queue fed — `run_sweep_loop` re-enqueues every repo with a
//! live pod (so drift on an existing session is caught) and `run_full_resync_loop`
//! enumerates the App's installations + repos (so a repo with a pending trigger
//! but no pod yet is discovered). Both producers FAIL OPEN: an enumeration error is
//! logged and the loop keeps running.

use std::collections::HashSet;
use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use tokio::sync::mpsc;

use crate::error::AppError;
use crate::k8s::session_launcher::{COMPONENT_LABEL_KEY, COMPONENT_LABEL_VALUE};
use crate::reconcile::execute::ReconcileCtx;
use crate::reconcile::repo::{reconcile_repo, repo_key_from_pod};

use super::{ReconcileHandle, RepoKey};

/// The single queue consumer: block for the next key, DEDUP everything already
/// queued into one batch, then reconcile each repo SERIALLY. Draining into a
/// deduped batch collapses a sweep + full-resync + webhook burst for the same repo
/// into a single reconcile; the single consumer guarantees per-repo serialization
/// (never two concurrent reconciles of the same repo). Exits when the queue closes.
pub async fn run_reconcile_loop(mut rx: mpsc::Receiver<RepoKey>, ctx: ReconcileCtx) {
    tracing::info!("reconcile loop: started");
    loop {
        let Some(first) = rx.recv().await else {
            tracing::info!("reconcile loop: channel closed; exiting");
            return;
        };
        for (installation, repo) in drain_pending(first, &mut rx) {
            if let Err(error) = reconcile_repo(installation, &repo, &ctx).await {
                tracing::warn!(
                    installation,
                    owner = %repo.owner,
                    name = %repo.name,
                    error = %error,
                    "reconcile loop: repo reconcile failed (will retry next sweep)"
                );
            }
        }
    }
}

/// Collect `first` plus every key already sitting in the queue into ONE deduped
/// batch (pure over the receiver; unit-tested). This is the "pending
/// `HashSet<RepoKey>`" dedup — a repo enqueued N times in the same window is
/// reconciled once.
fn drain_pending(first: RepoKey, rx: &mut mpsc::Receiver<RepoKey>) -> HashSet<RepoKey> {
    let mut batch: HashSet<RepoKey> = HashSet::new();
    batch.insert(first);
    while let Ok(key) = rx.try_recv() {
        batch.insert(key);
    }
    batch
}

/// The periodic pod sweep: every `reconcile_interval_secs`, enqueue every repo that
/// currently has a live substrate-session pod so drift on an existing session is
/// caught even without a webhook event. Fails open.
pub async fn run_sweep_loop(ctx: ReconcileCtx, handle: ReconcileHandle) {
    let interval = Duration::from_secs(ctx.config.reconcile.reconcile_interval_secs.max(1));
    tracing::info!(?interval, "reconcile sweep: started");
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        match sweep_once(&ctx, &handle).await {
            Ok(n) if n > 0 => {
                tracing::debug!(
                    enqueued = n,
                    "reconcile sweep: enqueued repos with live pods or open registrations"
                )
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(error = %error, "reconcile sweep: failed (will retry)"),
        }
    }
}

/// LIST the substrate-session pods, group them into `(installation, repo)` keys via
/// their stamped annotations, and enqueue each unique key. Returns how many unique
/// repos were enqueued.
async fn sweep_once(ctx: &ReconcileCtx, handle: &ReconcileHandle) -> Result<usize, AppError> {
    let pods: Api<Pod> = Api::namespaced(ctx.kube.client().clone(), ctx.kube.namespace());
    let selector = format!("{COMPONENT_LABEL_KEY}={COMPONENT_LABEL_VALUE}");
    let list = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(|e| {
            AppError::Internal(anyhow::anyhow!("sweep list substrate-session pods: {e}"))
        })?;

    let mut keys: HashSet<RepoKey> = HashSet::new();
    for pod in &list.items {
        if let Some(key) = repo_key_from_pod(pod) {
            keys.insert(key);
        }
    }
    // Also re-enqueue every repo with an open trigger registration, even those with
    // NO pod yet — so a first-spawn repo is reconciled every sweep (not only by the
    // slow full-resync), catching a search-lagged work issue within one sweep. See
    // `ActiveRepos`.
    {
        let active = ctx.active_repos.lock().unwrap_or_else(|e| e.into_inner());
        keys.extend(active.iter().cloned());
    }
    let enqueued = keys.len();
    for key in keys {
        handle.enqueue(key);
    }
    Ok(enqueued)
}

/// The periodic full resync: at startup + every `full_resync_interval_secs`,
/// enumerate the App's installations + their repos and enqueue every one, so a repo
/// with a pending trigger issue but no pod yet is discovered. Fails open (a bad
/// installation is skipped; an enumeration error is logged, never fatal).
pub async fn run_full_resync_loop(ctx: ReconcileCtx, handle: ReconcileHandle) {
    let interval = Duration::from_secs(ctx.config.reconcile.pod_full_resync_interval_secs.max(1));
    tracing::info!(
        ?interval,
        "reconcile full-resync: started (startup + each interval)"
    );
    // tokio's interval fires immediately on the first tick -> the startup resync.
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        match full_resync_once(&ctx, &handle).await {
            Ok(n) => {
                tracing::info!(
                    enqueued = n,
                    "reconcile full-resync: enumerated installations + repos"
                )
            }
            Err(error) => {
                tracing::warn!(error = %error, "reconcile full-resync: failed (fails open; will retry)")
            }
        }
    }
}

/// Enumerate installations (App-JWT) and each installation's repos
/// (installation-wide token), enqueuing every repo. A per-installation failure is
/// logged and skipped; only a failure to mint the App JWT or list installations
/// surfaces as `Err` (which the loop logs + continues on). Returns the enqueue count.
async fn full_resync_once(ctx: &ReconcileCtx, handle: &ReconcileHandle) -> Result<usize, AppError> {
    let app_jwt = ctx.github.app_jwt()?;
    let installations = ctx.listing.list_installations(&app_jwt).await?;

    let mut enqueued = 0usize;
    for inst in installations {
        let token = match ctx.github.installation_wide_token(inst.id).await {
            Ok(token) => token,
            Err(error) => {
                tracing::warn!(installation = inst.id, error = %error, "full-resync: installation token mint failed; skipping");
                continue;
            }
        };
        match ctx.listing.list_installation_repos(&token).await {
            Ok(repos) => {
                for repo in repos {
                    handle.enqueue((inst.id, repo));
                    enqueued += 1;
                }
            }
            Err(error) => {
                tracing::warn!(installation = inst.id, error = %error, "full-resync: list repos failed; skipping installation")
            }
        }
    }
    Ok(enqueued)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RepoRef;

    fn key(installation: i64, name: &str) -> RepoKey {
        (
            installation,
            RepoRef {
                owner: "acme".to_string(),
                name: name.to_string(),
            },
        )
    }

    #[tokio::test]
    async fn drain_pending_dedups_a_burst_into_one_batch() {
        let (tx, mut rx) = mpsc::channel::<RepoKey>(16);
        // Queue the same repo three times + a distinct one.
        tx.send(key(1, "site")).await.unwrap();
        tx.send(key(1, "site")).await.unwrap();
        tx.send(key(2, "other")).await.unwrap();
        // Pull the first off (as the loop does), then drain the rest.
        let first = rx.recv().await.unwrap();
        let batch = drain_pending(first, &mut rx);
        assert_eq!(batch.len(), 2, "duplicates collapse; distinct kept");
        assert!(batch.contains(&key(1, "site")));
        assert!(batch.contains(&key(2, "other")));
    }

    #[tokio::test]
    async fn drain_pending_of_a_single_key_is_just_that_key() {
        let (_tx, mut rx) = mpsc::channel::<RepoKey>(4);
        let batch = drain_pending(key(9, "solo"), &mut rx);
        assert_eq!(batch.len(), 1);
        assert!(batch.contains(&key(9, "solo")));
    }
}
