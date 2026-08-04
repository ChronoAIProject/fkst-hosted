//! Proves a named test actually exists in the file the matrix says owns it.
//!
//! Without this pass the matrix is a list of aspirations: a renamed Rust test, a
//! reworded Playwright title, or a deleted shell case would leave the claim
//! standing while the evidence quietly vanished.
//!
//! The check is language-aware for EVERY extension it accepts, and refuses the
//! ones it does not know. An earlier version fell back to a bare substring scan
//! for anything that was not Rust or TypeScript, which meant a matrix row could
//! claim evidence satisfied by a mention inside a comment — the exact decay this
//! pass exists to catch. Each form below is the one shape that cannot be written
//! accidentally: a function definition, a quoted test title, a constant
//! assignment, a YAML key.

use std::path::Path;

/// Why a claimed test could not be found.
#[derive(Debug, PartialEq, Eq)]
pub enum Missing {
    /// The suite file itself is absent.
    Suite,
    /// The suite exists but does not define the named test.
    Test,
    /// The suite's file type has no defined "this is a test definition" form, so
    /// no honest existence check is possible for it.
    UnknownSuiteKind,
}

/// Look for `test` inside `suite`, relative to the repository root.
pub fn find(repo_root: &Path, suite: &str, test: &str) -> Result<(), Missing> {
    let path = repo_root.join(suite);
    let Ok(source) = std::fs::read_to_string(&path) else {
        return Err(Missing::Suite);
    };
    let found = match extension(suite) {
        // A Rust test is a function definition; nothing else counts.
        "rs" => source.contains(&format!("fn {test}(")),
        // Playwright/Vitest titles are string literals. Both quote styles are in
        // use across the frontend suites, and a title may itself contain an
        // apostrophe, so match on the delimiter pair rather than the bare name.
        // A shell case is declared the same way — `expect_success "…"`.
        "ts" | "tsx" | "sh" => {
            source.contains(&format!("'{test}'"))
                || source.contains(&format!("\"{test}\""))
                || source.contains(&format!("`{test}`"))
        }
        // A Ruby policy list is a CONSTANT ASSIGNMENT. Its later uses read the
        // same name, and a comment could mention it, so the definition site is
        // the only form that proves the policy still exists.
        "rb" => {
            source.contains(&format!("{test} ="))
                || source.contains(&format!("def {test}("))
                || source.contains(&format!("def {test}\n"))
        }
        // A YAML fixture case is a mapping key: `FKSTAuditDeadLetters:` at some
        // indentation. A bare mention in the file's prose header does not match.
        "yaml" | "yml" => source
            .lines()
            .any(|line| line.trim_start().starts_with(&format!("{test}:"))),
        // Anything else has no defined definition form here. Refusing is the
        // honest answer: a substring scan would accept a comment.
        _ => return Err(Missing::UnknownSuiteKind),
    };
    if found {
        Ok(())
    } else {
        Err(Missing::Test)
    }
}

fn extension(suite: &str) -> &str {
    Path::new(suite)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        crate::acceptance::repo_root()
    }

    #[test]
    fn a_real_rust_test_is_found_and_a_renamed_one_is_not() {
        assert_eq!(
            find(
                &root(),
                "backend/tests/audit_outcomes.rs",
                "a_route_scoped_timeout_is_recorded_as_a_timeout"
            ),
            Ok(())
        );
        assert_eq!(
            find(
                &root(),
                "backend/tests/audit_outcomes.rs",
                "a_route_scoped_timeout_was_renamed"
            ),
            Err(Missing::Test)
        );
    }

    #[test]
    fn an_absent_suite_is_distinguished_from_an_absent_test() {
        assert_eq!(
            find(&root(), "backend/tests/no_such_suite.rs", "anything"),
            Err(Missing::Suite)
        );
    }

    /// A Ruby policy list is found at its assignment, and a mere mention of the
    /// name — the form a comment takes — is not enough.
    #[test]
    fn a_ruby_constant_is_matched_at_its_assignment_only() {
        assert_eq!(
            find(
                &root(),
                "deploy/kubernetes/validate-audit-relay.rb",
                "SECRET_VARS"
            ),
            Ok(())
        );
        // `secretKeyRef` is named in that file's header comment and is not a
        // policy list; a mention must not satisfy the check.
        assert_eq!(
            find(
                &root(),
                "deploy/kubernetes/validate-audit-relay.rb",
                "secretKeyRef"
            ),
            Err(Missing::Test)
        );
    }

    /// A YAML fixture case is found at its key, not in the file's prose header.
    #[test]
    fn a_yaml_fixture_case_is_matched_at_its_key_only() {
        assert_eq!(
            find(
                &root(),
                "deploy/kubernetes/monitoring/alert-fixtures.yaml",
                "FKSTAuditDeadLetters"
            ),
            Ok(())
        );
        // `promtool` is named in that file's header comment and is not a case.
        assert_eq!(
            find(
                &root(),
                "deploy/kubernetes/monitoring/alert-fixtures.yaml",
                "promtool"
            ),
            Err(Missing::Test)
        );
    }

    /// A shell case is found at its quoted name.
    #[test]
    fn a_shell_case_is_matched_by_its_quoted_name() {
        assert_eq!(
            find(
                &root(),
                "deploy/kubernetes/tests/audit-relay-verify-test.sh",
                "an unbound audit volume is refused"
            ),
            Ok(())
        );
        assert_eq!(
            find(
                &root(),
                "deploy/kubernetes/tests/audit-relay-verify-test.sh",
                "a case nobody ever wrote"
            ),
            Err(Missing::Test)
        );
    }

    /// A file type with no defined definition form is refused outright, rather
    /// than degrading to a substring scan a comment could satisfy.
    #[test]
    fn an_unknown_suite_kind_is_refused_rather_than_guessed() {
        assert_eq!(
            find(&root(), "CLAUDE.md", "Quick Rules Summary"),
            Err(Missing::UnknownSuiteKind)
        );
    }

    #[test]
    fn a_frontend_title_is_matched_by_its_quoted_literal() {
        assert_eq!(
            find(
                &root(),
                "frontend/src/pages/operations.security.test.tsx",
                "refreshes sandboxes every 5 seconds"
            ),
            Ok(())
        );
        assert_eq!(
            find(
                &root(),
                "frontend/src/pages/operations.security.test.tsx",
                "refreshes sandboxes every 6 seconds"
            ),
            Err(Missing::Test)
        );
    }
}
