//! The explicit audit policy for every documented operation.
//!
//! There is deliberately no default. Each `operationId` in the generated OpenAPI
//! document must appear in [`OPERATION_POLICIES`] exactly once, as either
//! [`OperationPolicy::Audited`] or [`OperationPolicy::Excluded`] with a stated
//! reason; an operation the table does not name makes
//! [`super::catalog::OperationCatalog::from_openapi`] fail, which fails the
//! router build, which fails CI. Adding a product endpoint therefore cannot
//! silently escape the audit trail — the omission is a build error, not a hole
//! someone notices months later.
//!
//! Exclusion is a narrow privilege reserved for traffic that carries no user
//! intent: the two liveness probes, the Prometheus scrape, the contract document
//! itself, and CORS preflights. Everything else — product calls, OAuth
//! redirects, authentication failures, the signature-verified webhook, and the
//! operations surface's own polling calls — is audited. The UI may hide its own
//! polling visually, but capture is never allowed to skip it (epic `AUD-01`).
//!
//! ## Why an [`AuditOperation`] object rather than a string table
//!
//! An operation's audit decision is TWO decisions: is it recorded, and what may
//! its record contain. Keeping them apart invites the second to be forgotten —
//! an audited operation with no argument policy still builds, still records, and
//! quietly emits nothing. Pairing them in one struct makes the omission
//! impossible to express: the catalog rejects an audited operation whose
//! [`ArgumentsPolicy`] is [`ArgumentsPolicy::NotRecorded`], so a new endpoint
//! fails the router build until BOTH decisions exist.

use crate::audit::arguments::catalog as arguments;
use crate::audit::arguments::SafeArgumentSpec;
use crate::audit::event::ArgumentsParseStatus;

/// Why an operation is kept out of the audit trail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExclusionReason {
    /// A liveness/readiness probe: kubelet traffic, no user intent.
    Probe,
    /// The Prometheus scrape: one caller, no user intent, high frequency.
    Scrape,
    /// The published API contract (`/openapi.json`): a static document.
    Contract,
    /// A CORS preflight: a browser mechanic, answered before any handler.
    CorsPreflight,
}

impl ExclusionReason {
    /// The stable wire string. A bounded closed enum, so it is also the only
    /// value safe to use as a metric label or a structured-log field.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Scrape => "scrape",
            Self::Contract => "contract",
            Self::CorsPreflight => "cors_preflight",
        }
    }
}

impl std::fmt::Display for ExclusionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether an operation is recorded, and why not when it is not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationPolicy {
    Audited,
    Excluded(ExclusionReason),
}

impl OperationPolicy {
    pub fn is_audited(self) -> bool {
        matches!(self, Self::Audited)
    }
}

/// What an operation's record may carry under `arguments`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentsPolicy {
    /// The operation is excluded from the audit trail entirely, so the question
    /// does not arise. Legal ONLY alongside [`OperationPolicy::Excluded`].
    NotRecorded,
    /// The operation takes no arguments at all. Its records say
    /// `not_applicable` rather than pretending something was unavailable.
    None,
    /// Exactly one named safe DTO produces this operation's arguments.
    Safe(SafeArgumentSpec),
}

impl ArgumentsPolicy {
    /// The named DTO spec, when there is one.
    pub fn spec(self) -> Option<SafeArgumentSpec> {
        match self {
            Self::Safe(spec) => Some(spec),
            _ => None,
        }
    }

    /// The status a record carries when nothing was ever recorded.
    ///
    /// An operation that HAS arguments but recorded none was rejected before its
    /// safe parse could run (authentication, the leader gate, a timeout), which
    /// is `unavailable`. One that has none by definition is `not_applicable`.
    /// Deriving it here is what keeps every rejection site free of the question.
    pub fn default_status(self) -> ArgumentsParseStatus {
        match self {
            Self::Safe(_) => ArgumentsParseStatus::Unavailable,
            Self::None | Self::NotRecorded => ArgumentsParseStatus::NotApplicable,
        }
    }
}

/// One operation's complete audit decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditOperation {
    pub operation_id: &'static str,
    pub policy: OperationPolicy,
    pub arguments: ArgumentsPolicy,
}

/// Declare an audited operation with its named safe-argument DTO.
const fn audited(
    operation_id: &'static str,
    dto: &'static str,
    fields: &'static [&'static str],
) -> AuditOperation {
    AuditOperation {
        operation_id,
        policy: OperationPolicy::Audited,
        arguments: ArgumentsPolicy::Safe(SafeArgumentSpec::new(dto, fields)),
    }
}

/// Declare an audited operation that takes no arguments at all.
const fn audited_without_arguments(operation_id: &'static str) -> AuditOperation {
    AuditOperation {
        operation_id,
        policy: OperationPolicy::Audited,
        arguments: ArgumentsPolicy::None,
    }
}

/// Declare an operation kept out of the trail, for a stated bounded reason.
const fn excluded(operation_id: &'static str, reason: ExclusionReason) -> AuditOperation {
    AuditOperation {
        operation_id,
        policy: OperationPolicy::Excluded(reason),
        arguments: ArgumentsPolicy::NotRecorded,
    }
}

/// The complete policy table, keyed by the `operation_id` declared on each
/// `#[utoipa::path]`.
///
/// Two operations here are mounted conditionally — `github_app_webhook` (only
/// with `FKST_GITHUB_APP_WEBHOOK_SECRET` set) and `chat_turn` (only with the
/// chat concierge configured) — so an entry may legitimately be absent from a
/// given deployment's document. The reverse is what must never happen, and is
/// what the catalog enforces: a documented operation with no entry here.
pub const OPERATION_POLICIES: &[AuditOperation] = &[
    // --- system surface: the only exclusions --------------------------------
    excluded("health", ExclusionReason::Probe),
    excluded("readiness", ExclusionReason::Probe),
    excluded("metrics", ExclusionReason::Scrape),
    // --- inbound webhook ----------------------------------------------------
    audited(
        "github_app_webhook",
        "SafeGithubAppWebhook",
        arguments::GITHUB_APP_WEBHOOK_FIELDS,
    ),
    // --- chat concierge (conditionally mounted) -----------------------------
    audited("chat_turn", "SafeChatTurn", arguments::CHAT_TURN_FIELDS),
    // --- named environment profiles ----------------------------------------
    audited_without_arguments("list_user_environment_profiles"),
    audited(
        "put_user_environment_profile",
        "SafePutEnvironmentProfile",
        arguments::PUT_USER_ENVIRONMENT_PROFILE_FIELDS,
    ),
    audited(
        "get_user_environment_profile",
        "SafeGetEnvironmentProfile",
        arguments::GET_USER_ENVIRONMENT_PROFILE_FIELDS,
    ),
    audited(
        "delete_user_environment_profile",
        "SafeDeleteEnvironmentProfile",
        arguments::DELETE_USER_ENVIRONMENT_PROFILE_FIELDS,
    ),
    // --- session logs -------------------------------------------------------
    audited(
        "download_session_logs",
        "SafeDownloadSessionLogs",
        arguments::DOWNLOAD_SESSION_LOGS_FIELDS,
    ),
    audited(
        "session_logs_oauth_callback",
        "SafeSessionLogsOauthCallback",
        arguments::SESSION_LOGS_OAUTH_CALLBACK_FIELDS,
    ),
    audited(
        "session_log_manifest",
        "SafeSessionLogManifest",
        arguments::SESSION_LOG_MANIFEST_FIELDS,
    ),
    audited(
        "session_log_file",
        "SafeSessionLogFile",
        arguments::SESSION_LOG_FILE_FIELDS,
    ),
    audited(
        "list_session_runs",
        "SafeListSessionRuns",
        arguments::LIST_SESSION_RUNS_FIELDS,
    ),
    audited(
        "session_health",
        "SafeSessionHealth",
        arguments::SESSION_HEALTH_FIELDS,
    ),
    audited(
        "session_health_report",
        "SafeSessionHealthReport",
        arguments::SESSION_HEALTH_REPORT_FIELDS,
    ),
    // --- browser authentication --------------------------------------------
    audited(
        "github_login",
        "SafeGithubLogin",
        arguments::GITHUB_LOGIN_FIELDS,
    ),
    audited(
        "github_login_callback",
        "SafeGithubLoginCallback",
        arguments::GITHUB_LOGIN_CALLBACK_FIELDS,
    ),
    audited(
        "github_refresh_token",
        "SafeGithubRefreshToken",
        arguments::GITHUB_REFRESH_TOKEN_FIELDS,
    ),
    audited(
        "github_broader_connect",
        "SafeGithubBroaderConnect",
        arguments::GITHUB_BROADER_CONNECT_FIELDS,
    ),
    audited(
        "github_broader_callback",
        "SafeGithubBroaderCallback",
        arguments::GITHUB_BROADER_CALLBACK_FIELDS,
    ),
    // --- repositories and installations ------------------------------------
    audited(
        "create_repo",
        "SafeCreateRepo",
        arguments::CREATE_REPO_FIELDS,
    ),
    audited(
        "uninstall_account",
        "SafeUninstallAccount",
        arguments::UNINSTALL_ACCOUNT_FIELDS,
    ),
    // --- canvas dashboard ---------------------------------------------------
    audited(
        "canvas_overview",
        "SafeCanvasOverview",
        arguments::CANVAS_OVERVIEW_FIELDS,
    ),
    audited(
        "canvas_repo_sessions",
        "SafeCanvasRepoSessions",
        arguments::CANVAS_REPO_SESSIONS_FIELDS,
    ),
    audited(
        "canvas_create_session",
        "SafeCanvasCreateSession",
        arguments::CANVAS_CREATE_SESSION_FIELDS,
    ),
    audited(
        "canvas_stop_session",
        "SafeCanvasStopSession",
        arguments::CANVAS_STOP_SESSION_FIELDS,
    ),
    audited(
        "canvas_create_work_item",
        "SafeCanvasCreateWorkItem",
        arguments::CANVAS_CREATE_WORK_ITEM_FIELDS,
    ),
    audited(
        "canvas_session_outcomes",
        "SafeCanvasSessionOutcomes",
        arguments::CANVAS_SESSION_OUTCOMES_FIELDS,
    ),
    audited(
        "canvas_outcome_blob",
        "SafeCanvasOutcomeBlob",
        arguments::CANVAS_OUTCOME_BLOB_FIELDS,
    ),
    // --- engine observe -----------------------------------------------------
    audited(
        "observe_session",
        "SafeObserveSession",
        arguments::OBSERVE_SESSION_FIELDS,
    ),
    // --- operations surface -------------------------------------------------
    // Audited like any other product call: the operations UI polls this route,
    // and the UI may hide its own polling visually, but capture is never allowed
    // to skip it (epic `AUD-01`).
    audited(
        "operations_list_activity",
        "SafeOperationsListActivity",
        arguments::OPERATIONS_LIST_ACTIVITY_FIELDS,
    ),
    audited(
        "operations_list_sandboxes",
        "SafeOperationsListSandboxes",
        arguments::OPERATIONS_LIST_SANDBOXES_FIELDS,
    ),
];

/// Argument policies for operations whose ROUTES do not exist yet.
///
/// Empty today: both milestone #22 operations have graduated into
/// [`OPERATION_POLICIES`] with their routes — `operations_list_activity` with
/// issue #5672 and `operations_list_sandboxes` with #5675.
///
/// The mechanism is kept because it is the ONLY way to review an argument
/// boundary ahead of its handler: the coverage guard requires every entry in the
/// live table to match a live operation, so an id parked there early would look
/// exactly like an operation that had silently disappeared. A future endpoint
/// whose DTO lands before its route declares it here and moves it across in the
/// same pull request as its handler.
pub const RESERVED_ARGUMENT_POLICIES: &[AuditOperation] = &[];

/// Routes that are served but carry no OpenAPI operation, with their policy.
///
/// Only the contract document qualifies today. Anything else reaching a matched
/// route without an operation is a genuinely undocumented endpoint, and is
/// audited under the `<unmatched>` operation id rather than being waved through.
const UNDOCUMENTED_ROUTE_POLICIES: &[(&str, &str, OperationPolicy)] = &[(
    "GET",
    "/openapi.json",
    OperationPolicy::Excluded(ExclusionReason::Contract),
)];

/// The complete declaration for an `operation_id`, or `None` when the table does
/// not name it (a build error for the catalog, never a silent default).
pub fn operation_for(operation_id: &str) -> Option<&'static AuditOperation> {
    operation_in(OPERATION_POLICIES, operation_id)
}

/// The same lookup against an explicit table.
///
/// Exists so the catalog's build-time guards can be driven against a
/// deliberately broken declaration in a test: [`OPERATION_POLICIES`] is built
/// from const constructors that CANNOT express an audited operation without an
/// argument policy, and a guard whose failure path nothing can reach is a guard
/// nobody has ever seen work.
pub(super) fn operation_in(
    table: &'static [AuditOperation],
    operation_id: &str,
) -> Option<&'static AuditOperation> {
    table
        .iter()
        .find(|operation| operation.operation_id == operation_id)
}

/// The declared audit policy for an `operation_id`.
pub fn policy_for(operation_id: &str) -> Option<OperationPolicy> {
    operation_for(operation_id).map(|operation| operation.policy)
}

/// The declared safe-argument policy for an `operation_id`.
pub fn arguments_policy_for(operation_id: &str) -> Option<ArgumentsPolicy> {
    operation_for(operation_id).map(|operation| operation.arguments)
}

/// The `arguments_parse_status` a record carries when no safe arguments were
/// recorded for `operation_id`.
///
/// An unknown operation — the `<unmatched>` sentinel — has no declared arguments
/// and so reports `not_applicable`: there was no argument contract to run.
pub fn default_arguments_status(operation_id: &str) -> ArgumentsParseStatus {
    arguments_policy_for(operation_id)
        .map(ArgumentsPolicy::default_status)
        .unwrap_or(ArgumentsParseStatus::NotApplicable)
}

/// The declared policy for a served route that has no OpenAPI operation.
pub fn undocumented_route_policy(method: &str, route_template: &str) -> Option<OperationPolicy> {
    UNDOCUMENTED_ROUTE_POLICIES
        .iter()
        .find(|(m, template, _)| *m == method && *template == route_template)
        .map(|(_, _, policy)| *policy)
}

/// Every `operation_id` the table names, for coverage guards.
pub fn declared_operation_ids() -> impl Iterator<Item = &'static str> {
    OPERATION_POLICIES
        .iter()
        .map(|operation| operation.operation_id)
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
