//! The Model B fully-qualified package reference grammar (`owner/repo@ref:path`).
//!
//! Model B FETCHES a session's packages from public GitHub at runtime rather than
//! from repo-local `.fkst/packages/` (issue #359, "packages are FETCHED"). Each
//! `### Packages` trigger-issue line names one such ref, and the launcher carries
//! those same refs (space-joined) in `FKST_SESSION_PACKAGE_ROOTS`. This module is
//! the SINGLE source of truth for the ref shape shared by the writer (the trigger
//! parser / launcher) and the reader (the `run-substrate` entrypoint), so the two
//! can never disagree on how a ref is spelled or validated.
//!
//! Secret hygiene: a package ref is non-secret public metadata; this module logs
//! nothing.

use std::sync::OnceLock;

use regex::Regex;

/// A fully-qualified package reference parsed from `owner/repo@ref:path/to/package`.
///
/// The `run-substrate` entrypoint clones `owner/repo@ref` (which brings the whole
/// workspace: sibling `libraries/*`, `fkst.workspace.toml`, `fkst.lock`) and
/// activates the package rooted at `path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRef {
    /// GitHub repo owner (`^[A-Za-z0-9_.-]+$`).
    pub owner: String,
    /// GitHub repo name (`^[A-Za-z0-9_.-]+$`).
    pub repo: String,
    /// Branch / tag / commit (`^[A-Za-z0-9_./-]+$`, no `..` segment).
    pub git_ref: String,
    /// Package directory within the repo — contains `fkst.toml`
    /// (`^[A-Za-z0-9_./-]+$`, no leading `/`, no `..` segment).
    pub path: String,
}

/// Anchored charset for a single `owner` / `repo` segment.
fn owner_repo_segment_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_.-]+$").expect("static owner/repo segment regex"))
}

/// Anchored charset shared by the `@ref` and the `:path` fields.
fn ref_or_path_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_./-]+$").expect("static ref/path regex"))
}

/// Parse one `owner/repo@ref:path/to/package` package reference, validating each
/// field's grammar (issue #359 "packages are FETCHED"). Returns an `Err` naming
/// the offending token + reason on any violation.
///
/// None of the field charsets contain `@` or `:`, so splitting on the FIRST of
/// each is unambiguous even when the ref/path themselves contain `/`.
pub fn parse_package_ref(raw: &str) -> Result<PackageRef, String> {
    let raw = raw.trim();
    let (owner_repo, ref_path) = raw
        .split_once('@')
        .ok_or_else(|| format!("package ref {raw:?} is missing the `@ref` separator"))?;
    let (git_ref, path) = ref_path
        .split_once(':')
        .ok_or_else(|| format!("package ref {raw:?} is missing the `:path` separator"))?;
    let (owner, repo) = owner_repo
        .split_once('/')
        .ok_or_else(|| format!("package ref {raw:?} is missing the `owner/repo` separator"))?;

    validate_owner_repo_segment(owner, "owner", raw)?;
    validate_owner_repo_segment(repo, "repo", raw)?;
    validate_git_ref(git_ref, raw)?;
    validate_package_path(path, raw)?;

    Ok(PackageRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
        path: path.to_string(),
    })
}

/// The LAST `/`-segment of a package path — the platform-package NAME the engine
/// activates (e.g. `packages/github-devloop` → `github-devloop`). A path with no
/// `/` is its own name.
pub fn package_name_from_path(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Enforce the `owner` / `repo` segment charset (also rejects an empty segment,
/// e.g. a leading/trailing `/`).
fn validate_owner_repo_segment(segment: &str, which: &str, raw: &str) -> Result<(), String> {
    if !owner_repo_segment_regex().is_match(segment) {
        return Err(format!(
            "package ref {raw:?} has an invalid {which} {segment:?}: must match ^[A-Za-z0-9_.-]+$"
        ));
    }
    Ok(())
}

/// Enforce the `@ref` charset + the `..`-segment ban (a `..` could escape the
/// intended ref namespace).
fn validate_git_ref(git_ref: &str, raw: &str) -> Result<(), String> {
    if !ref_or_path_regex().is_match(git_ref) {
        return Err(format!(
            "package ref {raw:?} has an invalid @ref {git_ref:?}: must match ^[A-Za-z0-9_./-]+$"
        ));
    }
    if git_ref.split('/').any(|segment| segment == "..") {
        return Err(format!(
            "package ref {raw:?} has a @ref {git_ref:?} containing a `..` segment"
        ));
    }
    Ok(())
}

/// Enforce the `:path` charset, the no-leading-`/` rule, and the `..`-segment ban
/// (traversal guard — the entrypoint joins `path` onto the clone root).
fn validate_package_path(path: &str, raw: &str) -> Result<(), String> {
    if path.starts_with('/') {
        return Err(format!(
            "package ref {raw:?} has a :path {path:?} that must not start with `/`"
        ));
    }
    if !ref_or_path_regex().is_match(path) {
        return Err(format!(
            "package ref {raw:?} has an invalid :path {path:?}: must match ^[A-Za-z0-9_./-]+$"
        ));
    }
    if path.split('/').any(|segment| segment == "..") {
        return Err(format!(
            "package ref {raw:?} has a :path {path:?} containing a `..` segment"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_worked_example() {
        let parsed = parse_package_ref("ChronoAIProject/fkst-packages@dev:packages/github-devloop")
            .expect("worked example parses");
        assert_eq!(
            parsed,
            PackageRef {
                owner: "ChronoAIProject".to_string(),
                repo: "fkst-packages".to_string(),
                git_ref: "dev".to_string(),
                path: "packages/github-devloop".to_string(),
            }
        );
    }

    #[test]
    fn accepts_slashed_ref_and_surrounding_whitespace() {
        // A ref may carry `/` (e.g. `refs/tags/v1`) and the token may be padded.
        let parsed =
            parse_package_ref("  o/r@refs/tags/v1.2:pkgs/a  ").expect("slashed ref parses");
        assert_eq!(parsed.git_ref, "refs/tags/v1.2");
        assert_eq!(parsed.path, "pkgs/a");
    }

    #[test]
    fn rejects_missing_separators() {
        for raw in ["owner/repo:path", "owner/repo@ref", "owneronly@ref:path"] {
            assert!(
                parse_package_ref(raw).is_err(),
                "must reject malformed ref {raw:?}"
            );
        }
    }

    #[test]
    fn rejects_traversal_and_absolute_path() {
        let dotdot = parse_package_ref("o/r@dev:../evil").expect_err("`..` path must fail");
        assert!(dotdot.contains(".."), "{dotdot}");
        let abs = parse_package_ref("o/r@dev:/abs").expect_err("absolute path must fail");
        assert!(abs.contains("must not start"), "{abs}");
        let bad_ref = parse_package_ref("o/r@../x:pkg").expect_err("`..` ref must fail");
        assert!(bad_ref.contains(".."), "{bad_ref}");
    }

    #[test]
    fn rejects_illegal_owner_chars() {
        let err = parse_package_ref("bad owner/r@dev:pkg").expect_err("space in owner must fail");
        assert!(err.contains("owner"), "must name the field: {err}");
    }

    #[test]
    fn package_name_is_the_last_path_segment() {
        assert_eq!(
            package_name_from_path("packages/github-devloop"),
            "github-devloop"
        );
        assert_eq!(package_name_from_path("github-proxy"), "github-proxy");
        assert_eq!(package_name_from_path("a/b/c"), "c");
    }
}
