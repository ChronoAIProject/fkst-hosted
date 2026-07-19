//! In-memory, TTL-bounded cache of the redacted log bundle each session uploads
//! to chrono-storage at `logs/<session_id>/latest.tar.gz`.
//!
//! Why this exists: the log viewer ([`crate::routes::logs::viewer`]) reads a
//! session's bundle for the *manifest* AND again for *every single file* the
//! browser opens, and the whole-bundle download reads it once more. Without a
//! cache each of those calls re-runs [`crate::storage::ChronoStorageClient::download`]
//! — a full authenticated GET of the entire `tar.gz` from chrono-storage, which the
//! viewer then gunzips in memory. For a session whose logs are megabytes that is a
//! wasteful re-download + re-decompress on every click.
//!
//! The cache holds the raw (still-gzip'd) bundle bytes keyed by `session_id`, so a
//! burst of manifest/file requests for one session hits chrono-storage at most once
//! per TTL window. A cheap `Arc`-backed handle, exactly like
//! [`crate::log_access::LogAccessRegistry`]: cloning it shares the one backing store,
//! and it lives on [`crate::state::AppState`] (NOT a module-level global) so each test
//! gets its own isolated cache and nothing leaks across tests.
//!
//! Freshness contract: the producer flushes a new bundle roughly every ~20s (the
//! upload cadence), so a [`TTL`] of 30s means a viewer sees content at most one flush
//! stale — a deliberate trade of perfect freshness for far fewer storage round-trips.
//! Errors (404 / transport) are NEVER cached; only a successful download is stored, so
//! a not-yet-uploaded bundle keeps returning 404 until it actually exists.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;

/// How long a cached bundle is considered fresh. Matched to ~1.5× the producer's
/// ~20s upload cadence so the viewer is at most one flush stale. `pub(crate)` so the
/// `fetch_bundle`-level cache test can seed an entry exactly past this boundary.
pub(crate) const TTL: Duration = Duration::from_secs(30);

/// Hard ceiling on the number of distinct sessions cached at once. The cache only
/// needs to serve the handful of sessions being actively viewed; this cap keeps a
/// long-lived process from accumulating one entry per session ever downloaded. When a
/// NEW session would exceed the cap, the oldest entry is evicted first.
const MAX_ENTRIES: usize = 64;

/// A shared, in-memory `session_id -> (inserted_at, gzip bundle bytes)` cache. Cloning
/// it shares the same backing store (an `Arc`), so the viewer and whole-bundle download
/// paths hold independent handles onto one cache.
#[derive(Clone, Default)]
pub struct LogBundleCache {
    inner: Arc<Mutex<HashMap<String, (Instant, Bytes)>>>,
}

impl LogBundleCache {
    /// A fresh, empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached bundle for `session_id` when it is still fresh (inserted
    /// within [`TTL`]), else `None`. Public entry point — delegates to
    /// [`Self::get_at`] with the real clock so callers never pass a time.
    pub fn get(&self, session_id: &str) -> Option<Bytes> {
        self.get_at(session_id, Instant::now())
    }

    /// Insert or overwrite the bundle for `session_id`, evicting stale entries and
    /// enforcing the entry cap. Public entry point — delegates to [`Self::put_at`]
    /// with the real clock.
    pub fn put(&self, session_id: String, bytes: Bytes) {
        self.put_at(session_id, bytes, Instant::now());
    }

    /// [`Self::get`] with an explicit `now`, so a test can assert TTL expiry without
    /// sleeping. `Bytes` is `Arc`-backed, so the clone is a cheap refcount bump.
    /// `pub(crate)` purely as a deterministic test seam — production always goes
    /// through [`Self::get`].
    ///
    /// Poison-safe: a panic elsewhere while the lock was held never wedges the cache
    /// (the lock is recovered rather than propagated).
    pub(crate) fn get_at(&self, session_id: &str, now: Instant) -> Option<Bytes> {
        let map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let (inserted, bytes) = map.get(session_id)?;
        (now.saturating_duration_since(*inserted) < TTL).then(|| bytes.clone())
    }

    /// [`Self::put`] with an explicit `now`. Evicts every entry whose TTL has elapsed
    /// (so the map never grows unbounded from sessions that stopped being requested),
    /// then, if inserting a NEW key would exceed [`MAX_ENTRIES`], evicts the
    /// oldest-inserted entry to make room. `pub(crate)` purely as a deterministic test
    /// seam — production always goes through [`Self::put`].
    pub(crate) fn put_at(&self, session_id: String, bytes: Bytes, now: Instant) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, (inserted, _)| now.saturating_duration_since(*inserted) < TTL);
        if map.len() >= MAX_ENTRIES && !map.contains_key(&session_id) {
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (inserted, _))| *inserted)
                .map(|(key, _)| key.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(session_id, (now, bytes));
    }

    /// The number of bundles currently cached (diagnostics + tests).
    pub fn len(&self) -> usize {
        let map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.len()
    }

    /// Whether the cache is empty (diagnostics + tests).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for LogBundleCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Render only the size, never the bundle bytes (keeps a `{:?}` of AppState
        // cheap and avoids incidentally dumping raw log bytes into a log line).
        f.debug_struct("LogBundleCache")
            .field("sessions", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(marker: &str) -> Bytes {
        Bytes::from(marker.to_string())
    }

    #[test]
    fn get_within_ttl_hits() {
        let cache = LogBundleCache::new();
        let now = Instant::now();
        cache.put_at("sess-1".to_string(), bytes("bundle-1"), now);
        // A read one second later is still inside the 30s window.
        let got = cache
            .get_at("sess-1", now + Duration::from_secs(1))
            .expect("fresh entry hits");
        assert_eq!(got, bytes("bundle-1"));
    }

    #[test]
    fn get_after_ttl_misses() {
        let cache = LogBundleCache::new();
        let now = Instant::now();
        cache.put_at("sess-1".to_string(), bytes("bundle-1"), now);
        // Exactly at the TTL boundary is already stale (strict `<`).
        assert!(cache.get_at("sess-1", now + TTL).is_none());
        // Well past the TTL is stale too.
        assert!(cache
            .get_at("sess-1", now + Duration::from_secs(31))
            .is_none());
    }

    #[test]
    fn get_unknown_session_is_none() {
        let cache = LogBundleCache::new();
        assert!(cache.get_at("nope", Instant::now()).is_none());
    }

    #[test]
    fn put_overwrites_and_refreshes_ttl() {
        let cache = LogBundleCache::new();
        let now = Instant::now();
        cache.put_at("sess-1".to_string(), bytes("old"), now);
        // Re-put 20s later with new bytes: the entry is overwritten AND its clock reset.
        let later = now + Duration::from_secs(20);
        cache.put_at("sess-1".to_string(), bytes("new"), later);
        assert_eq!(cache.len(), 1, "same key overwrites, not appends");
        // 20s after the SECOND put (40s after the first) it is still fresh.
        let got = cache
            .get_at("sess-1", later + Duration::from_secs(20))
            .expect("refreshed entry still fresh");
        assert_eq!(got, bytes("new"));
    }

    #[test]
    fn put_evicts_stale_entries() {
        let cache = LogBundleCache::new();
        let now = Instant::now();
        cache.put_at("stale".to_string(), bytes("old"), now);
        // A later put past the first entry's TTL evicts it as a side effect.
        cache.put_at(
            "fresh".to_string(),
            bytes("new"),
            now + Duration::from_secs(31),
        );
        assert_eq!(cache.len(), 1, "the stale entry was swept on put");
        assert!(cache
            .get_at("stale", now + Duration::from_secs(31))
            .is_none());
    }

    #[test]
    fn put_enforces_entry_cap_by_evicting_oldest() {
        let cache = LogBundleCache::new();
        let base = Instant::now();
        // Fill to the cap, each a little newer than the last (all within one TTL).
        for i in 0..MAX_ENTRIES {
            cache.put_at(
                format!("sess-{i}"),
                bytes("b"),
                base + Duration::from_millis(i as u64),
            );
        }
        assert_eq!(cache.len(), MAX_ENTRIES);
        // One more NEW key (still fresh) must evict the oldest (`sess-0`), not grow.
        let newest = base + Duration::from_millis(MAX_ENTRIES as u64);
        cache.put_at("overflow".to_string(), bytes("b"), newest);
        assert_eq!(cache.len(), MAX_ENTRIES, "cap holds");
        assert!(
            cache.get_at("sess-0", newest).is_none(),
            "the oldest entry was evicted to make room"
        );
        assert!(
            cache.get_at("overflow", newest).is_some(),
            "the new entry is present"
        );
    }

    #[test]
    fn a_clone_shares_the_same_backing_store() {
        let cache = LogBundleCache::new();
        let handle = cache.clone();
        let now = Instant::now();
        handle.put_at("sess-1".to_string(), bytes("b"), now);
        assert!(
            cache.get_at("sess-1", now).is_some(),
            "a write through one handle is visible through another"
        );
    }

    #[test]
    fn debug_reports_only_the_size() {
        let cache = LogBundleCache::new();
        cache.put("sess-1".to_string(), bytes("secret-log-bytes"));
        let debug = format!("{cache:?}");
        assert!(debug.contains("sessions"), "{debug}");
        assert!(
            !debug.contains("secret-log-bytes"),
            "bundle bytes must not be dumped: {debug}"
        );
    }

    #[test]
    fn new_cache_is_empty() {
        let cache = LogBundleCache::new();
        assert!(cache.is_empty());
        cache.put("sess-1".to_string(), bytes("b"));
        assert!(!cache.is_empty());
    }
}
