//! Branch configuration shared by trigger parsing and session provisioning.
//!
//! Source and target may explicitly name the same branch. That topology is
//! degenerate but valid: once the target exists, every work branch derives from
//! that target and the source is no longer consulted.

/// Target branch used when a trigger omits `### Target Branch`.
pub const DEFAULT_TARGET_BRANCH: &str = "fkst-hosted-default";

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
}
