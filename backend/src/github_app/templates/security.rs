//! Deterministic security reporting contact. Never merge repository-supplied
//! YAML into a template PR that the App may automatically merge.
const CONFIG: &str = include_str!("../templates_assets/config.yml");
const URL_SLOT: &str = "__FKST_SECURITY_POLICY_URL__";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InvalidRepository;

pub(crate) fn repository_config(owner: &str, repo: &str) -> Result<String, InvalidRepository> {
    // GitHub owner/repository path components only; no URL/YAML metacharacters,
    // credentials, percent encodings, extra segments or dot-segment traversal.
    if owner.is_empty()
        || owner.len() > 39
        || owner.starts_with('-')
        || owner.ends_with('-')
        || !owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || repo.is_empty()
        || repo.len() > 100
        || matches!(repo, "." | "..")
        || !repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(InvalidRepository);
    }
    // The policy page works whether or not private advisory reporting is enabled.
    // It does not imply that a repository has enabled a reporting channel.
    Ok(CONFIG.replace(
        URL_SLOT,
        &format!("https://github.com/{owner}/{repo}/security/policy"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nyxid_chooser_routes_through_maintained_policy() {
        let output = repository_config("ChronoAIProject", "NyxID").unwrap();
        assert!(output.contains("url: https://github.com/ChronoAIProject/NyxID/security/policy\n"));
        assert!(output.contains("Do not disclose vulnerabilities or secrets in public issues."));
        assert_eq!(
            output
                .matches("Security policy and vulnerability reporting")
                .count(),
            1
        );
        assert_eq!(
            output.matches("# fkst-issue-templates-version: ").count(),
            1
        );
        assert!(output.contains("blank_issues_enabled: false"));
        assert!(output.contains("name: fkst docs & help"));
        assert!(!output.contains(URL_SLOT));
    }
    #[test]
    fn generic_repository_does_not_assume_private_reporting() {
        let output = repository_config("some-owner", ".github_repo-2").unwrap();
        assert!(output.contains("https://github.com/some-owner/.github_repo-2/security/policy"));
        assert!(!output.contains("/advisories/new"));
        assert_eq!(
            output,
            repository_config("some-owner", ".github_repo-2").unwrap()
        );
    }
    #[test]
    fn malformed_components_cannot_inject_urls_or_yaml() {
        for bad in [
            "", "/", "a/b", "a:b", "a?b", "a#b", "a%2fb", "a\nb", "a b", "a@b", "é",
        ] {
            assert!(repository_config(bad, "repo").is_err(), "owner {bad:?}");
            assert!(repository_config("owner", bad).is_err(), "repo {bad:?}");
        }
        for bad in [".", ".."] {
            assert!(repository_config("owner", bad).is_err());
        }
        for bad in ["-owner", "owner-", &"a".repeat(40)] {
            assert!(repository_config(bad, "repo").is_err());
        }
        assert!(repository_config("owner", &"a".repeat(101)).is_err());
    }
    #[test]
    fn bundle_has_exactly_one_renderer_slot() {
        assert_eq!(CONFIG.matches(URL_SLOT).count(), 1);
    }
}
