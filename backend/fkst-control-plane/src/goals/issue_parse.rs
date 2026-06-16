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
//! package-name grammar (deferred to [`crate::goals::validate_goal_fields`]) or
//! Ornn pin grammar (deferred to [`crate::ornn::validate_pins`]); the submit
//! handler runs those after parsing so one validation path is shared with the
//! inline source. The parser DOES enforce the structural contract: the three
//! required/optional sections exist, the Goal and Package list are non-empty,
//! no `### ` heading is duplicated, and every Ornn line matches the pin
//! grammar (so a malformed pin line is a parse-time 422 naming the section).
//!
//! Secret hygiene: the `Goal` section becomes the engine prompt (secret
//! downstream). Never log the parsed `description`; this module logs nothing.

use crate::error::AppError;
use crate::ornn::{OrnnPinKind, OrnnSkillPin};

/// The three canonical section headings, in template order. A section's content
/// is every line after its heading up to the next `### ` heading (or EOF).
const HEADING_GOAL: &str = "### Goal";
const HEADING_PACKAGES: &str = "### Package Name List";
const HEADING_SKILLS: &str = "### Ornn Skill List";

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
    /// One pin per non-empty `### Ornn Skill List` line. Each line is structurally
    /// matched here against the pin grammar; cross-pin conflicts and byte limits
    /// are deferred to `validate_pins`. Empty/absent section => no pins.
    pub ornn_skills: Vec<OrnnSkillPin>,
}

/// Parse the `fkst-goal` issue body into [`ParsedGoal`].
///
/// Returns [`AppError::Unprocessable`] (→ 422) naming the offending section for
/// every malformed case: a missing/empty `### Goal`; a missing/empty
/// `### Package Name List`; an `### Ornn Skill List` line that does not match
/// the pin grammar; or a duplicate `### ` heading. The 422 status (not 400)
/// matches the issue's Definition-of-Done contract for template-format failures.
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

    // `### Ornn Skill List` — optional; each non-empty line must match the pin
    // grammar. An absent section means zero pins.
    let ornn_skills = match sections
        .iter()
        .find(|(heading, _)| heading == HEADING_SKILLS)
        .map(|(_, content)| content.as_str())
    {
        Some(block) => non_empty_lines(block)
            .into_iter()
            .map(|line| parse_pin_line(&line))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };

    Ok(ParsedGoal {
        description,
        package_names,
        ornn_skills,
    })
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

/// Parse one `### Ornn Skill List` line into a pin: `kind:name@major.minor`,
/// matching `^(skill|skillset):([a-z0-9][a-z0-9-]*)@(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$`.
/// A non-matching line is a 422 naming the section.
fn parse_pin_line(line: &str) -> Result<OrnnSkillPin, AppError> {
    let malformed = || {
        AppError::Unprocessable(format!(
            "the `### Ornn Skill List` section has a malformed pin line: `{line}` \
             (expected `skill:name@major.minor` or `skillset:name@major.minor`)"
        ))
    };

    let (kind_str, rest) = line.split_once(':').ok_or_else(malformed)?;
    let kind = match kind_str {
        "skill" => OrnnPinKind::Skill,
        "skillset" => OrnnPinKind::Skillset,
        _ => return Err(malformed()),
    };

    let (name, version) = rest.split_once('@').ok_or_else(malformed)?;
    if !is_valid_name(name) || !is_valid_version(version) {
        return Err(malformed());
    }

    Ok(OrnnSkillPin {
        kind,
        name: name.to_string(),
        version: version.to_string(),
    })
}

/// `^[a-z0-9][a-z0-9-]*$` — a fixed ASCII allow-list (cheaper than a regex and
/// matches the same grammar `validate_pins` re-checks for byte length).
fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$` — exactly two non-negative integers
/// (no leading zeros, `0` itself allowed) joined by a single dot.
fn is_valid_version(version: &str) -> bool {
    let mut parts = version.split('.');
    let (Some(major), Some(minor), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    is_valid_version_component(major) && is_valid_version_component(minor)
}

fn is_valid_version_component(component: &str) -> bool {
    match component {
        "" => false,
        "0" => true,
        other => other.bytes().all(|b| b.is_ascii_digit()) && !other.starts_with('0'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact worked example from `docs/api-reference.md` MUST parse to the
    /// documented result — this is the canonical contract conformance test.
    #[test]
    fn worked_example_from_docs_parses_to_documented_result() {
        let body = "\
### Goal

Build a CLI that summarizes a git repo's commit history into a weekly digest.

### Package Name List

repo-reader
digest-writer

### Ornn Skill List

skill:git-log@1.0
skillset:report-templates@2.3
";
        let parsed = parse_goal_issue_body(body).expect("worked example parses");
        assert_eq!(
            parsed.description,
            "Build a CLI that summarizes a git repo's commit history into a weekly digest."
        );
        assert_eq!(parsed.package_names, vec!["repo-reader", "digest-writer"]);
        assert_eq!(
            parsed.ornn_skills,
            vec![
                OrnnSkillPin {
                    kind: OrnnPinKind::Skill,
                    name: "git-log".to_string(),
                    version: "1.0".to_string(),
                },
                OrnnSkillPin {
                    kind: OrnnPinKind::Skillset,
                    name: "report-templates".to_string(),
                    version: "2.3".to_string(),
                },
            ]
        );
    }

    #[test]
    fn absent_ornn_section_means_no_pins() {
        let body = "### Goal\nDo a thing\n### Package Name List\npkg-a\n";
        let parsed = parse_goal_issue_body(body).expect("parses without an Ornn section");
        assert!(parsed.ornn_skills.is_empty());
        assert_eq!(parsed.package_names, vec!["pkg-a"]);
    }

    #[test]
    fn empty_ornn_section_means_no_pins() {
        let body = "### Goal\nDo a thing\n### Package Name List\npkg-a\n### Ornn Skill List\n\n";
        let parsed = parse_goal_issue_body(body).expect("parses with an empty Ornn section");
        assert!(parsed.ornn_skills.is_empty());
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
    fn malformed_pin_line_is_422_naming_ornn() {
        let msg = err_message(
            "### Goal\nG\n### Package Name List\npkg-a\n### Ornn Skill List\nnot-a-pin\n",
        );
        assert!(
            msg.contains("Ornn Skill List"),
            "must name the Ornn section: {msg}"
        );
    }

    #[test]
    fn pin_with_bad_kind_is_422() {
        let msg =
            err_message("### Goal\nG\n### Package Name List\np\n### Ornn Skill List\ntool:x@1.0\n");
        assert!(msg.contains("Ornn Skill List"), "{msg}");
    }

    #[test]
    fn pin_with_three_version_parts_is_422() {
        let msg = err_message(
            "### Goal\nG\n### Package Name List\np\n### Ornn Skill List\nskill:x@1.0.0\n",
        );
        assert!(msg.contains("Ornn Skill List"), "{msg}");
    }

    #[test]
    fn pin_with_leading_zero_version_is_422() {
        let msg = err_message(
            "### Goal\nG\n### Package Name List\np\n### Ornn Skill List\nskill:x@01.0\n",
        );
        assert!(msg.contains("Ornn Skill List"), "{msg}");
    }

    #[test]
    fn duplicate_heading_is_422() {
        let msg = err_message("### Goal\nG\n### Goal\nH\n### Package Name List\np\n");
        assert!(msg.contains("duplicate"), "must flag the duplicate: {msg}");
    }

    #[test]
    fn pin_version_zero_components_are_allowed() {
        let body = "### Goal\nG\n### Package Name List\np\n### Ornn Skill List\nskill:x@0.0\n";
        let parsed = parse_goal_issue_body(body).expect("0.0 is a valid version");
        assert_eq!(parsed.ornn_skills[0].version, "0.0");
    }

    #[test]
    fn deeper_heading_is_not_a_section_boundary() {
        // A `#### ` inside the Goal block is body text, not a new section.
        let body = "### Goal\nG\n#### sub\nmore\n### Package Name List\np\n";
        let parsed = parse_goal_issue_body(body).expect("#### is not a boundary");
        assert_eq!(parsed.description, "G\n#### sub\nmore");
    }
}
