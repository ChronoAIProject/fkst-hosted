//! Shared, credential-free fixtures for the session-access unit tests.
//!
//! Route tests must never need a live GitHub: every fixture here is built from
//! plain values, so a capability matrix or a registry generation can be asserted
//! without a token, a network call, or a cluster.

use crate::access_policy::AccessPolicy;
use crate::config::Config;
use crate::models::RepoRef;
use crate::reconcile::creator::SessionCreator;
use crate::state::{empty_self_router, AppState};

use super::context::SessionAccessContext;
use super::registry::SessionAccessRegistry;

/// A repository reference for `acme/<name>`.
pub(crate) fn repo(name: &str) -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: name.to_string(),
    }
}

/// A context for installation 1, `acme/site`, trigger issue 7.
pub(crate) fn context(
    creator_id: Option<i64>,
    creator_login: &str,
    collaborators: &[&str],
    log_access: &[&str],
) -> SessionAccessContext {
    context_in(
        1,
        "site",
        creator_id,
        creator_login,
        collaborators,
        log_access,
    )
}

/// A context for an explicit installation/repository, so replacement scoping can
/// be exercised.
pub(crate) fn context_in(
    installation_id: i64,
    repo_name: &str,
    creator_id: Option<i64>,
    creator_login: &str,
    collaborators: &[&str],
    log_access: &[&str],
) -> SessionAccessContext {
    SessionAccessContext {
        installation_id,
        repo: repo(repo_name),
        trigger_issue: 7,
        creator: SessionCreator {
            login: creator_login.to_string(),
            id: creator_id,
        },
        collaborators: collaborators.iter().map(|e| e.to_string()).collect(),
        log_access: log_access.iter().map(|e| e.to_string()).collect(),
    }
}

/// An access policy with the given `FKST_GLOBAL_ADMINS` entries and no list
/// enforcement (the open default every ordinary deployment starts from).
pub(crate) fn policy_with_admins(global_admins: &str) -> AccessPolicy {
    AccessPolicy::from_vars(&[("FKST_GLOBAL_ADMINS".to_string(), global_admins.to_string())])
        .expect("global-admin policy parses")
}

/// A cluster-free application state pointing at a mocked GitHub API base.
///
/// Nothing but the identity/authorization surface is wired: no cluster, no
/// storage, no reconciler — a route test must never need any of them to prove an
/// authorization outcome.
pub(crate) fn app_state(
    github_api_base_url: &str,
    access: AccessPolicy,
    registry: SessionAccessRegistry,
) -> AppState {
    let mut config = Config {
        github_api_base_url: github_api_base_url.to_string(),
        ..Config::default()
    };
    config.access = access;
    AppState {
        config,
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: super::SessionAccessState::new(registry),
        operations: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: empty_self_router(),
        chat: None,
        audit: Default::default(),
    }
}

/// A denylist policy blocking `blocked`, with optional global admins.
pub(crate) fn denylist(blocked: &str, global_admins: &str) -> AccessPolicy {
    AccessPolicy::from_vars(&[
        ("FKST_AUTH_MODEL".to_string(), "denylist".to_string()),
        ("FKST_ACCESS_BLOCKED_USERS".to_string(), blocked.to_string()),
        ("FKST_GLOBAL_ADMINS".to_string(), global_admins.to_string()),
    ])
    .expect("denylist policy parses")
}
