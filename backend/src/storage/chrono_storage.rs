//! chrono-storage object API client (behind the NyxID proxy).
//!
//! Wraps the object endpoints chrono-storage exposes over `{base}/api/buckets/…`,
//! authenticating each with a NyxID service-account bearer token minted by
//! [`NyxidSaTokenProvider`]:
//!
//! - [`ChronoStorageClient::upload`]   `POST   /api/buckets/{bucket}/objects?key&contentType`
//! - [`ChronoStorageClient::download`] `GET    /api/buckets/{bucket}/presigned-url?key` then a
//!   direct, unauthenticated `GET` of the returned signed URL.
//! - [`ChronoStorageClient::delete`]   `DELETE /api/buckets/{bucket}/objects?key`
//! - [`ChronoStorageClient::copy`]     `POST   /api/buckets/{bucket}/objects/copy`
//! - [`ChronoStorageClient::bucket_ok`] `GET   /api/buckets` (startup readiness)
//!
//! The object `key` is carried as a query parameter, URL-encoded by reqwest's
//! `.query(..)`. There is deliberately no list-by-prefix method: chrono-storage
//! exposes no such endpoint.
//!
//! Secret hygiene: the bearer token rides only the `Authorization` header (never
//! a URL or a log); a non-2xx maps to [`StorageError::Status`] carrying only the
//! numeric code; transport errors are reduced to a URL-free category by
//! [`scrub_transport`] so a signed download URL can never leak.

use std::sync::Arc;

use axum::body::Bytes; // re-export of `bytes::Bytes`; avoids a redundant direct dep.
use secrecy::ExposeSecret;
use serde::Deserialize;

use super::config::ChronoStorageConfig;
use super::nyxid_token::NyxidSaTokenProvider;
use super::{scrub_transport, StorageError};

/// `{ "data": { "url": "..." } }` — the upload success envelope.
#[derive(Deserialize)]
struct UploadResponse {
    data: UploadData,
}

#[derive(Deserialize)]
struct UploadData {
    url: String,
}

/// `{ "data": { "presignedUrl": "...", "expiresAt": "..." } }` — the presigned-GET
/// envelope. Only `presignedUrl` is acted on; `expiresAt` is ignored.
#[derive(Deserialize)]
struct PresignedResponse {
    data: PresignedData,
}

#[derive(Deserialize)]
struct PresignedData {
    #[serde(rename = "presignedUrl")]
    presigned_url: String,
}

/// A chrono-storage object-store client bound to a single bucket, sharing one
/// [`reqwest::Client`] with its [`NyxidSaTokenProvider`].
#[derive(Debug)]
pub struct ChronoStorageClient {
    http: reqwest::Client,
    config: ChronoStorageConfig,
    /// `Arc` so the token provider can be shared/cloned into other tasks without
    /// re-minting; the provider caches internally.
    token: Arc<NyxidSaTokenProvider>,
}

impl ChronoStorageClient {
    /// Build a client over a shared HTTP client and resolved config, wiring up a
    /// token provider from the same credentials.
    pub fn new(http: reqwest::Client, config: ChronoStorageConfig) -> Self {
        let token = Arc::new(NyxidSaTokenProvider::new(http.clone(), &config));
        Self {
            http,
            config,
            token,
        }
    }

    /// The base URL with any trailing slash trimmed.
    fn base(&self) -> &str {
        self.config.base_url.trim_end_matches('/')
    }

    /// `{base}/api/buckets/{bucket}/objects`.
    fn objects_url(&self) -> String {
        format!("{}/api/buckets/{}/objects", self.base(), self.config.bucket)
    }

    /// Upload `bytes` under `key` with the given `content_type`. Returns the
    /// object URL from the success envelope.
    pub async fn upload(
        &self,
        key: &str,
        bytes: Bytes,
        content_type: &str,
    ) -> Result<String, StorageError> {
        let token = self.token.access_token().await?;
        let response = self
            .http
            .post(self.objects_url())
            .query(&[("key", key), ("contentType", content_type)])
            .bearer_auth(token.expose_secret())
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|e| transport("upload", &e))?;

        ensure_success("upload", &response)?;
        let body: UploadResponse = response.json().await.map_err(|e| {
            tracing::warn!(error = %scrub_transport(&e), "chrono-storage upload response did not parse");
            StorageError::Malformed
        })?;
        Ok(body.data.url)
    }

    /// Mint a presigned GET URL for `key`, requesting a `expires_in_secs` lifetime.
    ///
    /// Unlike [`Self::download`] (which resolves then immediately fetches the bytes
    /// server-side), this RETURNS the signed URL for a caller to hand to an
    /// already-authenticated, already-authorized client (the log-download endpoint).
    /// The URL is short-lived + capability-bearing, so it is NEVER logged. Best-effort
    /// on the TTL: the `expiresIn` hint is honoured by chrono-storage where supported
    /// and otherwise falls back to the server default (an unknown query param is
    /// ignored), so a caller must treat `expires_in_secs` as the intended lifetime.
    pub async fn presigned_get_url(
        &self,
        key: &str,
        expires_in_secs: u64,
    ) -> Result<String, StorageError> {
        self.presigned_url(key, Some(expires_in_secs)).await
    }

    /// Download the object at `key`: resolve a presigned GET URL, then fetch it
    /// directly (no auth header — the URL is pre-signed).
    pub async fn download(&self, key: &str) -> Result<Bytes, StorageError> {
        let presigned = self.presigned_url(key, None).await?;
        // Direct fetch of the signed URL: NO Authorization header.
        let response = self
            .http
            .get(&presigned)
            .send()
            .await
            .map_err(|e| transport("download", &e))?;
        ensure_success("download", &response)?;
        response
            .bytes()
            .await
            .map_err(|e| transport("download-body", &e))
    }

    /// Resolve a presigned GET URL for `key`, optionally requesting a specific
    /// `expires_in_secs` lifetime (carried as the `expiresIn` query param when set).
    async fn presigned_url(
        &self,
        key: &str,
        expires_in_secs: Option<u64>,
    ) -> Result<String, StorageError> {
        let token = self.token.access_token().await?;
        let url = format!(
            "{}/api/buckets/{}/presigned-url",
            self.base(),
            self.config.bucket
        );
        // `key` first; append `expiresIn` only when a TTL is requested so the
        // default-lifetime path (download) sends exactly the same query as before.
        let mut query: Vec<(&str, String)> = vec![("key", key.to_string())];
        if let Some(secs) = expires_in_secs {
            query.push(("expiresIn", secs.to_string()));
        }
        let response = self
            .http
            .get(url)
            .query(&query)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| transport("presigned-url", &e))?;
        ensure_success("presigned-url", &response)?;
        let body: PresignedResponse = response.json().await.map_err(|e| {
            tracing::warn!(error = %scrub_transport(&e), "chrono-storage presigned-url response did not parse");
            StorageError::Malformed
        })?;
        Ok(body.data.presigned_url)
    }

    /// Delete the object at `key`.
    pub async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let token = self.token.access_token().await?;
        let response = self
            .http
            .delete(self.objects_url())
            .query(&[("key", key)])
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| transport("delete", &e))?;
        ensure_success("delete", &response)?;
        Ok(())
    }

    /// Copy the object at `src` to `dst` (server-side copy within the bucket).
    pub async fn copy(&self, src: &str, dst: &str) -> Result<(), StorageError> {
        let token = self.token.access_token().await?;
        let url = format!(
            "{}/api/buckets/{}/objects/copy",
            self.base(),
            self.config.bucket
        );
        let response = self
            .http
            .post(url)
            .bearer_auth(token.expose_secret())
            .json(&serde_json::json!({ "sourceKey": src, "destKey": dst }))
            .send()
            .await
            .map_err(|e| transport("copy", &e))?;
        ensure_success("copy", &response)?;
        Ok(())
    }

    /// Startup readiness probe: `true` when `GET /api/buckets` returns 2xx.
    ///
    /// Best-effort: any failure (token mint, transport, or non-2xx) resolves to
    /// `false` with a warning rather than an error, so a caller can fail-fast at
    /// startup on a misconfigured storage endpoint.
    pub async fn bucket_ok(&self) -> bool {
        let token = match self.token.access_token().await {
            Ok(token) => token,
            Err(e) => {
                tracing::warn!(error = %e, "chrono-storage bucket check could not mint a token");
                return false;
            }
        };
        let url = format!("{}/api/buckets", self.base());
        match self
            .http
            .get(url)
            .bearer_auth(token.expose_secret())
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => true,
            Ok(response) => {
                tracing::warn!(status = %response.status(), "chrono-storage bucket check non-success");
                false
            }
            Err(e) => {
                tracing::warn!(error = %scrub_transport(&e), "chrono-storage bucket check transport error");
                false
            }
        }
    }
}

/// Build a [`StorageError::Transport`] from a reqwest error, logging (and
/// carrying) only the URL-free category so a signed URL / object key never leaks.
fn transport(op: &str, e: &reqwest::Error) -> StorageError {
    let detail = scrub_transport(e);
    tracing::warn!(op, error = %detail, "chrono-storage transport error");
    StorageError::Transport(detail)
}

/// Map a non-2xx response to [`StorageError::Status`] (numeric code only).
fn ensure_success(op: &str, response: &reqwest::Response) -> Result<(), StorageError> {
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        tracing::warn!(op, status = %status, "chrono-storage returned non-success");
        Err(StorageError::Status {
            status: status.as_u16(),
        })
    }
}

#[cfg(test)]
#[path = "chrono_storage_tests.rs"]
mod tests;
