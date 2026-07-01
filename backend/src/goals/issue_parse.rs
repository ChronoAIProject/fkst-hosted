//! Parser for the `fkst-goal` issue template body (milestone #5, #178).
//!
//! A user can declare a goal by opening a GitHub issue from the `fkst goal`
//! Issue Form (`.github/ISSUE_TEMPLATE/fkst-goal.yml`, #177) instead of the
//! inline submit payload. This module turns that user-authored issue **body**
//! into the structured goal fields the submit handler needs, keying ONLY on the
//! canonical, machine-parseable parse contract documented in
//! `docs/api-reference.md` § "The `fkst-goal` issue template & parse contract".
//!
//! Scope boundary: this parser only *structures* the text. It does NOT validate
//! package-name grammar (deferred to [`crate::goals::validate_goal_fields`]); the
//! submit handler runs that after parsing so one validation path is shared with
//! the inline source. The parser DOES enforce the structural contract: the
//! required sections exist, the Goal and Package list are non-empty, and no
//! `### ` heading is duplicated.
//!
//! Secret hygiene: the `Goal` section becomes the engine prompt (secret
//! downstream). Never log the parsed `description`; this module logs nothing.

use std::sync::OnceLock;

use regex::Regex;

use crate::error::AppError;

/// The canonical section headings, in template order. A section's content is
/// every line after its heading up to the next `### ` heading (or EOF).
const HEADING_GOAL: &str = "### Goal";
const HEADING_PACKAGES: &str = "### Package Name List";
/// OPTIONAL section (PR4b): each non-blank line names ONE env var KEY to inject
/// into the session from the issue author's `fkst-user-<id>` store. Absent →
/// no injection.
const HEADING_ENVIRONMENT: &str = "### Environment";

/// Anchored env-var-name pattern, identical to the env-var key grammar the
/// named-environment API (`crate::routes::environments`) enforces. A name that
/// passes here is also a valid Kubernetes ConfigMap/Secret data key, so the
/// requested key can be looked up in the author's store and mounted unescaped.
fn env_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z_][A-Za-z0-9_]*$").expect("static env key regex"))
}

/// The structured result of parsing an `fkst-goal` issue body. Carries only the
/// body-derived fields — the title comes from the GitHub issue title, not the
/// body, so it is supplied by the handler, not this parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGoal {
    /// The verbatim trimmed `### Goal` block — the engine prompt (secret
    /// downstream; never echoed into a server-rendered marker or summary).
    pub description: String,
    /// One package name per non-empty `### Package Name List` line (grammar
    /// validation deferred to `validate_goal_fields`).
    pub package_names: Vec<String>,
    /// The env var KEY NAMES from the OPTIONAL `### Environment` section, one per
    /// non-blank line (PR4b). Each is a validated env-var name; the trigger
    /// resolves these against the issue author's store and injects the matching
    /// values into the session. Empty when the section is absent.
    pub env_keys: Vec<String>,
}

/// Parse the `fkst-goal` issue body into [`ParsedGoal`].
///
/// Returns [`AppError::Unprocessable`] (→ 422) naming the offending section for
/// every malformed case: a missing/empty `### Goal`; a missing/empty
/// `### Package Name List`; or a duplicate `### ` heading. The 422 status (not
/// 400) matches the issue's Definition-of-Done contract for template-format
/// failures.
pub fn parse_goal_issue_body(body: &str) -> Result<ParsedGoal, AppError> {
    let sections = split_sections(body)?;

    // `### Goal` — required, non-empty after trim.
    let description = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_GOAL)
        .map(|(_, content)| content.trim().to_string())
        .filter(|c| !c.is_empty())
        .ok_or_else(|| {
            AppError::Unprocessable(
                "the `### Goal` section is required and must not be empty".to_string(),
            )
        })?;

    // `### Package Name List` — required; at least one non-empty line.
    let package_block = sections
        .iter()
        .find(|(heading, _)| heading == HEADING_PACKAGES)
        .map(|(_, content)| content.as_str())
        .ok_or_else(|| {
            AppError::Unprocessable("the `### Package Name List` section is required".to_string())
        })?;
    let package_names: Vec<String> = non_empty_lines(package_block);
    if package_names.is_empty() {
        return Err(AppError::Unprocessable(
            "the `### Package Name List` section must list at least one package".to_string(),
        ));
    }

    // `### Environment` — OPTIONAL. Each non-blank line names one env var KEY to
    // inject (PR4b). A malformed name is a 422 naming the section (consistent
    // with the other structural failures); an absent section means no injection.
    let env_keys = match sections
        .iter()
        .find(|(heading, _)| heading == HEADING_ENVIRONMENT)
    {
        Some((_, content)) => parse_env_keys(content)?,
        None => Vec::new(),
    };

    Ok(ParsedGoal {
        description,
        package_names,
        env_keys,
    })
}

/// Parse the `### Environment` block into validated env var KEY names. Each
/// non-blank trimmed line must match the env-var-name grammar; the first
/// violation is a 422 that names the offending key and the section.
fn parse_env_keys(block: &str) -> Result<Vec<String>, AppError> {
    let keys = non_empty_lines(block);
    for key in &keys {
        if !env_key_regex().is_match(key) {
            return Err(AppError::Unprocessable(format!(
                "the `### Environment` section has an invalid env var name {key:?}: \
                 must match ^[A-Za-z_][A-Za-z0-9_]*$"
            )));
        }
    }
    Ok(keys)
}

/// Split a body into `(heading, content)` sections at each `### ` heading line.
/// Content is the raw text (not yet trimmed) between this heading and the next.
/// A duplicate `### ` heading is a 422 (the contract forbids ambiguity).
fn split_sections(body: &str) -> Result<Vec<(String, String)>, AppError> {
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, String)> = None;

    for line in body.lines() {
        if is_heading(line) {
            // A `### ` heading line opens a new section; flush the previous one.
            if let Some(section) = current.take() {
                sections.push(section);
            }
            let heading = line.trim_end().to_string();
            if sections.iter().any(|(h, _)| h == &heading)
                || current.as_ref().is_some_and(|(h, _)| h == &heading)
            {
                return Err(AppError::Unprocessable(format!(
                    "duplicate section heading: `{heading}`"
                )));
            }
            current = Some((heading, String::new()));
        } else if let Some((_, content)) = current.as_mut() {
            content.push_str(line);
            content.push('\n');
        }
        // Lines before the first heading (the form's intro markdown) are ignored.
    }
    if let Some(section) = current.take() {
        sections.push(section);
    }
    Ok(sections)
}

/// A line is a section heading when it starts with the literal `### ` marker.
/// (`####` and deeper are NOT section headings — only exactly-3 `#`.)
fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.starts_with("### ") && !trimmed.starts_with("#### ")
}

/// Split a block on newlines, trim each line, drop empties.
fn non_empty_lines(block: &str) -> Vec<String> {
    block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Goal + Package sections of the worked example from
    /// `docs/api-reference.md` MUST parse to the documented result — this is the
    /// canonical contract conformance test. Any extra (e.g. legacy `### Ornn
    /// Skill List`) section is ignored, not an error.
    #[test]
    fn worked_example_from_docs_parses_to_documented_result() {
        let body = "\
### Goal

Build a CLI that summarizes a git repo's commit history into a weekly digest.

### Package Name List

repo-reader
digest-writer
";
        let parsed = parse_goal_issue_body(body).expect("worked example parses");
        assert_eq!(
            parsed.description,
            "Build a CLI that summarizes a git repo's commit history into a weekly digest."
        );
        assert_eq!(parsed.package_names, vec!["repo-reader", "digest-writer"]);
    }

    #[test]
    fn environment_section_absent_yields_no_env_keys() {
        let body = "### Goal\nG\n### Package Name List\npkg-a\n";
        let parsed = parse_goal_issue_body(body).expect("parses without Environment");
        assert!(parsed.env_keys.is_empty());
    }

    #[test]
    fn environment_section_parses_one_key_per_non_blank_line() {
        let body =
            "### Goal\nG\n### Package Name List\npkg-a\n### Environment\n  FOO \n\nBAR_2\n_BAZ\n";
        let parsed = parse_goal_issue_body(body).expect("parses Environment");
        assert_eq!(parsed.env_keys, vec!["FOO", "BAR_2", "_BAZ"]);
    }

    #[test]
    fn environment_section_may_be_empty_and_yields_no_keys() {
        let body = "### Goal\nG\n### Package Name List\npkg-a\n### Environment\n   \n";
        let parsed = parse_goal_issue_body(body).expect("empty Environment is allowed");
        assert!(parsed.env_keys.is_empty());
    }

    #[test]
    fn malformed_env_key_name_is_422_naming_environment() {
        let body = "### Goal\nG\n### Package Name List\npkg-a\n### Environment\nMY-VAR\n";
        let msg = err_message(body);
        assert!(
            msg.contains("Environment"),
            "must name the Environment section: {msg}"
        );
        assert!(msg.contains("MY-VAR"), "must name the offending key: {msg}");
    }

    #[test]
    fn an_extra_unknown_section_is_ignored() {
        // A legacy `### Ornn Skill List` (or any unrecognized) section is no
        // longer parsed; it is simply ignored, never a parse error.
        let body = "### Goal\nDo a thing\n### Package Name List\npkg-a\n### Ornn Skill List\nskill:x@1.0\n";
        let parsed = parse_goal_issue_body(body).expect("parses, ignoring the extra section");
        assert_eq!(parsed.description, "Do a thing");
        assert_eq!(parsed.package_names, vec!["pkg-a"]);
    }

    #[test]
    fn multiline_goal_block_is_preserved_verbatim_and_trimmed() {
        let body = "### Goal\n\nLine one\nLine two\n\n### Package Name List\npkg-a\n";
        let parsed = parse_goal_issue_body(body).expect("multiline goal parses");
        assert_eq!(parsed.description, "Line one\nLine two");
    }

    #[test]
    fn intro_markdown_before_first_heading_is_ignored() {
        let body =
            "Some intro the Issue Form renders\n\n### Goal\nG\n### Package Name List\npkg-a\n";
        let parsed = parse_goal_issue_body(body).expect("intro ignored");
        assert_eq!(parsed.description, "G");
    }

    // ---- Malformed cases (each names the offending section in the 422) ----

    fn err_message(body: &str) -> String {
        match parse_goal_issue_body(body) {
            Err(AppError::Unprocessable(msg)) => msg,
            other => panic!("expected Unprocessable (422), got {other:?}"),
        }
    }

    #[test]
    fn missing_goal_section_is_422_naming_goal() {
        let msg = err_message("### Package Name List\npkg-a\n");
        assert!(msg.contains("Goal"), "must name the Goal section: {msg}");
    }

    #[test]
    fn empty_goal_section_is_422_naming_goal() {
        let msg = err_message("### Goal\n   \n### Package Name List\npkg-a\n");
        assert!(msg.contains("Goal"), "must name the Goal section: {msg}");
    }

    #[test]
    fn missing_package_section_is_422_naming_packages() {
        let msg = err_message("### Goal\nG\n");
        assert!(
            msg.contains("Package Name List"),
            "must name the package section: {msg}"
        );
    }

    #[test]
    fn empty_package_section_is_422_naming_packages() {
        let msg = err_message("### Goal\nG\n### Package Name List\n\n");
        assert!(
            msg.contains("Package Name List"),
            "must name the package section: {msg}"
        );
    }

    #[test]
    fn duplicate_heading_is_422() {
        let msg = err_message("### Goal\nG\n### Goal\nH\n### Package Name List\np\n");
        assert!(msg.contains("duplicate"), "must flag the duplicate: {msg}");
    }

    #[test]
    fn deeper_heading_is_not_a_section_boundary() {
        // A `#### ` inside the Goal block is body text, not a new section.
        let body = "### Goal\nG\n#### sub\nmore\n### Package Name List\np\n";
        let parsed = parse_goal_issue_body(body).expect("#### is not a boundary");
        assert_eq!(parsed.description, "G\n#### sub\nmore");
    }
}
