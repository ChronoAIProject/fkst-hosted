//! BSON document shape for the `github_installations` collection (issue #108).
//!
//! A GitHub App installation is owned by an *account* — which may be a personal
//! user account **or an organization**. The installer is not necessarily the
//! account owner (an org owner installs for the whole org), so this record
//! tracks the account, its type, and which repositories the installation
//! covers. It is the persisted answer to "which installation can act on
//! `owner/repo`?" so resolution does not require a live GitHub probe and
//! survives a pod restart.
//!
//! Conventions mirror the rest of `models`: `bson::DateTime` timestamps,
//! lowercase-on-the-wire enums, and `_id` carries the natural key (here the
//! GitHub-assigned `installation_id`, which is globally unique and stable for
//! the life of the installation).

use serde::{Deserialize, Serialize};

/// Sentinel covered-repo entry meaning "the installation covers every repo of
/// the account" (`repository_selection: all`). Stored in `repos` so a single
/// query shape answers coverage for both selection modes; an `all` installation
/// never enumerates concrete repos in the webhook payload, so the explicit
/// sentinel keeps the persisted record self-describing.
pub const COVERS_ALL: &str = "*";

/// The kind of account an installation is owned by. GitHub sends `User` or
/// `Organization` in the `installation.account.type` field; the enterprise
/// variant is mapped to [`AccountType::Organization`] (it behaves like an org
/// for installation purposes). Serializes capitalized to match GitHub.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AccountType {
    User,
    Organization,
}

impl AccountType {
    /// Map GitHub's `account.type` string onto the enum. Anything that is not a
    /// plain `User` is treated as an organization-like account (an org install
    /// is owner-gated; defaulting unknown/enterprise to `Organization` keeps the
    /// stricter, safer install guidance — see the install-hint logic in #108).
    pub fn from_github(account_type: &str) -> Self {
        if account_type.eq_ignore_ascii_case("User") {
            AccountType::User
        } else {
            AccountType::Organization
        }
    }
}

/// How the installation selects the repositories it covers, mirroring GitHub's
/// `repository_selection`. `All` covers every current and future repo of the
/// account; `Selected` covers only the enumerated `repos`. Serializes lowercase
/// to match GitHub.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RepositorySelection {
    All,
    Selected,
}

impl RepositorySelection {
    /// Map GitHub's `repository_selection` string. Defaults to `Selected` when
    /// the field is absent or unrecognized: the conservative choice, since a
    /// `Selected` record only ever claims coverage of repos it has explicitly
    /// recorded (an over-broad `All` would wrongly skip the on-demand resolve).
    pub fn from_github(selection: Option<&str>) -> Self {
        match selection {
            Some(s) if s.eq_ignore_ascii_case("all") => RepositorySelection::All,
            _ => RepositorySelection::Selected,
        }
    }
}

/// `github_installations` collection: `_id` is the GitHub-assigned
/// `installation_id` (a `u64`, globally unique and stable for the life of the
/// install). The record is upserted from `installation` /
/// `installation_repositories` webhook events and from on-demand resolution.
///
/// `repos` holds the lowercase `owner/name` full names the installation covers.
/// For a `RepositorySelection::All` installation it holds the single
/// [`COVERS_ALL`] sentinel (GitHub does not enumerate repos for an `all`
/// install), so [`Self::covers`] can answer coverage uniformly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubInstallationDoc {
    /// The GitHub installation id, stored as the document `_id` so an upsert
    /// keyed on it is a single atomic operation and redeliveries are idempotent.
    #[serde(rename = "_id")]
    pub installation_id: i64,
    /// The login of the account the App is installed on (user or org), stored
    /// lowercase for case-insensitive lookup by `owner`.
    pub account_login: String,
    /// Whether the account is a user or an organization (#108: do not assume a
    /// user account — an org install is owner-gated).
    pub account_type: AccountType,
    /// `all` vs `selected`. Drives whether `repos` is authoritative or just the
    /// known subset of an account-wide install.
    pub repository_selection: RepositorySelection,
    /// Lowercase `owner/name` full names covered by the installation. Holds the
    /// [`COVERS_ALL`] sentinel for an `all` install.
    pub repos: Vec<String>,
    /// `true` when GitHub has *suspended* the installation (the `suspend`
    /// event): the install still exists but cannot mint tokens, so resolution
    /// must treat it as unavailable until `unsuspend`.
    pub suspended: bool,
    /// Last time this record was written (any upsert or coverage change).
    pub updated_at: bson::DateTime,
}

impl GithubInstallationDoc {
    /// Canonicalize an `owner/name` repo reference to the stored form
    /// (lowercase). GitHub repo and owner names are case-insensitive, so storing
    /// and querying lowercase avoids a case-mismatched miss that would force an
    /// unnecessary on-demand resolve.
    pub fn canonical_repo(owner: &str, name: &str) -> String {
        format!("{}/{}", owner.to_lowercase(), name.to_lowercase())
    }

    /// True when this installation covers `owner/name`. An `all` install (the
    /// [`COVERS_ALL`] sentinel present) covers any repo of its account; a
    /// `selected` install covers only repos it has explicitly recorded. A
    /// suspended installation covers nothing (it cannot mint a token).
    pub fn covers(&self, owner: &str, name: &str) -> bool {
        if self.suspended {
            return false;
        }
        if self.repos.iter().any(|r| r == COVERS_ALL) {
            // `all` install: coverage is scoped to the account's own repos.
            return owner.eq_ignore_ascii_case(&self.account_login);
        }
        let target = Self::canonical_repo(owner, name);
        self.repos.iter().any(|r| r == &target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> GithubInstallationDoc {
        GithubInstallationDoc {
            installation_id: 42,
            account_login: "acme".to_string(),
            account_type: AccountType::Organization,
            repository_selection: RepositorySelection::Selected,
            repos: vec!["acme/site".to_string()],
            suspended: false,
            updated_at: bson::DateTime::from_millis(1_700_000_000_000),
        }
    }

    #[test]
    fn account_type_maps_from_github() {
        assert_eq!(AccountType::from_github("User"), AccountType::User);
        assert_eq!(AccountType::from_github("user"), AccountType::User);
        assert_eq!(
            AccountType::from_github("Organization"),
            AccountType::Organization
        );
        // Unknown/enterprise => org-like (the stricter, owner-gated default).
        assert_eq!(
            AccountType::from_github("Enterprise"),
            AccountType::Organization
        );
    }

    #[test]
    fn account_type_serializes_capitalized() {
        assert_eq!(
            bson::to_bson(&AccountType::Organization).unwrap(),
            bson::Bson::String("Organization".to_string())
        );
        assert_eq!(
            bson::to_bson(&AccountType::User).unwrap(),
            bson::Bson::String("User".to_string())
        );
    }

    #[test]
    fn repository_selection_maps_and_defaults_conservatively() {
        assert_eq!(
            RepositorySelection::from_github(Some("all")),
            RepositorySelection::All
        );
        assert_eq!(
            RepositorySelection::from_github(Some("selected")),
            RepositorySelection::Selected
        );
        // Absent / unknown => Selected (never wrongly claim account-wide cover).
        assert_eq!(
            RepositorySelection::from_github(None),
            RepositorySelection::Selected
        );
        assert_eq!(
            RepositorySelection::from_github(Some("weird")),
            RepositorySelection::Selected
        );
    }

    #[test]
    fn repository_selection_serializes_lowercase() {
        assert_eq!(
            bson::to_bson(&RepositorySelection::All).unwrap(),
            bson::Bson::String("all".to_string())
        );
        assert_eq!(
            bson::to_bson(&RepositorySelection::Selected).unwrap(),
            bson::Bson::String("selected".to_string())
        );
    }

    #[test]
    fn doc_round_trips_with_id_as_installation_id() {
        let doc = sample();
        let raw = bson::to_document(&doc).expect("serialize");
        // `_id` must carry the installation id (an i64), not a separate field.
        assert_eq!(raw.get_i64("_id").expect("_id is i64"), 42);
        assert!(
            !raw.contains_key("installation_id"),
            "installation_id maps onto _id only"
        );
        let back: GithubInstallationDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn covers_selected_is_case_insensitive_and_scoped() {
        let doc = sample();
        assert!(doc.covers("acme", "site"));
        assert!(
            doc.covers("ACME", "Site"),
            "owner/name are case-insensitive"
        );
        assert!(!doc.covers("acme", "other"), "unlisted repo not covered");
    }

    #[test]
    fn covers_all_install_covers_any_repo_of_the_account() {
        let mut doc = sample();
        doc.repository_selection = RepositorySelection::All;
        doc.repos = vec![COVERS_ALL.to_string()];
        assert!(doc.covers("acme", "anything"));
        assert!(doc.covers("acme", "else"));
        // But not another account's repo (coverage is account-scoped).
        assert!(!doc.covers("other-org", "repo"));
    }

    #[test]
    fn suspended_install_covers_nothing() {
        let mut doc = sample();
        doc.suspended = true;
        assert!(!doc.covers("acme", "site"));
        doc.repository_selection = RepositorySelection::All;
        doc.repos = vec![COVERS_ALL.to_string()];
        assert!(!doc.covers("acme", "site"), "suspended cannot mint a token");
    }

    #[test]
    fn canonical_repo_lowercases() {
        assert_eq!(
            GithubInstallationDoc::canonical_repo("Acme", "Site"),
            "acme/site"
        );
    }
}
