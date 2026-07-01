//! Shared `### `-heading section-splitting helpers for GitHub-issue bodies.
//!
//! Both the `fkst-goal` parser ([`crate::goals::issue_parse`]) and the
//! `fkst-substrate-trigger` parser ([`crate::goals::trigger_parse`]) turn a
//! user-authored issue **body** into structured fields by splitting it at `### `
//! headings and reading each section's non-empty lines. This module is the single
//! home of that reusable machinery so neither parser depends on the other, and the
//! section-splitting contract is defined once: a duplicate `### ` heading is a
//! 422, intro text before the first heading is ignored, and `#### ` (or deeper) is
//! body text — never a section boundary.
//!
//! Scope boundary: these helpers only *structure* text and enforce the shared
//! structural rules (section splitting + the reusable environment-name rule). Each
//! caller layers its own section-specific validation on top.
//!
//! Secret hygiene: these helpers log nothing (a `### Goal`/`### Session Name`
//! block can become a secret engine prompt downstream), and never echo content.

use std::sync::OnceLock;

use regex::Regex;

use crate::error::AppError;

/// The maximum length of an environment `name`, mirroring the named-environment
/// API (`crate::routes::environments`) so a name that parses here is also
/// accepted by that API and composes into a valid Kubernetes object name.
pub(crate) const MAX_ENV_NAME_LEN: usize = 40;

/// Anchored environment-NAME pattern, identical to the rule the named-environment
/// API (`crate::routes::environments`) enforces: lower-case DNS-1123-label-ish,
/// so the selected name resolves to the same `fkst-env-<id>-<name>` object.
pub(crate) fn env_name_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[a-z0-9]([a-z0-9-]*[a-z0-9])?$").expect("static env name regex"))
}

/// True when `name` satisfies the environments API naming rule (grammar + the
/// 1..=40 length budget). The regex already forbids the empty string.
pub(crate) fn is_valid_env_name(name: &str) -> bool {
    name.len() <= MAX_ENV_NAME_LEN && env_name_regex().is_match(name)
}

/// Parse the OPTIONAL `### Environment` block into the single environment NAME it
/// selects. The section, when present, must contain EXACTLY ONE non-blank line —
/// the name of a named environment the issue author owns — that satisfies the
/// environments API naming rule. Zero non-blank lines yields `None` (equivalent to
/// omitting the section); two or more is a 422 (ambiguous selection); an invalid
/// name is a 422 that names the rule.
pub(crate) fn parse_environment_name(block: &str) -> Result<Option<String>, AppError> {
    let names = non_empty_lines(block);
    match names.as_slice() {
        [] => Ok(None),
        [name] => {
            if is_valid_env_name(name) {
                Ok(Some(name.clone()))
            } else {
                Err(AppError::Unprocessable(format!(
                    "the `### Environment` section names an invalid environment {name:?}: \
                     must match ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ and be 1..=40 characters"
                )))
            }
        }
        _ => Err(AppError::Unprocessable(
            "the `### Environment` section must name exactly one environment".to_string(),
        )),
    }
}

/// Split a body into `(heading, content)` sections at each `### ` heading line.
/// Content is the raw text (not yet trimmed) between this heading and the next.
/// A duplicate `### ` heading is a 422 (the contract forbids ambiguity).
pub(crate) fn split_sections(body: &str) -> Result<Vec<(String, String)>, AppError> {
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
pub(crate) fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.starts_with("### ") && !trimmed.starts_with("#### ")
}

/// Split a block on newlines, trim each line, drop empties.
pub(crate) fn non_empty_lines(block: &str) -> Vec<String> {
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

    fn err_message(result: Result<Vec<(String, String)>, AppError>) -> String {
        match result {
            Err(AppError::Unprocessable(msg)) => msg,
            other => panic!("expected Unprocessable (422), got {other:?}"),
        }
    }

    // ---- split_sections ----

    #[test]
    fn split_sections_keeps_content_between_headings() {
        let body = "### A\nline one\nline two\n### B\nother\n";
        let sections = split_sections(body).expect("splits");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "### A");
        assert_eq!(sections[0].1, "line one\nline two\n");
        assert_eq!(sections[1].0, "### B");
        assert_eq!(sections[1].1, "other\n");
    }

    #[test]
    fn split_sections_ignores_intro_before_first_heading() {
        let body = "intro line the form renders\n\n### A\nbody\n";
        let sections = split_sections(body).expect("splits");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "### A");
        assert_eq!(sections[0].1, "body\n");
    }

    #[test]
    fn split_sections_treats_deeper_heading_as_body_text() {
        // `#### ` inside a section is content, not a new boundary.
        let body = "### A\nbody\n#### sub\nmore\n";
        let sections = split_sections(body).expect("splits");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].1, "body\n#### sub\nmore\n");
    }

    #[test]
    fn split_sections_rejects_a_duplicate_heading() {
        let msg = err_message(split_sections("### A\nx\n### A\ny\n"));
        assert!(msg.contains("duplicate"), "must flag the duplicate: {msg}");
        assert!(msg.contains("### A"), "must name the heading: {msg}");
    }

    // ---- is_heading ----

    #[test]
    fn is_heading_recognizes_exactly_three_hashes_and_a_space() {
        assert!(is_heading("### Goal"));
        assert!(is_heading("### Goal   "), "trailing whitespace tolerated");
        assert!(
            !is_heading("#### sub"),
            "four hashes is not a section heading"
        );
        assert!(!is_heading("###no-space"), "no space is not a heading");
        assert!(!is_heading("## two"), "two hashes is not a heading");
        assert!(!is_heading("plain text"));
    }

    // ---- non_empty_lines ----

    #[test]
    fn non_empty_lines_trims_and_drops_blanks() {
        let block = "  first  \n\n   \n second\n";
        assert_eq!(non_empty_lines(block), vec!["first", "second"]);
    }

    #[test]
    fn non_empty_lines_on_blank_block_is_empty() {
        assert!(non_empty_lines("   \n\n\t\n").is_empty());
    }

    // ---- env_name_regex / is_valid_env_name ----

    #[test]
    fn is_valid_env_name_accepts_dns_label_names() {
        assert!(is_valid_env_name("prod"));
        assert!(is_valid_env_name("my-env-1"));
        assert!(is_valid_env_name("a"));
    }

    #[test]
    fn is_valid_env_name_rejects_bad_grammar_and_overlong_names() {
        assert!(!is_valid_env_name(""), "empty is rejected");
        assert!(!is_valid_env_name("MY_ENV"), "upper-case is rejected");
        assert!(!is_valid_env_name("-lead"), "leading dash is rejected");
        assert!(!is_valid_env_name("trail-"), "trailing dash is rejected");
        // Exactly the length budget passes; one over it fails.
        let max = "a".repeat(MAX_ENV_NAME_LEN);
        assert!(
            is_valid_env_name(&max),
            "{MAX_ENV_NAME_LEN} chars is allowed"
        );
        let over = "a".repeat(MAX_ENV_NAME_LEN + 1);
        assert!(!is_valid_env_name(&over), "one over the budget is rejected");
    }

    // ---- parse_environment_name ----

    #[test]
    fn parse_environment_name_absent_lines_yield_none() {
        assert_eq!(
            parse_environment_name("   \n\n").expect("blank is none"),
            None
        );
    }

    #[test]
    fn parse_environment_name_single_valid_name() {
        assert_eq!(
            parse_environment_name("  my-env  \n").expect("valid"),
            Some("my-env".to_string())
        );
    }

    #[test]
    fn parse_environment_name_invalid_name_is_422_naming_the_rule() {
        let msg = match parse_environment_name("MY_ENV\n") {
            Err(AppError::Unprocessable(msg)) => msg,
            other => panic!("expected 422, got {other:?}"),
        };
        assert!(msg.contains("Environment"), "names the section: {msg}");
        assert!(msg.contains("MY_ENV"), "names the offending value: {msg}");
        assert!(msg.contains("a-z0-9"), "states the naming rule: {msg}");
    }

    #[test]
    fn parse_environment_name_two_names_is_422() {
        let msg = match parse_environment_name("first\nsecond\n") {
            Err(AppError::Unprocessable(msg)) => msg,
            other => panic!("expected 422, got {other:?}"),
        };
        assert!(msg.contains("Environment"), "names the section: {msg}");
        assert!(msg.contains("exactly one"), "flags the ambiguity: {msg}");
    }
}
