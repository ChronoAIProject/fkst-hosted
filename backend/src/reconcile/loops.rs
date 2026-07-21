//! The reconciler's long-lived loops (issue #359 §4.2/§5.4, PR5b): the queue
//! consumer, the periodic pod sweep, and the periodic full resync.
//!
//! One `run_reconcile_loop` CONSUMER drains the queue and reconciles each repo
//! serially (deduping a burst of enqueues for the same repo into one pass); two
//! PRODUCERS keep the queue fed — `run_sweep_loop` re-enqueues every repo with a
//! live pod (so drift on an existing session is caught) and `run_full_resync_loop`
//! serially enumerates the App's installations + repos (so a repo with a pending
//! trigger but no pod yet is discovered). An incomplete full resync retries with
//! bounded backoff before returning to the ordinary periodic cadence.

use std::collections::HashSet;
use std::future::Future;
use std::time::{Duration, Instant};

use rand::Rng;
use tokio::sync::mpsc;

use crate::error::AppError;
use crate::reconcile::execute::ReconcileCtx;
use crate::reconcile::repo::reconcile_repo;
use crate::recovery::{RecoveryMonitor, ResyncResult};

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

/// Enumerate the substrate-session fleet (through the session backend), group the
/// handles into `(installation, repo)` keys, and enqueue each unique key. Returns how
/// many unique repos were enqueued.
async fn sweep_once(ctx: &ReconcileCtx, handle: &ReconcileHandle) -> Result<usize, AppError> {
    let fleet = ctx.backend.list_fleet().await.map_err(|e| {
        AppError::Internal(anyhow::anyhow!("sweep list substrate-session pods: {e}"))
    })?;
    // Dedup the fleet into unique reconcile keys (one repo may host several live
    // sessions; each maps to the same `(installation, repo)`).
    let mut keys: HashSet<RepoKey> = fleet
        .into_iter()
        .map(|h| (h.installation_id, h.repo))
        .collect();

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

/// One serialized full-resync coordinator. It attempts immediately, retries global
/// and partial failures with bounded exponential backoff, and waits for the ordinary
/// full-resync interval only after a complete pass. There is exactly one call to
/// [`full_resync_once`] in flight, so enumeration never overlaps within a process.
pub async fn run_full_resync_loop(
    ctx: ReconcileCtx,
    handle: ReconcileHandle,
    recovery: RecoveryMonitor,
) {
    let periodic_interval =
        Duration::from_secs(ctx.config.reconcile.pod_full_resync_interval_secs.max(1));
    let retry_initial_secs = ctx.config.reconcile.startup_resync_retry_initial_secs;
    let retry_max_secs = ctx.config.reconcile.startup_resync_retry_max_secs;
    let jitter_percent = ctx.config.reconcile.startup_resync_retry_jitter_percent;
    tracing::info!(
        ?periodic_interval,
        retry_initial_secs,
        retry_max_secs,
        jitter_percent,
        "reconcile full-resync: serialized coordinator started"
    );

    run_resync_coordinator(
        periodic_interval,
        retry_initial_secs,
        retry_max_secs,
        jitter_percent,
        recovery,
        move || {
            let ctx = ctx.clone();
            let handle = handle.clone();
            async move { full_resync_once(&ctx, &handle).await }
        },
    )
    .await;
}

/// Drive one full-resync attempt at a time. The attempt future must finish before
/// any delay begins, and the next attempt is not constructed until that delay ends;
/// this ordering is the process-local non-overlap guarantee.
async fn run_resync_coordinator<F, Fut>(
    periodic_interval: Duration,
    retry_initial_secs: u64,
    retry_max_secs: u64,
    jitter_percent: u64,
    recovery: RecoveryMonitor,
    mut attempt: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<FullResyncSummary, AppError>>,
{
    let mut retry = RetryBackoff::new(retry_initial_secs, retry_max_secs);
    loop {
        let started = Instant::now();
        match attempt().await {
            Ok(summary) if summary.is_complete() => {
                let duration = started.elapsed();
                recovery.record_attempt(
                    ResyncResult::Success,
                    duration,
                    summary.repositories_enqueued,
                );
                retry.reset();
                tracing::info!(
                    installations = summary.installations_total,
                    enqueued = summary.repositories_enqueued,
                    duration_ms = duration.as_millis(),
                    next_delay_secs = periodic_interval.as_secs(),
                    "reconcile full-resync: complete"
                );
                tokio::time::sleep(periodic_interval).await;
            }
            Ok(summary) => {
                let duration = started.elapsed();
                recovery.record_attempt(
                    ResyncResult::Partial,
                    duration,
                    summary.repositories_enqueued,
                );
                let base_delay = retry.next_delay();
                let delay = jittered_delay(base_delay, jitter_percent);
                tracing::warn!(
                    installations = summary.installations_total,
                    failed_installations = summary.installations_failed,
                    enqueued = summary.repositories_enqueued,
                    duration_ms = duration.as_millis(),
                    retry_base_secs = base_delay.as_secs(),
                    retry_delay_ms = delay.as_millis(),
                    "reconcile full-resync: partial pass; retrying"
                );
                tokio::time::sleep(delay).await;
            }
            Err(error) => {
                let duration = started.elapsed();
                recovery.record_attempt(ResyncResult::Failure, duration, 0);
                let base_delay = retry.next_delay();
                let delay = jittered_delay(base_delay, jitter_percent);
                tracing::warn!(
                    error = %error,
                    duration_ms = duration.as_millis(),
                    retry_base_secs = base_delay.as_secs(),
                    retry_delay_ms = delay.as_millis(),
                    "reconcile full-resync: failed; retrying"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RetryBackoff {
    initial_secs: u64,
    max_secs: u64,
    next_secs: u64,
}

impl RetryBackoff {
    fn new(initial_secs: u64, max_secs: u64) -> Self {
        let initial_secs = initial_secs.max(1);
        let max_secs = max_secs.max(initial_secs);
        Self {
            initial_secs,
            max_secs,
            next_secs: initial_secs,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let delay = self.next_secs;
        self.next_secs = self.next_secs.saturating_mul(2).min(self.max_secs);
        Duration::from_secs(delay)
    }

    fn reset(&mut self) {
        self.next_secs = self.initial_secs;
    }
}

fn jittered_delay(base: Duration, jitter_percent: u64) -> Duration {
    if jitter_percent == 0 || base.is_zero() {
        return base;
    }
    let base_ms = u64::try_from(base.as_millis()).unwrap_or(u64::MAX);
    let spread_ms = base_ms.saturating_mul(jitter_percent.min(100)) / 100;
    let lower = base_ms.saturating_sub(spread_ms);
    let upper = base_ms.saturating_add(spread_ms);
    Duration::from_millis(rand::thread_rng().gen_range(lower..=upper))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FullResyncSummary {
    installations_total: usize,
    installations_failed: usize,
    repositories_enqueued: usize,
}

impl FullResyncSummary {
    fn is_complete(&self) -> bool {
        self.installations_failed == 0
    }
}

/// Enumerate installations (App-JWT) and each installation's repos
/// (installation-wide token), enqueuing every repo. A per-installation failure is
/// logged and counted; only a failure to mint the App JWT or list installations
/// surfaces as `Err`. The summary makes a per-installation failure an incomplete
/// pass, so the coordinator retries promptly instead of waiting for the periodic
/// interval.
async fn full_resync_once(
    ctx: &ReconcileCtx,
    handle: &ReconcileHandle,
) -> Result<FullResyncSummary, AppError> {
    let app_jwt = ctx.github.app_jwt()?;
    let installations = ctx.listing.list_installations(&app_jwt).await?;

    let mut summary = FullResyncSummary {
        installations_total: installations.len(),
        ..FullResyncSummary::default()
    };
    for inst in installations {
        let token = match ctx.github.installation_wide_token(inst.id).await {
            Ok(token) => token,
            Err(error) => {
                summary.installations_failed += 1;
                tracing::warn!(installation = inst.id, error = %error, "full-resync: installation token mint failed; skipping");
                continue;
            }
        };
        match ctx.listing.list_installation_repos(&token).await {
            Ok(repos) => {
                for repo in repos {
                    handle.enqueue((inst.id, repo));
                    summary.repositories_enqueued += 1;
                }
            }
            Err(error) => {
                summary.installations_failed += 1;
                tracing::warn!(installation = inst.id, error = %error, "full-resync: list repos failed; skipping installation")
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::models::RepoRef;
    use crate::reconcile::execute_test_support::{test_ctx, FakeSessionBackend};
    use crate::reconcile::reconcile_channel;
    use crate::session_backend::SessionHandle;

    fn key(installation: i64, name: &str) -> RepoKey {
        (
            installation,
            RepoRef {
                owner: "acme".to_string(),
                name: name.to_string(),
            },
        )
    }

    fn fleet_handle(name: &str, installation: i64, issue: u64) -> SessionHandle {
        SessionHandle {
            session_id: format!("sess-{name}-{issue}"),
            installation_id: installation,
            repo: RepoRef {
                owner: "acme".to_string(),
                name: name.to_string(),
            },
            trigger_issue: Some(issue),
        }
    }

    #[tokio::test]
    async fn sweep_once_enqueues_one_key_per_distinct_repo() {
        // Two live sessions on `site` + one on `web`: the sweep collapses the two
        // `site` handles into one reconcile key and keeps `web` distinct.
        let fleet = vec![
            fleet_handle("site", 1, 7),
            fleet_handle("site", 1, 9),
            fleet_handle("web", 1, 3),
        ];
        let backend = Arc::new(FakeSessionBackend::default().with_fleet(fleet));
        let ctx = test_ctx(backend);
        let (tx, mut rx) = reconcile_channel(16);

        let enqueued = sweep_once(&ctx, &tx).await.expect("sweep ok");
        assert_eq!(enqueued, 2, "two distinct repos (the duplicate collapsed)");

        let mut got: HashSet<RepoKey> = HashSet::new();
        while let Ok(k) = rx.try_recv() {
            got.insert(k);
        }
        assert_eq!(got.len(), 2);
        assert!(got.contains(&key(1, "site")));
        assert!(got.contains(&key(1, "web")));
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
