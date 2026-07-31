//! The pure, capability-aware session authorization policy.
//!
//! One decision function, four capabilities, no I/O and no clock — so the whole
//! allow/deny matrix is exhaustively unit-testable and every surface (log
//! download, engine observe, work authority, operations visibility) reaches the
//! same verdict from the same facts.
//!
//! ## Why capabilities instead of one `is_authorized` boolean
//!
//! The tiers genuinely differ, and collapsing them would silently widen access:
//!
//! | capability | creator | collaborators | log access / legacy log admins | global admin | base `AccessPolicy` |
//! |---|---|---|---|---|---|
//! | [`SessionCapability::OperationsVisibility`] | yes | yes | yes | yes | enforced |
//! | [`SessionCapability::LogDownload`] | yes | **no** | yes | yes | enforced |
//! | [`SessionCapability::Observe`] | yes | **no** | yes | yes | enforced |
//! | [`SessionCapability::WorkAuthority`] | yes | yes | **no** | yes | enforced |
//!
//! A session collaborator must not gain the redacted log bundle just because a
//! new surface needed a broader tier, and a log-allowlist entry must not gain the
//! right to raise work. Those two "no"s are the entire reason this type exists.
//!
//! ## The deployment gate applies to every capability
//!
//! `AccessPolicy` is consulted for ALL four, per the milestone contract: "the base
//! `AccessPolicy` must admit an ordinary human before a session capability is
//! considered". `LogDownload`/`Observe` previously skipped it because their routes
//! resolve identity outside the
//! [`GithubUser`](crate::github_identity::GithubUser) extractor (a raw bearer
//! token, or a browser OAuth round-trip), so nothing had ever applied it there.
//!
//! That gap is now closed rather than preserved: a `FKST_ACCESS_BLOCKED_USERS`
//! entry is documented to lose EVERY gate — its trigger issues are ignored and its
//! running sessions are torn down — and a blocked human who could still pull the
//! session's log bundle would contradict that in the one place it matters most.
//! Global-admin precedence is unchanged: an administrator passes
//! [`AccessPolicy::allows`] by construction, including one also (mis)listed as
//! blocked.
//!
//! ## Repository role is not a tier
//!
//! Repository visibility, repository admin/owner status, trigger-issue
//! readability, trigger authorship alone, and knowledge of a session id are all
//! deliberately absent. A pure function cannot look them up, which is precisely
//! why the policy is pure: an implicit tier cannot creep in through a convenient
//! GitHub call.

use crate::access_policy::{entry_matches, AccessPolicy};
use crate::reconcile::creator::is_expected_bot_login;

use super::context::SessionAuthorizationFacts;

/// What the caller is trying to do with the session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionCapability {
    /// Inspect live sandbox data and sandbox lifecycle events (milestone #22).
    OperationsVisibility,
    /// Download the session's redacted log bundle.
    LogDownload,
    /// Read the engine's observe snapshot for the session.
    Observe,
    /// Raise work items against the session.
    WorkAuthority,
}

impl SessionCapability {
    /// The stable wire string; safe as a closed-enum metric label.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionCapability::OperationsVisibility => "operations_visibility",
            SessionCapability::LogDownload => "log_download",
            SessionCapability::Observe => "observe",
            SessionCapability::WorkAuthority => "work_authority",
        }
    }
}

/// Which tier produced the verdict.
///
/// Diagnostics and tests only. If it is ever returned publicly it describes the
/// CURRENT caller and nothing else — it must never let a caller learn who else is
/// configured, which is why there is no "which entry matched" detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessBasis {
    Creator,
    Collaborator,
    LogAccess,
    LegacyLogAdmin,
    GlobalAdmin,
    /// The configured FKST GitHub App acting as a system principal.
    AppSystem,
    /// No tier matched.
    None,
}

impl AccessBasis {
    /// The stable wire string; safe as a closed-enum metric label.
    pub fn as_str(self) -> &'static str {
        match self {
            AccessBasis::Creator => "creator",
            AccessBasis::Collaborator => "collaborator",
            AccessBasis::LogAccess => "log_access",
            AccessBasis::LegacyLogAdmin => "legacy_log_admin",
            AccessBasis::GlobalAdmin => "global_admin",
            AccessBasis::AppSystem => "app_system",
            AccessBasis::None => "none",
        }
    }
}

/// The verdict for one `(capability, caller, session)` triple.
///
/// The fields are private and the two mints ([`Self::allow`] / [`Self::deny`]) are
/// module-private, so this type is a PROOF: holding an allowing decision means the
/// tier table above actually ran on real session facts. That matters because
/// [`super::activity::authorize_lifecycle_session`] turns an allowing decision into
/// an [`AuthorizedSessionId`](super::activity::AuthorizedSessionId), the token the
/// personal-activity constraint trusts. If a query/DTO layer could write
/// `SessionAccessDecision { allowed: true, .. }` it would mint that token for any
/// session id it liked, and the whole row-level authorization argument would
/// collapse (epic `AUTH-03`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionAccessDecision {
    allowed: bool,
    basis: AccessBasis,
}

impl SessionAccessDecision {
    /// Whether the capability is granted.
    pub fn allowed(self) -> bool {
        self.allowed
    }

    /// Which tier decided it. Diagnostics only; a denial always reports
    /// [`AccessBasis::None`].
    pub fn basis(self) -> AccessBasis {
        self.basis
    }

    fn allow(basis: AccessBasis) -> Self {
        Self {
            allowed: true,
            basis,
        }
    }

    /// Deny. The basis is always [`AccessBasis::None`]: reporting the tier a
    /// caller *nearly* matched would leak list membership.
    fn deny() -> Self {
        Self {
            allowed: false,
            basis: AccessBasis::None,
        }
    }
}

/// The verified caller.
///
/// The fields are private so a caller cannot be assembled from a struct literal
/// anywhere in the crate: every construction goes through one of the two named
/// constructors below, each of which states WHERE its identity came from. That is
/// as far as the type system can go — this value is an input to the policy, not a
/// proof like [`SessionAccessDecision`] — but it makes "who claims this identity is
/// verified?" a one-line grep instead of a whole-crate audit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedCaller<'a> {
    /// The immutable GitHub numeric id.
    id: i64,
    /// The login snapshot, used only for list-entry matching.
    login: &'a str,
}

impl<'a> VerifiedCaller<'a> {
    /// The request path: an identity the
    /// [`GithubUser`](crate::github_identity::GithubUser) extractor resolved by
    /// calling GitHub `/user` with the caller's bearer token.
    pub fn from_verified_user(user: &'a crate::github_identity::GithubUser) -> Self {
        Self {
            id: user.id,
            login: &user.login,
        }
    }

    /// The reconcile path: an identity GitHub itself reported in issue metadata
    /// (an author, an assignee) over an authenticated App call.
    ///
    /// Never call this with a value that reached the process as request-controlled
    /// input — a path segment, a body field, a query parameter, or a header.
    pub fn from_github_metadata(id: i64, login: &'a str) -> Self {
        Self { id, login }
    }

    /// The immutable GitHub numeric id.
    pub fn id(self) -> i64 {
        self.id
    }

    /// The login snapshot.
    pub fn login(self) -> &'a str {
        self.login
    }
}

/// The deployment-level inputs a decision may consult.
#[derive(Clone, Copy, Debug)]
pub struct PolicyEnvironment<'a> {
    pub access: &'a AccessPolicy,
    /// `FKST_LOG_ADMINS`: the legacy cross-session observability grant.
    pub legacy_log_admins: &'a [String],
    /// The configured FKST GitHub App login, if any. Only [`SessionCapability::WorkAuthority`]
    /// accepts it, and only as a system principal for workflow-generated issues.
    pub github_bot_login: Option<&'a str>,
}

/// One authorization question.
#[derive(Clone, Copy, Debug)]
pub struct SessionAccessRequest<'a> {
    pub capability: SessionCapability,
    pub caller: VerifiedCaller<'a>,
    pub facts: SessionAuthorizationFacts<'a>,
    pub environment: PolicyEnvironment<'a>,
    /// Whether the deployment global-admin tier may decide this request.
    ///
    /// `false` for the operations `scope=accessible` view, where even a global
    /// admin is evaluated on their DIRECT tiers so the UI can intentionally show
    /// "the sessions I own or was granted" rather than everything.
    pub allow_global_admin: bool,
}

impl<'a> SessionAccessRequest<'a> {
    /// A request with the global-admin tier enabled (the normal case).
    pub fn new(
        capability: SessionCapability,
        caller: VerifiedCaller<'a>,
        facts: SessionAuthorizationFacts<'a>,
        environment: PolicyEnvironment<'a>,
    ) -> Self {
        Self {
            capability,
            caller,
            facts,
            environment,
            allow_global_admin: true,
        }
    }

    /// The same request with the global-admin bypass disabled.
    pub fn without_global_admin(mut self) -> Self {
        self.allow_global_admin = false;
        self
    }
}

/// Decide one capability question. Pure: a function of its inputs only.
pub fn decide(request: &SessionAccessRequest<'_>) -> SessionAccessDecision {
    match request.capability {
        SessionCapability::OperationsVisibility => operations_visibility(request),
        SessionCapability::LogDownload | SessionCapability::Observe => {
            session_observability(request)
        }
        SessionCapability::WorkAuthority => work_authority(request),
    }
}

/// Operations sandbox/lifecycle visibility: the five explicit tiers, gated by the
/// deployment access policy.
fn operations_visibility(request: &SessionAccessRequest<'_>) -> SessionAccessDecision {
    let env = &request.environment;
    let caller = request.caller;
    // The deployment gate first: an ordinary human the deployment does not admit
    // loses EVERY tier, exactly as work authority already behaves. Global admins
    // pass `allows` by construction, preserving today's precedence over a
    // conflicting blocked-users entry.
    if !env.access.allows(caller.id, caller.login) {
        return SessionAccessDecision::deny();
    }
    if request.allow_global_admin && env.access.is_global_admin(caller.id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::GlobalAdmin);
    }
    if request.facts.creator_matches(caller.id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::Creator);
    }
    let caller_id = caller.id.to_string();
    if matches_any(request.facts.collaborators, &caller_id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::Collaborator);
    }
    if matches_any(request.facts.log_access, &caller_id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::LogAccess);
    }
    if matches_any(env.legacy_log_admins, &caller_id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::LegacyLogAdmin);
    }
    SessionAccessDecision::deny()
}

/// Log download and engine observe: creator + per-issue log access + legacy log
/// admins + global admins, gated by the deployment access policy. A collaborator
/// alone is NOT a tier here.
fn session_observability(request: &SessionAccessRequest<'_>) -> SessionAccessDecision {
    let env = &request.environment;
    let caller = request.caller;
    // The deployment gate first, exactly as the other two human capabilities do.
    // A global admin passes `allows` by construction, so the existing precedence
    // over a conflicting blocked-users entry survives the ordering.
    if !env.access.allows(caller.id, caller.login) {
        return SessionAccessDecision::deny();
    }
    if request.allow_global_admin && env.access.is_global_admin(caller.id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::GlobalAdmin);
    }
    log_download_tier(request.facts, caller, env.legacy_log_admins)
}

/// Work-item authority: creator + collaborators + global admins, gated by the
/// deployment access policy. A log-access or legacy-log-admin entry alone is NOT
/// a tier here.
fn work_authority(request: &SessionAccessRequest<'_>) -> SessionAccessDecision {
    let env = &request.environment;
    let caller = request.caller;
    // The configured App is a SYSTEM principal for workflow-generated child
    // issues, not a human tier, so it is checked before the human access gate.
    if is_expected_bot_login(caller.login, env.github_bot_login) {
        return SessionAccessDecision::allow(AccessBasis::AppSystem);
    }
    if !env.access.allows(caller.id, caller.login) {
        return SessionAccessDecision::deny();
    }
    if request.facts.creator_matches(caller.id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::Creator);
    }
    if request.allow_global_admin && env.access.is_global_admin(caller.id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::GlobalAdmin);
    }
    let caller_id = caller.id.to_string();
    if matches_any(request.facts.collaborators, &caller_id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::Collaborator);
    }
    SessionAccessDecision::deny()
}

/// Whether any list entry matches the caller by numeric id or case-insensitive
/// login. Shared with the deployment access policy so the two allow-list
/// grammars can never diverge; a blank/`@`-only entry never matches.
fn matches_any(entries: &[String], caller_id: &str, caller_login: &str) -> bool {
    entries
        .iter()
        .any(|entry| entry_matches(entry, caller_id, caller_login))
}

/// The pure log/observe tier decision WITHOUT the deployment gate and WITHOUT the
/// global-admin bypass — [`session_observability`] applies both around it.
fn log_download_tier(
    facts: SessionAuthorizationFacts<'_>,
    caller: VerifiedCaller<'_>,
    legacy_log_admins: &[String],
) -> SessionAccessDecision {
    if facts.creator_matches(caller.id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::Creator);
    }
    let caller_id = caller.id.to_string();
    if matches_any(facts.log_access, &caller_id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::LogAccess);
    }
    if matches_any(legacy_log_admins, &caller_id, caller.login) {
        return SessionAccessDecision::allow(AccessBasis::LegacyLogAdmin);
    }
    SessionAccessDecision::deny()
}

/// The pure work-authority decision, exposed for
/// [`crate::reconcile::work_authz`]'s compatibility signature.
pub(crate) fn work_authority_tier(
    facts: SessionAuthorizationFacts<'_>,
    caller: VerifiedCaller<'_>,
    access: &AccessPolicy,
    github_bot_login: Option<&str>,
) -> SessionAccessDecision {
    work_authority(&SessionAccessRequest::new(
        SessionCapability::WorkAuthority,
        caller,
        facts,
        PolicyEnvironment {
            access,
            legacy_log_admins: &[],
            github_bot_login,
        },
    ))
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "policy_log_download_tests.rs"]
mod log_download_tests;
