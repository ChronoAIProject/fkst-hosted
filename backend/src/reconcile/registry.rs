//! Pure trigger-issue → [`SessionRegistration`] parse (issue #359 §4.3).
//!
//! The reconciler enumerates a repo's open `fkst-substrate-trigger` issues as
//! [`IssueSummary`]s; this module turns ONE such issue into either a valid
//! [`SessionRegistration`] (its launch inputs, its deterministic session id, and
//! the config hash over the inputs) or an invalid marker `(issue_number, detail)`
//! that the planner flags back onto the issue.
//!
//! Pure: it parses the issue BODY with the shared
//! [`crate::goals::trigger_parse::parse_trigger_issue_body`] and derives the
//! session id with [`crate::session_spec::derive_session_id`]. No I/O.

use crate::github_app::listing::IssueSummary;
use crate::goals::trigger_parse::parse_trigger_issue_body;
use crate::models::RepoRef;
use crate::reconcile::desired::{config_hash, SessionDef, SessionRegistration};
use crate::session_spec::derive_session_id;

/// Parse one trigger issue into a [`SessionRegistration`], or an invalid marker.
///
/// On a well-formed body: builds the registration — the launch [`SessionDef`] from
/// the parsed [`crate::goals::trigger_parse::TriggerSpec`], the deterministic
/// `session_id` from `(installation_id, repo.owner, repo.name, issue.number)`, the
/// `config_hash` over the launch inputs, and `trigger_author_id` from the issue
/// author's numeric GitHub id.
///
/// On a malformed body: returns `Err((issue.number, detail))` — the 422 message
/// from the shared parser, which names the offending section. The planner turns
/// that into a [`crate::reconcile::desired::ReconcileAction::FlagInvalid`].
pub fn parse_registration(
    installation_id: i64,
    repo: &RepoRef,
    issue: &IssueSummary,
) -> Result<SessionRegistration, (i64, String)> {
    let spec =
        parse_trigger_issue_body(&issue.body).map_err(|err| (issue.number, err.to_string()))?;

    let session_id = derive_session_id(installation_id, &repo.owner, &repo.name, issue.number);
    // Hash the launch inputs BEFORE moving them into the def (config_hash borrows).
    let hash = config_hash(
        &spec.packages,
        &spec.work_label,
        spec.environment.as_deref(),
    );

    Ok(SessionRegistration {
        installation_id,
        repo: repo.clone(),
        trigger_issue: issue.number,
        trigger_author_id: issue.user_id,
        def: SessionDef {
            name: spec.name,
            packages: spec.packages,
            work_label: spec.work_label,
            environment: spec.environment,
        },
        session_id,
        config_hash: hash,
    })
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
