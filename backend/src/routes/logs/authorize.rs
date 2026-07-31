//! The shared session-scoped authorization gate for the log-download and
//! engine-observe routes.
//!
//! Both surfaces reach a handler with only a `session_id` and a verified caller,
//! and both must answer the same question: may THIS person read THIS session's
//! observability data? The answer comes from the reconciler-maintained projection
//! (a one-way `session_id` cannot yield the trigger context on its own) plus the
//! shared capability policy — never from a route-local reinterpretation of
//! "creator".

use crate::error::AppError;
use crate::github_identity::GithubUser;
use crate::session_access::{
    self, PolicyEnvironment, SessionAccessRequest, SessionCapability, VerifiedCaller,
};
use crate::state::AppState;

/// Authorize `user` against the session's trigger context. Looks the context up in
/// the reconciler-maintained registry (a one-way `session_id` cannot yield it
/// otherwise); an unknown session → 404 (never reveals more), an unauthorized caller
/// → 403. The token is NEVER referenced here; only the resolved (public) identity is.
pub(crate) fn authorize(
    state: &AppState,
    session_id: &str,
    user: &GithubUser,
) -> Result<(), AppError> {
    let Some(context) = state.session_access.registry.get(session_id) else {
        // Deny-by-default: with no context we cannot authorize, so we do not serve.
        return Err(AppError::NotFound(
            "no logs available for this session".to_string(),
        ));
    };
    // The shared capability policy, asked the LogDownload question: creator +
    // per-issue log access + legacy `FKST_LOG_ADMINS` + deployment global admins.
    // A session collaborator is deliberately NOT a tier here — that difference is
    // the whole reason the policy is capability-aware rather than one boolean.
    let decision = session_access::decide(&SessionAccessRequest::new(
        SessionCapability::LogDownload,
        VerifiedCaller {
            id: user.id,
            login: &user.login,
        },
        context.facts(),
        PolicyEnvironment {
            access: &state.config.access,
            legacy_log_admins: &state.config.log.admins,
            github_bot_login: None,
        },
    ));
    if decision.allowed {
        tracing::info!(
            session_id = %session_id,
            requester_id = user.id,
            requester_login = %user.login,
            basis = decision.basis.as_str(),
            "log download authorized"
        );
        Ok(())
    } else {
        tracing::info!(
            session_id = %session_id,
            requester_id = user.id,
            requester_login = %user.login,
            "log download denied (not authorized)"
        );
        Err(AppError::Forbidden(
            "not authorized to access these logs".to_string(),
        ))
    }
}
