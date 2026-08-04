//! The identity-gated **session health** read surface.
//!
//! Two endpoints beside the existing observe endpoint: a listing cheap enough to
//! render a badge on every session card, and one full report.
//!
//! # Authorization is borrowed, never reinvented
//!
//! Both call [`crate::routes::logs::authorize`], exactly as `observe` does. Someone
//! allowed to read a session's logs is allowed to read its health reports: one grant,
//! now three read-only views. There is deliberately no new authorization tier, no new
//! allowlist section, and no new admin list here.
//!
//! # The control plane stays package-agnostic
//!
//! `status` and `headline` are relayed exactly as the producer wrote them, and
//! `body_markdown` is served byte-for-byte and never parsed, summarized, or
//! reinterpreted. The only judgement this module makes is the heartbeat verdict in
//! [`staleness`] — which is about the *absence* of reports, not about their content.
//!
//! # No documented security scheme, on purpose
//!
//! Like `observe` and the log endpoints, these operations authenticate via the
//! per-request GitHub token (the `GithubUser` extractor) rather than a documented
//! OpenAPI security scheme. `tests/openapi.rs` asserts the document declares no
//! `NyxIdIdentity` scheme at all, so an operation-level `security` requirement here
//! would reference a scheme that does not exist and make the document invalid.
//!
//! # Explicitly out of scope
//!
//! No label is written, no issue comment is posted, and `fkst-degraded` is untouched.
//! This epic is a read-only surface; the infra-layer scrape remains that label's sole
//! writer.

pub mod staleness;

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use k8s_openapi::chrono::Utc;

use crate::audit::arguments::logs::{SafeSessionHealth, SafeSessionHealthReport};
use crate::audit::arguments::{record_safe, AuditedPath};
use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::session_health::{
    health_index_key, parse_index, parse_report, EvidenceEntry, HealthIndexEntry, HealthStatus,
    WorkItemProgress,
};
use crate::state::AppState;
use crate::storage::StorageError;

use staleness::Staleness;

/// Cache-key namespace, so a health index and a log bundle can never collide in the
/// shared TTL cache. A session id is a UUID and cannot contain `#`.
const INDEX_CACHE_PREFIX: &str = "health-index#";

/// Returned when the deployment has no object store at all.
const NO_STORAGE: &str = "health storage is not configured";

/// One report, denormalized from the index — everything a badge or a history row
/// needs, with no report object read at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct HealthReportSummary {
    /// URL-safe id addressing the full report.
    pub id: String,
    /// RFC3339 UTC.
    pub generated_at: String,
    /// The producer's verdict mapped onto the v1 taxonomy.
    pub status: HealthStatus,
    /// The producer's verdict exactly as written — preserved so an unrecognized
    /// future verdict is still displayable.
    pub status_raw: String,
    /// The producer's one-line summary.
    pub headline: String,
    /// `<name>@<version>` of the producing package.
    pub producer: String,
}

impl From<&HealthIndexEntry> for HealthReportSummary {
    fn from(entry: &HealthIndexEntry) -> Self {
        Self {
            id: entry.id.clone(),
            generated_at: entry.generated_at.clone(),
            status: HealthStatus::from_raw(&entry.status),
            status_raw: entry.status.clone(),
            headline: entry.headline.clone(),
            producer: entry.producer.clone(),
        }
    }
}

/// A session's health listing plus the heartbeat verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct SessionHealth {
    /// The session these reports belong to.
    pub session_id: String,
    /// Newest first. Empty is a normal state, not an error.
    pub reports: Vec<HealthReportSummary>,
    /// The newest report, when there is one.
    pub latest: Option<HealthReportSummary>,
    /// The heartbeat verdict — see [`staleness`].
    pub staleness: Staleness,
}

/// One report in full, including its opaque body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct SessionHealthReport {
    /// The session this report belongs to.
    pub session_id: String,
    /// The report's id.
    pub id: String,
    /// RFC3339 UTC.
    pub generated_at: String,
    /// Start of the observed window, when the producer declared one.
    pub window_start: Option<String>,
    /// The producer's verdict mapped onto the v1 taxonomy.
    pub status: HealthStatus,
    /// The producer's verdict exactly as written.
    pub status_raw: String,
    /// The producer's one-line summary.
    pub headline: String,
    /// `<name>@<version>` of the producing package.
    pub producer: String,
    /// Producer-declared confidence, conventionally `high` / `medium` / `low`.
    pub confidence: Option<String>,
    /// The producer's declared cadence, in seconds.
    pub expected_interval_secs: u64,
    /// Observations backing the verdict.
    pub evidence: Vec<EvidenceEntry>,
    /// Work items the producer observed.
    pub work_items: Vec<WorkItemProgress>,
    /// The producer's narrative, **verbatim**.
    ///
    /// Untrusted: authored by an LLM inside a session pod. A client MUST render it as
    /// untrusted markdown with no raw HTML.
    pub body_markdown: String,
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/health",
    tag = "sessions",
    operation_id = "session_health",
    params(("session_id" = String, Path, description = "The session id (from the trigger announce comment)")),
    responses(
        (status = 200, description = "The session's health reports, newest first, plus the heartbeat verdict. An empty list is a normal state, not an error", body = SessionHealth),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Authenticated but not authorized for this session's logs", body = ErrorEnvelope),
        (status = 404, description = "Unknown session", body = ErrorEnvelope),
        (status = 502, description = "The health index could not be read", body = ErrorEnvelope),
        (status = 503, description = "Health storage is not configured for this deployment", body = ErrorEnvelope),
    )
)]
async fn session_health(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    AuditedPath(session_id): AuditedPath<String>,
    user: GithubUser,
) -> Result<Json<SessionHealth>, AppError> {
    // Recorded before authorization, so a denied read still describes which
    // session was asked for. Report bodies are package-authored prose and never
    // become audit properties.
    record_safe(&extensions, &SafeSessionHealth::new(&session_id));
    crate::routes::logs::record_session_correlation(&extensions, &session_id);
    crate::routes::logs::authorize(&state, &session_id, &user)?;

    let entries = fetch_index(&state, &session_id).await?;
    let reports: Vec<HealthReportSummary> = entries.iter().map(HealthReportSummary::from).collect();

    Ok(Json(SessionHealth {
        latest: reports.first().cloned(),
        staleness: staleness::evaluate(
            entries.first(),
            is_live(&state, &session_id).await,
            Utc::now(),
        ),
        reports,
        session_id,
    }))
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/health/{report_id}",
    tag = "sessions",
    operation_id = "session_health_report",
    params(
        ("session_id" = String, Path, description = "The session id"),
        ("report_id" = String, Path, description = "The report id, exactly as listed by the health endpoint"),
    ),
    responses(
        (status = 200, description = "One health report, including its verbatim (untrusted) markdown body", body = SessionHealthReport),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Authenticated but not authorized for this session's logs", body = ErrorEnvelope),
        (status = 404, description = "Unknown session, or no such report", body = ErrorEnvelope),
        (status = 502, description = "The report could not be read", body = ErrorEnvelope),
        (status = 503, description = "Health storage is not configured for this deployment", body = ErrorEnvelope),
    )
)]
async fn session_health_report(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    AuditedPath((session_id, report_id)): AuditedPath<(String, String)>,
    user: GithubUser,
) -> Result<Json<SessionHealthReport>, AppError> {
    record_safe(
        &extensions,
        &SafeSessionHealthReport::new(&session_id, &report_id),
    );
    crate::routes::logs::record_session_correlation(&extensions, &session_id);
    crate::routes::logs::authorize(&state, &session_id, &user)?;

    // THE TRAVERSAL GUARD: an id absent from the index is a 404 and no storage call is
    // made, so a caller-supplied id can never form a key. Exact-match only, the same
    // discipline the bundle viewer uses for file paths.
    let entries = fetch_index(&state, &session_id).await?;
    let Some(entry) = entries.iter().find(|entry| entry.id == report_id) else {
        return Err(AppError::NotFound("no such health report".to_string()));
    };

    let storage = storage(&state)?;
    let bytes = match storage.download(&entry.key).await {
        Ok(bytes) => bytes,
        Err(StorageError::Status { status: 404 }) => {
            return Err(AppError::NotFound("no such health report".to_string()));
        }
        Err(error) => {
            tracing::warn!(session_id = %session_id, error = %error, "health report read failed");
            return Err(AppError::Upstream("health storage error".to_string()));
        }
    };

    let report = parse_report(&String::from_utf8_lossy(&bytes)).map_err(|_| {
        // The parse error can quote the offending input, which came out of a session
        // pod, so the detail is deliberately NOT logged here — the in-pod publisher
        // already logged a redacted version, and it refuses to publish anything that
        // does not parse, so reaching this arm means the object was tampered with.
        tracing::warn!(
            session_id = %session_id,
            report_id = %report_id,
            "stored health report did not parse"
        );
        AppError::Upstream("health storage error".to_string())
    })?;

    Ok(Json(SessionHealthReport {
        session_id,
        id: entry.id.clone(),
        generated_at: entry.generated_at.clone(),
        window_start: report
            .window_start
            .map(|at| at.to_rfc3339_opts(k8s_openapi::chrono::SecondsFormat::Secs, true)),
        status: report.status,
        status_raw: report.status_raw,
        headline: report.headline,
        producer: report.producer,
        confidence: report.confidence,
        expected_interval_secs: report.expected_interval_secs,
        evidence: report.evidence,
        work_items: report.work_items,
        body_markdown: report.body_markdown,
    }))
}

/// Read the session's index, through the shared TTL cache.
///
/// An absent index is an EMPTY list, not a 404: the session exists and the caller is
/// authorized, there is simply nothing yet. A 404 here would leave the UI unable to
/// tell "not authorized" from "first report still pending" — exactly the distinction a
/// user needs in the first ten minutes.
async fn fetch_index(
    state: &AppState,
    session_id: &str,
) -> Result<Vec<HealthIndexEntry>, AppError> {
    let cache_key = format!("{INDEX_CACHE_PREFIX}{session_id}");
    if let Some(bytes) = state.log_bundle_cache.get(&cache_key) {
        return Ok(parse_index(&bytes));
    }
    let storage = storage(state)?;
    match storage.download(&health_index_key(session_id)).await {
        Ok(bytes) => {
            state.log_bundle_cache.put(cache_key, bytes.clone());
            Ok(parse_index(&bytes))
        }
        Err(StorageError::Status { status: 404 }) => Ok(Vec::new()),
        Err(error) => {
            tracing::warn!(session_id = %session_id, error = %error, "health index read failed");
            Err(AppError::Upstream("health storage error".to_string()))
        }
    }
}

fn storage(state: &AppState) -> Result<&crate::storage::ChronoStorageClient, AppError> {
    state
        .storage
        .as_deref()
        .ok_or_else(|| AppError::Unavailable(NO_STORAGE.to_string()))
}

/// Is a runtime observably live right now?
///
/// FAILS OPEN in both directions that matter: session dispatch disabled is `false`
/// (and NOT a 503 — the listing still serves its history), and a backend error is
/// `false` too. An unreachable backend is a control-plane problem and must never be
/// rendered to a user as "your session is stuck".
async fn is_live(state: &AppState, session_id: &str) -> bool {
    let Some(backend) = state.session_backend.as_ref() else {
        return false;
    };
    match backend.status_summary(session_id).await {
        Ok(status) => status.phase.as_deref() == Some("Running"),
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "health staleness: runtime liveness unreadable; reporting not_running"
            );
            false
        }
    }
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(session_health))
        .routes(routes!(session_health_report))
}

#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests_staleness.rs"]
mod tests_staleness;
