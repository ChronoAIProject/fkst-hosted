//! Publication of a repository's session-access contexts.
//!
//! The reconciler is the only writer of the projection every session-scoped route
//! authorizes against ([`crate::session_access`]). It lives in its own module
//! because it is an authorization concern, not a reconcile-planning one: a change
//! here changes who can see what, and that deserves to be reviewed on its own.

use crate::models::RepoRef;
use crate::reconcile::creator::SessionCreator;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::execute::ReconcileCtx;
use crate::session_access::SessionAccessContext;

/// Publish this repository's complete [`SessionAccessContext`] set into the shared
/// projection every session-scoped route authorizes against.
///
/// REPLACEMENT, not upsert: this sweep has just enumerated every open trigger
/// issue for the repository, so its registration list is authoritative. A session
/// whose trigger was closed, retired, or deleted is absent from `regs` and must
/// therefore disappear from the projection — an upsert would leave it behind as a
/// stale grant that outlives the session it described.
///
/// Only public metadata travels: ids, the repository/issue correlation, and the
/// frozen collaborator/log-access entries. Never a token, an issue body, or a
/// title.
pub(crate) fn record_session_contexts(
    ctx: &ReconcileCtx,
    installation_id: i64,
    repo: &RepoRef,
    regs: &[SessionRegistration],
) {
    let contexts = regs
        .iter()
        .map(|reg| {
            (
                reg.session_id.clone(),
                SessionAccessContext {
                    installation_id: reg.installation_id,
                    repo: reg.repo.clone(),
                    trigger_issue: reg.trigger_issue,
                    creator: SessionCreator {
                        login: reg.creator_login.clone(),
                        id: reg.creator_id,
                    },
                    collaborators: reg.collaborators.clone(),
                    log_access: reg.log_access.clone(),
                },
            )
        })
        .collect();
    ctx.session_access
        .replace_repo(installation_id, repo, contexts);
}

#[cfg(test)]
#[path = "session_contexts_tests.rs"]
mod tests;
