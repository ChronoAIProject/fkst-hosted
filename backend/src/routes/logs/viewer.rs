//! The in-bundle log VIEWER: read a session's redacted bundle without downloading
//! the whole `tar.gz`.
//!
//! - `GET /api/v1/logs/{session_id}/manifest` — the bundle's file list (path, byte
//!   size, and a friendly label from the fixed collector layout).
//! - `GET /api/v1/logs/{session_id}/file?path=&tail_bytes=` — ONE bundle file as
//!   UTF-8 text, optionally only its last `tail_bytes` bytes (snapped to a line
//!   boundary) for a cheap live tail.
//!
//! Both authorize IDENTICALLY to the whole-bundle download: the [`GithubUser`]
//! extractor establishes identity, then [`super::authorize`] runs the same
//! deny-by-default three-tier check (trigger author / per-issue allow-list /
//! global admins). An unknown session → 404; an unauthorized caller → 403.
//!
//! The bundle is fetched server-side (via [`super::fetch_bundle`]) and decompressed
//! in memory. Bundle files are ALREADY redacted, so they are served verbatim. The
//! `path` MUST match a manifest entry exactly, so a traversal / unknown path can
//! never escape the archive — it simply matches nothing (404).

use std::io::{Cursor, Read};

use axum::extract::{Path, Query, State};
use axum::Json;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::state::AppState;

/// The per-page ceiling on a single tailed file read is the bundle's own bound;
/// the collector caps each class file, so no extra ceiling is imposed here.
///
/// The bundle's fixed collector layout (see `session_pod::log_stream::bundle`):
/// `fkst-hosted/driver.log`, `fkst-substrate/framework/supervise.log`,
/// `fkst-substrate/codex/codex.log`, `fkst-substrate/etc/misc.log`, plus
/// `README.md` and `meta.json`.
/// A session's redacted-log bundle manifest.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LogManifest {
    pub session_id: String,
    /// One entry per file in the bundle (directories excluded), path-sorted.
    pub files: Vec<LogFileEntry>,
    /// ISO-8601 time the manifest was generated (a live read).
    pub generated_at: String,
}

/// One file in the bundle.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LogFileEntry {
    /// The archive path, e.g. `fkst-substrate/codex/codex.log`.
    pub path: String,
    /// The file's byte size.
    pub size: i64,
    /// A friendly label from the fixed layout: `Driver`/`Supervise`/`Codex`/`Misc`/`README`/`Meta`.
    pub label: String,
}

/// One bundle file's (optionally tailed) text content.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LogFileContent {
    pub session_id: String,
    pub path: String,
    /// The UTF-8 text (lossily decoded — bundle files are text logs).
    pub content: String,
    /// The file's full byte size.
    pub total_bytes: i64,
    /// The byte length actually returned (== `total_bytes` unless tailed).
    pub returned_bytes: i64,
    /// True when only a tail of the file was returned.
    pub truncated: bool,
}

/// Query for [`log_file`].
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LogFileQuery {
    /// The bundle-relative file path (MUST match a manifest entry exactly).
    pub path: String,
    /// When set, return only the last N bytes (snapped to a line boundary).
    pub tail_bytes: Option<u64>,
}

/// `GET /api/v1/logs/{session_id}/manifest`.
#[utoipa::path(
    get,
    path = "/logs/{session_id}/manifest",
    tag = "logs",
    operation_id = "session_log_manifest",
    params(("session_id" = String, Path, description = "The deterministic session id")),
    responses(
        (status = 200, description = "The bundle's file manifest", body = LogManifest),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Authenticated but not authorized for this session's logs", body = ErrorEnvelope),
        (status = 404, description = "Unknown session, or no logs retained yet", body = ErrorEnvelope),
        (status = 502, description = "The log bundle could not be read", body = ErrorEnvelope),
        (status = 503, description = "Log storage is not configured", body = ErrorEnvelope),
    )
)]
pub(super) async fn log_manifest(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    user: GithubUser,
) -> Result<Json<LogManifest>, AppError> {
    super::authorize(&state, &session_id, &user)?;
    let bytes = super::fetch_bundle(&state, &session_id).await?;

    let mut entries = manifest_entries(bytes.as_ref())?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let files = entries
        .into_iter()
        .map(|(path, size)| LogFileEntry {
            label: classify_bundle_path(&path).to_string(),
            size: i64::try_from(size).unwrap_or(i64::MAX),
            path,
        })
        .collect();

    Ok(Json(LogManifest {
        session_id,
        files,
        generated_at: k8s_openapi::chrono::Utc::now().to_rfc3339(),
    }))
}

/// `GET /api/v1/logs/{session_id}/file?path=&tail_bytes=`.
#[utoipa::path(
    get,
    path = "/logs/{session_id}/file",
    tag = "logs",
    operation_id = "session_log_file",
    params(
        ("session_id" = String, Path, description = "The deterministic session id"),
        LogFileQuery,
    ),
    responses(
        (status = 200, description = "The file's (optionally tailed) UTF-8 content", body = LogFileContent),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Authenticated but not authorized for this session's logs", body = ErrorEnvelope),
        (status = 404, description = "Unknown session, no logs yet, or no such file in the bundle", body = ErrorEnvelope),
        (status = 502, description = "The log bundle could not be read", body = ErrorEnvelope),
        (status = 503, description = "Log storage is not configured", body = ErrorEnvelope),
    )
)]
pub(super) async fn log_file(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<LogFileQuery>,
    user: GithubUser,
) -> Result<Json<LogFileContent>, AppError> {
    super::authorize(&state, &session_id, &user)?;
    let bytes = super::fetch_bundle(&state, &session_id).await?;

    // An exact-match read is the traversal guard: a `../…` or unknown path
    // matches no archive entry and falls through to 404.
    let content = read_bundle_file(bytes.as_ref(), &query.path)?
        .ok_or_else(|| AppError::NotFound(format!("no such log file: {}", query.path)))?;

    let total_bytes = content.len();
    let (returned, truncated) = tail(&content, query.tail_bytes);
    Ok(Json(LogFileContent {
        session_id,
        path: query.path,
        total_bytes: i64::try_from(total_bytes).unwrap_or(i64::MAX),
        returned_bytes: i64::try_from(returned.len()).unwrap_or(i64::MAX),
        truncated,
        content: String::from_utf8_lossy(returned).into_owned(),
    }))
}

/// Return the last `tail_bytes` bytes of `content`, snapped FORWARD to the next
/// line boundary so the first returned line is never partial. `None` (or a tail
/// at least as large as the file) returns the whole file untruncated.
fn tail(content: &[u8], tail_bytes: Option<u64>) -> (&[u8], bool) {
    let Some(n) = tail_bytes else {
        return (content, false);
    };
    let n = usize::try_from(n).unwrap_or(usize::MAX);
    if content.len() <= n {
        return (content, false);
    }
    let slice = &content[content.len() - n..];
    match slice.iter().position(|b| *b == b'\n') {
        // Drop the partial leading line (everything up to and incl. the newline).
        Some(idx) => (&slice[idx + 1..], true),
        None => (slice, true),
    }
}

/// Map a bundle-relative path to its friendly label from the fixed collector
/// layout. The match is by anchor dir / well-known name, case-insensitively.
fn classify_bundle_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower == "readme.md" || lower.ends_with("/readme.md") {
        "README"
    } else if lower == "meta.json" || lower.ends_with("/meta.json") {
        "Meta"
    } else if lower.starts_with("fkst-hosted/") {
        "Driver"
    } else if lower.contains("/framework/") || lower.contains("supervise") {
        "Supervise"
    } else if lower.contains("/codex/") || lower.contains("codex") {
        "Codex"
    } else {
        "Misc"
    }
}

/// List every non-directory entry in the gzip'd tar as `(path, size)`.
fn manifest_entries(bytes: &[u8]) -> Result<Vec<(String, u64)>, AppError> {
    let mut archive = tar::Archive::new(GzDecoder::new(Cursor::new(bytes)));
    let mut out = Vec::new();
    for entry in archive.entries().map_err(bundle_err)? {
        let entry = entry.map_err(bundle_err)?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let path = entry
            .path()
            .map_err(bundle_err)?
            .to_string_lossy()
            .into_owned();
        let size = entry.header().size().unwrap_or(0);
        out.push((path, size));
    }
    Ok(out)
}

/// Read the raw bytes of the archive entry whose path matches `target` exactly,
/// or `Ok(None)` when no entry matches.
fn read_bundle_file(bytes: &[u8], target: &str) -> Result<Option<Vec<u8>>, AppError> {
    let mut archive = tar::Archive::new(GzDecoder::new(Cursor::new(bytes)));
    for entry in archive.entries().map_err(bundle_err)? {
        let mut entry = entry.map_err(bundle_err)?;
        let path = entry
            .path()
            .map_err(bundle_err)?
            .to_string_lossy()
            .into_owned();
        if path == target {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(bundle_err)?;
            return Ok(Some(buf));
        }
    }
    Ok(None)
}

/// A bundle-decode failure renders as a URL-free 502 (the detail stays in logs).
fn bundle_err(err: std::io::Error) -> AppError {
    tracing::warn!(error = %err, "log viewer: bundle decode failed");
    AppError::Upstream("the log bundle could not be read".to_string())
}

#[cfg(test)]
#[path = "viewer_tests.rs"]
mod tests;
