//! The dual-mode, identity-gated session-log download endpoint.
//!
//! `GET /api/v1/logs/{session_id}` serves a session's redacted log bundle (uploaded
//! by the producer to chrono-storage at `logs/<session_id>/latest.tar.gz`) to — and
//! only to — a caller the session's trigger issue authorizes. It is UNAUTHENTICATED
//! at the routing layer (like the webhook): identity + authorization are enforced
//! INSIDE the handler, so the handler must be robust to junk input.
//!
//! Two modes establish identity:
//!
//! - **API mode** — an `Authorization: Bearer <github-token>` header. The token is
//!   traded for `{login, id}` via `GET {api_base}/user` (never logged, used only for
//!   that call, never stored; the lookup is cached briefly by token HASH). A rejected
//!   token → 401. On success the endpoint STREAMS the redacted bundle back as a gzip
//!   attachment.
//! - **Browser mode** — no header. The endpoint 302-redirects to GitHub user-OAuth
//!   with a SIGNED `state` carrying the `session_id` (CSRF/tamper guard); the
//!   `/api/v1/logs/oauth/callback` route verifies `state`, exchanges the code for a
//!   user token, resolves `/user`, and — on authorization — STREAMS the bundle back as
//!   an attachment (the download starts).
//!
//! Authorization (both modes) asks the shared capability policy
//! ([`crate::session_access`]) the `LogDownload` question against the session's
//! trigger context, looked up in the reconciler-maintained projection. The
//! session id is a one-way hash, so this reverse map recovers the author id and
//! `### Log Access Allowlist`; the route also grants the deployment-wide
//! `FKST_GLOBAL_ADMINS` role. Deny → 403. Unknown session / missing object → 404.
//!
//! Secret hygiene: the caller's token and the OAuth client secret are NEVER logged, and
//! NO presigned S3 URL is ever exposed to the caller — the control plane fetches the
//! bundle server-side (a presigned URL is used only internally) and returns the bytes.

// `pub(crate)` for its test-only cache reset: the chat dispatch tests drive this
// same router and must clear the process-global token→identity cache too.
pub(crate) mod identity;
// Shared with `crate::routes::auth` (the frontend login flow reuses the signed-state
// + authorize-URL + token-exchange primitives).
/// The shared session-scoped authorization gate, applied by both modes and reused
/// by the engine-observe route.
mod authorize;
// Browser mode in full: the OAuth entry redirect, the callback, and the HTML
// error pages that surface every browser-path failure.
mod browser;
pub(crate) mod oauth;
// The per-run listing endpoint (`GET /logs/{sid}/runs`), identity-gated by `authorize`.
mod run_list;
// The in-bundle log viewer: a manifest of the redacted bundle's files + one
// decompressed (optionally tailed) file, both identity-gated by `authorize`.
mod viewer;

use axum::extract::{Path, Query, State};
use axum::http::{header, Extensions, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) use authorize::authorize;

use crate::error::{AppError, ErrorEnvelope};
use crate::session_pod::log_stream::runs;
use crate::state::AppState;
use crate::storage::StorageError;

/// The chrono-storage object key a session's redacted bundle is read from, per
/// requested run (issue #568): `None` / `Some("latest")` → the authoritative
/// `logs/{sid}/latest.tar.gz` the producer always overwrites (byte-for-byte the
/// legacy key — zero regression for the default download path); `Some(run_id)` →
/// that run's immutable per-incarnation object.
fn object_key_for(session_id: &str, run: Option<&str>) -> String {
    match run {
        None | Some("latest") => format!("logs/{session_id}/latest.tar.gz"),
        Some(run_id) => runs::run_bundle_key(session_id, run_id),
    }
}

/// The optional `?run=<run_id>` selector shared by the whole-bundle download + the
/// viewer manifest: absent (or `latest`) reads the authoritative `latest.tar.gz`;
/// any other value reads that run's per-incarnation bundle. Run ids come from
/// `GET /logs/{session_id}/runs`.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RunQuery {
    /// The run to read; absent → the latest (whole-session) bundle.
    #[serde(default)]
    pub run: Option<String>,
}

/// `GET /api/v1/logs/{session_id}` — download a session's redacted logs.
///
/// UNAUTHENTICATED at the routing layer; identity + authorization run in-handler
/// (see the module docs). With a Bearer token it resolves identity and streams the
/// redacted bundle as a gzip attachment; without one it 302-redirects into the browser
/// OAuth flow. No presigned S3 URL is ever exposed to the caller — the control plane
/// fetches the bytes server-side and returns them.
#[utoipa::path(
    get,
    path = "/logs/{session_id}",
    tag = "logs",
    operation_id = "download_session_logs",
    params(
        ("session_id" = String, Path, description = "The deterministic session id (from the announce link)"),
        RunQuery,
    ),
    responses(
        (status = 200, description = "The redacted log bundle, streamed as a gzip attachment (API mode — a Bearer token was supplied)", content_type = "application/gzip"),
        (status = 302, description = "Redirect: browser mode → GitHub OAuth"),
        (status = 401, description = "The supplied Bearer token was rejected by GitHub", body = ErrorEnvelope),
        (status = 403, description = "Authenticated but not authorized to access these logs", body = ErrorEnvelope),
        (status = 404, description = "Unknown session, or no logs retained yet", body = ErrorEnvelope),
        (status = 503, description = "Log storage / browser login not configured", body = ErrorEnvelope),
    )
)]
async fn download_session_logs(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(session_id): Path<String>,
    Query(query): Query<RunQuery>,
    headers: HeaderMap,
) -> Response {
    match bearer_token(&headers) {
        // API mode: a Bearer token is present — resolve identity + serve the bundle
        // for the requested run (absent → latest).
        Some(token) => {
            api_mode(
                &state,
                &extensions,
                &session_id,
                &token,
                query.run.as_deref(),
            )
            .await
        }
        // Browser mode: no token — redirect into the GitHub OAuth flow. The run
        // selector is not carried across the OAuth round-trip (the signed state holds
        // only the session id); browser downloads always serve the latest bundle.
        None => browser::browser_redirect(&state, &session_id),
    }
}

// ---- API mode ---------------------------------------------------------------

/// API mode: resolve identity from the Bearer `token`, authorize, and stream the
/// redacted bundle back as a gzip attachment — identical to the browser path, so NO
/// presigned S3 URL is ever handed to a caller (the presigned URL is used server-side
/// only, inside [`stream_download`]). Every failure renders the JSON [`AppError`] envelope.
async fn api_mode(
    state: &AppState,
    extensions: &Extensions,
    session_id: &str,
    token: &str,
    run: Option<&str>,
) -> Response {
    let user = match identity::resolve(&state.config.github_api_base_url, token).await {
        Ok(user) => user,
        Err(err) => return err.into_response(),
    };
    crate::audit::identity::record_identity(
        extensions,
        crate::audit::AuditIdentity::github_bearer(user.id, user.login.clone()),
    );
    if let Err(err) = authorize(state, session_id, &user) {
        return err.into_response();
    }
    match stream_download(state, session_id, run).await {
        Ok(response) => response,
        Err(err) => err.into_response(),
    }
}

// ---- Serve ------------------------------------------------------------------

/// Fetch a session's redacted bundle from chrono-storage (server-side, gzip'd tar).
/// Shared by the whole-bundle download and the in-bundle log viewer ([`viewer`]),
/// each of which reads the bundle repeatedly (the viewer once per manifest AND once
/// per file). A fresh cache hit ([`crate::log_bundle_cache`], ~30s TTL) short-circuits
/// the download entirely; on a miss the bundle is fetched, cached, and returned.
///
/// A missing object → 404; any other storage failure → a URL-free 502. Errors are
/// NEVER cached — only a successful download is stored — so a not-yet-uploaded bundle
/// keeps returning 404 until it exists.
pub(super) async fn fetch_bundle(
    state: &AppState,
    session_id: &str,
    run: Option<&str>,
) -> Result<axum::body::Bytes, AppError> {
    // Normalize an empty / whitespace `?run=` to "latest" (None): `Some("")` must
    // NOT resolve to `logs/<sid>/runs/.tar.gz` (a guaranteed 404). Done once here so
    // download, manifest, and file all treat a blank run selector as the latest.
    let run = run.filter(|r| !r.trim().is_empty());
    // Cache key: the bare session id for the latest bundle (so the existing cache
    // hits are unchanged), else `<session_id>#<run>` so per-run bundles never collide
    // with each other or with latest.
    let cache_key = match run {
        None | Some("latest") => session_id.to_string(),
        Some(run_id) => format!("{session_id}#{run_id}"),
    };
    // Serve from the cache when a fresh bundle is already in hand: a burst of
    // manifest/file requests for one session then hits storage at most once per TTL.
    if let Some(bytes) = state.log_bundle_cache.get(&cache_key) {
        return Ok(bytes);
    }
    let Some(storage) = state.storage.as_ref() else {
        return Err(AppError::Unavailable(
            "log storage is not configured".to_string(),
        ));
    };
    let key = object_key_for(session_id, run);
    match storage.download(&key).await {
        Ok(bytes) => {
            state.log_bundle_cache.put(cache_key, bytes.clone());
            Ok(bytes)
        }
        Err(StorageError::Status { status: 404 }) => {
            Err(AppError::NotFound("no logs available yet".to_string()))
        }
        Err(err) => {
            tracing::warn!(session_id = %session_id, error = %err, "log download failed");
            Err(AppError::Upstream("log storage error".to_string()))
        }
    }
}

/// Fetch the session's bundle from chrono-storage (server-side) and return it as an
/// `attachment` download. Serving it THROUGH the control plane — rather than 302-ing the
/// browser to the presigned S3 URL — means the caller only ever talks to THIS host (robust
/// for a browser on a different machine/network than the cluster), and the explicit
/// `Content-Disposition: attachment` makes the browser SAVE the bundle rather than fetch it
/// into the void (a cross-origin nav to an `application/gzip` URL lacking that header is
/// silently discarded by some browsers). API (Bearer) callers still receive a presigned URL.
pub(super) async fn stream_download(
    state: &AppState,
    session_id: &str,
    run: Option<&str>,
) -> Result<Response, AppError> {
    let bytes = fetch_bundle(state, session_id, run).await?;
    let disposition = format!("attachment; filename=\"fkst-logs-{session_id}.tar.gz\"");
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/gzip"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposition)
                    .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
            ),
        ],
        bytes,
    )
        .into_response())
}

// ---- Small helpers ----------------------------------------------------------

/// Extract a non-empty bearer token from the `Authorization` header (either casing of
/// the scheme). `None` when the header is absent, non-bearer, or empty — that steers
/// the request into browser mode rather than erroring.
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?
        .trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// The log-download router (nested under `/api/v1`). Open at the app layer — both
/// identity and authorization are enforced INSIDE each handler (GitHub token or
/// OAuth), so there is no documented security scheme (like the webhook).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(download_session_logs))
        .routes(routes!(browser::oauth_callback))
        .routes(routes!(run_list::list_session_runs))
        .routes(routes!(viewer::log_manifest))
        .routes(routes!(viewer::log_file))
}

#[cfg(test)]
#[path = "bundle_cache_tests.rs"]
mod bundle_cache_tests;
// The cluster-free, wiremock-backed fixtures for driving the real router. Shared
// with the chat dispatch tests (`crate::chat::dispatch`), which need exactly this:
// an endpoint that reaches a 200 without a Kubernetes cluster.
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[cfg(test)]
#[path = "tests_browser.rs"]
mod tests_browser;
#[cfg(test)]
#[path = "tests_runs.rs"]
mod tests_runs;
