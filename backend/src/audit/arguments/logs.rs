//! Safe arguments for the session-log surfaces and the engine observe snapshot.
//!
//! Every route here exists to hand a caller CONTENT — an archive, a decompressed
//! log file, an engine snapshot. None of that content, and nothing derived from
//! it, is an audit property: not the archive bytes, not a log line, not a queue
//! or delivery id, not the presigned storage URL the control plane uses
//! internally, and not the `Authorization` header or OAuth token that
//! authorized the read.
//!
//! What remains is the request's own shape: which session, which run, which
//! CLASS of file, how much tail was asked for, and which transport established
//! identity. The requested file PATH is deliberately replaced by its class — the
//! bundle's own fixed classifier — because a path is caller-supplied text that
//! is matched against the archive, and an unmatched one is exactly the kind of
//! probe string that must not be retained.

use serde::Serialize;

use super::bounds::{safe_run_id, safe_session_id};
use super::catalog;
use super::{sealed::Sealed, BoundedAuditArguments, ToSafeAuditArguments};
use crate::audit::event::ArgumentsParseStatus;

/// How a log download established identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogDownloadMode {
    /// An `Authorization: Bearer` header was supplied and traded for `/user`.
    Bearer,
    /// No header: the request was redirected into the browser OAuth round-trip.
    BrowserRedirect,
}

/// The bundle file classes the viewer's own classifier produces, plus the
/// catch-all the spec reserves. A closed enum, so a requested path can never
/// become a property value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFileClass {
    Driver,
    Supervise,
    Codex,
    Misc,
    Readme,
    Meta,
    Other,
}

impl LogFileClass {
    /// Map the viewer's friendly label onto the closed enum.
    ///
    /// The classifier is the single source of truth for what a bundle path
    /// means; this only narrows its output to a wire-safe value, and answers
    /// [`LogFileClass::Other`] for anything it does not recognize rather than
    /// passing an unknown string through.
    pub fn from_label(label: &str) -> Self {
        match label {
            "Driver" => Self::Driver,
            "Supervise" => Self::Supervise,
            "Codex" => Self::Codex,
            "Misc" => Self::Misc,
            "README" => Self::Readme,
            "Meta" => Self::Meta,
            _ => Self::Other,
        }
    }
}

/// The session-id + run-selector pair every log record carries.
#[derive(Clone, Debug, Serialize)]
struct SessionRun {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// The validated run id, or `latest` for the authoritative bundle. Absent
    /// when the caller's `?run=` value was not a run id.
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id_or_latest: Option<String>,
}

impl SessionRun {
    fn new(session_id: &str, run: Option<&str>) -> Self {
        Self {
            session_id: safe_session_id(session_id),
            run_id_or_latest: safe_run_id(run),
        }
    }

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.session_id.is_some() && self.run_id_or_latest.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

/// `download_session_logs` — the whole redacted bundle.
#[derive(Clone, Debug, Serialize)]
pub struct SafeDownloadSessionLogs {
    #[serde(flatten)]
    session: SessionRun,
    mode: LogDownloadMode,
}

impl SafeDownloadSessionLogs {
    pub fn new(session_id: &str, run: Option<&str>, mode: LogDownloadMode) -> Self {
        Self {
            session: SessionRun::new(session_id, run),
            mode,
        }
    }
}

impl BoundedAuditArguments for SafeDownloadSessionLogs {
    const OPERATION_ID: &'static str = "download_session_logs";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::DOWNLOAD_SESSION_LOGS_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        self.session.parse_status()
    }
}

/// `list_session_runs` — the session's pod incarnations.
#[derive(Clone, Debug, Serialize)]
pub struct SafeListSessionRuns {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

impl SafeListSessionRuns {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: safe_session_id(session_id),
        }
    }
}

impl BoundedAuditArguments for SafeListSessionRuns {
    const OPERATION_ID: &'static str = "list_session_runs";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::LIST_SESSION_RUNS_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.session_id.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

/// `session_health` — the session's health reports plus the heartbeat verdict.
///
/// Only the validated session id. The reports themselves are package-authored
/// prose whose `status`/`headline` the control plane relays verbatim, so nothing
/// about their CONTENT is an audit property.
#[derive(Clone, Debug, Serialize)]
pub struct SafeSessionHealth {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

impl SafeSessionHealth {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: safe_session_id(session_id),
        }
    }
}

impl BoundedAuditArguments for SafeSessionHealth {
    const OPERATION_ID: &'static str = "session_health";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::SESSION_HEALTH_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.session_id.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

/// `session_health_report` — one report out of that list.
///
/// `report_id` is caller-supplied and is admitted only in the same bounded ASCII
/// token shape a session id must take. An id outside that shape is OMITTED and
/// the record reports `invalid`: the handler answers `404` for anything absent
/// from the index, and echoing a traversal-shaped selector into the trail is the
/// exact "never echo invalid material" rule. The report BODY is untrusted
/// markdown and is never an argument.
#[derive(Clone, Debug, Serialize)]
pub struct SafeSessionHealthReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    report_id: Option<String>,
}

impl SafeSessionHealthReport {
    pub fn new(session_id: &str, report_id: &str) -> Self {
        Self {
            session_id: safe_session_id(session_id),
            report_id: safe_session_id(report_id),
        }
    }
}

impl BoundedAuditArguments for SafeSessionHealthReport {
    const OPERATION_ID: &'static str = "session_health_report";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::SESSION_HEALTH_REPORT_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.session_id.is_some() && self.report_id.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

/// `session_log_manifest` — the bundle's file list.
#[derive(Clone, Debug, Serialize)]
pub struct SafeSessionLogManifest {
    #[serde(flatten)]
    session: SessionRun,
}

impl SafeSessionLogManifest {
    pub fn new(session_id: &str, run: Option<&str>) -> Self {
        Self {
            session: SessionRun::new(session_id, run),
        }
    }
}

impl BoundedAuditArguments for SafeSessionLogManifest {
    const OPERATION_ID: &'static str = "session_log_manifest";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::SESSION_LOG_MANIFEST_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        self.session.parse_status()
    }
}

/// `session_log_file` — one file out of the bundle.
#[derive(Clone, Debug, Serialize)]
pub struct SafeSessionLogFile {
    #[serde(flatten)]
    session: SessionRun,
    file_class: LogFileClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    tail_bytes: Option<u64>,
}

impl BoundedAuditArguments for SafeSessionLogFile {
    const OPERATION_ID: &'static str = "session_log_file";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::SESSION_LOG_FILE_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        self.session.parse_status()
    }
}

/// The input view for `session_log_file`. The requested path is consumed by the
/// classifier and never stored.
pub struct SessionLogFileInput<'a> {
    pub session_id: &'a str,
    pub run: Option<&'a str>,
    /// The label the bundle classifier produced for the requested path.
    pub file_label: &'a str,
    pub tail_bytes: Option<u64>,
}

impl Sealed for SessionLogFileInput<'_> {}

impl ToSafeAuditArguments for SessionLogFileInput<'_> {
    type Safe = SafeSessionLogFile;

    fn to_safe_audit_arguments(&self) -> Self::Safe {
        SafeSessionLogFile {
            session: SessionRun::new(self.session_id, self.run),
            file_class: LogFileClass::from_label(self.file_label),
            tail_bytes: self.tail_bytes,
        }
    }
}

/// `observe_session` — the engine's queue/delivery snapshot.
///
/// `effective_limit` is the CLAMPED value, so the record describes what the
/// handler executed rather than what an untrusted query asked for.
#[derive(Clone, Debug, Serialize)]
pub struct SafeObserveSession {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    effective_limit: u32,
}

impl SafeObserveSession {
    pub fn new(session_id: &str, effective_limit: u32) -> Self {
        Self {
            session_id: safe_session_id(session_id),
            effective_limit,
        }
    }
}

impl BoundedAuditArguments for SafeObserveSession {
    const OPERATION_ID: &'static str = "observe_session";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::OBSERVE_SESSION_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.session_id.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

#[cfg(test)]
#[path = "logs_tests.rs"]
mod tests;
