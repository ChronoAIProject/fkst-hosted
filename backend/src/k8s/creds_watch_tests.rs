//! Unit tests for the credentials watch DECISION (issue #5927): a live session
//! missing its credential bundle enqueues its repo; a healthy one does not; a probe
//! failure is swallowed. Runs against the shared [`FakeSessionBackend`], so no
//! network / cluster is touched and no GitHub call is made.

use super::*;
use crate::models::RepoRef;
use crate::reconcile::reconcile_channel;
use crate::session_backend::test_support::FakeSessionBackend;
use crate::session_backend::SessionHandle;

fn handle_for(session_id: &str, owner: &str, name: &str) -> SessionHandle {
    SessionHandle {
        session_id: session_id.to_string(),
        installation_id: 42,
        repo: RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        },
        trigger_issue: Some(7),
    }
}

#[tokio::test]
async fn a_session_missing_its_credentials_enqueues_its_repo() {
    let backend = FakeSessionBackend::default()
        .with_fleet(vec![handle_for("sess-a", "acme", "site")])
        .with_creds_probe("sess-a", Some(true));
    let (handle, mut rx) = reconcile_channel(8);

    watch_once(&backend, &handle).await.expect("sweep ok");

    let key = rx.try_recv().expect("repo enqueued for recovery");
    assert_eq!(key.0, 42);
    assert_eq!(key.1.owner, "acme");
    assert_eq!(key.1.name, "site");
}

#[tokio::test]
async fn a_session_with_credentials_present_enqueues_nothing() {
    let backend = FakeSessionBackend::default()
        .with_fleet(vec![handle_for("sess-a", "acme", "site")])
        .with_creds_probe("sess-a", Some(false));
    let (handle, mut rx) = reconcile_channel(8);

    watch_once(&backend, &handle).await.expect("sweep ok");

    assert!(
        rx.try_recv().is_err(),
        "a healthy session must cost one probe and no enqueue"
    );
}

#[tokio::test]
async fn a_probe_failure_is_swallowed_and_never_enqueues() {
    // The expected shape while a replacement pod is still starting: its execd is
    // not listening yet, so the proxy cannot connect. The next tick re-probes.
    let backend = FakeSessionBackend::default()
        .with_fleet(vec![handle_for("sess-a", "acme", "site")])
        .with_creds_probe("sess-a", None);
    let (handle, mut rx) = reconcile_channel(8);

    watch_once(&backend, &handle).await.expect("sweep still ok");

    assert!(rx.try_recv().is_err(), "a failed probe must not enqueue");
}

#[tokio::test]
async fn one_bad_session_never_stalls_the_rest_of_the_fleet() {
    let backend = FakeSessionBackend::default()
        .with_fleet(vec![
            handle_for("sess-bad", "acme", "bad"),
            handle_for("sess-needs", "acme", "needs"),
        ])
        .with_creds_probe("sess-bad", None)
        .with_creds_probe("sess-needs", Some(true));
    let (handle, mut rx) = reconcile_channel(8);

    watch_once(&backend, &handle).await.expect("sweep ok");

    let key = rx
        .try_recv()
        .expect("the healthy-probe session still enqueued");
    assert_eq!(key.1.name, "needs");
}

#[tokio::test]
async fn a_fleet_listing_failure_surfaces_as_an_error() {
    let backend = FakeSessionBackend::default().with_list_failures(1);
    let (handle, _rx) = reconcile_channel(8);

    assert!(
        watch_once(&backend, &handle).await.is_err(),
        "only a failure to LIST the fleet surfaces as Err"
    );
}
