//! Deterministic recovery tests for the serialized full-resync coordinator.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;
use tokio::sync::Notify;

use super::*;
use crate::github_app::listing::{GithubListing, InstallationSummary, IssueSummary};
use crate::github_app::GithubAppError;
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::execute_test_support::{test_ctx, FakeSessionBackend};
use crate::reconcile::reconcile_channel;

async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
    for _ in 0..32 {
        if counter.load(Ordering::SeqCst) == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(counter.load(Ordering::SeqCst), expected);
}

async fn yield_and_assert_count(counter: &AtomicUsize, expected: usize) {
    tokio::task::yield_now().await;
    assert_eq!(counter.load(Ordering::SeqCst), expected);
}

fn complete_summary(repositories_enqueued: usize) -> FullResyncSummary {
    FullResyncSummary {
        installations_total: 1,
        installations_failed: 0,
        repositories_enqueued,
    }
}

#[tokio::test(start_paused = true)]
async fn coordinator_is_immediate_retries_exactly_and_resets_after_success() {
    let calls = Arc::new(AtomicUsize::new(0));
    let task_calls = calls.clone();
    let recovery = RecoveryMonitor::new(true);
    let task_recovery = recovery.clone();

    let task = tokio::spawn(async move {
        run_resync_coordinator(
            Duration::from_secs(100),
            5,
            60,
            0,
            task_recovery,
            move || {
                let attempt = task_calls.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    match attempt {
                        // The partial pass proves it follows the same retry schedule
                        // as a global error while retaining its enqueue count.
                        2 => Ok(FullResyncSummary {
                            installations_total: 2,
                            installations_failed: 1,
                            repositories_enqueued: 3,
                        }),
                        7 | 9 => Ok(complete_summary(5)),
                        _ => Err(AppError::Unavailable(
                            "transient GitHub failure".to_string(),
                        )),
                    }
                }
            },
        )
        .await;
    });

    // The first attempt happens before any timer is advanced.
    wait_for_count(&calls, 1).await;

    tokio::time::advance(Duration::from_secs(4)).await;
    yield_and_assert_count(&calls, 1).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_count(&calls, 2).await;

    tokio::time::advance(Duration::from_secs(9)).await;
    yield_and_assert_count(&calls, 2).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_count(&calls, 3).await;

    tokio::time::advance(Duration::from_secs(19)).await;
    yield_and_assert_count(&calls, 3).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_count(&calls, 4).await;

    tokio::time::advance(Duration::from_secs(39)).await;
    yield_and_assert_count(&calls, 4).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_count(&calls, 5).await;

    tokio::time::advance(Duration::from_secs(59)).await;
    yield_and_assert_count(&calls, 5).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_count(&calls, 6).await;

    // The cap holds at 60 seconds for the next retry.
    tokio::time::advance(Duration::from_secs(59)).await;
    yield_and_assert_count(&calls, 6).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_count(&calls, 7).await;

    // A complete pass enters the ordinary cadence rather than retry cadence.
    tokio::time::advance(Duration::from_secs(99)).await;
    yield_and_assert_count(&calls, 7).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_count(&calls, 8).await;

    // That later periodic failure uses the reset initial delay of five seconds.
    tokio::time::advance(Duration::from_secs(4)).await;
    yield_and_assert_count(&calls, 8).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_count(&calls, 9).await;

    let snapshot = recovery.snapshot();
    assert_eq!(snapshot.attempts.success, 2);
    assert_eq!(snapshot.attempts.partial, 1);
    assert_eq!(snapshot.attempts.failure, 6);
    assert!(snapshot.ready);
    assert!(snapshot.startup_resync_complete);

    task.abort();
    let _ = task.await;
}

#[tokio::test(start_paused = true)]
async fn coordinator_never_overlaps_a_blocked_attempt() {
    let calls = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_in_flight = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(Notify::new());

    let task_calls = calls.clone();
    let task_in_flight = in_flight.clone();
    let task_max = max_in_flight.clone();
    let task_release = release.clone();
    let task = tokio::spawn(async move {
        run_resync_coordinator(
            Duration::from_secs(30),
            5,
            60,
            0,
            RecoveryMonitor::new(true),
            move || {
                let calls = task_calls.clone();
                let in_flight = task_in_flight.clone();
                let max_in_flight = task_max.clone();
                let release = task_release.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let active = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_in_flight.fetch_max(active, Ordering::SeqCst);
                    release.notified().await;
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    Ok(complete_summary(1))
                }
            },
        )
        .await;
    });

    wait_for_count(&calls, 1).await;
    tokio::time::advance(Duration::from_secs(3_600)).await;
    yield_and_assert_count(&calls, 1).await;
    assert_eq!(max_in_flight.load(Ordering::SeqCst), 1);

    release.notify_one();
    for _ in 0..32 {
        if in_flight.load(Ordering::SeqCst) == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(in_flight.load(Ordering::SeqCst), 0);

    tokio::time::advance(Duration::from_secs(30)).await;
    wait_for_count(&calls, 2).await;
    assert_eq!(max_in_flight.load(Ordering::SeqCst), 1);

    task.abort();
    let _ = task.await;
}

struct SequencedListing {
    installations: Vec<InstallationSummary>,
    repo_results: Mutex<VecDeque<Result<Vec<RepoRef>, GithubAppError>>>,
    installation_calls: AtomicUsize,
}

#[async_trait]
impl GithubListing for SequencedListing {
    async fn list_issues_by_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError> {
        Ok(Vec::new())
    }

    async fn count_open_issues_with_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
    ) -> Result<u64, GithubAppError> {
        Ok(0)
    }

    async fn list_installations(
        &self,
        _app_jwt: &SecretString,
    ) -> Result<Vec<InstallationSummary>, GithubAppError> {
        self.installation_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.installations.clone())
    }

    async fn list_installation_repos(
        &self,
        _token: &SecretString,
    ) -> Result<Vec<RepoRef>, GithubAppError> {
        self.repo_results
            .lock()
            .unwrap()
            .pop_front()
            .expect("one programmed repo-list result per installation")
    }

    async fn list_repo_admins(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<Vec<GithubActor>, GithubAppError> {
        Ok(Vec::new())
    }
}

fn repo(name: &str) -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: name.to_string(),
    }
}

#[tokio::test]
async fn partial_pass_keeps_successes_enqueued_and_retries_failed_installations() {
    let listing = Arc::new(SequencedListing {
        installations: vec![
            InstallationSummary {
                id: 1,
                account_login: "acme".to_string(),
            },
            InstallationSummary {
                id: 2,
                account_login: "other".to_string(),
            },
        ],
        repo_results: Mutex::new(VecDeque::from([
            Ok(vec![repo("site")]),
            Err(GithubAppError::Http(
                "temporary repo-list failure".to_string(),
            )),
            Ok(vec![repo("site")]),
            Ok(vec![repo("docs")]),
        ])),
        installation_calls: AtomicUsize::new(0),
    });
    let backend = Arc::new(FakeSessionBackend::default());
    let mut ctx = test_ctx(backend);
    ctx.listing = listing.clone();
    let (handle, mut rx) = reconcile_channel(16);

    let first = full_resync_once(&ctx, &handle).await.expect("partial pass");
    assert_eq!(
        first,
        FullResyncSummary {
            installations_total: 2,
            installations_failed: 1,
            repositories_enqueued: 1,
        }
    );
    assert_eq!(
        rx.try_recv().expect("successful repo enqueued"),
        (1, repo("site"))
    );
    assert!(rx.try_recv().is_err());

    let retry = full_resync_once(&ctx, &handle).await.expect("retry pass");
    assert!(retry.is_complete());
    assert_eq!(retry.repositories_enqueued, 2);
    assert_eq!(
        rx.try_recv().expect("successful repo re-enqueued"),
        (1, repo("site"))
    );
    assert_eq!(
        rx.try_recv().expect("failed installation retried"),
        (2, repo("docs"))
    );
    assert!(rx.try_recv().is_err());
    assert_eq!(listing.installation_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn retry_backoff_is_bounded_and_resettable() {
    let mut backoff = RetryBackoff::new(5, 60);
    let delays: Vec<u64> = (0..7).map(|_| backoff.next_delay().as_secs()).collect();
    assert_eq!(delays, vec![5, 10, 20, 40, 60, 60, 60]);
    backoff.reset();
    assert_eq!(backoff.next_delay(), Duration::from_secs(5));
}
