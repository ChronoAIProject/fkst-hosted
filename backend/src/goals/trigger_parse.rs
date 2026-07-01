//! Parser for the `fkst-substrate-trigger` issue body (Model B, #359 §3).
//!
//! Model B triggers a substrate session from a GitHub issue opened via the
//! `fkst-substrate-trigger` Issue Form. This module turns that user-authored issue
//! **body** into a [`TriggerSpec`] — the launch inputs the Model B launcher needs
//! — keying only on the four canonical `### ` sections. It reuses the shared
//! section-splitting skeleton in [`crate::goals::section_parse`] so the structural
//! contract (duplicate `### ` heading → 422; intro before the first heading
//! ignored) is identical to the `fkst-goal` parser.
//!
//! Scope boundary: this parser *structures + validates the shape* of the launch
//! inputs. It does NOT fetch the referenced packages, resolve them to concrete
//! directories, nor reconcile the work label against GitHub — that is deferred to
//! the launcher and a later fetch/reachability pass. What it DOES enforce is the
//! safety-relevant grammar: a DNS-label session name, fully-qualified GitHub
//! package references (`owner/repo@ref:path`) whose ref and path are path-safe (no
//! absolute path, no `..` traversal), and a single-value comma-free work label
//! (the substrate reads it from a comma-separated env var).
//!
//! Secret hygiene: this module logs nothing and never echoes section content.

use std::sync::OnceLock;

use regex::Regex;

use crate::error::AppError;
use crate::goals::section_parse::{
    env_name_regex, is_valid_env_name, non_empty_lines, parse_environment_name, split_sections,
    MAX_ENV_NAME_LEN,
};

/// The canonical `fkst-substrate-trigger` section headings, in template order.
const HEADING_SESSION_NAME: &str = "### Session Name";
const HEADING_PACKAGES: &str = "### Packages";
const HEADING_WORK_LABEL: &str = "### Work Label";
const HEADING_ENVIRONMENT: &str = "### Environment";

/// GitHub caps a label name at 50 characters; the Work Label must fit so the
/// launcher can apply it verbatim.
const MAX_WORK_LABEL_LEN: usize = 50;

/// The expected form of one `### Packages` line, echoed in every 422 so the author
/// can self-correct without leaving the issue.
const PACKAGE_REF_FORM: &str = "owner/repo@ref:path/to/package";

/// Anchored owner/repo-segment pattern: the safe token set a single `owner` or
/// `repo` segment of a package reference may draw from (letters, digits, `.`, `_`,
/// `-`). A `/` is deliberately absent — it separates owner from repo, so neither
/// segment may itself contain one.
fn owner_repo_segment_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_.-]+$").expect("static owner/repo segment regex"))
}

/// Anchored pattern for the `ref` and `path` parts of a package reference: the
/// safe token set (letters, digits, `.`, `_`, `/`, `-`). The leading-`/` and
/// `..`-segment checks run separately so their 422 messages can be specific.
fn ref_path_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_./-]+$").expect("static ref/path regex"))
}

/// A fully-qualified GitHub package reference parsed from one `### Packages` line,
/// of the form `owner/repo@ref:path`. Every part is shape-validated here; fetching
/// the package and checking reachability is deferred to a later pass. (`git_ref`
/// rather than `ref` because `ref` is a Rust keyword.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRef {
    /// The GitHub repository owner (user or org) — e.g. `ChronoAIProject`.
    pub owner: String,
    /// The GitHub repository name — e.g. `fkst-packages`.
    pub repo: String,
    /// The git ref (branch, tag, or SHA) the package is fetched at — e.g. `dev`.
    pub git_ref: String,
    /// The repo-relative path to the package directory — e.g.
    /// `packages/github-devloop`.
    pub path: String,
}

/// The structured launch inputs parsed from an `fkst-substrate-trigger` issue
/// body. Every field is shape-validated; semantic resolution (fetching a package,
/// whether the label exists) is the launcher's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerSpec {
    /// The session name — a DNS-1123-label-ish token (same rule as an environment
    /// name) so it composes into valid Kubernetes object names downstream.
    pub name: String,
    /// One [`PackageRef`] per non-empty `### Packages` line, in author order. Each
    /// is a fully-qualified GitHub package reference (`owner/repo@ref:path`);
    /// fetching + resolving it is deferred to a later pass.
    pub packages: Vec<PackageRef>,
    /// The single GitHub work label the launcher applies to drive the session.
    /// Guaranteed ≤ 50 chars and comma-free.
    pub work_label: String,
    /// The single named ENVIRONMENT the OPTIONAL `### Environment` section selects,
    /// or `None` when the section is absent or blank.
    pub environment: Option<String>,
}

/// Parse the `fkst-substrate-trigger` issue body into a [`TriggerSpec`].
///
/// Returns [`AppError::Unprocessable`] (→ 422) whose message NAMES the offending
/// section for every malformed case: a missing/mis-shaped `### Session Name`; a
/// missing/empty/mis-shaped `### Packages`; a missing/mis-shaped `### Work Label`;
/// an invalid `### Environment`; or a duplicate `### ` heading. The 422 (not 400)
/// matches the template-format contract shared with the `fkst-goal` parser.
pub fn parse_trigger_issue_body(body: &str) -> Result<TriggerSpec, AppError> {
    let sections = split_sections(body)?;

    let name = parse_session_name(&sections)?;
    let packages = parse_packages(&sections)?;
    let work_label = parse_work_label(&sections)?;

    // `### Environment` — OPTIONAL, reusing the shared rule verbatim: absent or
    // blank → `None`; one valid name → `Some`; two or more or an invalid name → a
    // 422 naming the section.
    let environment = match sections
        .iter()
        .find(|(heading, _)| heading == HEADING_ENVIRONMENT)
    {
        Some((_, content)) => parse_environment_name(content)?,
        None => None,
    };

    Ok(TriggerSpec {
        name,
        packages,
        work_label,
        environment,
    })
}

/// `### Session Name` — required; EXACTLY ONE non-empty line that satisfies the
/// shared environment-name rule (so the name composes into valid Kubernetes object
/// names). Zero or two-plus lines is a 422; an ill-formed name is a 422 naming the
/// rule.
fn parse_session_name(sections: &[(String, String)]) -> Result<String, AppError> {
    let block = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_SESSION_NAME)
        .map(|(_, content)| content.as_str())
        .ok_or_else(|| {
            AppError::Unprocessable("the `### Session Name` section is required".to_string())
        })?;
    match non_empty_lines(block).as_slice() {
        [name] if is_valid_env_name(name) => Ok(name.clone()),
        [name] => Err(AppError::Unprocessable(format!(
            "the `### Session Name` section names an invalid session name {name:?}: must match {} \
             and be 1..={MAX_ENV_NAME_LEN} characters",
            env_name_regex().as_str()
        ))),
        _ => Err(AppError::Unprocessable(
            "the `### Session Name` section must contain exactly one non-empty line".to_string(),
        )),
    }
}

/// `### Packages` — required; at least one non-empty line, EACH a fully-qualified
/// GitHub package reference `owner/repo@ref:path`. A missing/empty section, or any
/// malformed line, is a 422 naming the section (and, for a malformed line, the
/// offending value and which part failed).
fn parse_packages(sections: &[(String, String)]) -> Result<Vec<PackageRef>, AppError> {
    let block = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_PACKAGES)
        .map(|(_, content)| content.as_str())
        .ok_or_else(|| {
            AppError::Unprocessable("the `### Packages` section is required".to_string())
        })?;
    let lines = non_empty_lines(block);
    if lines.is_empty() {
        return Err(AppError::Unprocessable(
            "the `### Packages` section must list at least one package".to_string(),
        ));
    }
    let mut packages = Vec::with_capacity(lines.len());
    for line in &lines {
        packages.push(parse_package_ref(line)?);
    }
    Ok(packages)
}

/// Parse one `### Packages` line as a fully-qualified GitHub package reference
/// `owner/repo@ref:path`. The split is greedy on the FIRST `@` (`owner/repo` vs
/// `ref:path`) then the FIRST `:` (`ref` vs `path`). Every failure is a 422 that
/// names the section, echoes the offending value, states which part failed, and
/// recalls the expected form.
fn parse_package_ref(value: &str) -> Result<PackageRef, AppError> {
    let reject = |reason: &str| {
        AppError::Unprocessable(format!(
            "the `### Packages` section lists an invalid package reference {value:?}: {reason}; \
             expected the form {PACKAGE_REF_FORM}"
        ))
    };

    // Split on the FIRST `@`: everything before is `owner/repo`, everything after
    // is `ref:path`.
    let (owner_repo, ref_path) = value
        .split_once('@')
        .ok_or_else(|| reject("missing `@` separating `owner/repo` from `ref:path`"))?;
    // Split `ref:path` on the FIRST `:`: the ref, then the repo-relative path.
    let (git_ref, path) = ref_path
        .split_once(':')
        .ok_or_else(|| reject("missing `:` separating the ref from the path"))?;

    // `owner/repo`: exactly one `/`, each side a non-empty safe segment.
    if owner_repo.matches('/').count() != 1 {
        return Err(reject(
            "the part before `@` must be exactly `owner/repo` with a single `/`",
        ));
    }
    let (owner, repo) = owner_repo
        .split_once('/')
        .expect("a single `/` is present after the count check");
    for (segment, which) in [(owner, "owner"), (repo, "repo")] {
        if segment.is_empty() {
            return Err(reject(&format!("the {which} must not be empty")));
        }
        if !owner_repo_segment_regex().is_match(segment) {
            return Err(reject(&format!("the {which} must match ^[A-Za-z0-9_.-]+$")));
        }
    }

    // `ref`: non-empty, no `..` traversal segment, only the safe token set.
    if git_ref.is_empty() {
        return Err(reject("the ref must not be empty"));
    }
    if git_ref.split('/').any(|segment| segment == "..") {
        return Err(reject("the ref must not contain a `..` path segment"));
    }
    if !ref_path_regex().is_match(git_ref) {
        return Err(reject("the ref must match ^[A-Za-z0-9_./-]+$"));
    }

    // `path`: non-empty, not absolute, no `..` traversal segment, only the safe
    // token set. Mirrors the path-safety checks the old Package Roots applied.
    if path.is_empty() {
        return Err(reject("the path must not be empty"));
    }
    if path.starts_with('/') {
        return Err(reject("the path must not start with `/`"));
    }
    if path.split('/').any(|segment| segment == "..") {
        return Err(reject("the path must not contain a `..` path segment"));
    }
    if !ref_path_regex().is_match(path) {
        return Err(reject("the path must match ^[A-Za-z0-9_./-]+$"));
    }

    Ok(PackageRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
        path: path.to_string(),
    })
}

/// `### Work Label` — required; EXACTLY ONE non-empty line that is a valid GitHub
/// label: ≤ 50 characters and comma-free. The comma ban is load-bearing — the
/// substrate reads the label from a comma-separated env var, so a comma would split
/// it into two labels. Zero or two-plus lines is a 422; an over-long or comma-bearing
/// value is a 422 naming the section.
fn parse_work_label(sections: &[(String, String)]) -> Result<String, AppError> {
    let block = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_WORK_LABEL)
        .map(|(_, content)| content.as_str())
        .ok_or_else(|| {
            AppError::Unprocessable("the `### Work Label` section is required".to_string())
        })?;
    let label = match non_empty_lines(block).as_slice() {
        [label] => label.clone(),
        _ => {
            return Err(AppError::Unprocessable(
                "the `### Work Label` section must contain exactly one non-empty line".to_string(),
            ))
        }
    };
    if label.chars().count() > MAX_WORK_LABEL_LEN {
        return Err(AppError::Unprocessable(format!(
            "the `### Work Label` section names a label {label:?} longer than \
             {MAX_WORK_LABEL_LEN} characters"
        )));
    }
    if label.contains(',') {
        return Err(AppError::Unprocessable(format!(
            "the `### Work Label` section names a label {label:?} containing a comma; the \
             substrate reads a comma-separated env var, so a comma would split it into two labels"
        )));
    }
    Ok(label)
}

#[cfg(test)]
#[path = "trigger_parse_tests.rs"]
mod tests;
