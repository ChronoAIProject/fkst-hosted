//! The typed personal-activity invariant (epic `AUTH-03`).
//!
//! An [`ActivityVisibilityConstraint`] is built here and executed elsewhere:
//! issue #5672 embeds it in fixed HogQL, #5678 embeds the same value in relay
//! SQL. Both apply it at the SOURCE, before any `LIMIT` or cursor — never by
//! fetching a global page and filtering it in application memory.
//!
//! ## Why the constructors are sealed
//!
//! `Mine` is only sound because `actor_id` provably came from a verified
//! identity, and a lifecycle session id is only sound because the session policy
//! authorized that exact id. If either could be built from a query DTO, a caller
//! could hand the query layer a constraint naming somebody else and the source
//! predicate would faithfully enforce the wrong thing.
//!
//! So both payloads keep private fields: outside this module, the only ways to
//! obtain a value are [`ActivityVisibilityConstraint::for_scope`] (which consumes
//! a [`ViewerScope`], itself mintable only by a verified viewer) and
//! [`authorize_lifecycle_session`] (which consumes a policy decision).
//!
//! ## What `Mine` means, exactly
//!
//! - an API-request row requires exact top-level `actor_id == viewer_id`;
//! - `actor_login`, PostHog `distinct_id`, principal identity, repository access,
//!   and session access are NOT substitutes;
//! - unattributed/anonymous rows are excluded, because ownership cannot be
//!   proven — they remain global-admin-only;
//! - lifecycle rows require the separately authorized exact session id;
//! - adding an authorized session NEVER removes the own-actor predicate, so a
//!   shared session cannot surface another human's API calls.

use super::policy::SessionAccessDecision;
use super::viewer::ViewerScope;

/// A session id that has passed [`super::policy::SessionCapability::OperationsVisibility`].
///
/// The private field is the seal: possessing one is proof the policy allowed it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedSessionId(String);

impl AuthorizedSessionId {
    /// The session id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Mint an [`AuthorizedSessionId`] from an allowing decision.
///
/// Returns `None` for a denial, so a caller cannot obtain the token by ignoring
/// the verdict — the only path to the type runs through the policy.
pub fn authorize_lifecycle_session(
    session_id: &str,
    decision: &SessionAccessDecision,
) -> Option<AuthorizedSessionId> {
    decision
        .allowed()
        .then(|| AuthorizedSessionId(session_id.to_string()))
}

/// The row-visibility predicate a source query must apply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityVisibilityConstraint {
    /// Only rows provably owned by this verified actor, plus lifecycle rows for
    /// one separately authorized session.
    Mine(PersonalActivityScope),
    /// Every actor and record kind. Global administrators only.
    All(GlobalActivityScope),
}

impl ActivityVisibilityConstraint {
    /// Build the constraint for a resolved scope.
    ///
    /// `lifecycle_session` is ignored in global scope: an administrator already
    /// sees every session's lifecycle rows, so carrying the token would only
    /// invite a caller to believe it narrowed something.
    pub fn for_scope(scope: &ViewerScope, lifecycle_session: Option<AuthorizedSessionId>) -> Self {
        match scope {
            ViewerScope::Mine(personal) => {
                ActivityVisibilityConstraint::Mine(PersonalActivityScope {
                    actor_id: personal.viewer_id(),
                    lifecycle_session_id: lifecycle_session,
                })
            }
            ViewerScope::All(global) => ActivityVisibilityConstraint::All(GlobalActivityScope {
                admin_id: global.admin_id(),
            }),
        }
    }

    /// The bounded label for metrics and structured logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            ActivityVisibilityConstraint::Mine(_) => "mine",
            ActivityVisibilityConstraint::All(_) => "all",
        }
    }

    /// The mandatory actor predicate, or `None` in global scope.
    ///
    /// A source that ignores a `Some` here is a row-authorization bug; the type
    /// exists so that mistake is visible at the call site.
    pub fn required_actor_id(&self) -> Option<i64> {
        match self {
            ActivityVisibilityConstraint::Mine(scope) => Some(scope.actor_id),
            ActivityVisibilityConstraint::All(_) => None,
        }
    }
}

/// The payload of [`ActivityVisibilityConstraint::Mine`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonalActivityScope {
    actor_id: i64,
    lifecycle_session_id: Option<AuthorizedSessionId>,
}

impl PersonalActivityScope {
    /// The verified viewer id every API-request row must carry.
    pub fn actor_id(&self) -> i64 {
        self.actor_id
    }

    /// The one session whose SYSTEM lifecycle rows this viewer may additionally
    /// see. It never widens the actor predicate above.
    pub fn lifecycle_session_id(&self) -> Option<&str> {
        self.lifecycle_session_id
            .as_ref()
            .map(AuthorizedSessionId::as_str)
    }
}

/// The payload of [`ActivityVisibilityConstraint::All`].
///
/// A marker with the administrator's id for audit correlation. It is a struct
/// rather than a unit variant precisely so `All` cannot be written down by a
/// module that never proved anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlobalActivityScope {
    admin_id: i64,
}

impl GlobalActivityScope {
    /// The verified administrator's immutable id.
    pub fn admin_id(&self) -> i64 {
        self.admin_id
    }
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
