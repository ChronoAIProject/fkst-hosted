//! Regression coverage for #3410: the GitHub token a session is HANDED must
//! outlive the wait for the rotation sweep that will replace it.
//!
//! The shared installation-token cache legally serves any token with more than its
//! 5-minute expiry buffer left — correct for the reconciler's own millisecond read
//! calls, and exactly wrong for a session, which holds the delivered token for as
//! long as it takes the fleet-wide sweep (`pod_token_refresh_secs`, 45 min by
//! default) to come around. A session that inherited such a cache entry started
//! returning `gh: Bad credentials (HTTP 401)` minutes after spawn and stayed broken
//! until that tick.
//!
//! These tests pin the invariant `delivered_ttl > pod_token_refresh_secs` at the one
//! place both delivery paths — spawn and crash-recovery, on either session backend —
//! funnel through: [`super::resolve_session_credentials`].

use std::time::{Duration, SystemTime};

use k8s_openapi::chrono::DateTime;

use super::*;
use crate::reconcile::execute_test_support::*;
use crate::session_spec::creds::GITHUB_TOKEN_FILE;

/// The remaining lifetime the incident's session was delivered (6 m 12 s): past the
/// cache's 5-minute buffer, so a cache hit was legal, and far short of the 45-minute
/// rotation interval, so nothing re-minted before it died.
const NEAR_EXPIRY: Duration = Duration::from_secs(372);

/// Seed the shared token cache with a near-expiry SESSION-scoped token, exactly as an
/// earlier reconciler call on the same repo does, and return the recording transport.
async fn github_with_a_near_expiry_cached_session_token(
) -> (Arc<RecordingApi>, GithubAppTokens, SystemTime) {
    let api = Arc::new(RecordingApi::default().with_mint_lifetimes([NEAR_EXPIRY]));
    let github = tokens(api.clone());
    let (_, expires_at) = github
        .token_with_expiry_for_repo("acme/site", Some(session_permissions()))
        .await
        .expect("cache priming mint");
    assert_eq!(
        api.mints_with_perms(&session_permissions()),
        1,
        "priming performed exactly one session-scoped mint"
    );
    (api, github, expires_at)
}

/// Remaining life of the session-scoped token the service would serve for the test
/// repo right now.
///
/// After a delivery this is the very token the session was handed: a forced mint
/// writes its fresh token into the cache, so an ordinary cached read returns it (and
/// mints nothing). Used by the action-level tests, which cannot see inside the
/// delivered bundle — the fake session backend deliberately records credential keys
/// only, never values.
async fn cached_session_token_remaining(github: &GithubAppTokens) -> Duration {
    let (_, expires_at) = github
        .token_with_expiry_for_repo("acme/site", Some(session_permissions()))
        .await
        .expect("cached read");
    expires_at
        .duration_since(SystemTime::now())
        .expect("cached session token must not already be expired")
}

/// Remaining life of the `github-token` credential in an assembled bundle.
fn delivered_remaining(creds: &BTreeMap<String, SecretString>) -> Duration {
    let raw = creds
        .get(GITHUB_TOKEN_FILE)
        .expect("bundle carries a github-token")
        .expose_secret();
    let parsed: serde_json::Value = serde_json::from_str(raw).expect("token json");
    let expires_at = DateTime::parse_from_rfc3339(
        parsed["expires_at"]
            .as_str()
            .expect("token json carries expires_at"),
    )
    .expect("rfc3339 expiry");
    let expires_at = SystemTime::UNIX_EPOCH
        + Duration::from_secs(u64::try_from(expires_at.timestamp()).expect("expiry after epoch"));
    expires_at
        .duration_since(SystemTime::now())
        .expect("delivered token must not already be expired")
}

#[tokio::test]
async fn delivered_session_token_outlives_the_rotation_interval() {
    let (api, github, cached_expiry) = github_with_a_near_expiry_cached_session_token().await;

    // The primed entry is one the cache would happily serve: still valid, still past
    // the 5-minute re-mint buffer — and nowhere near enough life for a session.
    let cached_remaining = cached_expiry
        .duration_since(SystemTime::now())
        .expect("primed token is still valid");
    assert!(
        cached_remaining > Duration::from_secs(300),
        "the primed token is a legal cache hit ({cached_remaining:?} left)"
    );

    let ctx = test_ctx_with_github(Arc::new(FakeSessionBackend::default()), github);
    let reg = registration();
    let Ok((_spec, creds)) =
        resolve_session_credentials(&reg, &["fkst-run".to_string()], &ctx).await
    else {
        panic!("credential resolution must succeed against the fake transport");
    };

    let interval = Duration::from_secs(ctx.config.reconcile.pod_token_refresh_secs);
    let remaining = delivered_remaining(&creds);
    assert!(
        remaining > interval,
        "the delivered token ({remaining:?}) must outlive the wait for the next \
         rotation sweep ({interval:?}); a cache hit would have delivered \
         {cached_remaining:?}"
    );
    assert_eq!(
        api.mints_with_perms(&session_permissions()),
        2,
        "delivery force-minted instead of reusing the near-expiry cache entry"
    );
}

#[tokio::test]
async fn spawn_delivers_a_freshly_minted_token() {
    let (api, github, _) = github_with_a_near_expiry_cached_session_token().await;
    let backend = Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx_with_github(backend.clone(), github);

    // Empty EFFECTIVE package set → the reachability pre-flight touches no network;
    // no named environment → no env-store read. The spawn then reaches the resolver.
    let mut reg = registration();
    reg.def.packages = Vec::new();
    reg.effective_packages = Vec::new();
    reg.def.environment = None;
    let repo = reg.repo.clone();

    execute(
        ReconcileAction::Spawn {
            reg,
            detected_work_labels: vec!["fkst-run".to_string()],
        },
        &repo,
        &ctx,
    )
    .await;

    assert_eq!(
        backend.ensured.lock().unwrap().len(),
        1,
        "the spawn reached ensure_session"
    );
    let interval = Duration::from_secs(ctx.config.reconcile.pod_token_refresh_secs);
    let remaining = cached_session_token_remaining(&ctx.github).await;
    assert!(
        remaining > interval,
        "the token the spawn delivered ({remaining:?}) must outlive the rotation \
         interval ({interval:?})"
    );
    assert_eq!(
        api.mints_with_perms(&session_permissions()),
        2,
        "exactly one mint beyond the priming one — the spawn's own, forced"
    );
}

#[tokio::test]
async fn crash_recovery_delivers_a_freshly_minted_token() {
    // The recovery path rehydrates a live runtime's bundle after the control plane
    // lost its process-local copy (the OpenSandbox backend's restart path). Restoring
    // a near-expiry token there is the same defect as delivering one at spawn.
    let (api, github, _) = github_with_a_near_expiry_cached_session_token().await;
    let backend = Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx_with_github(backend.clone(), github);
    let reg = registration();
    let repo = reg.repo.clone();

    execute(
        ReconcileAction::RecoverCredentials {
            reg,
            detected_work_labels: vec!["fkst-run".to_string()],
        },
        &repo,
        &ctx,
    )
    .await;

    assert_eq!(
        backend.ensured.lock().unwrap().len(),
        1,
        "recovery re-ensured the session with a rebuilt bundle"
    );
    let interval = Duration::from_secs(ctx.config.reconcile.pod_token_refresh_secs);
    let remaining = cached_session_token_remaining(&ctx.github).await;
    assert!(
        remaining > interval,
        "the token recovery rebuilt the bundle with ({remaining:?}) must outlive \
         the rotation interval ({interval:?})"
    );
    assert_eq!(
        api.mints_with_perms(&session_permissions()),
        2,
        "exactly one mint beyond the priming one — recovery's own, forced"
    );
}
