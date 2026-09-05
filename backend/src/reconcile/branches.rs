//! Branch configuration shared by trigger parsing and session provisioning.
//!
//! Source and target may explicitly name the same branch. That topology is
//! degenerate but valid: once the target exists, every work branch derives from
//! that target and the source is no longer consulted.

/// Target branch used when a trigger omits `### Target Branch`.
pub const DEFAULT_TARGET_BRANCH: &str = "fkst-hosted-default";

/// The sentinel that resolves to a repository's *current* default branch.
///
/// FKST Evolution configuration names its trusted source branch this way so a
/// repository that renames or switches its default branch does not silently
/// keep reconciling against the old one.
pub const DEFAULT_BRANCH_SENTINEL: &str = "@default";

/// A configured branch: either the dynamic sentinel or a literal name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchRef {
    /// Resolve against the repository's default branch at reconcile time.
    Dynamic,
    /// A literal branch name, already validated by [`validate_branch_name`].
    Named(String),
}

impl BranchRef {
    /// The branch this reference denotes, given the repository's current default.
    pub fn resolve<'a>(&'a self, repository_default: &'a str) -> &'a str {
        match self {
            BranchRef::Dynamic => repository_default,
            BranchRef::Named(name) => name.as_str(),
        }
    }

    /// True when this reference re-resolves on every reconcile.
    pub fn is_dynamic(&self) -> bool {
        matches!(self, BranchRef::Dynamic)
    }
}

/// Parse a branch reference, accepting the [`DEFAULT_BRANCH_SENTINEL`].
///
/// WHY this is a separate function rather than a relaxation of
/// [`validate_branch_name`]: that validator is shared by trigger parsing,
/// delivery grants, and audit-argument bounds, where a `@`-bearing value is
/// meaningless and admitting one would widen a security-relevant input check for
/// every caller to serve one. The sentinel is a *reference* concept; branch
/// names stay exactly as strict as before.
pub fn parse_branch_ref(value: &str) -> Result<BranchRef, String> {
    if value == DEFAULT_BRANCH_SENTINEL {
        return Ok(BranchRef::Dynamic);
    }
    if value.starts_with('@') {
        // Named so a typo reports the supported sentinel instead of the generic
        // character-class rule, which sends the reader looking for the wrong bug.
        return Err(format!(
            "unknown branch sentinel `{value}` (the only supported sentinel is `{DEFAULT_BRANCH_SENTINEL}`)"
        ));
    }
    validate_branch_name(value)?;
    Ok(BranchRef::Named(value.to_string()))
}

/// Validate the conservative branch-name subset accepted in trigger issues.
pub fn validate_branch_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("must not be empty".to_string());
    }
    if name.len() > 200 {
        return Err("must be at most 200 characters".to_string());
    }
    if name == "@" {
        return Err("must not equal `@`".to_string());
    }
    if name.starts_with(['-', '/', '.']) {
        return Err("must not start with `-`, `/`, or `.`".to_string());
    }
    if name.ends_with('/') || name.ends_with('.') {
        return Err("must not end with `/` or `.`".to_string());
    }
    if name.ends_with(".lock") {
        return Err("must not end with `.lock`".to_string());
    }
    if name.contains("..") {
        return Err("must not contain `..`".to_string());
    }
    if name.contains("//") {
        return Err("must not contain `//`".to_string());
    }
    if name.contains("@{") {
        return Err("must not contain `@{`".to_string());
    }
    if !name
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'/' | b'-'))
    {
        return Err("may contain only `[A-Za-z0-9._/-]` characters".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_branch_names() {
        for name in ["main", "release/v1.2", "feature_one", DEFAULT_TARGET_BRANCH] {
            assert_eq!(validate_branch_name(name), Ok(()), "{name}");
        }
    }

    #[test]
    fn rejects_every_forbidden_shape() {
        let too_long = "a".repeat(201);
        for (name, rule) in [
            ("", "empty"),
            (too_long.as_str(), "200"),
            ("@", "equal"),
            ("-topic", "start"),
            ("/topic", "start"),
            (".topic", "start"),
            ("topic/", "end"),
            ("topic.", "end"),
            ("topic.lock", ".lock"),
            ("topic..next", ".."),
            ("topic//next", "//"),
            ("topic@{next", "@{"),
            ("topic next", "only"),
            ("topic~next", "only"),
        ] {
            let error = validate_branch_name(name).expect_err(name);
            assert!(error.contains(rule), "{name:?}: {error}");
        }
    }

    #[test]
    fn parses_the_default_branch_sentinel() {
        assert_eq!(
            parse_branch_ref(DEFAULT_BRANCH_SENTINEL),
            Ok(BranchRef::Dynamic)
        );
        assert!(parse_branch_ref(DEFAULT_BRANCH_SENTINEL)
            .unwrap()
            .is_dynamic());
    }

    #[test]
    fn parses_a_literal_branch_name() {
        assert_eq!(
            parse_branch_ref("release/v1.2"),
            Ok(BranchRef::Named("release/v1.2".to_string()))
        );
        assert!(!parse_branch_ref("main").unwrap().is_dynamic());
    }

    #[test]
    fn rejects_an_unknown_sentinel_by_name() {
        // A generic character-class error here would send the reader looking for
        // the wrong bug, so the message must name the supported sentinel.
        let error = parse_branch_ref("@head").expect_err("@head");
        assert!(error.contains("@default"), "{error}");
        assert!(error.contains("unknown branch sentinel"), "{error}");
    }

    #[test]
    fn rejects_an_invalid_literal_name_exactly_as_before() {
        for name in ["", "-topic", "topic..next", "topic next"] {
            assert!(parse_branch_ref(name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn accepting_the_sentinel_does_not_loosen_branch_name_validation() {
        // The whole point of keeping these separate: `validate_branch_name` is
        // shared by delivery grants and audit bounds, where `@default` is
        // meaningless and must stay rejected.
        assert!(validate_branch_name(DEFAULT_BRANCH_SENTINEL).is_err());
    }

    #[test]
    fn dynamic_resolves_to_the_repository_default_and_named_ignores_it() {
        assert_eq!(BranchRef::Dynamic.resolve("develop"), "develop");
        assert_eq!(
            BranchRef::Named("main".to_string()).resolve("develop"),
            "main"
        );
    }
}
