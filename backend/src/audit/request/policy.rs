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

/// The complete policy table, keyed by the `operation_id` declared on each
/// `#[utoipa::path]`.
///
/// Two operations here are mounted conditionally — `github_app_webhook` (only
/// with `FKST_GITHUB_APP_WEBHOOK_SECRET` set) and `chat_turn` (only with the
/// chat concierge configured) — so an entry may legitimately be absent from a
/// given deployment's document. The reverse is what must never happen, and is
/// what the catalog enforces: a documented operation with no entry here.
pub const OPERATION_POLICIES: &[(&str, OperationPolicy)] = &[
    // --- system surface: the only exclusions --------------------------------
    ("health", OperationPolicy::Excluded(ExclusionReason::Probe)),
    (
        "readiness",
        OperationPolicy::Excluded(ExclusionReason::Probe),
    ),
    (
        "metrics",
        OperationPolicy::Excluded(ExclusionReason::Scrape),
    ),
    // --- inbound webhook ----------------------------------------------------
    ("github_app_webhook", OperationPolicy::Audited),
    // --- chat concierge (conditionally mounted) -----------------------------
    ("chat_turn", OperationPolicy::Audited),
    // --- named environment profiles ----------------------------------------
    ("list_user_environment_profiles", OperationPolicy::Audited),
    ("put_user_environment_profile", OperationPolicy::Audited),
    ("get_user_environment_profile", OperationPolicy::Audited),
    ("delete_user_environment_profile", OperationPolicy::Audited),
    // --- session logs -------------------------------------------------------
    ("download_session_logs", OperationPolicy::Audited),
    ("session_logs_oauth_callback", OperationPolicy::Audited),
    ("session_log_manifest", OperationPolicy::Audited),
    ("session_log_file", OperationPolicy::Audited),
    ("list_session_runs", OperationPolicy::Audited),
    // --- browser authentication --------------------------------------------
    ("github_login", OperationPolicy::Audited),
    ("github_login_callback", OperationPolicy::Audited),
    ("github_refresh_token", OperationPolicy::Audited),
    ("github_broader_connect", OperationPolicy::Audited),
    ("github_broader_callback", OperationPolicy::Audited),
    // --- repositories and installations ------------------------------------
    ("create_repo", OperationPolicy::Audited),
    ("uninstall_account", OperationPolicy::Audited),
    // --- canvas dashboard ---------------------------------------------------
    ("canvas_overview", OperationPolicy::Audited),
    ("canvas_repo_sessions", OperationPolicy::Audited),
    ("canvas_create_session", OperationPolicy::Audited),
    ("canvas_stop_session", OperationPolicy::Audited),
    ("canvas_create_work_item", OperationPolicy::Audited),
    ("canvas_session_outcomes", OperationPolicy::Audited),
    ("canvas_outcome_blob", OperationPolicy::Audited),
    // --- engine observe -----------------------------------------------------
    ("observe_session", OperationPolicy::Audited),
];

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

/// The declared policy for an `operation_id`, or `None` when the table does not
/// name it (a build error for the catalog, never a silent default).
pub fn policy_for(operation_id: &str) -> Option<OperationPolicy> {
    OPERATION_POLICIES
        .iter()
        .find(|(id, _)| *id == operation_id)
        .map(|(_, policy)| *policy)
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
    OPERATION_POLICIES.iter().map(|(id, _)| *id)
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
