//! The reusable session-visibility gate the operations surfaces call.
//!
//! It joins the three pieces the epic keeps deliberately separate — readiness
//! ([`SessionAccessRegistry`]), the pure tiers ([`super::policy`]), and the HTTP
//! contract — into ONE place, so no route can invent its own interpretation of
//! "may this viewer see this session".
//!
//! The response shape is the anti-enumeration contract (epic `SBOX-06`):
//!
//! - unknown session id -> `404`;
//! - known session, viewer not authorized -> the SAME `404`, so an exact probe
//!   cannot distinguish "does not exist" from "not yours";
//! - projection cold/incomplete -> `503 session_visibility_unavailable`, never a
//!   misleading empty or a false `404`.

use crate::access_policy::AccessPolicy;
use crate::error::AppError;

use super::policy::{
    decide, PolicyEnvironment, SessionAccessDecision, SessionAccessRequest, SessionCapability,
    VerifiedCaller,
};
use super::registry::{ContextLookup, SessionAccessRegistry};
use super::viewer::{AuthenticatedViewer, ViewerScope};

/// Fixed client-facing text for an unresolvable session. Identical for "unknown"
/// and "not authorized" by design.
const SANDBOX_NOT_FOUND: &str = "no such session";

/// Authorize one exact session for operations visibility.
///
/// `scope` decides whether the deployment global-admin tier participates:
/// `accessible` (personal) evaluates even an administrator on their DIRECT tiers,
/// while `all` is the explicit administrator bypass.
pub fn authorize_session_visibility(
    registry: &SessionAccessRegistry,
    viewer: &AuthenticatedViewer,
    scope: &ViewerScope,
    access: &AccessPolicy,
    legacy_log_admins: &[String],
    session_id: &str,
) -> Result<SessionAccessDecision, AppError> {
    let context = match registry.lookup(session_id) {
        ContextLookup::Found(context) => context,
        ContextLookup::Unknown => return Err(AppError::NotFound(SANDBOX_NOT_FOUND.to_string())),
        ContextLookup::Unavailable => {
            tracing::info!("operations: session visibility projection is not ready");
            return Err(AppError::SessionVisibilityUnavailable(
                "session visibility is still recovering; retry shortly".to_string(),
            ));
        }
    };

    let mut request = SessionAccessRequest::new(
        SessionCapability::OperationsVisibility,
        VerifiedCaller {
            id: viewer.id(),
            login: viewer.login(),
        },
        context.facts(),
        PolicyEnvironment {
            access,
            legacy_log_admins,
            github_bot_login: None,
        },
    );
    if !scope.is_global() {
        request = request.without_global_admin();
    }

    let decision = decide(&request);
    if decision.allowed {
        Ok(decision)
    } else {
        // Same status and text as an unknown id: an exact probe must not become
        // an existence oracle.
        Err(AppError::NotFound(SANDBOX_NOT_FOUND.to_string()))
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
