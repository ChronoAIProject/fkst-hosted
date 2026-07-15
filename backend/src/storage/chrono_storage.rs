//! chrono-storage object API client (behind the NyxID proxy).
//!
//! Wraps the object endpoints chrono-storage exposes over `{base}/api/buckets/…`,
//! authenticating each with a NyxID service-account bearer token minted by
//! [`NyxidSaTokenProvider`]:
//!
//! - [`ChronoStorageClient::upload`]   `POST   /api/buckets/{bucket}/objects?key&contentType`
//! - [`ChronoStorageClient::download`] `GET    /api/buckets/{bucket}/objects/download?key`
//! - [`ChronoStorageClient::delete`]   `DELETE /api/buckets/{bucket}/objects?key`
//! - [`ChronoStorageClient::bucket_ok`] `GET   /api/buckets` (startup readiness)
//!
//! These are exactly the routes the deployed service (the "chrono-bucket API",
//! self-described at `{base}/openapi.json`) serves. It has NO presigned-URL and
//! no server-side copy endpoint (issue #497 removed the client surface that
//! assumed them — reads are direct and bearer-authenticated).
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

    /// Download the object at `key` via the service's authenticated content read,
    /// `GET {base}/api/buckets/{bucket}/objects/download?key=…` (issue #497: the
    /// deployed chrono-bucket API has no presigned URLs — reads are direct,
    /// bearer-authenticated, and return the raw object bytes).
    pub async fn download(&self, key: &str) -> Result<Bytes, StorageError> {
        let token = self.token.access_token().await?;
        let url = format!(
            "{}/api/buckets/{}/objects/download",
            self.base(),
            self.config.bucket
        );
        let response = self
            .http
            .get(url)
            .query(&[("key", key)])
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| transport("download", &e))?;
        ensure_success("download", &response)?;
        response
            .bytes()
            .await
            .map_err(|e| transport("download-body", &e))
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
