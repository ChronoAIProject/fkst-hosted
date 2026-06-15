//! [`OrnnClient`]: typed access to the Ornn skill registry through the NyxID
//! proxy, plus the pin-resolution / conflict logic (issue #114).
//!
//! Transport is INJECTED via the [`OrnnTransport`] trait (mirroring how
//! `github_app` injects its `GithubApi`), so the client's logic — endpoint
//! shapes, DTO mapping, closure expansion, conflict detection — is unit-testable
//! against a fake without a live NyxID/Ornn or a network.
//!
//! Two transport surfaces:
//! - `proxy_get` / `proxy_get_query`: an Ornn API call through the NyxID proxy
//!   slug `ornn-api`, presenting the session user's NyxID token (so Ornn
//!   enforces private/shared/system visibility).
//! - `download_direct`: hop 2 of the package fetch — a DIRECT GET of a
//!   pre-signed chrono-storage URL, with NO proxy and NO auth header (the URL
//!   is itself the time-limited capability). The URL is SENSITIVE; never logged.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::error::AppError;
use crate::nyxid::{NyxIdClient, ProxyResponse};
use crate::ornn::types::{
    ClosureResult, OrnnPinKind, OrnnSkillPin, ResolvedNode, SearchPage, SkillDetail, VersionRow,
};

/// The NyxID proxy slug Ornn is registered under. The per-deployment value is
/// configurable, but it is built once at client construction so the route shape
/// lives in one place (mirroring the GitHub proxy slug pattern).
pub const DEFAULT_ORNN_SLUG: &str = "ornn-api";

/// Ornn API path prefix under the proxy (`{base}/api/v1/proxy/s/{slug}` + this).
const ORNN_API_PREFIX: &str = "/api/v1";

/// Injected transport for the Ornn client. Production wires
/// [`NyxIdProxyTransport`]; tests inject a fake that returns canned responses.
#[async_trait]
pub trait OrnnTransport: Send + Sync {
    /// GET an Ornn API `path` (already including [`ORNN_API_PREFIX`]) with
    /// optional `query`, through the NyxID proxy as the user, returning the
    /// buffered upstream response (status surfaced verbatim).
    async fn proxy_get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        user_token: &SecretString,
    ) -> Result<ProxyResponse, AppError>;

    /// Hop 2: DIRECT GET of a pre-signed chrono-storage URL (no proxy, no auth
    /// header). Returns the verbatim package bytes. The URL is SENSITIVE.
    async fn download_direct(&self, presigned_url: &str) -> Result<Vec<u8>, AppError>;
}

/// Production transport: Ornn calls ride [`NyxIdClient::proxy_request`] with the
/// `ornn-api` slug; the direct download uses a plain `reqwest` GET.
pub struct NyxIdProxyTransport {
    nyxid: NyxIdClient,
    slug: String,
    http: reqwest::Client,
}

impl NyxIdProxyTransport {
    /// Build the production transport. `slug` is the NyxID proxy slug Ornn is
    /// registered under (default [`DEFAULT_ORNN_SLUG`]).
    pub fn new(nyxid: NyxIdClient, slug: &str) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("fkst-hosted-api")
            .build()
            .map_err(|error| {
                tracing::error!(error = %error, "failed to build ornn download client");
                AppError::Unavailable("failed to build ornn download client".to_string())
            })?;
        Ok(Self {
            nyxid,
            slug: slug.to_string(),
            http,
        })
    }
}

#[async_trait]
impl OrnnTransport for NyxIdProxyTransport {
    async fn proxy_get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        user_token: &SecretString,
    ) -> Result<ProxyResponse, AppError> {
        self.nyxid
            .proxy_request(
                &self.slug,
                reqwest::Method::GET,
                path,
                query,
                user_token,
                None,
            )
            .await
            .map_err(|error| {
                // The NyxID error never carries the token; the proxy being
                // unreachable is a 503 to the caller.
                tracing::error!(error = %error, "ornn proxy request failed");
                AppError::Unavailable("ornn registry proxy unavailable".to_string())
            })
    }

    async fn download_direct(&self, presigned_url: &str) -> Result<Vec<u8>, AppError> {
        // The pre-signed URL is the capability — NO auth header is attached.
        // It is SENSITIVE, so it is never logged (only the failure shape is).
        let response = self.http.get(presigned_url).send().await.map_err(|error| {
            tracing::error!(error = %error, "ornn package download failed");
            AppError::Unavailable("failed to download skill package".to_string())
        })?;
        let status = response.status();
        if !status.is_success() {
            tracing::error!(status = %status, "ornn package download non-success");
            return Err(AppError::Unavailable(format!(
                "skill package download failed (status {status})"
            )));
        }
        let bytes = response.bytes().await.map_err(|error| {
            tracing::error!(error = %error, "ornn package body read failed");
            AppError::Unavailable("failed to read skill package bytes".to_string())
        })?;
        Ok(bytes.to_vec())
    }
}

/// Conflict raised when one skill `name` resolves to two different versions
/// across the user's selection (a hard failure — never silently pick one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictError {
    pub name: String,
    pub version_a: String,
    pub version_b: String,
}

impl From<ConflictError> for AppError {
    fn from(error: ConflictError) -> Self {
        AppError::Unprocessable(format!(
            "conflicting ornn skill versions for {:?}: {} vs {}",
            error.name, error.version_a, error.version_b
        ))
    }
}

/// Outcome of resolving a set of pins: the deduped leaf nodes to install plus
/// the per-skillset master prompts to append to `AGENTS.md` (in pin order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPins {
    pub nodes: Vec<ResolvedNode>,
    /// `(skillset_name, instructions)` pairs for AGENTS.md, in pin order.
    pub skillset_instructions: Vec<(String, String)>,
}

/// Typed Ornn registry client. Cheaply cloneable (`Arc`-backed transport).
#[derive(Clone)]
pub struct OrnnClient {
    transport: Arc<dyn OrnnTransport>,
}

impl OrnnClient {
    /// Build a client over an injected transport (production or fake).
    pub fn new(transport: Arc<dyn OrnnTransport>) -> Self {
        Self { transport }
    }

    /// Build the production client wired to NyxID's proxy under `slug`.
    pub fn with_nyxid(nyxid: NyxIdClient, slug: &str) -> Result<Self, AppError> {
        let transport = NyxIdProxyTransport::new(nyxid, slug)?;
        Ok(Self::new(Arc::new(transport)))
    }

    /// Decode a proxy JSON body's `{ data: <T> }` envelope, surfacing Ornn's
    /// 4xx/5xx as the authoritative result (fkst-hosted adds no policy here).
    fn decode_data<T: serde::de::DeserializeOwned>(
        ctx: &str,
        response: ProxyResponse,
    ) -> Result<T, AppError> {
        pass_through_status(ctx, response.status)?;
        #[derive(serde::Deserialize)]
        struct Envelope<T> {
            data: T,
        }
        let envelope: Envelope<T> = serde_json::from_slice(&response.body).map_err(|error| {
            tracing::error!(ctx, error = %error, "failed to decode ornn response envelope");
            AppError::Upstream(format!("malformed ornn {ctx} response"))
        })?;
        Ok(envelope.data)
    }

    /// Hop 1: fetch a skill's detail (incl. the pre-signed package URL).
    /// `GET /api/v1/skills/<name>?version=<m.minor>`.
    pub async fn skill_detail(
        &self,
        user_token: &SecretString,
        name: &str,
        version: &str,
    ) -> Result<SkillDetail, AppError> {
        let path = format!("{ORNN_API_PREFIX}/skills/{name}");
        let response = self
            .transport
            .proxy_get(&path, &[("version", version)], user_token)
            .await?;
        Self::decode_data("skill detail", response)
    }

    /// Hop 2: download the verbatim package zip from the pre-signed URL.
    pub async fn download_package(&self, presigned_url: &str) -> Result<Vec<u8>, AppError> {
        self.transport.download_direct(presigned_url).await
    }

    /// Expand a skillset closure into its master prompt + member skills.
    /// `GET /api/v1/skillsets/<name>/closure?version=<m.minor>`.
    pub async fn skillset_closure(
        &self,
        user_token: &SecretString,
        name: &str,
        version: &str,
    ) -> Result<ClosureResult, AppError> {
        let path = format!("{ORNN_API_PREFIX}/skillsets/{name}/closure");
        let response = self
            .transport
            .proxy_get(&path, &[("version", version)], user_token)
            .await?;
        Self::decode_data("skillset closure", response)
    }

    /// List a skill's versions (newest-first). `GET /api/v1/skills/<name>/versions`.
    pub async fn skill_versions(
        &self,
        user_token: &SecretString,
        name: &str,
    ) -> Result<Vec<VersionRow>, AppError> {
        self.versions(user_token, "skills", name).await
    }

    /// List a skillset's versions (newest-first).
    /// `GET /api/v1/skillsets/<name>/versions`.
    pub async fn skillset_versions(
        &self,
        user_token: &SecretString,
        name: &str,
    ) -> Result<Vec<VersionRow>, AppError> {
        self.versions(user_token, "skillsets", name).await
    }

    async fn versions(
        &self,
        user_token: &SecretString,
        collection: &str,
        name: &str,
    ) -> Result<Vec<VersionRow>, AppError> {
        let path = format!("{ORNN_API_PREFIX}/{collection}/{name}/versions");
        let response = self.transport.proxy_get(&path, &[], user_token).await?;
        #[derive(serde::Deserialize)]
        struct Items {
            #[serde(default)]
            items: Vec<VersionRow>,
        }
        let items: Items = Self::decode_data("versions", response)?;
        Ok(items.items)
    }

    /// Search skills. `GET /api/v1/skill-search?scope=&systemFilter=&kind=&tags=&q=&page=`.
    pub async fn skill_search(
        &self,
        user_token: &SecretString,
        query: &[(&str, &str)],
    ) -> Result<SearchPage, AppError> {
        let path = format!("{ORNN_API_PREFIX}/skill-search");
        let response = self.transport.proxy_get(&path, query, user_token).await?;
        Self::decode_data("skill search", response)
    }

    /// Search skillsets. `GET /api/v1/skillset-search?scope=&kind=&tags=&q=&page=`.
    pub async fn skillset_search(
        &self,
        user_token: &SecretString,
        query: &[(&str, &str)],
    ) -> Result<SearchPage, AppError> {
        let path = format!("{ORNN_API_PREFIX}/skillset-search");
        let response = self.transport.proxy_get(&path, query, user_token).await?;
        Self::decode_data("skillset search", response)
    }

    /// Resolve a user's pins into the deduped leaf skills to install plus each
    /// skillset's master prompt.
    ///
    /// For each `Skillset` pin → expand its closure (members + instructions);
    /// for each `Skill` pin → a single leaf node. Then union/dedup by name. If
    /// the same `name` appears with two DIFFERENT versions across the whole
    /// selection, this hard-fails with [`ConflictError`] — never silently picks
    /// one. Any Ornn 404/403 propagates as the corresponding `AppError`, so the
    /// caller aborts the session start loudly.
    pub async fn resolve_pins(
        &self,
        user_token: &SecretString,
        pins: &[OrnnSkillPin],
    ) -> Result<ResolvedPins, AppError> {
        // Track the chosen version per name to detect cross-selection conflicts.
        let mut chosen: HashMap<String, String> = HashMap::new();
        let mut nodes: Vec<ResolvedNode> = Vec::new();
        let mut skillset_instructions: Vec<(String, String)> = Vec::new();

        let mut add_node = |name: String, version: String| -> Result<(), ConflictError> {
            match chosen.get(&name) {
                Some(existing) if existing != &version => Err(ConflictError {
                    name,
                    version_a: existing.clone(),
                    version_b: version,
                }),
                Some(_) => Ok(()), // identical version already present: dedup.
                None => {
                    chosen.insert(name.clone(), version.clone());
                    nodes.push(ResolvedNode { name, version });
                    Ok(())
                }
            }
        };

        for pin in pins {
            match pin.kind {
                OrnnPinKind::Skill => {
                    add_node(pin.name.clone(), pin.version.clone())?;
                }
                OrnnPinKind::Skillset => {
                    let closure = self
                        .skillset_closure(user_token, &pin.name, &pin.version)
                        .await?;
                    if !closure.instructions.is_empty() {
                        skillset_instructions
                            .push((pin.name.clone(), closure.instructions.clone()));
                    }
                    for member in closure.items {
                        add_node(member.name, member.version)?;
                    }
                }
            }
        }

        Ok(ResolvedPins {
            nodes,
            skillset_instructions,
        })
    }
}

/// Surface a non-success upstream status as the authoritative result: a 404 →
/// `NotFound`, a 403/401 → `Forbidden`/`Unauthorized`, 429 → `RateLimited`,
/// everything else 4xx → `Unprocessable`, 5xx → `Upstream`. fkst-hosted adds NO
/// permission logic — it relays Ornn's own decision.
fn pass_through_status(ctx: &str, status: reqwest::StatusCode) -> Result<(), AppError> {
    if status.is_success() {
        return Ok(());
    }
    tracing::warn!(ctx, status = %status, "ornn returned a non-success status");
    let message = format!("ornn {ctx} unavailable for the pinned item");
    match status {
        reqwest::StatusCode::NOT_FOUND => Err(AppError::NotFound(format!(
            "ornn skill or skillset not found ({ctx})"
        ))),
        reqwest::StatusCode::UNAUTHORIZED => Err(AppError::Unauthorized(
            "ornn rejected the session token".to_string(),
        )),
        reqwest::StatusCode::FORBIDDEN => Err(AppError::Forbidden(format!(
            "ornn denied access to the pinned item ({ctx})"
        ))),
        reqwest::StatusCode::TOO_MANY_REQUESTS => Err(AppError::RateLimited {
            message: "ornn rate limited the request".to_string(),
            retry_after_secs: 60,
        }),
        s if s.is_client_error() => Err(AppError::Unprocessable(message)),
        _ => Err(AppError::Upstream(message)),
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
