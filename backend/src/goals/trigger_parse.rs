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
//! inputs. It does NOT resolve package roots to concrete packages or dirs, nor
//! reconcile the work label against GitHub — that is deferred to the launcher.
//! What it DOES enforce is the safety-relevant grammar: a DNS-label session name,
//! path-safe package roots (no absolute path, no `..` traversal), and a
//! single-value comma-free work label (the substrate reads it from a
//! comma-separated env var).
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
const HEADING_PACKAGE_ROOTS: &str = "### Package Roots";
const HEADING_WORK_LABEL: &str = "### Work Label";
const HEADING_ENVIRONMENT: &str = "### Environment";

/// GitHub caps a label name at 50 characters; the Work Label must fit so the
/// launcher can apply it verbatim.
const MAX_WORK_LABEL_LEN: usize = 50;

/// Anchored package-root pattern: the safe token set a package root may draw from
/// (letters, digits, `.`, `_`, `/`, `-`). The leading-`/` and `..`-segment checks
/// run separately so their 422 messages can be specific.
fn package_root_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9._/-]+$").expect("static package root regex"))
}

/// The structured launch inputs parsed from an `fkst-substrate-trigger` issue
/// body. Every field is shape-validated; semantic resolution (which package a root
/// names, whether the label exists) is the launcher's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerSpec {
    /// The session name — a DNS-1123-label-ish token (same rule as an environment
    /// name) so it composes into valid Kubernetes object names downstream.
    pub name: String,
    /// One package root per non-empty `### Package Roots` line. Each is a path-safe
    /// token naming a platform package or a repo-relative package directory;
    /// interpretation is deferred to the launcher.
    pub package_roots: Vec<String>,
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
/// missing/empty/unsafe `### Package Roots`; a missing/mis-shaped `### Work Label`;
/// an invalid `### Environment`; or a duplicate `### ` heading. The 422 (not 400)
/// matches the template-format contract shared with the `fkst-goal` parser.
pub fn parse_trigger_issue_body(body: &str) -> Result<TriggerSpec, AppError> {
    let sections = split_sections(body)?;

    let name = parse_session_name(&sections)?;
    let package_roots = parse_package_roots(&sections)?;
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
        package_roots,
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

/// `### Package Roots` — required; at least one non-empty line, EACH a path-safe
/// token. A root may not be absolute (leading `/`), may not contain a `..` path
/// segment, and must draw only from the safe token set. Any violation is a 422
/// naming the section and the offending value.
fn parse_package_roots(sections: &[(String, String)]) -> Result<Vec<String>, AppError> {
    let block = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_PACKAGE_ROOTS)
        .map(|(_, content)| content.as_str())
        .ok_or_else(|| {
            AppError::Unprocessable("the `### Package Roots` section is required".to_string())
        })?;
    let roots = non_empty_lines(block);
    if roots.is_empty() {
        return Err(AppError::Unprocessable(
            "the `### Package Roots` section must list at least one package root".to_string(),
        ));
    }
    for root in &roots {
        validate_package_root(root)?;
    }
    Ok(roots)
}

/// Reject a package root that is absolute, escapes via `..`, or contains a
/// character outside the safe token set. Ordered so the 422 message pins the most
/// specific reason (absolute path, then traversal, then illegal character).
fn validate_package_root(root: &str) -> Result<(), AppError> {
    let reject = |reason: &str| {
        AppError::Unprocessable(format!(
            "the `### Package Roots` section lists an invalid package root {root:?}: {reason}"
        ))
    };
    if root.starts_with('/') {
        return Err(reject("must not start with `/`"));
    }
    if root.split('/').any(|segment| segment == "..") {
        return Err(reject("must not contain a `..` path segment"));
    }
    if !package_root_regex().is_match(root) {
        return Err(reject("must match ^[A-Za-z0-9._/-]+$"));
    }
    Ok(())
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
mod tests {
    use super::*;

    /// A fully-populated body parses to the documented [`TriggerSpec`], preserving
    /// the package-root order and reading the optional environment.
    #[test]
    fn worked_example_parses_all_four_sections() {
        let body = "\
### Session Name

my-session

### Package Roots

platform-pkg
repo/dir/pkg

### Work Label

fkst-cloud

### Environment

prod-env
";
        let spec = parse_trigger_issue_body(body).expect("worked example parses");
        assert_eq!(
            spec,
            TriggerSpec {
                name: "my-session".to_string(),
                package_roots: vec!["platform-pkg".to_string(), "repo/dir/pkg".to_string()],
                work_label: "fkst-cloud".to_string(),
                environment: Some("prod-env".to_string()),
            }
        );
    }

    #[test]
    fn absent_environment_section_yields_none() {
        let body = "### Session Name\nsess\n### Package Roots\npkg-a\n### Work Label\nlabel\n";
        let spec = parse_trigger_issue_body(body).expect("parses without Environment");
        assert!(spec.environment.is_none());
    }

    #[test]
    fn intro_before_first_heading_is_ignored() {
        let body =
            "Form intro the user never edits\n\n### Session Name\nsess\n### Package Roots\npkg-a\n### Work Label\nlabel\n";
        let spec = parse_trigger_issue_body(body).expect("intro ignored");
        assert_eq!(spec.name, "sess");
    }

    // ---- Malformed cases (each names the offending section in the 422) ----

    fn err_message(body: &str) -> String {
        match parse_trigger_issue_body(body) {
            Err(AppError::Unprocessable(msg)) => msg,
            other => panic!("expected Unprocessable (422), got {other:?}"),
        }
    }

    #[test]
    fn missing_session_name_is_422_naming_the_section() {
        let msg = err_message("### Package Roots\npkg-a\n### Work Label\nlabel\n");
        assert!(msg.contains("Session Name"), "must name the section: {msg}");
    }

    #[test]
    fn multiline_session_name_is_422_naming_the_section() {
        let body =
            "### Session Name\nfirst\nsecond\n### Package Roots\npkg-a\n### Work Label\nlabel\n";
        let msg = err_message(body);
        assert!(msg.contains("Session Name"), "must name the section: {msg}");
        assert!(msg.contains("exactly one"), "must flag the count: {msg}");
    }

    #[test]
    fn invalid_session_name_chars_is_422_naming_the_value() {
        let body =
            "### Session Name\nMy_Session\n### Package Roots\npkg-a\n### Work Label\nlabel\n";
        let msg = err_message(body);
        assert!(msg.contains("Session Name"), "must name the section: {msg}");
        assert!(msg.contains("My_Session"), "must name the value: {msg}");
    }

    #[test]
    fn missing_package_roots_lines_is_422_naming_the_section() {
        // The heading is present but has zero non-empty lines.
        let body = "### Session Name\nsess\n### Package Roots\n\n### Work Label\nlabel\n";
        let msg = err_message(body);
        assert!(
            msg.contains("Package Roots"),
            "must name the section: {msg}"
        );
        assert!(
            msg.contains("at least one"),
            "must flag the emptiness: {msg}"
        );
    }

    #[test]
    fn package_root_with_leading_slash_is_422_naming_the_value() {
        let body = "### Session Name\nsess\n### Package Roots\n/abs/pkg\n### Work Label\nlabel\n";
        let msg = err_message(body);
        assert!(
            msg.contains("Package Roots"),
            "must name the section: {msg}"
        );
        assert!(msg.contains("/abs/pkg"), "must name the value: {msg}");
    }

    #[test]
    fn package_root_with_dotdot_segment_is_422_naming_the_value() {
        let body = "### Session Name\nsess\n### Package Roots\nfoo/../bar\n### Work Label\nlabel\n";
        let msg = err_message(body);
        assert!(
            msg.contains("Package Roots"),
            "must name the section: {msg}"
        );
        assert!(msg.contains("foo/../bar"), "must name the value: {msg}");
        assert!(msg.contains(".."), "must flag the traversal: {msg}");
    }

    #[test]
    fn package_root_with_illegal_char_is_422_naming_the_value() {
        let body = "### Session Name\nsess\n### Package Roots\nbad pkg\n### Work Label\nlabel\n";
        let msg = err_message(body);
        assert!(
            msg.contains("Package Roots"),
            "must name the section: {msg}"
        );
        assert!(msg.contains("bad pkg"), "must name the value: {msg}");
    }

    #[test]
    fn missing_work_label_is_422_naming_the_section() {
        let msg = err_message("### Session Name\nsess\n### Package Roots\npkg-a\n");
        assert!(msg.contains("Work Label"), "must name the section: {msg}");
    }

    #[test]
    fn multiline_work_label_is_422_naming_the_section() {
        let body = "### Session Name\nsess\n### Package Roots\npkg-a\n### Work Label\none\ntwo\n";
        let msg = err_message(body);
        assert!(msg.contains("Work Label"), "must name the section: {msg}");
        assert!(msg.contains("exactly one"), "must flag the count: {msg}");
    }

    #[test]
    fn work_label_with_comma_is_422_naming_the_section() {
        let body = "### Session Name\nsess\n### Package Roots\npkg-a\n### Work Label\nred, blue\n";
        let msg = err_message(body);
        assert!(msg.contains("Work Label"), "must name the section: {msg}");
        assert!(msg.contains("comma"), "must flag the comma: {msg}");
    }

    #[test]
    fn duplicate_work_label_heading_is_422() {
        let body = "### Session Name\nsess\n### Package Roots\npkg-a\n### Work Label\nx\n### Work Label\ny\n";
        let msg = err_message(body);
        assert!(msg.contains("duplicate"), "must flag the duplicate: {msg}");
        assert!(msg.contains("Work Label"), "must name the section: {msg}");
    }

    #[test]
    fn environment_with_two_lines_is_422_naming_the_section() {
        let body = "### Session Name\nsess\n### Package Roots\npkg-a\n### Work Label\nlabel\n### Environment\nfirst\nsecond\n";
        let msg = err_message(body);
        assert!(msg.contains("Environment"), "must name the section: {msg}");
        assert!(
            msg.contains("exactly one"),
            "must flag the ambiguity: {msg}"
        );
    }
}
