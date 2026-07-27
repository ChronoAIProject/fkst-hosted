//! Tests for the token-rotation sweep: the fleet fan-out, the transient-vs-permanent
//! failure split, and the retry cadence (#3822) that keeps ONE dropped sweep from
//! costing a session ~30 minutes of expired credentials.
//!
//! The mint runs against a fake GitHub transport and the delivery against the shared
//! recording [`FakeSessionBackend`], so no cluster is touched. The loop-level tests run
//! on a PAUSED tokio clock: every `sleep` is virtual, so a test can assert "the retry
//! happened within a minute of virtual time" deterministically and instantly.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::config::GithubAppConfig;
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::models::RepoRef;
use crate::reconcile::reconcile_channel;
use crate::session_backend::test_support::FakeSessionBackend;
use crate::session_backend::SessionHandle;
use crate::session_spec::creds::GITHUB_TOKEN_FILE;

/// A GitHub transport that mints a fake token, optionally failing every mint with a
/// scripted error so the transient/permanent split can be exercised.
#[derive(Default)]
struct ScriptedApi {
    mint_count: AtomicUsize,
    /// Served to every mint while present; absent → the mint succeeds.
    mint_error: Mutex<Option<GithubAppError>>,
}

impl ScriptedApi {
    fn always_failing(error: GithubAppError) -> Self {
        Self {
            mint_error: Mutex::new(Some(error)),
            ..Self::default()
        }
    }

    fn mint_count(&self) -> usize {
        self.mint_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl GithubApi for ScriptedApi {
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        Ok(InstallationId(1))
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        _id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        self.mint_count.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.mint_error.lock().unwrap().clone() {
            return Err(error);
        }
        Ok(InstallationToken {
            token: SecretString::from("ghs_fake".to_string()),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        })
    }

    async fn create_issue_comment(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _body: &str,
    ) -> Result<(), GithubAppError> {
        Ok(())
    }

    async fn add_issue_labels(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _labels: &[String],
    ) -> Result<(), GithubAppError> {
        Ok(())
    }

    async fn remove_issue_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _label: &str,
    ) -> Result<(), GithubAppError> {
        Ok(())
    }

    async fn get_issue_labels(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
    ) -> Result<Vec<String>, GithubAppError> {
        Ok(Vec::new())
    }
}

fn test_config() -> GithubAppConfig {
    use rand::rngs::OsRng;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;
    let mut rng = OsRng;
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("key");
    let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");
    GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(pem.to_string()),
        app_slug: Some("fkst-test".to_string()),
        webhook_secret: None,
        api_base: "https://api.github.com".to_string(),
    }
}

fn tokens() -> GithubAppTokens {
    tokens_with(Arc::new(ScriptedApi::default()))
}

fn tokens_with(api: Arc<ScriptedApi>) -> GithubAppTokens {
    GithubAppTokens::with_api(&test_config(), api).expect("tokens")
}

fn handle_for(session_id: &str, name: &str, issue: Option<u64>) -> SessionHandle {
    SessionHandle {
        session_id: session_id.to_string(),
        installation_id: 1,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: name.to_string(),
        },
        trigger_issue: issue,
    }
}

/// The default cadence the loop falls back to between successful passes. A retry that
/// is not far below this is no better than the bug being fixed.
fn periodic_interval() -> Duration {
    Duration::from_secs(ReconcileConfig::default().pod_token_refresh_secs)
}

// ---- sweep fan-out ----------------------------------------------------------

#[tokio::test]
async fn rotation_delivers_a_credential_to_every_fleet_handle() {
    let github = tokens();
    let (handle, _rx) = reconcile_channel(16);
    let fleet = vec![
        handle_for("sess-1", "site", Some(7)),
        handle_for("sess-2", "web", Some(8)),
    ];
    let backend = FakeSessionBackend::default().with_fleet(fleet);

    let summary = rotate_once(&backend, &github, &handle)
        .await
        .expect("sweep ok");

    assert!(summary.retryable.is_empty(), "nothing to retry");
    assert_eq!(summary.attempted.len(), 2);
    let delivered = backend.delivered.lock().unwrap();
    assert_eq!(delivered.len(), 2, "one delivery per fleet handle");
    // Every delivery targets the rotating github-token credential file.
    assert!(delivered.iter().all(|(_, file)| file == GITHUB_TOKEN_FILE));
    let ids: Vec<&str> = delivered.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"sess-1") && ids.contains(&"sess-2"));
}

#[tokio::test]
async fn rotation_tolerates_a_gone_session() {
    let github = tokens();
    let (handle, _rx) = reconcile_channel(16);
    let fleet = vec![handle_for("sess-gone", "site", Some(7))];
    let backend = FakeSessionBackend::default()
        .with_fleet(fleet)
        .with_gone("sess-gone");

    let summary = rotate_once(&backend, &github, &handle)
        .await
        .expect("sweep ok despite a gone session");

    // A vanished runtime needs no token: complete, and NOT queued for retry.
    assert!(
        summary.retryable.is_empty(),
        "a gone session is not a failure"
    );
    assert_eq!(
        backend.delivered.lock().unwrap().len(),
        1,
        "delivery was still attempted"
    );
}

// ---- transient vs permanent -------------------------------------------------

#[tokio::test]
async fn a_transient_delivery_failure_is_reported_as_retryable() {
    let github = tokens();
    let (handle, _rx) = reconcile_channel(16);
    let fleet = vec![
        handle_for("sess-ok", "site", Some(7)),
        handle_for("sess-flaky", "web", Some(8)),
    ];
    let backend = FakeSessionBackend::default()
        .with_fleet(fleet)
        .with_deliver_failures("sess-flaky", 1);

    let summary = rotate_once(&backend, &github, &handle)
        .await
        .expect("the sweep itself succeeds");

    assert!(
        !summary.retryable.is_empty(),
        "one session still needs a token"
    );
    assert_eq!(summary.permanent, 0);
    let retryable: Vec<&str> = summary
        .retryable
        .iter()
        .map(|s| s.session_id.as_str())
        .collect();
    assert_eq!(
        retryable,
        vec!["sess-flaky"],
        "only the failed session is queued; the healthy one is done"
    );
}

#[tokio::test]
async fn a_repair_pass_rotates_only_the_failed_session() {
    // The point of returning the failed handles rather than re-sweeping: a retry costs
    // one mint per still-broken session, not one per session in the fleet.
    let api = Arc::new(ScriptedApi::default());
    let github = tokens_with(api.clone());
    let (handle, _rx) = reconcile_channel(16);
    let fleet = vec![
        handle_for("sess-ok", "site", Some(7)),
        handle_for("sess-flaky", "web", Some(8)),
    ];
    let backend = FakeSessionBackend::default()
        .with_fleet(fleet)
        .with_deliver_failures("sess-flaky", 1);

    let first = rotate_once(&backend, &github, &handle)
        .await
        .expect("first sweep");
    let mints_after_sweep = api.mint_count();
    assert_eq!(
        mints_after_sweep, 2,
        "one mint per session in the full sweep (distinct repos, so no cache sharing)"
    );

    let repair = rotate_sessions(&backend, &github, &handle, first.retryable).await;

    assert!(repair.retryable.is_empty(), "the retry converged");
    assert_eq!(
        repair.attempted.len(),
        1,
        "only the failed session was retried"
    );
    assert_eq!(
        api.mint_count(),
        mints_after_sweep + 1,
        "the repair pass minted once, not once per fleet member"
    );
    let delivered = backend.delivered.lock().unwrap();
    assert_eq!(delivered.len(), 3, "2 in the sweep + 1 in the repair");
    assert_eq!(delivered[2].0, "sess-flaky");
}

#[tokio::test]
async fn a_permanent_mint_failure_is_not_retried_and_enqueues_the_repo() {
    // A suspended installation cannot be fixed by trying again: retrying it every
    // backoff tick would burn the App's API budget forever. It is surfaced and handed
    // to the reconciler, which kills the orphaned session.
    let api = Arc::new(ScriptedApi::always_failing(GithubAppError::AppAuth));
    let github = tokens_with(api);
    let (handle, mut rx) = reconcile_channel(16);
    let backend =
        FakeSessionBackend::default().with_fleet(vec![handle_for("sess-1", "site", Some(7))]);

    let summary = rotate_once(&backend, &github, &handle)
        .await
        .expect("sweep ok");

    assert!(
        summary.retryable.is_empty(),
        "a permanent failure must NOT keep the retry loop alive"
    );
    assert_eq!(summary.permanent, 1);
    assert!(
        backend.delivered.lock().unwrap().is_empty(),
        "nothing was delivered — there was no token to deliver"
    );
    let (installation_id, repo) = rx.try_recv().expect("repo enqueued for reconcile");
    assert_eq!(installation_id, 1);
    assert_eq!(repo.name, "site");
}

#[tokio::test]
async fn a_transient_mint_failure_is_retried_rather_than_enqueued() {
    let api = Arc::new(ScriptedApi::always_failing(GithubAppError::RateLimited(30)));
    let github = tokens_with(api);
    let (handle, mut rx) = reconcile_channel(16);
    let backend =
        FakeSessionBackend::default().with_fleet(vec![handle_for("sess-1", "site", Some(7))]);

    let summary = rotate_once(&backend, &github, &handle)
        .await
        .expect("sweep ok");

    assert!(
        !summary.retryable.is_empty(),
        "a rate limit is worth retrying"
    );
    assert_eq!(summary.permanent, 0);
    assert!(
        rx.try_recv().is_err(),
        "a transient failure must not kill the session"
    );
}

#[test]
fn mint_failure_classification_defaults_to_retryable() {
    // Permanent: the App has lost its entitlement to mint for this repo.
    for error in [
        GithubAppError::InstallationGone {
            owner_repo: "acme/site".to_string(),
        },
        GithubAppError::NotInstalled {
            owner_repo: "acme/site".to_string(),
            install_url: None,
        },
        GithubAppError::AppAuth,
        GithubAppError::InvalidKey,
        GithubAppError::InvalidRepoRef,
        GithubAppError::TokenRequestRejected("missing contents".to_string()),
    ] {
        assert!(
            is_permanent_mint_failure(&error),
            "{error} must not be retried"
        );
    }
    // Everything else — including anything added later — is retried. A
    // misclassification then costs bounded extra mints, not an abandoned session.
    for error in [
        GithubAppError::RateLimited(60),
        GithubAppError::Http("connection reset".to_string()),
        GithubAppError::BlobTooLarge,
    ] {
        assert!(
            !is_permanent_mint_failure(&error),
            "{error} must be retried"
        );
    }
}

// ---- retry cadence (the #3822 regression) -----------------------------------

#[tokio::test(start_paused = true)]
async fn a_failed_delivery_is_retried_long_before_the_periodic_interval() {
    // THE REGRESSION. Before #3822 a failed delivery was logged and dropped, so the
    // session waited a full 2700 s for the next tick — by which point its token (a
    // 3600 s TTL last refreshed 2700 s ago) had been dead for ~30 minutes.
    let backend = Arc::new(
        FakeSessionBackend::default()
            .with_fleet(vec![handle_for("sess-1", "site", Some(7))])
            .with_deliver_failures("sess-1", 1),
    );
    let (handle, _rx) = reconcile_channel(16);
    tokio::spawn(run_token_rotation_loop(
        backend.clone(),
        tokens(),
        ReconcileConfig::default(),
        handle,
    ));

    // One virtual minute: far below the 2700 s cadence, far above the 15 s retry floor.
    tokio::time::sleep(Duration::from_secs(60)).await;

    assert_eq!(
        backend.delivered.lock().unwrap().len(),
        2,
        "the failed delivery was retried within a minute, not deferred a full {:?}",
        periodic_interval()
    );
}

#[tokio::test(start_paused = true)]
async fn a_failed_fleet_list_retries_the_whole_sweep() {
    // A failed list rotates NOBODY, so the entire fleet is exposed. The retry must
    // re-run the whole sweep rather than waiting out the cadence.
    let backend = Arc::new(
        FakeSessionBackend::default()
            .with_fleet(vec![
                handle_for("sess-1", "site", Some(7)),
                handle_for("sess-2", "web", Some(8)),
            ])
            .with_list_failures(1),
    );
    let (handle, _rx) = reconcile_channel(16);
    tokio::spawn(run_token_rotation_loop(
        backend.clone(),
        tokens(),
        ReconcileConfig::default(),
        handle,
    ));

    tokio::time::sleep(Duration::from_secs(60)).await;

    assert_eq!(
        backend.delivered.lock().unwrap().len(),
        2,
        "the whole fleet was rotated by the retry sweep"
    );
}

#[tokio::test(start_paused = true)]
async fn a_healthy_sweep_waits_the_full_interval() {
    // The retry path must not become a busy loop: with nothing failing, the loop
    // rotates once at startup and then sleeps the ordinary cadence.
    let backend = Arc::new(FakeSessionBackend::default().with_fleet(vec![handle_for(
        "sess-1",
        "site",
        Some(7),
    )]));
    let (handle, _rx) = reconcile_channel(16);
    tokio::spawn(run_token_rotation_loop(
        backend.clone(),
        tokens(),
        ReconcileConfig::default(),
        handle,
    ));

    tokio::time::sleep(periodic_interval() - Duration::from_secs(1)).await;

    assert_eq!(
        backend.delivered.lock().unwrap().len(),
        1,
        "exactly the immediate startup sweep — no extra rotations before the next tick"
    );
}

#[tokio::test(start_paused = true)]
async fn a_chronically_broken_session_does_not_slow_another_sessions_first_retry() {
    // The subtler sibling of the starvation hazard, and the reason each session carries
    // its OWN backoff. With a single fleet-wide counter, sess-stuck ratchets it to the
    // 2700 s ceiling within one cycle; sess-late's FIRST failure would then inherit that
    // delay and wait ~45 minutes for a retry — which is exactly the dead window this
    // whole loop exists to close, reintroduced through the back door.
    let backend = Arc::new(
        FakeSessionBackend::default()
            .with_fleet(vec![
                handle_for("sess-stuck", "site", Some(7)),
                handle_for("sess-late", "web", Some(8)),
            ])
            .with_deliver_failures("sess-stuck", usize::MAX),
    );
    let (handle, _rx) = reconcile_channel(64);
    tokio::spawn(run_token_rotation_loop(
        backend.clone(),
        tokens(),
        ReconcileConfig::default(),
        handle,
    ));

    // Let sess-stuck fail through a whole cycle of repair passes — long enough that a
    // shared backoff would be pinned at its 2700 s ceiling.
    tokio::time::sleep(periodic_interval() - Duration::from_secs(10)).await;
    let before = late_attempts(&backend);

    // Arm sess-late to fail on the full sweep that is about to run, then give it two
    // virtual minutes. Its FIRST retry must come at the 15 s floor of its own backoff,
    // not at whatever ceiling sess-stuck has climbed to.
    backend.fail_next_deliveries("sess-late", 1);
    tokio::time::sleep(Duration::from_secs(130)).await;

    let after = late_attempts(&backend);
    assert!(
        after >= before + 2,
        "sess-late's failed delivery must be retried on its own backoff within ~15 s; \
         attempts went {before} -> {after} (a shared backoff would defer it ~{:?})",
        periodic_interval()
    );
}

/// Rotation attempts recorded for `sess-late`.
fn late_attempts(backend: &FakeSessionBackend) -> usize {
    backend
        .delivered
        .lock()
        .unwrap()
        .iter()
        .filter(|(id, _)| id == "sess-late")
        .count()
}

#[tokio::test(start_paused = true)]
async fn a_permanently_broken_session_does_not_starve_the_rest_of_the_fleet() {
    // The hazard the repair path introduces: if the loop only ever retried the failed
    // set, ONE session that never recovers would keep the backlog non-empty forever and
    // no full sweep would run again — so every OTHER session's token would quietly
    // expire. That is strictly worse than the gap this loop exists to close, so a full
    // sweep is due every interval no matter how the repairs are going.
    let backend = Arc::new(
        FakeSessionBackend::default()
            .with_fleet(vec![
                handle_for("sess-stuck", "site", Some(7)),
                handle_for("sess-ok", "web", Some(8)),
            ])
            // Never recovers, so every repair pass leaves the backlog non-empty.
            .with_deliver_failures("sess-stuck", usize::MAX),
    );
    let (handle, _rx) = reconcile_channel(64);
    tokio::spawn(run_token_rotation_loop(
        backend.clone(),
        tokens(),
        ReconcileConfig::default(),
        handle,
    ));

    // Just past one full cadence: the startup sweep plus the next scheduled one.
    tokio::time::sleep(periodic_interval() + Duration::from_secs(60)).await;

    let healthy_rotations = backend
        .delivered
        .lock()
        .unwrap()
        .iter()
        .filter(|(id, _)| id == "sess-ok")
        .count();
    assert!(
        healthy_rotations >= 2,
        "the healthy session must keep rotating on the ordinary cadence while a \
         stuck sibling is being repaired; got {healthy_rotations} rotation(s)"
    );
}

#[tokio::test(start_paused = true)]
async fn a_permanently_failing_session_is_not_retried_in_a_loop() {
    // The API-budget guard: a permanent failure must not be re-minted every backoff
    // tick. One mint, one enqueue, then the ordinary cadence.
    let api = Arc::new(ScriptedApi::always_failing(GithubAppError::AppAuth));
    let backend = Arc::new(FakeSessionBackend::default().with_fleet(vec![handle_for(
        "sess-1",
        "site",
        Some(7),
    )]));
    let (handle, _rx) = reconcile_channel(64);
    tokio::spawn(run_token_rotation_loop(
        backend.clone(),
        tokens_with(api.clone()),
        ReconcileConfig::default(),
        handle,
    ));

    tokio::time::sleep(periodic_interval() - Duration::from_secs(1)).await;

    assert_eq!(
        api.mint_count(),
        1,
        "the permanently-failing session was minted once, not once per backoff tick"
    );
}
