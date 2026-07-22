//! Metadata-only authorization gate for session trigger creators.
//!
//! A creator may start a session when they are an fkst global administrator or
//! have GitHub's `admin`/`maintain` repository role. Repository-role reads are
//! cached by lowercase login for one reconcile pass. Transport failures produce
//! [`TriggerGateDecision::Deferred`], leaving the caller to apply the durable
//! announce-latch safety rule instead of making a false authorization decision.

use std::collections::HashMap;

use secrecy::SecretString;

use crate::access_policy::AccessPolicy;
use crate::github_app::listing::GithubListing;
use crate::models::RepoRef;
use crate::reconcile::creator::SessionCreator;

/// One trigger-creator authorization decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerGateDecision {
    Authorized,
    Unauthorized { reason: String },
    Deferred,
}

/// Per-reconcile-pass collaborator-role cache, keyed by lowercase login.
#[derive(Debug, Default)]
pub struct TriggerAuthzCache {
    roles: HashMap<String, Option<String>>,
}

/// Check whether `creator` may start a session in `repo`.
pub async fn check_trigger_creator(
    listing: &dyn GithubListing,
    token: &SecretString,
    repo: &RepoRef,
    access: &AccessPolicy,
    creator: &SessionCreator,
    cache: &mut TriggerAuthzCache,
) -> TriggerGateDecision {
    // An assignee-derived creator has no numeric id. Numeric-only
    // FKST_GLOBAL_ADMINS entries therefore cannot match that creator; operators
    // should list global administrators by login for coverage of seeded sessions.
    if access.is_global_admin(creator.id.unwrap_or(-1), &creator.login) {
        return TriggerGateDecision::Authorized;
    }

    let key = creator.login.to_ascii_lowercase();
    let role = if let Some(cached) = cache.roles.get(&key) {
        cached.clone()
    } else {
        match listing
            .get_collaborator_role(token, &repo.owner, &repo.name, &creator.login)
            .await
        {
            Ok(role) => {
                cache.roles.insert(key, role.clone());
                role
            }
            Err(error) => {
                tracing::warn!(
                    repo = %format!("{}/{}", repo.owner, repo.name),
                    creator = %creator.login,
                    error = %error,
                    "trigger creator role lookup failed; deferring authorization decision"
                );
                return TriggerGateDecision::Deferred;
            }
        }
    };

    if matches!(role.as_deref(), Some("admin" | "maintain")) {
        TriggerGateDecision::Authorized
    } else {
        TriggerGateDecision::Unauthorized {
            reason: format!(
                "@{} does not have admin or maintain permission on {}/{}",
                creator.login, repo.owner, repo.name
            ),
        }
    }
}

#[cfg(test)]
#[path = "trigger_authz_tests.rs"]
mod tests;
