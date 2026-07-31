//! Shared, cluster-free fixtures for this endpoint's tests.
//!
//! The handler tests and the installation-event tests both need an `AppState`
//! with a live reconcile queue. One copy lives here so the two files cannot drift
//! into two subtly different "app states".

use crate::config::Config;
use crate::models::RepoRef;
use crate::reconcile::{reconcile_channel, ReconcileDispatcher, RepoKey};
use crate::state::AppState;
use tokio::sync::mpsc::Receiver;

/// An `AppState` with a live reconcile queue (`github_app: None`, so the
/// `CacheBust` impl's eviction/fail steps are logged no-ops — no cluster
/// needed). Returns the queue receiver so a test can assert what was enqueued.
pub(super) fn state_with_reconciler() -> (AppState, Receiver<RepoKey>) {
    let (handle, rx) = reconcile_channel(16);
    let state = AppState {
        config: Config::default(),
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: Some(ReconcileDispatcher::from_handle(&handle)),
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: crate::state::empty_self_router(),
        chat: None,
        audit: Default::default(),
    };
    (state, rx)
}

pub(super) fn key(installation: i64, owner: &str, name: &str) -> RepoKey {
    (
        installation,
        RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        },
    )
}
