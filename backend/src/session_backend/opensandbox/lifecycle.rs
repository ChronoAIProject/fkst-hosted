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
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::dto::{CreateSandboxRequest, OsbError, SandboxView};

/// The header carrying the API key (`apiKeyAuth` in the spec). `pub(super)` so the
/// sibling execd client stamps the SAME key header through its own choke-point.
pub(super) const API_KEY_HEADER: &str = "OPEN-SANDBOX-API-KEY";

/// `POST /sandboxes` is SYNCHRONOUS on the server: it holds the request open until
/// the sandbox is Running or its `sandbox_create_timeout_seconds` elapses — 300s in
/// the production deployment (scale-from-zero gVisor spot pool; cold start 1–3
/// min). Client budget = that ceiling + 30s slack, so a slow-but-healthy create is
/// never aborted client-side. A create that DOES time out here may still
/// materialize server-side; the spawn path's pre-create list-guard absorbs the
/// orphan on the next reconcile tick.
const CREATE_TIMEOUT: Duration = Duration::from_secs(330);

/// Every other lifecycle verb (get / list / patch-metadata / delete / diagnostics
/// logs) answers promptly; tens of seconds is generous. Without a budget, a wedged
/// connection (server pod rescheduling, half-open TCP) blocks the calling
/// reconciler verb indefinitely — reqwest's default is NO request timeout.
const VERB_TIMEOUT: Duration = Duration::from_secs(30);

/// Per-verb request budgets, injectable so tests can shrink them to milliseconds
/// (wiremock stall tests must not sleep 30s). Production always uses `default()`.
#[derive(Debug, Clone, Copy)]
pub(super) struct LifecycleTimeouts {
    /// Budget for `POST /sandboxes` (see [`CREATE_TIMEOUT`]).
    pub(super) create: Duration,
    /// Budget for every other verb (see [`VERB_TIMEOUT`]).
    pub(super) verb: Duration,
}

impl Default for LifecycleTimeouts {
    fn default() -> Self {
        Self {
            create: CREATE_TIMEOUT,
            verb: VERB_TIMEOUT,
        }
    }
}

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
    timeouts: LifecycleTimeouts,
}

impl OsbLifecycleClient {
    /// Build a client over an injected HTTP client. `base_url` is the API root (the
    /// client owns the `/v1` path prefix); a trailing slash is trimmed so path joins
    /// never double up. Request budgets are the production
    /// [`LifecycleTimeouts::default`].
    pub fn new(base_url: reqwest::Url, api_key: SecretString, http: reqwest::Client) -> Self {
        Self {
            base_url: base_url.as_str().trim_end_matches('/').to_string(),
            api_key,
            http,
            timeouts: LifecycleTimeouts::default(),
        }
    }

    /// Test-only budget override (milliseconds-scale stall tests).
    #[cfg(test)]
    pub(super) fn with_timeouts(mut self, timeouts: LifecycleTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Start a request with the API-key header AND the verb's request budget set.
    /// The ONLY place the key is exposed; every verb builds its request through here
    /// so none can omit the header or ride without a timeout.
    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        timeout: Duration,
    ) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.base_url);
        self.http
            .request(method, url)
            .timeout(timeout)
            .header(API_KEY_HEADER, self.api_key.expose_secret())
    }

    /// `POST /v1/sandboxes` — create a sandbox (the API answers `202 Accepted`).
    pub async fn create_sandbox(
        &self,
        req: &CreateSandboxRequest,
    ) -> Result<SandboxView, OsbError> {
        let path = "/v1/sandboxes";
        let method = reqwest::Method::POST;
        let response = self
            .request(method.clone(), path, self.timeouts.create)
            .json(req)
            .send()
            .await?;
        let response = map_response(&method, path, response).await?;
        Ok(response.json::<SandboxView>().await?)
    }

    /// `GET /v1/sandboxes/{id}` — fetch one sandbox. 404 -> [`OsbError::NotFound`].
    pub async fn get_sandbox(&self, id: &str) -> Result<SandboxView, OsbError> {
        let path = format!("/v1/sandboxes/{id}");
        let method = reqwest::Method::GET;
        let response = self
            .request(method.clone(), &path, self.timeouts.verb)
            .send()
            .await?;
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
                .request(method.clone(), path, self.timeouts.verb)
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
            .request(method.clone(), &path, self.timeouts.verb)
            .json(metadata)
            .send()
            .await?;
        map_response(&method, &path, response).await?;
        Ok(())
    }

    /// `GET /v1/sandboxes/{id}/diagnostics/logs?tail=&since=` — the DEPRECATED
    /// plain-text sandbox log tail, read by the best-effort health scrape
    /// ([`super::backend::OsbBackend::recent_output`]).
    ///
    /// Returns `Ok(Some(text))` on `200` (the raw plain-text body), `Ok(None)` on `404`
    /// (the sandbox / its logs are gone — a benign empty window), and `Err` on any other
    /// non-2xx. The `200`/`404` split is kept RAW here (NOT funnelled through
    /// [`map_response`], which folds `404` into an error) because the caller must map a
    /// gone sandbox to a benign empty window, distinct from a transport error it must
    /// WITHHOLD a health-clear on.
    ///
    /// DEPRECATION + swap plan: this is the upstream's deprecated plain-text diagnostics
    /// endpoint (the structured `scope=`-param JSON diagnostics endpoint currently
    /// answers `501 Not Implemented`). When that structured endpoint ships, swap this
    /// call for it and parse the JSON frames — the `recent_output` 3-state taxonomy
    /// (`Some(text)` / `Some("")` / `None`) it feeds stays the contract. NEVER logs the
    /// body.
    pub async fn diagnostics_logs(
        &self,
        id: &str,
        tail: u32,
        since: &str,
    ) -> Result<Option<String>, OsbError> {
        let path = format!("/v1/sandboxes/{id}/diagnostics/logs");
        let method = reqwest::Method::GET;
        let response = self
            .request(method.clone(), &path, self.timeouts.verb)
            .query(&[("tail", tail.to_string()), ("since", since.to_string())])
            .send()
            .await?;
        let status = response.status();
        tracing::debug!(method = %method, path = %path, status = status.as_u16(), "opensandbox diagnostics logs");
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(OsbError::Api {
                status: status.as_u16(),
                message,
            });
        }
        Ok(Some(response.text().await?))
    }

    /// `DELETE /v1/sandboxes/{id}` — delete a sandbox (the API answers `204`). 404 ->
    /// [`OsbError::NotFound`] (LITERAL — a later backend layer owns benign-ness).
    pub async fn delete_sandbox(&self, id: &str) -> Result<(), OsbError> {
        let path = format!("/v1/sandboxes/{id}");
        let method = reqwest::Method::DELETE;
        let response = self
            .request(method.clone(), &path, self.timeouts.verb)
            .send()
            .await?;
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
