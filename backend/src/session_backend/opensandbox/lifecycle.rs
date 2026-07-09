//! [`OsbLifecycleClient`]: a thin, hand-rolled reqwest client over the OpenSandbox
//! sandbox-lifecycle endpoints (create / get / list / patch-metadata / delete).
//!
//! Mirrors the repo's other HTTP clients (`github_app::api`, `storage::chrono_storage`):
//! an injected [`reqwest::Client`], a trimmed base URL, and a single choke-point
//! ([`OsbLifecycleClient::request`]) that stamps the API-key header so no verb can
//! forget or leak it. A free [`map_response`] funnels every response through one
//! status -> error mapping. No retry, no config, no `SessionBackend` impl, and no
//! execd endpoints — those are deferred to later issues.

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::dto::{CreateSandboxRequest, OsbError, SandboxView};

/// The header carrying the API key (`apiKeyAuth` in the spec). `pub(super)` so the
/// sibling execd client stamps the SAME key header through its own choke-point.
pub(super) const API_KEY_HEADER: &str = "OPEN-SANDBOX-API-KEY";

/// Page size requested when walking the paginated list endpoint.
const LIST_PAGE_SIZE: u32 = 100;

/// Hard backstop on the list page-walk: a server that never clears `hasNextPage`
/// can't spin this loop forever. At [`LIST_PAGE_SIZE`] per page this bounds a walk
/// to 100 000 sandboxes — far beyond any real fleet — after which the walk stops
/// with a loud warning and returns what it has.
const MAX_LIST_PAGES: u32 = 1_000;

/// A client bound to one OpenSandbox lifecycle base URL + API key, sharing an
/// injected [`reqwest::Client`].
///
/// `#[derive(Debug)]` is safe: `secrecy::SecretString` redacts itself in `Debug`,
/// so the key never appears (asserted by a unit test).
#[derive(Debug)]
pub struct OsbLifecycleClient {
    base_url: String,
    api_key: SecretString,
    http: reqwest::Client,
}

impl OsbLifecycleClient {
    /// Build a client over an injected HTTP client. `base_url` is the API root (the
    /// client owns the `/v1` path prefix); a trailing slash is trimmed so path joins
    /// never double up.
    pub fn new(base_url: reqwest::Url, api_key: SecretString, http: reqwest::Client) -> Self {
        Self {
            base_url: base_url.as_str().trim_end_matches('/').to_string(),
            api_key,
            http,
        }
    }

    /// Start a request with the API-key header set. The ONLY place the key is
    /// exposed; every verb builds its request through here so none can omit the
    /// header.
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.base_url);
        self.http
            .request(method, url)
            .header(API_KEY_HEADER, self.api_key.expose_secret())
    }

    /// `POST /v1/sandboxes` — create a sandbox (the API answers `202 Accepted`).
    pub async fn create_sandbox(
        &self,
        req: &CreateSandboxRequest,
    ) -> Result<SandboxView, OsbError> {
        let path = "/v1/sandboxes";
        let method = reqwest::Method::POST;
        let response = self.request(method.clone(), path).json(req).send().await?;
        let response = map_response(&method, path, response).await?;
        Ok(response.json::<SandboxView>().await?)
    }

    /// `GET /v1/sandboxes/{id}` — fetch one sandbox. 404 -> [`OsbError::NotFound`].
    pub async fn get_sandbox(&self, id: &str) -> Result<SandboxView, OsbError> {
        let path = format!("/v1/sandboxes/{id}");
        let method = reqwest::Method::GET;
        let response = self.request(method.clone(), &path).send().await?;
        let response = map_response(&method, &path, response).await?;
        Ok(response.json::<SandboxView>().await?)
    }

    /// `GET /v1/sandboxes` — list sandboxes, walking every page and aggregating.
    ///
    /// `metadata_filter` pairs are joined into ONE `metadata` query value
    /// (`k1=v1&k2=v2`); reqwest url-encodes it once (`=` -> `%3D`, `&` -> `%26`) to
    /// match the spec's single-string filter param. Pagination is page-number based:
    /// the walk increments `page` while the response's `pagination.hasNextPage` is
    /// true (bounded by [`MAX_LIST_PAGES`]).
    pub async fn list_sandboxes(
        &self,
        metadata_filter: &[(String, String)],
    ) -> Result<Vec<SandboxView>, OsbError> {
        let path = "/v1/sandboxes";
        let method = reqwest::Method::GET;
        // Assemble the filter once; reqwest owns the percent-encoding.
        let metadata_value = (!metadata_filter.is_empty()).then(|| {
            metadata_filter
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("&")
        });

        let mut aggregated = Vec::new();
        let mut page: u32 = 1;
        loop {
            let mut query: Vec<(&str, String)> = vec![
                ("page", page.to_string()),
                ("pageSize", LIST_PAGE_SIZE.to_string()),
            ];
            if let Some(value) = metadata_value.as_ref() {
                query.push(("metadata", value.clone()));
            }

            let response = self
                .request(method.clone(), path)
                .query(&query)
                .send()
                .await?;
            let response = map_response(&method, path, response).await?;
            let body: SandboxListPage = response.json().await?;
            aggregated.extend(body.items);

            if !body.pagination.has_next_page {
                break;
            }
            if page >= MAX_LIST_PAGES {
                tracing::warn!(
                    pages = page,
                    "opensandbox list page-walk hit the safety cap; returning a truncated fleet"
                );
                break;
            }
            page += 1;
        }
        Ok(aggregated)
    }

    /// `PATCH /v1/sandboxes/{id}/metadata` — RFC 7396 merge-patch the metadata map.
    ///
    /// The spec's request body media type is plain `application/json` (which
    /// reqwest's `.json()` sets) despite the RFC 7396 *semantics* — do NOT switch to
    /// `application/merge-patch+json`, which the endpoint does not advertise (risking
    /// a 415). 404 -> [`OsbError::NotFound`]; the returned sandbox body is discarded.
    pub async fn patch_metadata(
        &self,
        id: &str,
        metadata: &BTreeMap<String, String>,
    ) -> Result<(), OsbError> {
        let path = format!("/v1/sandboxes/{id}/metadata");
        let method = reqwest::Method::PATCH;
        let response = self
            .request(method.clone(), &path)
            .json(metadata)
            .send()
            .await?;
        map_response(&method, &path, response).await?;
        Ok(())
    }

    /// `DELETE /v1/sandboxes/{id}` — delete a sandbox (the API answers `204`). 404 ->
    /// [`OsbError::NotFound`] (LITERAL — a later backend layer owns benign-ness).
    pub async fn delete_sandbox(&self, id: &str) -> Result<(), OsbError> {
        let path = format!("/v1/sandboxes/{id}");
        let method = reqwest::Method::DELETE;
        let response = self.request(method.clone(), &path).send().await?;
        map_response(&method, &path, response).await?;
        Ok(())
    }
}

/// Map a response's status into the [`OsbError`] taxonomy, logging the outcome at
/// debug (never the key, body values, or metadata). `404` -> `NotFound`; any other
/// non-2xx -> `Api { status, message }` with the body text; `2xx` -> the response
/// for the caller to decode. `pub(super)` so the sibling execd client funnels its
/// responses through the SAME status -> error mapping.
pub(super) async fn map_response(
    method: &reqwest::Method,
    path: &str,
    response: reqwest::Response,
) -> Result<reqwest::Response, OsbError> {
    let status = response.status();
    tracing::debug!(method = %method, path = %path, status = status.as_u16(), "opensandbox lifecycle");
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(OsbError::NotFound);
    }
    if !status.is_success() {
        let message = response.text().await.unwrap_or_default();
        return Err(OsbError::Api {
            status: status.as_u16(),
            message,
        });
    }
    Ok(response)
}

/// The list endpoint's response envelope (`{items, pagination}`). Private to the
/// list walk.
#[derive(Deserialize)]
struct SandboxListPage {
    #[serde(default)]
    items: Vec<SandboxView>,
    #[serde(default)]
    pagination: PageInfo,
}

/// Just the pagination bit the walk reads: whether another page follows. A missing
/// field defaults to `false` (stop) so a malformed pagination object can't loop.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    #[serde(default)]
    has_next_page: bool,
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
