//! The per-session health **index**: the small object that makes the current verdict
//! readable without pulling a whole log bundle.
//!
//! # Why an index exists at all
//!
//! Reports are durable in the session's `latest.tar.gz`, but that is typically
//! megabytes and must be fetched and decompressed in full to answer "what is this
//! session's health right now?". The dashboard asks that on every session card and the
//! staleness watchdog asks it on every read, so both need a kilobyte read, not a
//! megabyte one. chrono-storage has no list-by-prefix API, so an index object is also
//! the only way reports are *discoverable* at all — the same reason
//! [`crate::session_pod::log_stream::runs`] exists for log runs.
//!
//! # Layout
//!
//! ```text
//! health/<session_id>/index.json                <- small, polled
//! health/<session_id>/<report-filename>.md      <- immutable, written once
//! ```
//!
//! Deliberately a SIBLING of `logs/<session_id>/…` rather than nested inside it:
//! health reports have a different retention story and a different read path, and
//! flattening them under `logs/` would make the log-bundle cache key ambiguous.
//!
//! # Denormalization is the point
//!
//! Each entry carries `status` / `headline` / `generated_at` /
//! `expected_interval_secs`, so one small GET answers both the badge and the staleness
//! check without touching a single report object.
//!
//! This module is PURE — the transforms below are the machinery the in-pod publisher
//! and the read API share, and neither performs I/O here.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::naming::ReportName;
use super::report::HealthReport;

/// Version of the index envelope itself (distinct from a report's schema version).
pub const HEALTH_INDEX_SCHEMA: u32 = 1;

/// Most entries retained in the index.
///
/// Older entries drop out while their `.md` objects remain addressable — the index
/// bounds what is *listed*, never what exists.
pub const MAX_INDEX_ENTRIES: usize = 200;

/// Object key of a session's index.
pub fn health_index_key(session_id: &str) -> String {
    format!("health/{session_id}/index.json")
}

/// Object key of one report.
///
/// `file_name` MUST have come from [`super::parse_report_filename`], which is what
/// guarantees it carries no path separator and cannot escape the prefix.
pub fn health_report_key(session_id: &str, file_name: &str) -> String {
    format!("health/{session_id}/{file_name}")
}

/// One report, denormalized enough to render a badge and judge staleness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HealthIndexEntry {
    /// The report filename without its extension — URL-safe by construction, so it
    /// doubles as the API path segment addressing this report.
    pub id: String,
    /// Object key of the report itself.
    pub key: String,
    /// RFC3339 UTC, normalized on write so lexical order is chronological order.
    pub generated_at: String,
    /// The producer's declared cadence, carried so the staleness check never
    /// hardcodes a package's tick.
    pub expected_interval_secs: u64,
    /// The producer's verdict, **raw**. Relayed verbatim and mapped onto the taxonomy
    /// by the reader, so an unrecognized future verdict survives the round trip
    /// instead of being flattened to `unknown` on the way in.
    pub status: String,
    /// The producer's one-line summary.
    pub headline: String,
    /// `<name>@<version>` of the producing package.
    pub producer: String,
}

/// A session's index object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HealthIndex {
    /// Always [`HEALTH_INDEX_SCHEMA`].
    pub schema: u32,
    /// The session these reports belong to.
    pub session_id: String,
    /// Newest first, capped at [`MAX_INDEX_ENTRIES`].
    #[serde(default)]
    pub reports: Vec<HealthIndexEntry>,
}

/// Build the index entry for a parsed report.
pub fn index_entry(session_id: &str, name: &ReportName, report: &HealthReport) -> HealthIndexEntry {
    HealthIndexEntry {
        id: name.id.clone(),
        key: health_report_key(
            session_id,
            &format!("{}{}", name.id, super::naming::REPORT_FILENAME_SUFFIX),
        ),
        // Normalized rather than echoed: the producer may write any RFC3339 rendering,
        // and the newest-first sort below is a string compare.
        generated_at: report
            .generated_at
            .to_rfc3339_opts(k8s_openapi::chrono::SecondsFormat::Secs, true),
        expected_interval_secs: report.expected_interval_secs,
        status: report.status_raw.clone(),
        headline: report.headline.clone(),
        producer: report.producer.clone(),
    }
}

/// Read an index object leniently: malformed, truncated, or absent bytes all degrade
/// to an empty list rather than ever failing.
///
/// The publisher is best-effort and must never crash a session over a corrupt index,
/// and the reader must never turn one into a user-visible error — the next tick
/// rewrites it either way.
pub fn parse_index(bytes: &[u8]) -> Vec<HealthIndexEntry> {
    serde_json::from_slice::<HealthIndex>(bytes)
        .map(|index| index.reports)
        .unwrap_or_default()
}

/// Fold `entry` into the existing index and render the new object.
///
/// Deduped **by id, replacing in place** — unlike
/// [`crate::session_pod::log_stream::runs::upsert_run`], which leaves an existing
/// record untouched. A report file can legitimately be rewritten in place (the
/// producer writes atomically, but a retry within the same second reuses the
/// filename), and in that case the newer verdict is the true one; keeping the stale
/// entry would pin a wrong status on the dashboard until the next tick.
///
/// Sorted newest-first and truncated to [`MAX_INDEX_ENTRIES`].
pub fn upsert_report(existing: Option<&[u8]>, session_id: &str, entry: HealthIndexEntry) -> String {
    let mut reports = existing.map(parse_index).unwrap_or_default();
    reports.retain(|candidate| candidate.id != entry.id);
    reports.push(entry);
    // `generated_at` is normalized on write, so the string compare is a time compare;
    // `id` breaks a tie deterministically (it carries the same stamp plus identity).
    reports.sort_by(|a, b| {
        b.generated_at
            .cmp(&a.generated_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    reports.truncate(MAX_INDEX_ENTRIES);

    let index = HealthIndex {
        schema: HEALTH_INDEX_SCHEMA,
        session_id: session_id.to_string(),
        reports,
    };
    // Infallible by construction: the value is plain owned strings and integers.
    serde_json::to_string_pretty(&index)
        .map(|mut json| {
            json.push('\n');
            json
        })
        .unwrap_or_else(|_| format!("{{\"schema\":{HEALTH_INDEX_SCHEMA},\"reports\":[]}}\n"))
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
