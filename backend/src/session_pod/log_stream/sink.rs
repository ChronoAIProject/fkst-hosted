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
use crate::storage::{ChronoStorageClient, ChronoStorageConfig};

/// The content type every log bundle is uploaded with.
pub const BUNDLE_CONTENT_TYPE: &str = "application/gzip";

/// A sink failure. Deliberately narrow: it carries at most the leak-free
/// [`crate::storage::StorageError`] rendering, so logging or rendering it can never
/// spill a credential or a signed URL.
#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    /// The bundle upload failed. The detail is the (leak-free) storage-error
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

/// A recording [`LogSink`] fake for tests: it captures every `(key, bytes)` put and
/// can be programmed to fail, so the collector's upload cadence + the error path are
/// exercised without a network. Shared across the sink + collector test modules.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct FakeSink {
    /// Every `(key, gz)` the collector uploaded, in order.
    pub calls: std::sync::Arc<std::sync::Mutex<Vec<(String, Bytes)>>>,
    /// When set, every `put` returns an error (drives the swallow-and-continue path).
    pub fail: bool,
}

#[cfg(test)]
impl FakeSink {
    pub fn calls(&self) -> Vec<(String, Bytes)> {
        self.calls.lock().expect("lock").clone()
    }
}

#[cfg(test)]
#[async_trait]
impl LogSink for FakeSink {
    async fn put(&self, key: &str, gz: Bytes) -> Result<(), SinkError> {
        self.calls
            .lock()
            .expect("lock")
            .push((key.to_string(), gz.clone()));
        if self.fail {
            Err(SinkError::Upload("status 500".to_string()))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "sink_tests.rs"]
mod tests;
