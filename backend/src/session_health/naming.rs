//! Report filenames: one generator, one parser, so the two can never drift.
//!
//! ```text
//! <namespace>-<session_id>-health-agent-status-report-<YYYYMMDD>-<HHMMSS>.md
//! ```
//!
//! # Why the timestamp is colon-free
//!
//! The same string becomes a chrono-storage object key, a tar entry path, and a URL
//! path segment. A `:` is legal in an object key but must be percent-encoded in a URL
//! and breaks a good deal of tooling, so the stamp is `HHMMSS` — sortable and safe
//! everywhere. It is always **UTC**.
//!
//! # Why `<namespace>` may be absent
//!
//! It is the fkst **work-label** namespace (`FKST_WORK_LABEL_NAMESPACE`), not a
//! Kubernetes namespace. When it is unset the session's labels are unnamespaced, so
//! there is genuinely nothing to name: the segment **and its joining hyphen** are
//! omitted rather than filled with a placeholder.
//!
//! # How the two hyphen-bearing segments are told apart
//!
//! Both a namespace (`chronoai-fkst`) and a session id
//! (`8f2c1d64-…`, a hyphenated UUID from
//! [`crate::session_spec::derive_session_id`]) contain hyphens, so the join is
//! ambiguous on its face. The parser resolves it by anchoring on the **UUID shape**:
//! the session id is the trailing 36 characters of the prefix, and whatever precedes
//! its joining hyphen is the namespace. Because the true session id is always the last
//! 36 characters, this is exact even if a namespace itself ended in something
//! UUID-shaped.
//!
//! For a session id that is *not* a UUID — which production never produces, but a test
//! may — the split is genuinely unrecoverable, and the parser reports the whole prefix
//! as the session id with no namespace. Round-tripping is therefore guaranteed for
//! UUID session ids (the real contract) and for the no-namespace case.

use k8s_openapi::chrono::{DateTime, Utc};

/// The fixed middle segment that makes a report filename recognizable.
pub const REPORT_FILENAME_MARKER: &str = "-health-agent-status-report-";

/// The extension every report carries, including the dot.
pub const REPORT_FILENAME_SUFFIX: &str = ".md";

/// `strftime` format of the filename's date segment (UTC).
pub const REPORT_STAMP_FORMAT: &str = "%Y%m%d-%H%M%S";

/// Length of a canonical hyphenated UUID, which is what a session id is.
const UUID_LEN: usize = 36;

/// A parsed report filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportName {
    /// The fkst work-label namespace, when the filename carries one and it is
    /// recoverable (see the module docs).
    pub namespace: Option<String>,
    /// The session id the report belongs to.
    pub session_id: String,
    /// The `YYYYMMDD-HHMMSS` UTC stamp, verbatim.
    pub stamp: String,
    /// The filename without its extension.
    ///
    /// This is the report's public **id**: URL-safe by construction — no `:`, no path
    /// separator — so it is usable directly as an API path segment and as an object
    /// key component.
    pub id: String,
}

/// Render the filename a producer must write for a report generated at `generated_at`.
///
/// `namespace` is omitted along with its joining hyphen when `None`.
pub fn report_filename(
    namespace: Option<&str>,
    session_id: &str,
    generated_at: DateTime<Utc>,
) -> String {
    let stamp = generated_at.format(REPORT_STAMP_FORMAT);
    match namespace.map(str::trim).filter(|ns| !ns.is_empty()) {
        Some(namespace) => {
            format!(
                "{namespace}-{session_id}{REPORT_FILENAME_MARKER}{stamp}{REPORT_FILENAME_SUFFIX}"
            )
        }
        None => format!("{session_id}{REPORT_FILENAME_MARKER}{stamp}{REPORT_FILENAME_SUFFIX}"),
    }
}

/// Parse a report filename, or `None` when it is not one.
///
/// This doubles as the **traversal guard** for every consumer that turns a
/// producer-chosen filename into a path or an object key: a name carrying a path
/// separator, a `.` / `..` component, or a control character is rejected outright, so a
/// conforming name can never escape the directory or key prefix it belongs to.
pub fn parse_report_filename(name: &str) -> Option<ReportName> {
    if name.contains('/') || name.contains('\\') || name.contains(['\0', '\n', '\r']) {
        return None;
    }
    let id = name.strip_suffix(REPORT_FILENAME_SUFFIX)?;
    if id.is_empty() || id == "." || id == ".." || id.starts_with('.') {
        return None;
    }

    // `rfind`: if a namespace or session id somehow contained the marker, the real
    // marker is still the last one, because the stamp that follows it cannot.
    let marker_at = id.rfind(REPORT_FILENAME_MARKER)?;
    let prefix = &id[..marker_at];
    let stamp = &id[marker_at + REPORT_FILENAME_MARKER.len()..];

    if prefix.is_empty() || !is_stamp(stamp) {
        return None;
    }
    let (namespace, session_id) = split_prefix(prefix);

    Some(ReportName {
        namespace,
        session_id,
        stamp: stamp.to_string(),
        id: id.to_string(),
    })
}

/// `YYYYMMDD-HHMMSS`: exactly eight digits, a hyphen, exactly six digits.
fn is_stamp(stamp: &str) -> bool {
    let Some((date, time)) = stamp.split_once('-') else {
        return false;
    };
    date.len() == 8
        && time.len() == 6
        && date.bytes().all(|b| b.is_ascii_digit())
        && time.bytes().all(|b| b.is_ascii_digit())
}

/// Split `<namespace>-<session_id>` (or a bare `<session_id>`) by anchoring on the
/// trailing UUID. See the module docs for why this is exact for real session ids.
fn split_prefix(prefix: &str) -> (Option<String>, String) {
    if prefix.len() == UUID_LEN && is_uuid(prefix) {
        return (None, prefix.to_string());
    }
    if prefix.len() > UUID_LEN + 1 {
        let split_at = prefix.len() - UUID_LEN;
        if prefix.is_char_boundary(split_at)
            && is_uuid(&prefix[split_at..])
            && prefix.as_bytes()[split_at - 1] == b'-'
        {
            return (
                Some(prefix[..split_at - 1].to_string()),
                prefix[split_at..].to_string(),
            );
        }
    }
    // No UUID anchor: the namespace, if any, is not recoverable — report what is
    // certain rather than guessing at a split.
    (None, prefix.to_string())
}

fn is_uuid(candidate: &str) -> bool {
    candidate.len() == UUID_LEN && uuid::Uuid::parse_str(candidate).is_ok()
}

#[cfg(test)]
#[path = "naming_tests.rs"]
mod tests;
