//! Safe arguments for the canvas READ surfaces plus the two pure-identifier
//! mutations (stop a session, stream one committed blob).
//!
//! The canvas is the route family that fans out into GitHub, so the temptation
//! is to record what came back. Nothing here does: a record names the repository
//! and the issue the caller addressed, never an issue body, a comment, a pull
//! request title, a file name, a presigned URL, or a byte of blob content.
//!
//! The create/queue mutations, whose inputs are large enough to need their own
//! projection rules, live in [`super::canvas_write`].

use serde::Serialize;

use super::bounds::{safe_blob_sha, safe_owner, safe_repo};
use super::catalog;
use super::{sealed::Sealed, BoundedAuditArguments, ToSafeAuditArguments};
use crate::audit::event::ArgumentsParseStatus;

/// The validated `owner`/`repo` pair every repository-scoped canvas record
/// carries. Both are `None` together when the request's segments were not in the
/// validated form, which is what makes the shared `parse_status` below honest.
#[derive(Clone, Debug, Default, Serialize)]
struct RepoPair {
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
}

impl RepoPair {
    fn new(owner: &str, repo: &str) -> Self {
        Self {
            owner: safe_owner(owner),
            repo: safe_repo(repo),
        }
    }

    /// `Parsed` only when BOTH segments validated: a half-identified repository
    /// would be a misleading correlation handle.
    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.owner.is_some() && self.repo.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

/// `canvas_overview` — the whole-account canvas.
///
/// The optional broader-visibility header is a GitHub credential. Only its
/// PRESENCE is recorded, and only after the route recognized the header; the
/// value is never read here, and whether it was ultimately trusted is a
/// same-user check the record deliberately does not second-guess.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SafeCanvasOverview {
    broader_visibility_requested: bool,
}

impl SafeCanvasOverview {
    pub fn new(broader_visibility_requested: bool) -> Self {
        Self {
            broader_visibility_requested,
        }
    }
}

impl BoundedAuditArguments for SafeCanvasOverview {
    const OPERATION_ID: &'static str = "canvas_overview";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CANVAS_OVERVIEW_FIELDS;
}

/// `canvas_repo_sessions` — one repository's live session list.
#[derive(Clone, Debug, Serialize)]
pub struct SafeCanvasRepoSessions {
    #[serde(flatten)]
    repo: RepoPair,
}

impl SafeCanvasRepoSessions {
    pub fn new(owner: &str, repo: &str) -> Self {
        Self {
            repo: RepoPair::new(owner, repo),
        }
    }
}

impl BoundedAuditArguments for SafeCanvasRepoSessions {
    const OPERATION_ID: &'static str = "canvas_repo_sessions";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CANVAS_REPO_SESSIONS_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        self.repo.parse_status()
    }
}

/// `canvas_stop_session` — closing a session's trigger issue.
#[derive(Clone, Debug, Serialize)]
pub struct SafeCanvasStopSession {
    #[serde(flatten)]
    repo: RepoPair,
    trigger_issue: i64,
}

impl SafeCanvasStopSession {
    pub fn new(owner: &str, repo: &str, trigger_issue: i64) -> Self {
        Self {
            repo: RepoPair::new(owner, repo),
            trigger_issue,
        }
    }
}

impl BoundedAuditArguments for SafeCanvasStopSession {
    const OPERATION_ID: &'static str = "canvas_stop_session";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CANVAS_STOP_SESSION_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        self.repo.parse_status()
    }
}

/// `canvas_session_outcomes` — a session's devloop pull requests and their
/// changed-file lists.
#[derive(Clone, Debug, Serialize)]
pub struct SafeCanvasSessionOutcomes {
    #[serde(flatten)]
    repo: RepoPair,
    trigger_issue: i64,
}

impl SafeCanvasSessionOutcomes {
    pub fn new(owner: &str, repo: &str, trigger_issue: i64) -> Self {
        Self {
            repo: RepoPair::new(owner, repo),
            trigger_issue,
        }
    }
}

impl BoundedAuditArguments for SafeCanvasSessionOutcomes {
    const OPERATION_ID: &'static str = "canvas_session_outcomes";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CANVAS_SESSION_OUTCOMES_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        self.repo.parse_status()
    }
}

/// `canvas_outcome_blob` — streaming one committed file.
///
/// The `?name=` query drives the content type and the download filename, and is
/// caller-supplied free text (a path, possibly with a token in it). It is
/// deliberately absent: `blob_sha` already identifies the object exactly, and
/// does so in a form that has been validated as a git object id.
#[derive(Clone, Debug, Serialize)]
pub struct SafeCanvasOutcomeBlob {
    #[serde(flatten)]
    repo: RepoPair,
    #[serde(skip_serializing_if = "Option::is_none")]
    blob_sha: Option<String>,
    download: bool,
}

impl BoundedAuditArguments for SafeCanvasOutcomeBlob {
    const OPERATION_ID: &'static str = "canvas_outcome_blob";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CANVAS_OUTCOME_BLOB_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.blob_sha.is_none() {
            return ArgumentsParseStatus::Invalid;
        }
        self.repo.parse_status()
    }
}

/// The input view for `canvas_outcome_blob`.
pub struct OutcomeBlobInput<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    /// The raw `{sha}` segment; captured only once it validates as hex.
    pub blob_sha: &'a str,
    /// The handler's own interpretation of `?download=` (`1` means attachment).
    pub download: bool,
}

impl Sealed for OutcomeBlobInput<'_> {}

impl ToSafeAuditArguments for OutcomeBlobInput<'_> {
    type Safe = SafeCanvasOutcomeBlob;

    fn to_safe_audit_arguments(&self) -> Self::Safe {
        SafeCanvasOutcomeBlob {
            repo: RepoPair::new(self.owner, self.repo),
            blob_sha: safe_blob_sha(self.blob_sha),
            download: self.download,
        }
    }
}

// --- scheduled workflows ----------------------------------------------------
//
// Six operations that differ only in which identifiers they address, so their
// records are generated rather than hand-written six times. What is deliberately
// NOT here is anything out of a definition's body — the workflow id, the
// arguments, the cadence — because that is author-written content, and the whole
// point of this module is that a record names what was addressed, never what it
// contained.

/// Declare one repository-and-definition-scoped safe record.
macro_rules! schedule_record {
    ($name:ident, $operation:literal, $fields:ident) => {
        #[derive(Clone, Debug, Serialize)]
        pub struct $name {
            #[serde(flatten)]
            repo: RepoPair,
            schedule_issue: i64,
        }

        impl $name {
            pub fn new(owner: &str, repo: &str, schedule_issue: i64) -> Self {
                Self {
                    repo: RepoPair::new(owner, repo),
                    schedule_issue,
                }
            }
        }

        impl BoundedAuditArguments for $name {
            const OPERATION_ID: &'static str = $operation;
            const ALLOWED_FIELDS: &'static [&'static str] = catalog::$fields;

            fn parse_status(&self) -> ArgumentsParseStatus {
                self.repo.parse_status()
            }
        }
    };
}

/// `canvas_repo_schedules` — one repository's scheduled workflows.
#[derive(Clone, Debug, Serialize)]
pub struct SafeCanvasRepoSchedules {
    #[serde(flatten)]
    repo: RepoPair,
}

impl SafeCanvasRepoSchedules {
    pub fn new(owner: &str, repo: &str) -> Self {
        Self {
            repo: RepoPair::new(owner, repo),
        }
    }
}

impl BoundedAuditArguments for SafeCanvasRepoSchedules {
    const OPERATION_ID: &'static str = "canvas_repo_schedules";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CANVAS_REPO_SCHEDULES_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        self.repo.parse_status()
    }
}

schedule_record!(
    SafeCanvasScheduleDetail,
    "canvas_schedule_detail",
    CANVAS_SCHEDULE_DETAIL_FIELDS
);
schedule_record!(
    SafeCanvasPauseSchedule,
    "canvas_pause_schedule",
    CANVAS_PAUSE_SCHEDULE_FIELDS
);
schedule_record!(
    SafeCanvasResumeSchedule,
    "canvas_resume_schedule",
    CANVAS_RESUME_SCHEDULE_FIELDS
);
schedule_record!(
    SafeCanvasRunScheduleNow,
    "canvas_run_schedule_now",
    CANVAS_RUN_SCHEDULE_NOW_FIELDS
);

/// `canvas_schedule_run` — one run of one scheduled workflow.
///
/// Carries the slot as well. A slot is a clock instant the control plane itself
/// chose, never author text, so it is safe to record and it is the only handle
/// that identifies WHICH run was read.
#[derive(Clone, Debug, Serialize)]
pub struct SafeCanvasScheduleRun {
    #[serde(flatten)]
    repo: RepoPair,
    schedule_issue: i64,
    /// `None` when the request's slot segment was not an RFC 3339 instant, which
    /// keeps a malformed request from writing arbitrary text into the trail.
    #[serde(skip_serializing_if = "Option::is_none")]
    slot: Option<String>,
}

impl SafeCanvasScheduleRun {
    pub fn new(owner: &str, repo: &str, schedule_issue: i64, slot: &str) -> Self {
        Self {
            repo: RepoPair::new(owner, repo),
            schedule_issue,
            slot: k8s_openapi::chrono::DateTime::parse_from_rfc3339(slot)
                .ok()
                .map(|slot| {
                    slot.with_timezone(&k8s_openapi::chrono::Utc)
                        .to_rfc3339_opts(k8s_openapi::chrono::SecondsFormat::Secs, true)
                }),
        }
    }
}

impl BoundedAuditArguments for SafeCanvasScheduleRun {
    const OPERATION_ID: &'static str = "canvas_schedule_run";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CANVAS_SCHEDULE_RUN_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        self.repo.parse_status()
    }
}

#[cfg(test)]
#[path = "canvas_tests.rs"]
mod tests;
