//! The NORMATIVE per-operation safe-argument allowlist.
//!
//! Each constant below is the complete set of property names one operation's
//! record may ever carry under `arguments`. They are transcribed from issue
//! #5671's operation catalog and are the single source of truth shared by three
//! independent enforcement points:
//!
//! - the DTO's own [`BoundedAuditArguments::ALLOWED_FIELDS`], so a field a DTO
//!   grows without updating this table is DROPPED at record time and logged;
//! - [`crate::audit::request::policy::OPERATION_POLICIES`], which pairs every
//!   audited operation with exactly one [`SafeArgumentSpec`] — an audited
//!   operation with no spec fails the router build, and therefore CI;
//! - the coverage tests, which compare this table against the live OpenAPI
//!   document so an operation id cannot drift without the table drifting too.
//!
//! [`BoundedAuditArguments::ALLOWED_FIELDS`]: super::BoundedAuditArguments::ALLOWED_FIELDS
//!
//! ## Why the truncation markers are here
//!
//! The spec's table names `package_refs[]` / `package_count`; the same spec
//! separately requires an over-long list to keep `count`, a bounded prefix, and
//! `truncated=true`. The `*_truncated` keys are that marker, emitted only when a
//! list really was clipped, so a reader can never mistake a prefix for the whole
//! request.

/// One operation's safe-argument policy: the DTO that produces it, and the exact
/// property names it may emit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SafeArgumentSpec {
    /// The Rust type name of the DTO, for error messages and drift tests.
    pub dto: &'static str,
    /// Every property name the DTO may emit. Nothing else is ever recorded.
    pub fields: &'static [&'static str],
}

impl SafeArgumentSpec {
    pub const fn new(dto: &'static str, fields: &'static [&'static str]) -> Self {
        Self { dto, fields }
    }
}

/// The universal shape of a rejected/unparseable request body.
///
/// Deliberately NOT per-operation: a body that failed to parse has no operation
/// arguments, only transport metadata. Nothing derived from the bytes, the query
/// string, or the parser's message ever appears here.
pub const INVALID_INPUT_FIELDS: &[&str] = &[
    "content_type",
    "content_length_declared",
    "body_bytes_observed",
];

// --- authentication and OAuth ------------------------------------------------

pub const GITHUB_LOGIN_FIELDS: &[&str] = &["flow"];
pub const GITHUB_LOGIN_CALLBACK_FIELDS: &[&str] = &["flow", "result"];
pub const GITHUB_REFRESH_TOKEN_FIELDS: &[&str] = &["flow", "result"];
pub const GITHUB_BROADER_CONNECT_FIELDS: &[&str] = &["flow"];
pub const GITHUB_BROADER_CALLBACK_FIELDS: &[&str] = &["flow", "result"];
/// `session_id` appears ONLY after the signed state verified: an unverified
/// state is attacker-chosen text, so nothing is extracted from it.
pub const SESSION_LOGS_OAUTH_CALLBACK_FIELDS: &[&str] = &["flow", "session_id", "result"];

// --- repositories and installations ------------------------------------------

pub const CREATE_REPO_FIELDS: &[&str] = &[
    "owner",
    "name",
    "private",
    "description_present",
    "description_bytes",
];
pub const UNINSTALL_ACCOUNT_FIELDS: &[&str] = &["owner"];

// --- canvas -------------------------------------------------------------------

/// The optional broader-visibility header is recorded as a PRESENCE flag; its
/// value is a GitHub credential and is never read into a record.
pub const CANVAS_OVERVIEW_FIELDS: &[&str] = &["broader_visibility_requested"];
pub const CANVAS_REPO_SESSIONS_FIELDS: &[&str] = &["owner", "repo"];
pub const CANVAS_CREATE_SESSION_FIELDS: &[&str] = &[
    "owner",
    "repo",
    "package_refs",
    "package_count",
    "package_refs_truncated",
    "manifest_refs",
    "manifest_count",
    "manifest_refs_truncated",
    "work_label",
    "environment_name",
    "disposable_environment_present",
    "disposable_variable_count",
    "disposable_secret_count",
    "source_branch",
    "target_branch",
    "auto_merge",
    "log_access_count",
    "collaborator_count",
    "output_language",
];
pub const CANVAS_STOP_SESSION_FIELDS: &[&str] = &["owner", "repo", "trigger_issue"];
pub const CANVAS_CREATE_WORK_ITEM_FIELDS: &[&str] = &[
    "owner",
    "repo",
    "trigger_issue",
    "selected_label",
    "title_bytes",
    "body_present",
    "body_bytes",
];
pub const CANVAS_SESSION_OUTCOMES_FIELDS: &[&str] = &["owner", "repo", "trigger_issue"];
pub const CANVAS_OUTCOME_BLOB_FIELDS: &[&str] = &["owner", "repo", "blob_sha", "download"];

// --- named environment profiles ----------------------------------------------

pub const PUT_USER_ENVIRONMENT_PROFILE_FIELDS: &[&str] = &[
    "environment_name",
    "install_command_count",
    "variable_count",
    "secret_count",
];
pub const GET_USER_ENVIRONMENT_PROFILE_FIELDS: &[&str] = &["environment_name"];
pub const DELETE_USER_ENVIRONMENT_PROFILE_FIELDS: &[&str] = &["environment_name"];

// --- logs and observe ---------------------------------------------------------

pub const DOWNLOAD_SESSION_LOGS_FIELDS: &[&str] = &["session_id", "run_id_or_latest", "mode"];
pub const LIST_SESSION_RUNS_FIELDS: &[&str] = &["session_id"];
pub const SESSION_LOG_MANIFEST_FIELDS: &[&str] = &["session_id", "run_id_or_latest"];
pub const SESSION_LOG_FILE_FIELDS: &[&str] =
    &["session_id", "run_id_or_latest", "file_class", "tail_bytes"];
/// `effective_limit` is the CLAMPED value the handler actually executed with, so
/// the record describes execution rather than untrusted input.
pub const OBSERVE_SESSION_FIELDS: &[&str] = &["session_id", "effective_limit"];

// --- inbound webhook ----------------------------------------------------------

/// `signature_valid` is the ONLY property a rejected delivery contributes; every
/// other field is populated exclusively from a body whose HMAC already verified.
pub const GITHUB_APP_WEBHOOK_FIELDS: &[&str] = &[
    "signature_valid",
    "event_type",
    "action",
    "installation_id",
    "repo_full_name",
    "trigger_issue",
    "delivery_id",
    "handling",
];

// --- chat concierge -----------------------------------------------------------

/// Added after the spec's table was written. Counts and sizes only: a chat turn
/// carries the user's prompt and the model's answer, none of which is a valid
/// audit property.
pub const CHAT_TURN_FIELDS: &[&str] = &[
    "message_count",
    "user_message_count",
    "assistant_message_count",
    "total_content_bytes",
    "last_message_bytes",
    "broader_visibility_requested",
];

// --- operations surface (reserved) -------------------------------------------

/// Reserved for `GET /api/v1/operations/activity` (issue #5672).
pub const OPERATIONS_LIST_ACTIVITY_FIELDS: &[&str] = &[
    "scope",
    "requested_scope",
    "record_kind",
    "from",
    "to",
    "limit",
    "cursor_present",
    "actor_filter_present",
    "session_id",
    "repo_full_name",
    "trigger_issue",
    "request_id",
    "method",
    "operation_id",
    "status",
];

/// Reserved for `GET /api/v1/operations/sandboxes` (issue #5675).
pub const OPERATIONS_LIST_SANDBOXES_FIELDS: &[&str] = &[
    "scope",
    "requested_scope",
    "session_id",
    "repo_full_name",
    "trigger_issue",
    "status",
    "limit",
];

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
