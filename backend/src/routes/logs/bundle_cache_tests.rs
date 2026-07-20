//! Cache-behaviour tests for [`super::fetch_bundle`]: prove that the TTL-bounded
//! [`crate::log_bundle_cache::LogBundleCache`] on `AppState` actually spares
//! chrono-storage a re-download when the same session's bundle is fetched again
//! within the TTL, and that a stale entry forces a fresh download.
//!
//! These drive the REAL `fetch_bundle` against a wiremock chrono-storage, counting
//! the download-route requests it receives — the concrete evidence that the manifest
//! + per-file viewer reads (each a `fetch_bundle` call) collapse onto one storage GET.

use std::time::{Duration, Instant};

use axum::body::Bytes;

use super::fetch_bundle;
use crate::log_bundle_cache::TTL;
use crate::routes::logs::test_support::{
    log_config, registry, state, storage_server, BUNDLE_BYTES, SESSION_ID,
};

/// The chrono-bucket content-read path every `download()` hits; counting these
/// isolates real storage round-trips from the (also-mocked) token-mint POST.
const DOWNLOAD_PATH: &str = "/api/buckets/logs/objects/download";

/// How many requests the mock storage server saw against the download route (only
/// `download()` targets this path — the token mint is a POST to `/oauth/token` — so a
/// path filter uniquely counts real bundle downloads).
async fn download_hits(server: &wiremock::MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path() == DOWNLOAD_PATH)
        .count()
}

#[tokio::test]
async fn second_fetch_within_ttl_serves_from_cache() {
    let (storage, server) = storage_server(true).await;
    let st = state(
        "https://unused".to_string(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    // First fetch: a cache miss → one real storage download, and it is cached.
    let first = fetch_bundle(&st, SESSION_ID, None).await.expect("first ok");
    assert_eq!(first.as_ref(), BUNDLE_BYTES);
    assert_eq!(download_hits(&server).await, 1, "first fetch hits storage");

    // Second fetch (real clock, microseconds later → well inside the 30s TTL): a
    // cache hit that must NOT touch storage again.
    let second = fetch_bundle(&st, SESSION_ID, None)
        .await
        .expect("second ok");
    assert_eq!(second.as_ref(), BUNDLE_BYTES);
    assert_eq!(
        download_hits(&server).await,
        1,
        "second fetch within TTL is served from cache, not re-downloaded"
    );
}

#[tokio::test]
async fn fetch_after_ttl_redownloads() {
    let (storage, server) = storage_server(true).await;
    let st = state(
        "https://unused".to_string(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    // Seed a deliberately STALE entry: inserted just past the TTL boundary, so the
    // next fetch's real-clock `get` treats it as expired. Deterministic, no sleeping.
    let stale_at = Instant::now()
        .checked_sub(TTL + Duration::from_secs(1))
        .expect("monotonic clock has more than TTL of history");
    st.log_bundle_cache.put_at(
        SESSION_ID.to_string(),
        Bytes::from_static(b"stale-bundle"),
        stale_at,
    );

    // The stale entry is ignored → a real download happens and returns the fresh bytes.
    let fetched = fetch_bundle(&st, SESSION_ID, None).await.expect("ok");
    assert_eq!(
        fetched.as_ref(),
        BUNDLE_BYTES,
        "stale cache is bypassed; the fresh bundle is returned"
    );
    assert_eq!(
        download_hits(&server).await,
        1,
        "an expired entry forces exactly one re-download"
    );

    // The re-download refreshed the cache: a follow-up fetch is a hit again.
    let again = fetch_bundle(&st, SESSION_ID, None).await.expect("ok");
    assert_eq!(again.as_ref(), BUNDLE_BYTES);
    assert_eq!(
        download_hits(&server).await,
        1,
        "the refreshed entry now serves from cache"
    );
}
