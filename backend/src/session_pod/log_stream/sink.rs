//! The log-bundle SINK: the single swap-point behind which the collector uploads
//! its `tar.gz`.
//!
//! The collector's job ends at "produce a redacted `tar.gz`"; WHERE that bundle
//! goes is one narrow trait, [`LogSink`], so the destination (chrono-storage today,
//! anything tomorrow) is a plug-and-play implementation the collector never names
//! directly. The production impl is [`ChronoStorageSink`], which PUTs the bundle
//! through the reopened [`ChronoStorageClient`] as the mounted storage SA — the
//! sink itself only ever uploads.
//!
//! Secret hygiene: an upload failure is reduced to [`SinkError`], whose `Display`
//! carries only the leak-free [`StorageError`] rendering (a numeric HTTP status or
//! a URL-free transport category) — never the bearer token, the client secret, a
//! signed URL, or a response body (asserted by the `errors_never_leak…` test).

use async_trait::async_trait;
use axum::body::Bytes;

use crate::session_spec::creds::CredsLayout;
use crate::storage::{ChronoStorageClient, ChronoStorageConfig, StorageError};

/// The content type every log bundle is uploaded with.
pub const BUNDLE_CONTENT_TYPE: &str = "application/gzip";

/// A sink failure. Deliberately narrow: it carries at most the leak-free
/// [`crate::storage::StorageError`] rendering, so logging or rendering it can never
/// spill a credential or a signed URL.
#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    /// A bundle transfer failed — an upload PUT, or a run-index read GET (both go
    /// through this one variant). The detail is the (leak-free) storage-error
    /// string — a numeric status or a URL-free transport category only.
    #[error("log bundle upload failed: {0}")]
    Upload(String),
}

/// Where a redacted log bundle is written. The single swap-point: the collector
/// depends only on this trait, so the concrete destination is interchangeable.
#[async_trait]
pub trait LogSink: Send + Sync {
    /// Upload the gzip'd bundle `gz` under object `key`. Best-effort at the call
    /// site (the collector logs + swallows an `Err`); an implementation must never
    /// leak a credential into the returned error.
    async fn put(&self, key: &str, gz: Bytes) -> Result<(), SinkError>;

    /// Read the object at `key`, or `Ok(None)` when it does not exist (a `404`).
    /// Used for the run-index read-modify-write: the collector `get`s the current
    /// index, folds its run in, and `put`s it back. Best-effort at the call site;
    /// like [`Self::put`], an implementation must never leak a credential into the
    /// returned error.
    async fn get(&self, key: &str) -> Result<Option<Bytes>, SinkError>;
}

/// The production [`LogSink`]: uploads each bundle to chrono-storage as the mounted
/// storage SA via [`ChronoStorageClient::upload`].
pub struct ChronoStorageSink {
    client: ChronoStorageClient,
}

impl ChronoStorageSink {
    /// Wrap an already-built client.
    pub fn new(client: ChronoStorageClient) -> Self {
        Self { client }
    }

    /// Build the sink from the storage SA creds mounted under `creds`
    /// (`storage-client-id` / `storage-client-secret` / `storage-token-url` +
    /// non-secret `storage-base-url` / `storage-bucket`), mirroring how the
    /// collector reads `github-token`.
    ///
    /// Returns `None` when ANY of the five files is absent/blank — the fail-closed
    /// path: without injected storage creds the collector simply produces no bundle
    /// (the uploader is not spawned) rather than crashing the session.
    pub fn from_creds(creds: &CredsLayout) -> Option<Self> {
        let base_url = read_trimmed(&creds.storage_base_url())?;
        let bucket = read_trimmed(&creds.storage_bucket())?;
        let token_url = read_trimmed(&creds.storage_token_url())?;
        let client_id = read_trimmed(&creds.storage_client_id())?;
        let client_secret = read_trimmed(&creds.storage_client_secret())?;
        let config = ChronoStorageConfig {
            base_url,
            bucket,
            nyxid_token_url: token_url,
            // In-pod the injected SA IS the SA the client authenticates as, so its
            // id/secret become the client-credentials the token provider mints with.
            nyxid_client_id: client_id,
            nyxid_client_secret: secrecy::SecretString::from(client_secret),
        };
        Some(Self::new(crate::storage::client_from_config(config)))
    }
}

#[async_trait]
impl LogSink for ChronoStorageSink {
    async fn put(&self, key: &str, gz: Bytes) -> Result<(), SinkError> {
        // Discard the returned object URL; map any failure through the leak-free
        // StorageError Display so no token/secret/signed-URL can ride the error.
        self.client
            .upload(key, gz, BUNDLE_CONTENT_TYPE)
            .await
            .map(|_url| ())
            .map_err(|e| SinkError::Upload(e.to_string()))
    }

    async fn get(&self, key: &str) -> Result<Option<Bytes>, SinkError> {
        // A 404 is "no such object yet" (Ok(None)); any other failure maps through
        // the same leak-free StorageError Display so no credential can ride it.
        match self.client.download(key).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(StorageError::Status { status: 404 }) => Ok(None),
            Err(e) => Err(SinkError::Upload(e.to_string())),
        }
    }
}

/// Read a mounted credential file, trimming the trailing newline a Secret write
/// leaves. `None` on a missing/blank file (drives the fail-closed sink build).
fn read_trimmed(path: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// A recording [`LogSink`] fake for tests: it captures every `(key, bytes)` put IN
/// ORDER (`calls`) AND keeps the last-put value per key (`store`) so `get` returns
/// what was last `put` (and `None` for an absent key) — letting the run-index
/// read-modify-write be exercised without a network. Failures can be programmed
/// globally (`fail`) OR scoped to keys containing a substring (`fail_key_contains`),
/// so an index-only or per-run-only failure is isolable. Shared across the sink +
/// collector test modules.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct FakeSink {
    /// Every `(key, gz)` the collector uploaded, in order.
    pub calls: std::sync::Arc<std::sync::Mutex<Vec<(String, Bytes)>>>,
    /// The last value `put` per key, so `get(key)` returns the current object.
    pub store: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Bytes>>>,
    /// When set, every `put`/`get` returns an error (drives the swallow-and-continue path).
    pub fail: bool,
    /// When set, only `put`/`get` on a key CONTAINING this substring fail — so a
    /// best-effort/partial path (e.g. an index-only failure while bundle PUTs
    /// succeed) can be exercised in isolation. `fail` still fails every op.
    pub fail_key_contains: Option<String>,
    /// Makes `fail_key_contains` TRANSIENT: fail only the first N matching ops (an
    /// outage that later clears), then let them succeed — so a lost one-shot write
    /// followed by a successful shutdown recovery can be exercised. `None` (default)
    /// = permanent (every matching op fails). Shared across clones + interior-mutable.
    pub fail_key_remaining: std::sync::Arc<std::sync::Mutex<Option<usize>>>,
}

#[cfg(test)]
impl FakeSink {
    pub fn calls(&self) -> Vec<(String, Bytes)> {
        self.calls.lock().expect("lock").clone()
    }

    /// The current stored object for `key` (the last successful `put`), or `None`.
    pub fn stored(&self, key: &str) -> Option<Bytes> {
        self.store.lock().expect("lock").get(key).cloned()
    }

    /// Whether an op on `key` should fail: globally (`fail`), or because `key`
    /// matches `fail_key_contains` — permanently (`fail_key_remaining` unset) or
    /// while a transient budget lasts (decremented per matching op).
    fn should_fail(&self, key: &str) -> bool {
        if self.fail {
            return true;
        }
        let Some(needle) = self.fail_key_contains.as_deref() else {
            return false;
        };
        if !key.contains(needle) {
            return false;
        }
        let mut remaining = self.fail_key_remaining.lock().expect("lock");
        match *remaining {
            // Permanent outage: every matching op fails.
            None => true,
            // Transient outage: fail while the budget lasts, then succeed.
            Some(0) => false,
            Some(n) => {
                *remaining = Some(n - 1);
                true
            }
        }
    }
}

#[cfg(test)]
#[async_trait]
impl LogSink for FakeSink {
    async fn put(&self, key: &str, gz: Bytes) -> Result<(), SinkError> {
        // Record the attempt (order-sensitive) BEFORE the fail check, so a
        // programmed-failure put is still observable in `calls`.
        self.calls
            .lock()
            .expect("lock")
            .push((key.to_string(), gz.clone()));
        if self.should_fail(key) {
            return Err(SinkError::Upload("status 500".to_string()));
        }
        self.store.lock().expect("lock").insert(key.to_string(), gz);
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Bytes>, SinkError> {
        if self.should_fail(key) {
            return Err(SinkError::Upload("status 500".to_string()));
        }
        Ok(self.store.lock().expect("lock").get(key).cloned())
    }
}

#[cfg(test)]
#[path = "sink_tests.rs"]
mod tests;
