//! Safe arguments for the repository and installation mutations.
//!
//! A repository description is free text a user typed: it can name a customer,
//! quote an incident, or paste a token. It is therefore reduced to the two
//! properties that answer an operational question without carrying any of it —
//! whether one was supplied, and how large it was.
//!
//! The bearer token that authorizes the call and GitHub's response body are
//! absent by construction: neither is ever handed to this module.

use serde::Serialize;

use super::bounds::{byte_len, safe_owner, safe_repo};
use super::catalog;
use super::{sealed::Sealed, BoundedAuditArguments, ToSafeAuditArguments};
use crate::audit::event::ArgumentsParseStatus;

/// `create_repo` — creating a repository as the signed-in user.
#[derive(Clone, Debug, Serialize)]
pub struct SafeCreateRepo {
    /// The account the repository is created under: the requested organization,
    /// or the viewer's own login for a personal repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    private: bool,
    description_present: bool,
    description_bytes: u64,
}

impl BoundedAuditArguments for SafeCreateRepo {
    const OPERATION_ID: &'static str = "create_repo";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CREATE_REPO_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        // A dropped owner/name means the caller's value was not in the form the
        // route validates. The counts still describe the attempt; the value that
        // failed is never echoed.
        if self.owner.is_some() && self.name.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

/// The input view for `create_repo`, built from the handler's already-resolved
/// effective owner and its validated name.
pub struct CreateRepoInput<'a> {
    /// The organization when one was requested, else the viewer's own login.
    pub owner: &'a str,
    /// The trimmed repository name the route validated.
    pub name: &'a str,
    pub private: bool,
    /// The trimmed description, or `None` when absent/blank.
    pub description: Option<&'a str>,
}

impl Sealed for CreateRepoInput<'_> {}

impl ToSafeAuditArguments for CreateRepoInput<'_> {
    type Safe = SafeCreateRepo;

    fn to_safe_audit_arguments(&self) -> Self::Safe {
        SafeCreateRepo {
            owner: safe_owner(self.owner),
            name: safe_repo(self.name),
            private: self.private,
            description_present: self.description.is_some(),
            description_bytes: self.description.map(byte_len).unwrap_or(0),
        }
    }
}

/// `uninstall_account` — removing the App's installation from an account.
#[derive(Clone, Debug, Serialize)]
pub struct SafeUninstallAccount {
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
}

impl SafeUninstallAccount {
    pub fn new(owner: &str) -> Self {
        Self {
            owner: safe_owner(owner),
        }
    }
}

impl BoundedAuditArguments for SafeUninstallAccount {
    const OPERATION_ID: &'static str = "uninstall_account";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::UNINSTALL_ACCOUNT_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.owner.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

#[cfg(test)]
#[path = "repos_tests.rs"]
mod tests;
