//! Loop-level tests: sweep key derivation, queue dedup, and how a per-repository
//! reconcile outcome reaches the session-access projection.

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::listing::{GithubListing, InstallationSummary, IssueSummary};
use crate::github_app::GithubAppError;
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::execute_test_support::{test_ctx, FakeSessionBackend};
use crate::reconcile::reconcile_channel;
use crate::session_access::{RegistryState, SessionAccessRegistry};
use crate::session_backend::SessionHandle;

/// Pre-satisfy the issue-template ensure gate for `key`, so a reconcile pass in
/// a test never reaches the template transport (which only the real HTTP client
/// implements). The ensure is best-effort and unrelated to what these tests
/// assert.
fn skip_template_ensure(ctx: &ReconcileCtx, key: &RepoKey) {
    ctx.ensured_templates
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            key.clone(),
            crate::reconcile::EnsuredMark {
                version: crate::github_app::templates::FKST_ISSUE_TEMPLATES_VERSION,
                checked_at: Instant::now(),
            },
        );
}

/// A listing transport whose issue enumeration always fails — the shape of a
/// repository with its Issues tab disabled (`410`), which no retry can fix.
struct AlwaysFailingListing;

#[async_trait]
impl GithubListing for AlwaysFailingListing {
    async fn list_issues_by_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError> {
        Err(GithubAppError::Http("issues are disabled".to_string()))
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
        Ok(Vec::new())
    }

    async fn list_installation_repos(
        &self,
        _token: &SecretString,
    ) -> Result<Vec<RepoRef>, GithubAppError> {
        Ok(Vec::new())
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

#[tokio::test]
async fn a_failed_repo_reconcile_releases_the_staged_session_access_generation() {
    // The loop's failure path is the only signal the projection ever gets that
    // a repository will not report: `reconcile_repo` `?`-returns long before it
    // publishes contexts. Without it, one permanently broken repository holds
    // the staged generation open and freezes every other repository's writes.
    let backend = Arc::new(FakeSessionBackend::default());
    let mut ctx = test_ctx(backend);
    ctx.listing = Arc::new(AlwaysFailingListing);
    ctx.session_access = SessionAccessRegistry::new(true);
    skip_template_ensure(&ctx, &key(1, "web"));
    ctx.session_access
        .begin_generation([key(1, "site"), key(1, "web")].into_iter().collect());
    assert_eq!(ctx.session_access.snapshot().pending_repositories, 2);

    reconcile_one(1, &key(1, "web").1, &ctx).await;

    assert_eq!(
        ctx.session_access.snapshot().pending_repositories,
        0,
        "the doomed generation must not keep swallowing writes"
    );
    assert_eq!(
        ctx.session_access.snapshot().state,
        RegistryState::Cold,
        "completeness is still unknown, so the projection stays fail-closed"
    );
}

#[tokio::test]
async fn a_successful_repo_reconcile_leaves_the_generation_pending() {
    // The counterpart: the ordinary fake reconciles cleanly, so `site` reports
    // and only `web` is still expected — no degradation.
    let backend = Arc::new(FakeSessionBackend::default());
    let mut ctx = test_ctx(backend);
    ctx.session_access = SessionAccessRegistry::new(true);
    skip_template_ensure(&ctx, &key(1, "site"));
    ctx.session_access
        .begin_generation([key(1, "site"), key(1, "web")].into_iter().collect());

    reconcile_one(1, &key(1, "site").1, &ctx).await;

    assert_eq!(ctx.session_access.snapshot().pending_repositories, 1);
    assert_eq!(
        ctx.session_access.snapshot().state,
        RegistryState::Recovering
    );
}
