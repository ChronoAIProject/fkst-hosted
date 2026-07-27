//! Exact operator-owned cross-repository delivery grants.
//!
//! The control plane normally gives a session access to only its lifecycle
//! repository. `FKST_CROSS_REPO_DELIVERY_GRANTS` is the narrow exception: a JSON
//! array mapping one exact lifecycle issue to one implementation repository and
//! branch. The policy is parsed and validated once at startup. Session launchers
//! receive only the entries for their lifecycle repository, never the global
//! deployment policy.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::models::RepoRef;
use crate::reconcile::branches::validate_branch_name;

/// Operator configuration. Unset or blank preserves the historical
/// single-repository session behavior.
pub const CROSS_REPO_DELIVERY_GRANTS_ENV: &str = "FKST_CROSS_REPO_DELIVERY_GRANTS";

/// Launcher-to-driver contract containing only the validated entries applicable
/// to one session's lifecycle repository.
pub const SESSION_DELIVERY_GRANTS_ENV: &str = "FKST_SESSION_DELIVERY_GRANTS";

/// Driver-to-package contract. The in-pod driver adds the resolved checkout root
/// before exposing grants under this name to the supervised packages.
pub const DEVLOOP_DELIVERY_GRANTS_ENV: &str = "FKST_DEVLOOP_DELIVERY_GRANTS";

const MAX_GRANTS: usize = 128;
const MAX_POLICY_BYTES: usize = 64 * 1024;

/// One exact lifecycle-to-implementation route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryGrant {
    pub lifecycle_repo: String,
    pub lifecycle_issue: u64,
    pub implementation_repo: String,
    pub implementation_branch: String,
}

/// A worker-facing grant after the hosted driver has resolved and cloned the
/// implementation checkout. This remains non-secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDeliveryGrant {
    pub lifecycle_repo: String,
    pub lifecycle_issue: u64,
    pub implementation_repo: String,
    pub implementation_branch: String,
    pub implementation_root: String,
}

/// Validated, deterministically ordered deployment policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeliveryGrantPolicy {
    grants: Vec<DeliveryGrant>,
}

impl DeliveryGrantPolicy {
    /// Parse the operator environment snapshot. Blank/unset is intentionally the
    /// empty policy so deployments without the capability remain unchanged.
    pub fn from_vars(vars: &[(String, String)]) -> Result<Self, AppError> {
        let raw = vars
            .iter()
            .find(|(key, _)| key == CROSS_REPO_DELIVERY_GRANTS_ENV)
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        Self::parse(raw).map_err(AppError::Config)
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(Self::default());
        }
        if raw.len() > MAX_POLICY_BYTES {
            return Err(format!(
                "{CROSS_REPO_DELIVERY_GRANTS_ENV} must be at most {MAX_POLICY_BYTES} bytes"
            ));
        }

        let mut grants: Vec<DeliveryGrant> = serde_json::from_str(raw).map_err(|error| {
            format!(
                "{CROSS_REPO_DELIVERY_GRANTS_ENV} must be a JSON array of exact grants: {error}"
            )
        })?;
        if grants.len() > MAX_GRANTS {
            return Err(format!(
                "{CROSS_REPO_DELIVERY_GRANTS_ENV} must contain at most {MAX_GRANTS} grants"
            ));
        }

        let mut lifecycle_keys = HashSet::new();
        for (index, grant) in grants.iter_mut().enumerate() {
            grant.lifecycle_repo = normalized_repo(&grant.lifecycle_repo, index, "lifecycle_repo")?;
            grant.implementation_repo =
                normalized_repo(&grant.implementation_repo, index, "implementation_repo")?;
            grant.implementation_branch = grant.implementation_branch.trim().to_string();

            if grant.lifecycle_issue == 0 {
                return Err(format!(
                    "{CROSS_REPO_DELIVERY_GRANTS_ENV}[{index}].lifecycle_issue must be at least 1"
                ));
            }
            validate_branch_name(&grant.implementation_branch).map_err(|rule| {
                format!("{CROSS_REPO_DELIVERY_GRANTS_ENV}[{index}].implementation_branch {rule}")
            })?;
            if grant
                .lifecycle_repo
                .eq_ignore_ascii_case(&grant.implementation_repo)
            {
                return Err(format!(
                    "{CROSS_REPO_DELIVERY_GRANTS_ENV}[{index}] must target a different repository"
                ));
            }

            let key = format!(
                "{}#{}",
                grant.lifecycle_repo.to_ascii_lowercase(),
                grant.lifecycle_issue
            );
            if !lifecycle_keys.insert(key) {
                return Err(format!(
                    "{CROSS_REPO_DELIVERY_GRANTS_ENV} contains a duplicate lifecycle issue at index {index}"
                ));
            }
        }

        grants.sort_by(|left, right| {
            left.lifecycle_repo
                .to_ascii_lowercase()
                .cmp(&right.lifecycle_repo.to_ascii_lowercase())
                .then(left.lifecycle_issue.cmp(&right.lifecycle_issue))
        });
        Ok(Self { grants })
    }

    /// Parse the launcher-to-driver contract. Absence preserves historical
    /// behavior; a present value must be nonblank, valid, and contain only grants
    /// for the current lifecycle repository. The driver distrusts its environment
    /// even though the control plane produced it.
    pub fn parse_session_value(
        raw: Option<&str>,
        lifecycle_repo: &str,
    ) -> Result<Vec<DeliveryGrant>, String> {
        let Some(raw) = raw else {
            return Ok(Vec::new());
        };
        if raw.trim().is_empty() {
            return Err(format!(
                "{SESSION_DELIVERY_GRANTS_ENV} is present but empty"
            ));
        }
        let lifecycle_repo = normalized_repo(lifecycle_repo, 0, "lifecycle_repo")?;
        let policy = Self::parse(raw)?;
        if let Some(grant) = policy
            .grants
            .iter()
            .find(|grant| !grant.lifecycle_repo.eq_ignore_ascii_case(&lifecycle_repo))
        {
            return Err(format!(
                "{SESSION_DELIVERY_GRANTS_ENV} contains grant for {}, expected only {lifecycle_repo}",
                grant.lifecycle_repo
            ));
        }
        Ok(policy.grants)
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    /// Exact GitHub identity lookup. Repository matching follows GitHub's
    /// case-insensitive semantics; issue numbers are exact.
    pub fn find(&self, lifecycle_repo: &RepoRef, issue: u64) -> Option<&DeliveryGrant> {
        let owner_repo = format!("{}/{}", lifecycle_repo.owner, lifecycle_repo.name);
        self.grants.iter().find(|grant| {
            grant.lifecycle_issue == issue && grant.lifecycle_repo.eq_ignore_ascii_case(&owner_repo)
        })
    }

    /// Every grant applicable to a session serving `lifecycle_repo`, in stable
    /// issue order. A session is long-lived and may receive any of these exact
    /// work issues after startup, so its token and worker contract need the set.
    pub fn for_lifecycle_repo(&self, lifecycle_repo: &RepoRef) -> Vec<DeliveryGrant> {
        let owner_repo = format!("{}/{}", lifecycle_repo.owner, lifecycle_repo.name);
        self.grants
            .iter()
            .filter(|grant| grant.lifecycle_repo.eq_ignore_ascii_case(&owner_repo))
            .cloned()
            .collect()
    }

    /// Distinct implementation repositories for one session. Comparison is
    /// case-insensitive while the operator-provided canonical spelling is kept.
    pub fn implementation_repos_for(&self, lifecycle_repo: &RepoRef) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut repos = Vec::new();
        for grant in self.for_lifecycle_repo(lifecycle_repo) {
            let folded = grant.implementation_repo.to_ascii_lowercase();
            if seen.insert(folded) {
                repos.push(grant.implementation_repo);
            }
        }
        repos.sort_by_key(|repo| repo.to_ascii_lowercase());
        repos
    }

    /// Serialize only the current session's grants. `None` is load-bearing: the
    /// launcher omits the env key entirely when no policy applies.
    pub fn session_json_for(&self, lifecycle_repo: &RepoRef) -> Option<String> {
        let grants = self.for_lifecycle_repo(lifecycle_repo);
        if grants.is_empty() {
            return None;
        }
        Some(serde_json::to_string(&grants).expect("validated grants serialize"))
    }
}

fn normalized_repo(value: &str, index: usize, field: &str) -> Result<String, String> {
    let value = value.trim();
    let mut segments = value.split('/');
    let owner = segments.next().unwrap_or_default();
    let repo = segments.next().unwrap_or_default();
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    };
    if segments.next().is_some() || !valid_segment(owner) || !valid_segment(repo) {
        return Err(format!(
            "{CROSS_REPO_DELIVERY_GRANTS_ENV}[{index}].{field} must be exactly `owner/repo` without wildcards"
        ));
    }
    Ok(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(owner: &str, name: &str) -> RepoRef {
        RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn empty_policy_preserves_no_grant_behavior() {
        let policy = DeliveryGrantPolicy::parse("  ").unwrap();
        assert!(policy.is_empty());
        assert_eq!(policy.session_json_for(&repo("acme", "site")), None);
    }

    #[test]
    fn exact_routes_are_sorted_looked_up_and_scoped_per_lifecycle_repo() {
        let policy = DeliveryGrantPolicy::parse(
            r#"[
              {"lifecycle_repo":"Acme/Site","lifecycle_issue":9,"implementation_repo":"Acme/Tools","implementation_branch":"release/v1"},
              {"lifecycle_repo":"acme/site","lifecycle_issue":7,"implementation_repo":"Acme/Platform","implementation_branch":"main"},
              {"lifecycle_repo":"acme/other","lifecycle_issue":1,"implementation_repo":"Acme/Elsewhere","implementation_branch":"main"}
            ]"#,
        )
        .unwrap();

        let lifecycle = repo("ACME", "SITE");
        assert_eq!(
            policy.find(&lifecycle, 7).unwrap().implementation_repo,
            "Acme/Platform"
        );
        assert!(policy.find(&lifecycle, 8).is_none());
        let scoped = policy.for_lifecycle_repo(&lifecycle);
        assert_eq!(
            scoped
                .iter()
                .map(|grant| grant.lifecycle_issue)
                .collect::<Vec<_>>(),
            vec![7, 9]
        );
        assert!(!policy
            .session_json_for(&lifecycle)
            .unwrap()
            .contains("Elsewhere"));
    }

    #[test]
    fn parser_rejects_ambiguous_or_unsafe_policy() {
        let cases = [
            (
                r#"[{"lifecycle_repo":"acme/*","lifecycle_issue":1,"implementation_repo":"acme/tools","implementation_branch":"main"}]"#,
                "without wildcards",
            ),
            (
                r#"[{"lifecycle_repo":"acme/site","lifecycle_issue":0,"implementation_repo":"acme/tools","implementation_branch":"main"}]"#,
                "at least 1",
            ),
            (
                r#"[{"lifecycle_repo":"acme/site","lifecycle_issue":1,"implementation_repo":"acme/tools","implementation_branch":"bad branch"}]"#,
                "may contain only",
            ),
            (
                r#"[{"lifecycle_repo":"acme/site","lifecycle_issue":1,"implementation_repo":"ACME/SITE","implementation_branch":"main"}]"#,
                "different repository",
            ),
            (
                r#"[
              {"lifecycle_repo":"acme/site","lifecycle_issue":1,"implementation_repo":"acme/a","implementation_branch":"main"},
              {"lifecycle_repo":"ACME/SITE","lifecycle_issue":1,"implementation_repo":"acme/b","implementation_branch":"main"}
            ]"#,
                "duplicate lifecycle issue",
            ),
            (
                r#"[{"lifecycle_repo":"acme/site","lifecycle_issue":1,"implementation_repo":"acme/tools","implementation_branch":"main","extra":true}]"#,
                "unknown field",
            ),
        ];
        for (raw, expected) in cases {
            let error = DeliveryGrantPolicy::parse(raw).expect_err(raw);
            assert!(
                error.contains(expected),
                "{error:?} did not contain {expected:?}"
            );
        }
    }

    #[test]
    fn session_contract_is_absent_by_default_and_rejects_mismatched_scope() {
        assert!(DeliveryGrantPolicy::parse_session_value(None, "acme/site")
            .unwrap()
            .is_empty());
        assert!(
            DeliveryGrantPolicy::parse_session_value(Some("  "), "acme/site")
                .unwrap_err()
                .contains("present but empty")
        );

        let error = DeliveryGrantPolicy::parse_session_value(
            Some(
                r#"[{"lifecycle_repo":"acme/other","lifecycle_issue":1,"implementation_repo":"acme/tools","implementation_branch":"main"}]"#,
            ),
            "acme/site",
        )
        .unwrap_err();
        assert!(error.contains("expected only acme/site"), "{error}");
    }
}
